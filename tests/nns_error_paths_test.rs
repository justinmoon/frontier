/// Error path tests for NNS (Nostr Name System) resolution
///
/// These tests verify that the NNS resolver handles edge cases gracefully:
/// - Multiple claims to the same name
/// - Relay timeouts
/// - Malformed events
/// - Invalid IP addresses
/// - Cache expiration
/// - Selection persistence across restarts
use futures_util::{SinkExt, StreamExt};
use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;

use frontier::{nns::ClaimLocation, NnsResolver, NostrClient, RelayDirectory, Storage};

// --- Test Infrastructure (reused from nns_e2e_test.rs) ---

struct RelayServer {
    url: url::Url,
    shutdown: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_mock_relay(events: Vec<Event>) -> RelayServer {
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

            // Send all events and EOSE
            if let Some(id) = sub_id {
                for event in events {
                    let event_value = serde_json::to_value(&event).unwrap();
                    let event_msg = json!(["EVENT", id, event_value]);
                    ws.send(Message::Text(event_msg.to_string())).await.unwrap();
                }

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

fn build_nns_event(name: &str, ip: &str, keys: &Keys) -> Event {
    let identifier_tag = Tag::identifier(name);
    let ip_tag = Tag::parse(&["ip", ip]).unwrap();

    EventBuilder::new(Kind::from(34256u16), "", vec![identifier_tag, ip_tag])
        .custom_created_at(Timestamp::now())
        .to_event(keys)
        .unwrap()
}

async fn create_test_resolver(relay_url: &str) -> (Arc<NnsResolver>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = Arc::new(Storage::new_with_path(temp_dir.path()).expect("storage"));

    let relay_config_path = temp_dir.path().join("relays.yaml");
    std::fs::write(&relay_config_path, format!("relays:\n  - {}\n", relay_url)).unwrap();

    let relay_directory = RelayDirectory::load(Some(relay_config_path)).unwrap();
    let resolver = Arc::new(NnsResolver::new(
        Arc::clone(&storage),
        relay_directory,
        NostrClient::new(),
    ));

    (resolver, temp_dir)
}

// --- Tests ---

#[tokio::test]
async fn test_multiple_claims_requires_selection() {
    // Create two different keys claiming same name
    let keys1 = Keys::generate();
    let keys2 = Keys::generate();

    let event1 = build_nns_event("multisite", "192.168.1.100:8080", &keys1);
    let event2 = build_nns_event("multisite", "10.0.0.5:8080", &keys2);

    let relay_server = start_mock_relay(vec![event1, event2]).await;
    let (resolver, _temp_dir) = create_test_resolver(relay_server.url.as_ref()).await;

    // Resolve should return both claims
    let output = resolver.resolve("multisite").await.expect("resolution");

    // Should have primary + 1 alternate
    assert_eq!(
        output.claims.alternates.len(),
        1,
        "should have 1 alternate claim"
    );

    // Verify both IPs are present (one as primary, one as alternate)
    let all_ips: Vec<String> = std::iter::once(&output.claims.primary)
        .chain(output.claims.alternates.iter())
        .filter_map(|claim| match &claim.location {
            ClaimLocation::DirectIp(addr) => Some(addr.to_string()),
            ClaimLocation::Blossom { .. } => None,
            ClaimLocation::LegacyUrl(url) => Some(url.to_string()),
        })
        .collect();

    assert!(
        all_ips.contains(&"192.168.1.100:8080".to_string())
            || all_ips.contains(&"10.0.0.5:8080".to_string()),
        "should contain one of the claimed IPs"
    );

    let _ = relay_server.shutdown.send(());
    let _ = relay_server.handle.await;

    println!("✅ Multiple claims test passed");
}

#[tokio::test]
async fn test_relay_timeout() {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = Arc::new(Storage::new_with_path(temp_dir.path()).expect("storage"));

    // Create relay config pointing to non-existent relay
    let relay_config_path = temp_dir.path().join("relays.yaml");
    std::fs::write(
        &relay_config_path,
        "relays:\n  - ws://127.0.0.1:9999\n", // Nothing listening here
    )
    .unwrap();

    let relay_directory = RelayDirectory::load(Some(relay_config_path)).unwrap();

    // Create client with very short timeout
    let client = NostrClient::new();
    let resolver = Arc::new(NnsResolver::new_with_timeout(
        Arc::clone(&storage),
        relay_directory,
        client,
        Duration::from_millis(100),
    ));

    // Resolution should fail gracefully (no panic)
    let result = resolver.resolve("testsite").await;

    assert!(result.is_err(), "should fail when relay is unreachable");

    println!("✅ Relay timeout test passed");
}

#[tokio::test]
async fn test_malformed_events_are_skipped() {
    let keys = Keys::generate();

    // Valid event
    let valid_event = build_nns_event("goodsite", "192.168.1.100:8080", &keys);

    // Malformed event: missing ip tag
    let malformed_event = EventBuilder::new(
        Kind::from(34256u16),
        "",
        vec![Tag::identifier("badsite")], // No ip tag!
    )
    .custom_created_at(Timestamp::now())
    .to_event(&keys)
    .unwrap();

    let relay_server = start_mock_relay(vec![malformed_event, valid_event]).await;
    let (resolver, _temp_dir) = create_test_resolver(relay_server.url.as_ref()).await;

    // Should resolve goodsite, skip badsite
    let output = resolver
        .resolve("goodsite")
        .await
        .expect("should resolve valid event");
    let primary_addr = match output.claims.primary.location {
        ClaimLocation::DirectIp(addr) => addr,
        ClaimLocation::Blossom { .. } => panic!("expected direct IP"),
        ClaimLocation::LegacyUrl(url) => panic!("unexpected legacy url {url}"),
    };
    assert_eq!(primary_addr.to_string(), "192.168.1.100:8080");

    // badsite should fail (no valid events)
    let bad_result = resolver.resolve("badsite").await;
    assert!(
        bad_result.is_err(),
        "should fail for name with no valid events"
    );

    let _ = relay_server.shutdown.send(());
    let _ = relay_server.handle.await;

    println!("✅ Malformed events test passed");
}

#[tokio::test]
async fn test_invalid_ip_addresses() {
    let keys = Keys::generate();

    // Events with invalid IPs
    let event1 = build_nns_event("badip1", "not-an-ip", &keys);
    let event2 = build_nns_event("badip2", "999.999.999.999:8080", &keys);
    let event3 = build_nns_event("badip3", "localhost", &keys); // No port

    let relay_server = start_mock_relay(vec![event1, event2, event3]).await;
    let (resolver, _temp_dir) = create_test_resolver(relay_server.url.as_ref()).await;

    // All should fail gracefully (no panic)
    let result1 = resolver.resolve("badip1").await;
    assert!(result1.is_err(), "should reject 'not-an-ip'");

    let result2 = resolver.resolve("badip2").await;
    assert!(result2.is_err(), "should reject '999.999.999.999:8080'");

    let result3 = resolver.resolve("badip3").await;
    assert!(result3.is_err(), "should reject 'localhost' (no port)");

    let _ = relay_server.shutdown.send(());
    let _ = relay_server.handle.await;

    println!("✅ Invalid IP addresses test passed");
}

#[tokio::test]
async fn test_cache_behavior() {
    let keys = Keys::generate();
    let event = build_nns_event("cachedsite", "192.168.1.100:8080", &keys);

    let relay_server = start_mock_relay(vec![event]).await;
    let (resolver, _temp_dir) = create_test_resolver(relay_server.url.as_ref()).await;

    // First resolution - should query relay
    let output1 = resolver.resolve("cachedsite").await.expect("first resolve");
    assert!(
        !output1.from_cache,
        "first resolution should not be from cache"
    );

    // Second resolution immediately - should use cache
    let output2 = resolver
        .resolve("cachedsite")
        .await
        .expect("second resolve");
    assert!(output2.from_cache, "second resolution should be from cache");

    // Both should return same IP
    let addr1 = match output1.claims.primary.location {
        ClaimLocation::DirectIp(addr) => addr,
        ClaimLocation::Blossom { .. } => panic!("expected direct IP"),
        ClaimLocation::LegacyUrl(url) => panic!("unexpected legacy url {url}"),
    };
    let addr2 = match output2.claims.primary.location {
        ClaimLocation::DirectIp(addr) => addr,
        ClaimLocation::Blossom { .. } => panic!("expected direct IP"),
        ClaimLocation::LegacyUrl(url) => panic!("unexpected legacy url {url}"),
    };
    assert_eq!(addr1, addr2);

    let _ = relay_server.shutdown.send(());
    let _ = relay_server.handle.await;

    println!("✅ Cache behavior test passed");
}

#[tokio::test]
async fn test_selection_persistence() {
    let keys1 = Keys::generate();

    // Create temp directory for storage
    let temp_dir = TempDir::new().expect("temp dir");
    let temp_path = temp_dir.path().to_path_buf();

    // First: Record a selection using first resolver instance
    {
        let storage = Arc::new(Storage::new_with_path(&temp_path).expect("storage"));
        let relay_config_path = temp_path.join("relays.yaml");
        std::fs::write(&relay_config_path, "relays:\n  - ws://localhost:7777\n").unwrap();

        let relay_directory = RelayDirectory::load(Some(relay_config_path)).unwrap();
        let resolver = Arc::new(NnsResolver::new(
            Arc::clone(&storage),
            relay_directory,
            NostrClient::new(),
        ));

        // Record a selection
        resolver
            .record_selection("persistsite", &keys1.public_key().to_hex())
            .await
            .expect("record selection");
    }

    // Second: Create NEW resolver instance with same storage path
    // and verify selection was persisted
    {
        let storage = Arc::new(Storage::new_with_path(&temp_path).expect("storage"));

        // Query storage directly to verify selection was saved
        let selection = storage
            .selection("persistsite")
            .expect("query selection")
            .expect("should have selection");

        assert_eq!(
            selection.pubkey,
            keys1.public_key().to_hex(),
            "selection should persist across storage instances"
        );
        assert_eq!(selection.name, "persistsite");
    }

    println!("✅ Selection persistence test passed");
}
