#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! nostr-sdk = "0.37"
//! tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros"] }
//! ```

use nostr_sdk::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <name> [ip:port] [private_key_hex]", args[0]);
        eprintln!("Example: {} mysite 127.0.0.1:8080", args[0]);
        std::process::exit(1);
    }

    let name = &args[1];
    let ip_port = args.get(2).map(|s| s.as_str()).unwrap_or("127.0.0.1:8080");

    // Parse or generate keys
    let keys = if let Some(key_hex) = args.get(3) {
        Keys::parse(key_hex)?
    } else {
        let new_keys = Keys::generate();
        println!("Generated new keypair:");
        println!("  Private key (hex): {}", new_keys.secret_key()?.to_secret_hex());
        println!("  Public key (hex): {}", new_keys.public_key());
        println!("  Save your private key to reuse it later!\n");
        new_keys
    };

    // Create client
    let client = Client::new(keys);

    // Add default relays
    let relays = vec![
        "wss://relay.damus.io",
        "wss://nos.lol",
        "wss://relay.nostr.band",
    ];

    for relay in relays {
        client.add_relay(relay).await?;
    }

    client.connect().await;

    // Create the NNS event (kind 34256)
    let tags = vec![
        Tag::custom(TagKind::D, vec![name]),
        Tag::custom(TagKind::Custom("ip".into()), vec![ip_port]),
    ];

    let event = EventBuilder::new(Kind::ParameterizedReplaceable(34256), "", tags)
        .sign_with_keys(&client.keys().await)?;

    println!("\n📝 Created NNS claim event:");
    println!("  Event ID: {}", event.id);
    println!("  Name: {}", name);
    println!("  IP:Port: {}", ip_port);
    println!("  Pubkey: {}\n", event.pubkey);

    println!("📡 Publishing to relays...");
    let output = client.send_event(event).await?;
    println!("✅ Event sent: {}", output.id);

    // Keep connection open briefly to ensure event is sent
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    println!("\n✨ Done! Your NNS claim is now published.");
    println!("Try accessing it by entering '{}' in the Frontier browser URL bar.", name);

    Ok(())
}
