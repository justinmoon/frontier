use std::env;
use std::error::Error;
use std::fs;
use std::time::Duration;

use nostr_sdk::prelude::*;
use serde::Deserialize;
use tokio::time::sleep;

#[derive(Deserialize)]
struct RelayConfig {
    relays: Vec<String>,
}

const DEFAULT_RELAYS: &[&str] = &["wss://relay.damus.io", "wss://nos.lol"];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let name = args
        .next()
        .unwrap_or_else(|| panic!("usage: publish_nns_event <name> <ip:port>"));
    let socket = args
        .next()
        .unwrap_or_else(|| panic!("usage: publish_nns_event <name> <ip:port>"));

    let relay_urls = load_relays()?;

    let keys = Keys::generate();
    let client = Client::new(keys.clone());

    for relay in &relay_urls {
        client.add_relay(relay).await?;
    }
    client.connect().await;

    let identifier_tag = Tag::identifier(name.clone());
    let ip_tag = Tag::parse(&["ip", socket.as_str()])?;

    let event = EventBuilder::new(Kind::from(34256u16), "", vec![identifier_tag, ip_tag])
        .custom_created_at(Timestamp::now())
        .to_event(&keys)?;

    client.send_event(event).await?;

    sleep(Duration::from_secs(1)).await;
    let _ = client.disconnect().await;

    Ok(())
}

fn load_relays() -> Result<Vec<String>, Box<dyn Error>> {
    if let Ok(path) = env::var("FRONTIER_RELAY_CONFIG") {
        if !path.is_empty() {
            let contents = fs::read_to_string(path)?;
            let config: RelayConfig = serde_yaml::from_str(&contents)?;
            return Ok(config.relays);
        }
    }

    Ok(DEFAULT_RELAYS.iter().map(|r| r.to_string()).collect())
}
