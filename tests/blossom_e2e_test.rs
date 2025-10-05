use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

use blitz_dom::net::Resource;
use blitz_net::Provider;
use blitz_traits::net::DummyNetCallback;
use frontier::blossom::BlossomFetcher;
use frontier::navigation::{execute_fetch, prepare_navigation, NavigationPlan};
use frontier::net::{NostrClient, RelayDirectory};
use frontier::nns::{ClaimLocation, NnsResolver};
use frontier::storage::Storage;

struct BlossomHttpServer {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_blossom_http_server(blobs: HashMap<String, Vec<u8>>) -> BlossomHttpServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let blobs = Arc::new(blobs);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    if let Ok((mut stream, _)) = accept {
                        let blobs = Arc::clone(&blobs);
                        tokio::spawn(async move {
                            use tokio::io::{AsyncReadExt, AsyncWriteExt};
                            let mut buffer = vec![0u8; 4096];
                            match stream.read(&mut buffer).await {
                                Ok(0) => (),
                                Ok(n) => {
                                    let request = String::from_utf8_lossy(&buffer[..n]);
                                    let mut lines = request.lines();
                                    let first_line = lines.next().unwrap_or("GET /");
                                    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
                                    let key = path.trim_start_matches('/');
                                    let (status_line, body) = if let Some(content) = blobs.get(key) {
                                        ("HTTP/1.1 200 OK\r\n", content.clone())
                                    } else {
                                        ("HTTP/1.1 404 Not Found\r\n", Vec::new())
                                    };
                                    let header = format!(
                                        "{status}Content-Length: {}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n",
                                        body.len(),
                                        status = status_line
                                    );
                                    let _ = stream.write_all(header.as_bytes()).await;
                                    if !body.is_empty() {
                                        let _ = stream.write_all(&body).await;
                                    }
                                    let _ = stream.flush().await;
                                }
                                Err(_) => (),
                            }
                        });
                    }
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    BlossomHttpServer {
        addr,
        shutdown: shutdown_tx,
        handle,
    }
}

struct RelayServer {
    url: url::Url,
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_blossom_relay(claim: Event, manifests: Vec<Event>) -> RelayServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let claim = Arc::new(claim);
    let manifests = Arc::new(manifests);

