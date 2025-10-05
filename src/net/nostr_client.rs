use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ::url::Url;
use nostr_sdk::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NostrClientError {
    #[error("nostr error: {0}")]
    Nostr(#[from] nostr_sdk::prelude::Error),
}

#[derive(Debug, Clone)]
pub struct RelayEvent {
    pub relays: HashSet<Url>,
    pub event: Event,
}

#[derive(Clone)]
pub struct NostrClient;

impl Default for NostrClient {
    fn default() -> Self {
        Self::new()
    }
}

impl NostrClient {
    pub fn new() -> Self {
        Self
    }

    pub async fn fetch_events(
        &self,
        relays: &[Url],
        filter: Filter,
        timeout: Duration,
    ) -> Result<Vec<RelayEvent>, NostrClientError> {
        if relays.is_empty() {
            return Ok(Vec::new());
        }

        let keys = Keys::generate();
        let opts = Options::new().connection_timeout(Some(timeout));
        let client = Client::with_opts(&keys, opts);

        for relay in relays {
            client.add_relay(relay.as_str()).await?;
        }
        client.connect().await;

        let events = client.get_events_of(vec![filter], Some(timeout)).await?;

        let _ = client.disconnect().await;

        let mut collected: HashMap<EventId, RelayEvent> = HashMap::new();
        for event in events {
            collected.entry(event.id).or_insert_with(|| RelayEvent {
                relays: relays.first().cloned().into_iter().collect::<HashSet<_>>(),
                event,
            });
        }

        Ok(collected.into_values().collect())
    }
}
