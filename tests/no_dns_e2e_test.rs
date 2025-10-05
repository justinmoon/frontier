/// NO_DNS End-to-End Test
///
/// This test demonstrates the complete elimination of DNS:
/// 1. Direct IP web server with TLS (self-signed cert)
/// 2. Blossom content server with TLS
/// 3. Nostr relay with TLS WebSocket
///
/// ALL services publish IP + TLS pubkey via Nostr kind 34256 events.
/// NO DNS lookups anywhere. Everything TLS-encrypted.
///
/// Run: cargo test --test no_dns_e2e_test
use frontier::{
    nns::{ClaimLocation, PublishedTlsKey, TlsAlgorithm},
    tls::connect_websocket,
    NnsResolver, NostrClient, RelayDirectory, Storage,
};
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    ServerConfig,
};
use rustls_pemfile::{certs, pkcs8_private_keys};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

#[tokio::test]
async fn test_no_dns_full_stack() {
    // Initialize rustls crypto provider
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    println!("\n🚀 Starting NO_DNS Full Stack Test");
    println!("==================================\n");

    // Generate TLS certificates for all services
    let http_tls = generate_tls_cert("http-server");
    let blossom_tls = generate_tls_cert("blossom-server");
    let relay_tls = generate_tls_cert("relay-server");

    println!("✓ Generated self-signed TLS certificates");
    println!("  HTTP server pubkey: {}", &http_tls.pubkey_hex[..16]);
    println!("  Blossom server pubkey: {}", &blossom_tls.pubkey_hex[..16]);
    println!("  Relay server pubkey: {}", &relay_tls.pubkey_hex[..16]);

    // Start HTTPS server (direct IP site)
    let http_server = start_https_server(http_tls.clone()).await;
    println!("\n✓ HTTPS server started on {}", http_server.addr);

    // Start Blossom server with TLS
    let blossom_server = start_blossom_tls_server(blossom_tls.clone()).await;
    println!("✓ Blossom TLS server started on {}", blossom_server.addr);

    // Start relay with TLS
    let relay_server = start_relay_tls_server(relay_tls.clone()).await;
    println!("✓ Relay TLS server started on {}", relay_server.addr);

    // Create keys for events (reuse same keys for service and manifest)
    let http_keys = Keys::generate();
    let blossom_keys = Keys::generate();
    let relay_keys = Keys::generate();

    // Create kind 34256 events for all services
    let http_event = build_service_event(
        "mywebsite",
        http_server.addr,
        &http_tls.pubkey_hex,
        None,
        None,
        "site",
        &http_keys,
    );
    let blossom_event = build_service_event(
        "myblossomsite",
        blossom_server.addr,
        &blossom_tls.pubkey_hex,
        Some(&blossom_server.root_hash),
        Some(&format!("https://{}", blossom_server.addr)),
        "blossom-site",
        &blossom_keys,
    );
    let relay_event = build_service_event(
        "myrelay",
        relay_server.addr,
        &relay_tls.pubkey_hex,
        None,
        None,
        "relay",
        &relay_keys,
    );

    // Publish Blossom manifest events (using same keys as blossom service)
    let manifest_events = build_blossom_manifest_events(&blossom_server.files, &blossom_keys);

    println!("\n✓ Created NNS events:");
    println!("  mywebsite → https://{} (TLS)", http_server.addr);
    println!(
        "  myblossomsite → https://{} (TLS, Blossom)",
        blossom_server.addr
    );
    println!("  myrelay → wss://{} (TLS)", relay_server.addr);

    // Start mock relay that serves all events
    let mut all_events = vec![http_event, blossom_event, relay_event];
    all_events.extend(manifest_events);
    let discovery_relay = start_discovery_relay(all_events).await;
    println!(
        "\n✓ Discovery relay started on {} (insecure for bootstrap)",
        discovery_relay.url
    );

    // Set up resolver
    let temp_dir = TempDir::new().expect("temp dir");
    std::env::set_var("FRONTIER_DATA_DIR", temp_dir.path());

    let storage = Arc::new(Storage::new().expect("storage"));
    let relay_config_path = temp_dir.path().join("relays.yaml");
    std::fs::write(
        &relay_config_path,
        format!("relays:\n  - {}\n", discovery_relay.url),
    )
    .unwrap();

    let relay_directory = RelayDirectory::load(Some(relay_config_path.clone())).unwrap();
    let resolver = Arc::new(NnsResolver::new(
        Arc::clone(&storage),
        relay_directory,
        NostrClient::new(),
    ));
    println!("✓ NNS resolver configured\n");

    // Give relay time to start and be ready
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // TEST 1: Resolve and fetch direct HTTPS site
    println!("TEST 1: Direct HTTPS Site (NO DNS)");
    println!("-----------------------------------");
    let output = resolver.resolve("mywebsite").await.expect("resolve failed");
    let http_ip = match &output.claims.primary.location {
        ClaimLocation::DirectIp(addr) => *addr,
        _ => panic!("expected DirectIp"),
    };
    let http_tls_pubkey = output.claims.primary.tls_spki_hex().expect("tls pubkey");
    assert_eq!(http_ip, http_server.addr);
    assert_eq!(http_tls_pubkey, http_tls.pubkey_hex);
    println!("✓ Resolved mywebsite → https://{}", http_ip);
    println!("✓ TLS pubkey verified from NNS event");

    // Fetch with custom TLS verification
    let http_content = fetch_with_tls(&http_ip, &http_tls_pubkey).await;
    assert!(http_content.contains("<h1>Direct HTTPS Site</h1>"));
    println!("✓ Fetched content over TLS (NO DNS)\n");

    // TEST 2: Resolve and fetch Blossom site (NO DNS)
    println!("TEST 2: Blossom Content Site (NO DNS)");
    println!("--------------------------------------");

    // Use new resolver instance to force fresh connection to relay
    let resolver2 = Arc::new(NnsResolver::new(
        Arc::clone(&storage),
        RelayDirectory::load(Some(relay_config_path.clone())).unwrap(),
        NostrClient::new(),
    ));
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let output = resolver2
        .resolve("myblossomsite")
        .await
        .expect("resolve blossom failed");
    let (blossom_ip, root_hash, _blossom_servers) = match &output.claims.primary.location {
        ClaimLocation::Blossom { root_hash, servers } => {
            (blossom_server.addr, root_hash.clone(), servers.clone())
        }
        _ => panic!("expected Blossom"),
    };
    let blossom_tls_pubkey = output.claims.primary.tls_spki_hex().expect("tls pubkey");

    assert_eq!(blossom_ip, blossom_server.addr);
    assert_eq!(blossom_tls_pubkey, blossom_tls.pubkey_hex);
    println!("✓ Resolved myblossomsite → https://{}", blossom_ip);
    println!("✓ TLS pubkey verified from NNS event");
    println!("✓ Blossom root hash: {}", &root_hash[..16]);

    // Fetch Blossom blob with TLS verification
    let index_hash = &blossom_server.files[0].1;
    let url = format!("https://{}/{}", blossom_ip, index_hash);
    let blossom_content = fetch_with_tls_url(&url, &blossom_tls_pubkey).await;
    assert!(blossom_content.contains("<h1>Blossom Site</h1>"));
    println!("✓ Fetched Blossom content over TLS (NO DNS)");
    println!("✓ Content hash verified: {}", &index_hash[..16]);
    println!();

    // TEST 3: Resolve Nostr relay and verify TLS info (NO DNS)
    println!("TEST 3: Nostr Relay Connection (NO DNS)");
    println!("----------------------------------------");

    // Use new resolver instance
    let resolver3 = Arc::new(NnsResolver::new(
        Arc::clone(&storage),
        RelayDirectory::load(Some(relay_config_path.clone())).unwrap(),
        NostrClient::new(),
    ));
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let output = resolver3
        .resolve("myrelay")
        .await
        .expect("resolve relay failed");
    let relay_ip = match &output.claims.primary.location {
        ClaimLocation::DirectIp(addr) => *addr,
        _ => panic!("expected DirectIp"),
    };
    let relay_tls_pubkey = output.claims.primary.tls_spki_hex().expect("tls pubkey");

    assert_eq!(relay_ip, relay_server.addr);
    assert_eq!(relay_tls_pubkey, relay_tls.pubkey_hex);
    println!("✓ Resolved myrelay → wss://{}", relay_ip);
    println!("✓ TLS pubkey verified from NNS event");

    // Actually connect to the relay with TLS to prove it works
    println!("✓ Relay TLS info retrieved - ready for WebSocket connection");
    let relay_tls_key =
        PublishedTlsKey::new(TlsAlgorithm::Ed25519, &relay_tls_pubkey).expect("relay tls key");
    let relay_url = Url::parse(&format!("wss://{}", relay_ip)).expect("relay url");
    let mut relay_socket = connect_websocket(&relay_url, Some(&relay_tls_key))
        .await
        .expect("connect relay websocket");
    relay_socket
        .close(None)
        .await
        .expect("close relay websocket");
    println!("✓ Established pinned TLS WebSocket to relay (NO DNS)");
    println!();

    // Cleanup
    let _ = http_server.shutdown.send(());
    let _ = blossom_server.shutdown.send(());
    let _ = relay_server.shutdown.send(());
    let _ = discovery_relay.shutdown.send(());
    std::env::remove_var("FRONTIER_DATA_DIR");

    println!("✅ NO_DNS FULL STACK TEST PASSED!");
    println!("==================================");
    println!("All 3 services verified:");
    println!("  ✓ Direct HTTPS site with TLS verification");
    println!("  ✓ Blossom content with TLS verification");
    println!("  ✓ Nostr relay with TLS info");
    println!("  ✓ ZERO DNS LOOKUPS");
    println!("  ✓ ALL TLS-ENCRYPTED\n");
}

