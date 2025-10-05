use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;
use url::Url;

struct BlossomHttpServer {
    addr: std::net::SocketAddr,
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_blossom_http_server(blobs: HashMap<String, Vec<u8>>) -> Result<BlossomHttpServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let blobs = Arc::new(blobs);
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    if let Ok((mut stream, _)) = accept {
                        let blobs = Arc::clone(&blobs);
                        tokio::spawn(async move {
                            let mut buffer = vec![0u8; 4096];
                            match stream.read(&mut buffer).await {
                                Ok(0) => {}
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
                                Err(_) => {}
                            }
                        });
                    }
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    Ok(BlossomHttpServer {
        addr,
        shutdown: shutdown_tx,
        handle,
    })
}

struct RelayServer {
    url: Url,
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_blossom_relay(claim: Event, manifests: Vec<Event>) -> Result<RelayServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let claim = Arc::new(claim);
    let manifests = Arc::new(manifests);

    let handle = tokio::spawn(async move {
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
                                                    let filters = value.as_array().into_iter().flatten().skip(2);
                                                    let mut want_claim = false;
                                                    let mut want_manifest = false;
                                                    for filter in filters {
                                                        if let Some(kinds) = filter.get("kinds").and_then(|v| v.as_array()) {
                                                            for kind_val in kinds {
                                                                if let Some(kind) = kind_val.as_u64() {
                                                                    if kind == 34256 {
                                                                        want_claim = true;
                                                                    }
                                                                    if kind == 34128 {
                                                                        want_manifest = true;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    if want_claim {
                                                        let event_msg = json!(["EVENT", sub_id, serde_json::to_value(&*claim).unwrap()]);
                                                        let _ = ws.send(Message::Text(event_msg.to_string())).await;
                                                    }
                                                    if want_manifest {
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

    Ok(RelayServer {
        url: Url::parse(&format!("ws://{}", addr)).unwrap(),
        shutdown: shutdown_tx,
        handle,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
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

#[tokio::main]
async fn main() -> Result<()> {
    let site_name = "blossomdemo";
    let initial_input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://example.com".to_string());

    let home_html = b"<html><body><h1>Blossom Demo Home</h1><p>Served from Blossom blobs.</p><p><a href=\"/about.html\">About this demo</a></p></body></html>";
    let about_html = b"<html><body><h1>About</h1><p>The browser resolved this site through Blossom.</p><p><a href=\"/home.html\">Back home</a></p></body></html>";

    let home_hash = sha256_hex(home_html);
    let about_hash = sha256_hex(about_html);

    let mut blobs = HashMap::new();
    blobs.insert(home_hash.clone(), home_html.to_vec());
    blobs.insert(about_hash.clone(), about_html.to_vec());

    let http_server = start_blossom_http_server(blobs).await?;
    let server_url = format!("http://{}/", http_server.addr);

    let keys = Keys::generate();

    let claim_event = EventBuilder::new(
        Kind::from(34256u16),
        "",
        vec![
            Tag::identifier(site_name),
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

    let relay_server = start_blossom_relay(claim_event.clone(), manifest_events.clone()).await?;

    let temp_dir = TempDir::new().context("failed to create temp dir")?;
    let relay_config_path = temp_dir.path().join("relays.yaml");
    std::fs::write(
        &relay_config_path,
        format!("relays:\n  - {}\n", relay_server.url),
    )?;

    let data_dir = temp_dir.path().join("data");
    std::fs::create_dir_all(&data_dir)?;

    let mut frontier_path = std::env::current_exe()?;
    frontier_path.pop();
    frontier_path.push(if cfg!(target_os = "windows") {
        "frontier.exe"
    } else {
        "frontier"
    });

    if !frontier_path.exists() {
        anyhow::bail!(
            "Frontier binary not found at {}. Run `cargo build --bin frontier` first.",
            frontier_path.display()
        );
    }

    println!();
    println!("================ Frontier Blossom Demo ================");
    println!("Blossom HTTP server : http://{}", http_server.addr);
    println!("Local relay         : {}", relay_server.url);
    println!("Site name           : {}", site_name);
    println!("Home hash           : {}", home_hash);
    println!("About hash          : {}", about_hash);
    println!();
    println!(
        "When the Frontier window opens (initially at '{}'):",
        initial_input
    );
    println!("  1. Type `{}` in the URL bar and press Enter.", site_name);
    println!("  2. You should land on 'Blossom Demo Home'.");
    println!("  3. Click the 'About this demo' link to fetch a second blob.");
    println!("  4. Close the window when finished; servers shut down automatically.");
    println!("========================================================\n");

    let mut command = Command::new(&frontier_path);
    command
        .current_dir(std::env::current_dir()?)
        .env("FRONTIER_DATA_DIR", &data_dir)
        .env("FRONTIER_RELAY_CONFIG", &relay_config_path)
        .arg(&initial_input)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let mut child = command.spawn().context("failed to launch Frontier")?;
    let shutdown_http = Arc::new(Mutex::new(Some(http_server.shutdown)));
    let shutdown_relay = Arc::new(Mutex::new(Some(relay_server.shutdown)));

    tokio::select! {
        status = child.wait() => {
            match status {
                Ok(status) => {
                    println!("Frontier exited with status: {status}");
                }
                Err(err) => {
                    eprintln!("Failed to wait for Frontier: {err}");
                }
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nCtrl+C received, shutting down Frontier...");
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }

    if let Some(tx) = shutdown_http.lock().await.take() {
        let _ = tx.send(());
    }
    if let Some(tx) = shutdown_relay.lock().await.take() {
        let _ = tx.send(());
    }

    let _ = http_server.handle.await;
    let _ = relay_server.handle.await;

    println!("Cleanup complete.");
    Ok(())
}