    let handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                Ok((stream, _)) = listener.accept() => {
                    let claim = Arc::clone(&claim);
                    let manifests = Arc::clone(&manifests);
                    tokio::spawn(async move {
                        if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                            while let Some(msg) = ws.next().await {
                                match msg {
                                    Ok(Message::Text(text)) => {
                                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                            if value.get(0) == Some(&Value::String("REQ".into())) {
                                                if let Some(sub_id) = value.get(1).and_then(|v| v.as_str()) {
                                                    let mut kinds = Vec::new();
                                                    for filter in value.as_array().into_iter().flatten().skip(2) {
                                                        if let Some(array) = filter.get("kinds").and_then(|v| v.as_array()) {
                                                            for kind_value in array {
                                                                if let Some(kind) = kind_value.as_u64() {
                                                                    kinds.push(kind as u16);
                                                                }
                                                            }
                                                        }
                                                    }
                                                    if kinds.contains(&34256) {
                                                        let event_msg = json!(["EVENT", sub_id, serde_json::to_value(&*claim).unwrap()]);
                                                        let _ = ws.send(Message::Text(event_msg.to_string())).await;
                                                    }
                                                    if kinds.contains(&34128) {
                                                        for event in manifests.iter() {
                                                            let event_msg = json!(["EVENT", sub_id, serde_json::to_value(event).unwrap()]);
                                                            let _ = ws.send(Message::Text(event_msg.to_string())).await;
                                                        }
                                                    }
                                                    let eose_msg = json!(["EOSE", sub_id]);
                                                    let _ = ws.send(Message::Text(eose_msg.to_string())).await;
                                                }
                                            }
                                        }
                                    }
                                    Ok(Message::Ping(data)) => {
                                        let _ = ws.send(Message::Pong(data)).await;
                                    }
                                    Ok(Message::Close(_)) => break,
                                    _ => {}
                                }
                            }
                        }
                    });
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    RelayServer {
        url: url::Url::parse(&format!("ws://{}", addr)).unwrap(),
        shutdown: shutdown_tx,
        handle,
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn blossom_manifest_event(keys: &Keys, path: &str, hash: &str) -> Event {
    let identifier_tag = Tag::identifier(path);
    let hash_tag = Tag::parse(&["sha256", hash]).unwrap();
    EventBuilder::new(Kind::from(34128u16), "", vec![identifier_tag, hash_tag])
        .custom_created_at(Timestamp::now())
        .to_event(keys)
        .unwrap()
}

#[tokio::test]
async fn test_blossom_end_to_end() {
    let home_html =
        "<html><body><h1>Blossom Home</h1><a href=\"/about.html\">About</a></body></html>";
    let about_html =
        "<html><body><h1>About Blossom</h1><a href=\"/index.html\">Home</a></body></html>";

    let home_hash = sha256_hex(home_html.as_bytes());
    let about_hash = sha256_hex(about_html.as_bytes());

    let mut blobs = HashMap::new();
    blobs.insert(home_hash.clone(), home_html.as_bytes().to_vec());
    blobs.insert(about_hash.clone(), about_html.as_bytes().to_vec());

    let http_server = start_blossom_http_server(blobs).await;
    println!("✓ Blossom HTTP server on {}", http_server.addr);

    let server_url = format!("http://{}/", http_server.addr);

    let keys = Keys::generate();
    let name = "blossomsite";

    let claim_event = EventBuilder::new(
        Kind::from(34256u16),
        "",
        vec![
            Tag::identifier(name),
            Tag::parse(&["blossom", home_hash.as_str()]).unwrap(),
            Tag::parse(&["server", &server_url]).unwrap(),
        ],
    )
    .custom_created_at(Timestamp::now())
    .to_event(&keys)
    .unwrap();

    let manifest_events = vec![
        blossom_manifest_event(&keys, "/home.html", &home_hash),
        blossom_manifest_event(&keys, "/about.html", &about_hash),
        blossom_manifest_event(&keys, "/broken.html", "not-a-hex-hash"),
    ];

    let relay_server = start_blossom_relay(claim_event.clone(), manifest_events.clone()).await;
    println!("✓ Mock relay on {}", relay_server.url);

    let temp_dir = TempDir::new().expect("temp dir");
    std::env::set_var("FRONTIER_DATA_DIR", temp_dir.path());

    let storage = Arc::new(Storage::new().expect("storage"));

    let relay_config_path = temp_dir.path().join("relays.yaml");
    std::fs::write(
        &relay_config_path,
        format!("relays:\n  - {}\n", relay_server.url),
    )
    .unwrap();

    let relay_directory = RelayDirectory::load(Some(relay_config_path)).unwrap();

    let resolver = Arc::new(NnsResolver::new(
        Arc::clone(&storage),
        relay_directory.clone(),
        NostrClient::new(),
    ));
    let blossom_fetcher = Arc::new(BlossomFetcher::new(relay_directory.clone()).expect("blossom"));

    let output = resolver
        .resolve(name)
        .await
        .expect("failed to resolve blossom site");

    let claim = output.claims.primary.clone();
    let relays: Vec<url::Url> = claim.relays.iter().cloned().collect();
    let (root_hash, servers) = match &claim.location {
        ClaimLocation::Blossom { root_hash, servers } => (root_hash.clone(), servers.clone()),
        ClaimLocation::DirectIp(_) => panic!("expected blossom claim"),
    };

    assert_eq!(root_hash, home_hash, "root hash should match home hash");
    println!("Servers discovered: {:?}", servers);
    assert!(servers.iter().any(|url| url.as_str() == server_url));

    let (bytes, entry) = blossom_fetcher
        .fetch_document(&claim.pubkey_hex, &relays, &servers, "/home.html")
        .await
        .expect("fetch index");
    assert_eq!(bytes, home_html.as_bytes());
    assert_eq!(entry.hash, home_hash);

    let (about_bytes, about_entry) = blossom_fetcher
        .fetch_document(&claim.pubkey_hex, &relays, &servers, "/about.html")
        .await
        .expect("fetch about");
    assert_eq!(about_bytes, about_html.as_bytes());
    assert_eq!(about_entry.hash, about_hash);

    // Simulate navigation through the browser pipeline for a direct path request.
    let plan = prepare_navigation("blossomsite/about.html", Arc::clone(&resolver))
        .await
        .expect("prepare navigation for about");
    let fetch_request = match plan {
        NavigationPlan::Fetch(request) => request,
        NavigationPlan::RequiresSelection(_) => {
            panic!("blossomsite/about.html should not require selection");
        }
    };
    let dummy_callback: Arc<DummyNetCallback> = Arc::new(DummyNetCallback);
    let net_provider: Arc<Provider<Resource>> = Arc::new(Provider::new(dummy_callback));
    let document = execute_fetch(
        &fetch_request,
        Arc::clone(&net_provider),
        Arc::clone(&blossom_fetcher),
    )
    .await
    .expect("execute fetch for about.html");
    assert!(
        document.contents.contains("<h1>About Blossom</h1>"),
        "navigate to about.html should load about page"
    );
    assert_eq!(document.display_url, "blossomsite/about.html");

    // Shut down HTTP server to verify cache usage
    http_server.shutdown.send(()).ok();
    let _ = http_server.handle.await;

    let (cached_bytes, cached_entry) = blossom_fetcher
        .fetch_document(&claim.pubkey_hex, &relays, &servers, "/home.html")
        .await
        .expect("fetch index from cache");
    assert_eq!(cached_bytes, home_html.as_bytes());
    assert_eq!(cached_entry.hash, home_hash);
    println!("✓ Blossom fetch cached content after server shutdown");

    // Root hash lookup should return the correct entry even with invalid manifests mixed in
    let manifest = blossom_fetcher
        .manifest_for(&claim.pubkey_hex, &relays)
        .await
        .expect("manifest");
    let root_entry = manifest
        .find_by_hash(&root_hash)
        .expect("root hash entry missing");
    assert_eq!(root_entry.path, "/home.html");
    assert!(manifest.get("/broken.html").is_none());

    let _ = relay_server.shutdown.send(());
    let _ = relay_server.handle.await;
    std::env::remove_var("FRONTIER_DATA_DIR");
}