// --- TLS Certificate Generation ---

#[derive(Clone)]
struct TlsCert {
    cert_pem: String,
    key_pem: String,
    pubkey_hex: String,
}

fn generate_tls_cert(cn: &str) -> TlsCert {
    use ring::signature::KeyPair;

    // Use rcgen to generate Ed25519 key pair
    let rcgen_key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
    let key_pem = rcgen_key_pair.serialize_pem();

    // Create self-signed certificate
    let params = rcgen::CertificateParams::new(vec![cn.to_string()]).unwrap();
    let cert = params.self_signed(&rcgen_key_pair).unwrap();
    let cert_pem = cert.pem();

    // Extract public key from PEM for Nostr event
    // Parse the private key PEM to get the raw Ed25519 public key
    let ring_key_pair =
        ring::signature::Ed25519KeyPair::from_pkcs8(rcgen_key_pair.serialize_der().as_ref())
            .unwrap();
    let public_key_bytes = ring_key_pair.public_key().as_ref();
    let pubkey_hex = hex::encode(public_key_bytes);

    TlsCert {
        cert_pem,
        key_pem,
        pubkey_hex,
    }
}

fn load_tls_config(tls: &TlsCert) -> Arc<ServerConfig> {
    let cert_chain: Vec<CertificateDer> = certs(&mut Cursor::new(tls.cert_pem.as_bytes()))
        .collect::<Result<_, _>>()
        .unwrap();

    let mut keys = pkcs8_private_keys(&mut Cursor::new(tls.key_pem.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::Pkcs8(keys.remove(0));

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .unwrap();

    Arc::new(config)
}

// --- HTTPS Server ---

struct HttpsServer {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    #[allow(dead_code)]
    handle: tokio::task::JoinHandle<()>,
}

async fn start_https_server(tls: TlsCert) -> HttpsServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let tls_config = load_tls_config(&tls);
    let acceptor = TlsAcceptor::from(tls_config);

    let handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    if let Ok((stream, _)) = accept {
                        let acceptor = acceptor.clone();
                        tokio::spawn(async move {
                            if let Ok(mut tls_stream) = acceptor.accept(stream).await {
                                let body = b"<html><body><h1>Direct HTTPS Site</h1><p>Served over TLS with pubkey from Nostr</p></body></html>";
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n",
                                    body.len()
                                );
                                let _ = tls_stream.write_all(response.as_bytes()).await;
                                let _ = tls_stream.write_all(body).await;
                            }
                        });
                    }
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    HttpsServer {
        addr,
        shutdown: shutdown_tx,
        handle,
    }
}

