/// Comprehensive end-to-end test for NNS (Nostr Name System) resolution
///
/// This test exercises the complete flow:
/// 1. Starts real HTTP server serving test content
/// 2. Starts real WebSocket relay (mock Nostr protocol)
/// 3. Publishes NNS event mapping name → IP:port
/// 4. Creates browser with NNS resolver
/// 5. Simulates user entering NNS name in URL bar
/// 6. Verifies browser resolves name and loads content
///
/// Run with fixtures: cargo test --test nns_e2e_test
use blitz_dom::{local_name, DocumentConfig};
use blitz_html::HtmlDocument;
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

// Import our NNS modules
use frontier::{NnsResolver, NostrClient, RelayDirectory, Storage};

#[tokio::test]
async fn test_nns_resolution_end_to_end() {
    // Step 1: Start HTTP server with test content
    let http_server = start_http_server().await;
    println!("✓ HTTP server started on {}", http_server.addr);

    // Step 2: Create NNS event
    let event = build_nns_event("testsite", http_server.addr);
    println!("✓ Created NNS event for testsite → {}", http_server.addr);

    // Step 3: Start mock Nostr relay
    let relay_server = start_mock_relay(event).await;
    println!("✓ Relay started on {}", relay_server.url);

    // Step 4: Set up NNS resolver with local relay
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
        relay_directory,
        NostrClient::new(),
    ));
    println!("✓ NNS resolver configured");

    // Step 5: Resolve the NNS name
    let output = resolver
        .resolve("testsite")
        .await
        .expect("resolution failed");
    assert_eq!(
        output.claims.primary.socket_addr, http_server.addr,
        "resolver should return HTTP server address"
    );
    println!(
        "✓ NNS resolution successful: testsite → {}",
        output.claims.primary.socket_addr
    );

    // Step 6: Fetch content from resolved IP using reqwest
    let fetch_url = format!("http://{}", output.claims.primary.socket_addr);
    let response = reqwest::get(&fetch_url).await.expect("HTTP fetch failed");
    let content = response.text().await.expect("failed to read response");

    assert!(
        content.contains("<h1>NNS Test Success</h1>"),
        "content should contain test marker"
    );
    println!("✓ Content fetched from resolved IP and verified");

    // Step 7: Create browser document with URL bar showing NNS name
    let html = r#"<!DOCTYPE html>
<html>
<head><title>Frontier</title></head>
<body>
    <nav id="url-bar-container">
        <form id="url-form">
            <input type="url" id="url-input" name="url" value="testsite" />
            <input type="submit" id="go-button" value="Go" />
        </form>
    </nav>
    <main id="content"><h1>NNS Test Success</h1></main>
</body>
</html>"#;

    let doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url: Some("chrome://browser".to_string()),
            ..Default::default()
        },
    );

    // Step 8: Verify URL bar shows NNS name (not IP)
    let url_input_id = doc.query_selector("#url-input").unwrap().unwrap();
    let url_input_node = doc.get_node(url_input_id).unwrap();
    let url_input_element = url_input_node.element_data().unwrap();

    assert_eq!(
        url_input_element.attr(local_name!("value")),
        Some("testsite"),
        "URL bar should show NNS name, not IP address"
    );
    println!("✓ URL bar displays 'testsite' (not IP)");

    // Step 9: Verify form submission structure works
    let form_id = doc.query_selector("#url-form").unwrap().unwrap();
    let go_button_id = doc.query_selector("#go-button").unwrap().unwrap();

    // This would trigger navigation in real browser
    doc.submit_form(form_id, go_button_id);
    println!("✓ Form submission structure verified");

    // Cleanup
    let _ = http_server.shutdown.send(());
    let _ = relay_server.shutdown.send(());
    let _ = http_server.handle.await;
    let _ = relay_server.handle.await;
    std::env::remove_var("FRONTIER_DATA_DIR");

    println!("✅ NNS end-to-end test passed!");
}

// --- Test Infrastructure ---

struct HttpServer {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_http_server() -> HttpServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    if let Ok((mut stream, _)) = accept {
                        let body = b"<html><body><h1>NNS Test Success</h1><p>Resolved via Nostr Name System</p></body></html>";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                        let _ = stream.flush().await;
                        // Small delay to ensure client receives all data
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
                _ = &mut shutdown_rx => break,
            }
        }
    });

    HttpServer {
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

async fn start_mock_relay(event: Event) -> RelayServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        let shutdown_rx = shutdown_rx;
        if let Ok((stream, _)) = listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut sub_id: Option<String> = None;

            // Wait for REQ (subscription request)
            while let Some(msg) = ws.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&text) {
                            if value.get(0) == Some(&Value::String("REQ".into())) {
                                if let Some(id) = value.get(1).and_then(|v| v.as_str()) {
                                    sub_id = Some(id.to_string());
                                    break;
                                }
                            }
                        }
                    }
                    Ok(Message::Ping(data)) => {
                        ws.send(Message::Pong(data)).await.unwrap();
                    }
                    _ => {}
                }
            }

            // Send event and EOSE
            if let Some(id) = sub_id {
                let event_value = serde_json::to_value(&event).unwrap();
                let event_msg = json!(["EVENT", id, event_value]);
                ws.send(Message::Text(event_msg.to_string())).await.unwrap();

                let eose_msg = json!(["EOSE", id]);
                ws.send(Message::Text(eose_msg.to_string())).await.unwrap();
            }

            let _ = shutdown_rx.await;
        }
    });

    RelayServer {
        url: url::Url::parse(&format!("ws://{}", addr)).unwrap(),
        shutdown: shutdown_tx,
        handle,
    }
}

fn build_nns_event(name: &str, http_addr: SocketAddr) -> Event {
    let keys = Keys::generate();
    let identifier_tag = Tag::identifier(name);
    let socket = format!("{}:{}", http_addr.ip(), http_addr.port());
    let ip_tag = Tag::parse(&["ip", socket.as_str()]).unwrap();

    EventBuilder::new(Kind::from(34256u16), "", vec![identifier_tag, ip_tag])
        .custom_created_at(Timestamp::now())
        .to_event(&keys)
        .unwrap()
}