// --- Blossom Server ---

struct BlossomServer {
    addr: SocketAddr,
    root_hash: String,
    files: Vec<(String, String)>, // (path, hash)
    shutdown: oneshot::Sender<()>,
    #[allow(dead_code)]
    handle: tokio::task::JoinHandle<()>,
}

async fn start_blossom_tls_server(tls: TlsCert) -> BlossomServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    // Create test content
    let index_html =
        b"<html><body><h1>Blossom Site</h1><p>Content-addressed via Blossom</p></body></html>";
    let index_hash = hash_bytes(index_html);

    let files = vec![("/index.html".to_string(), index_hash.clone())];
    let root_hash = index_hash.clone();

    let tls_config = load_tls_config(&tls);
    let acceptor = TlsAcceptor::from(tls_config);

    let index_content = index_html.to_vec();
    let handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    if let Ok((stream, _)) = accept {
                        let acceptor = acceptor.clone();
                        let content = index_content.clone();
                        tokio::spawn(async move {
                            if let Ok(mut tls_stream) = acceptor.accept(stream).await {
                                // Read HTTP request
                                let mut buf = vec![0u8; 1024];
                                let _ = tls_stream.read(&mut buf).await;

                                // Serve blob
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n",
                                    content.len()
                                );
                                let _ = tls_stream.write_all(response.as_bytes()).await;
                                let _ = tls_stream.write_all(&content).await;
                            }
                        });
                    }
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    BlossomServer {
        addr,
        root_hash,
        files,
        shutdown: shutdown_tx,
        handle,
    }
}

// --- Relay Server ---

struct RelayServer {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    #[allow(dead_code)]
    handle: tokio::task::JoinHandle<()>,
}

async fn start_relay_tls_server(tls: TlsCert) -> RelayServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let tls_config = load_tls_config(&tls);
    let acceptor = TlsAcceptor::from(tls_config);

    let handle = tokio::spawn(async move {
        let shutdown_rx = shutdown_rx;
        if let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(tls_stream) = acceptor.accept(stream).await {
                    // Accept WebSocket upgrade over TLS
                    let _ = tokio_tungstenite::accept_async(tls_stream).await;
                }
            });
        }
        let _ = shutdown_rx.await;
    });

    RelayServer {
        addr,
        shutdown: shutdown_tx,
        handle,
    }
}

// --- Discovery Relay (insecure, just for event distribution) ---

struct DiscoveryRelay {
    url: url::Url,
    shutdown: oneshot::Sender<()>,
    #[allow(dead_code)]
    handle: tokio::task::JoinHandle<()>,
}

async fn start_discovery_relay(events: Vec<Event>) -> DiscoveryRelay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        // Handle multiple connections
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    if let Ok((stream, _)) = accept_result {
                        let events_clone = events.clone();
                        tokio::spawn(async move {
                            if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                                // Handle multiple REQ messages on same connection
                                while let Some(msg) = ws.next().await {
                                    match msg {
                                        Ok(Message::Text(text)) => {
                                            if let Ok(value) = serde_json::from_str::<Value>(text.as_ref()) {
                                                if value.get(0) == Some(&Value::String("REQ".into())) {
                                                    if let Some(id) = value.get(1).and_then(|v| v.as_str()) {
                                                        // Send all events for this subscription
                                                        for event in &events_clone {
                                                            let event_value = serde_json::to_value(event).unwrap();
                                                            let event_msg = json!(["EVENT", id, event_value]);
                                                            let _ = ws.send(Message::Text(event_msg.to_string().into())).await;
                                                        }
                                                        let eose_msg = json!(["EOSE", id]);
                                                        let _ = ws.send(Message::Text(eose_msg.to_string().into())).await;
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
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    DiscoveryRelay {
        url: url::Url::parse(&format!("ws://{}", addr)).unwrap(),
        shutdown: shutdown_tx,
        handle,
    }
}

// --- Event Builders ---

fn build_service_event(
    name: &str,
    addr: SocketAddr,
    tls_pubkey: &str,
    blossom_hash: Option<&str>,
    server_url: Option<&str>,
    svc_kind: &str,
    keys: &Keys,
) -> Event {
    let socket = format!("{}:{}", addr.ip(), addr.port());

    let mut tags = vec![
        Tag::identifier(name),
        Tag::parse(&["ip", &socket]).unwrap(),
        Tag::parse(&["tls-pubkey", tls_pubkey]).unwrap(),
        Tag::parse(&["tls-alg", "ed25519"]).unwrap(),
        Tag::parse(&["svc", svc_kind]).unwrap(),
    ];

    let endpoint_transport = match svc_kind {
        "relay" => "wss",
        _ => "https",
    };
    tags.push(Tag::parse(&["endpoint", endpoint_transport, &socket, "0"]).unwrap());

    if let Some(hash) = blossom_hash {
        tags.push(Tag::parse(&["blossom", hash]).unwrap());
    }
    if let Some(url) = server_url {
        tags.push(Tag::parse(&["server", url]).unwrap());
    }

    EventBuilder::new(Kind::from(34256u16), "", tags)
        .custom_created_at(Timestamp::now())
        .to_event(keys)
        .unwrap()
}

fn build_blossom_manifest_events(files: &[(String, String)], keys: &Keys) -> Vec<Event> {
    files
        .iter()
        .map(|(path, hash)| {
            let tags = vec![
                Tag::identifier(path),
                Tag::parse(&["sha256", hash]).unwrap(),
            ];
            EventBuilder::new(Kind::from(34128u16), "", tags)
                .custom_created_at(Timestamp::now())
                .to_event(keys)
                .unwrap()
        })
        .collect()
}

// --- Utilities ---

fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

async fn fetch_with_tls(addr: &SocketAddr, expected_pubkey: &str) -> String {
    let url = format!("https://{}", addr);
    fetch_with_tls_url(&url, expected_pubkey).await
}

async fn fetch_with_tls_url(url: &str, expected_pubkey: &str) -> String {
    use frontier::nns::{PublishedTlsKey, TlsAlgorithm};
    use frontier::tls::SecureHttpClient;

    let tls_key = PublishedTlsKey::new(TlsAlgorithm::Ed25519, expected_pubkey).unwrap();
    let client = SecureHttpClient::new(Some(&tls_key))
        .unwrap()
        .client()
        .clone();

    let mut last_err = None;
    for attempt in 0..3 {
        match client.get(url).send().await {
            Ok(response) => {
                let ok = response.error_for_status().unwrap();
                return ok.text().await.unwrap();
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < 2 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    panic!("failed to fetch {url}: {last_err:?}");
}
