use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nostr_sdk::prelude::*;
use thiserror::Error;
use tokio::task::{self, JoinError};

use crate::net::{NostrClient, NostrClientError, RelayDirectory};
use crate::nns::models::{ModelError, NnsClaim, ResolvedClaims};
use crate::nns::scoring::score_claim;
use crate::storage::{unix_timestamp, ClaimRecord, SelectionRecord, Storage, StorageError};

const CACHE_TTL_SECONDS: i64 = 600; // 10 minutes

#[derive(Debug, Error)]
pub enum ResolverError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("nostr error: {0}")]
    Nostr(#[from] NostrClientError),
    #[error("no claims found for {0}")]
    NoClaims(String),
    #[error("invalid cached pubkey: {0}")]
    InvalidCachedPubkey(String),
    #[error("invalid cached event id: {0}")]
    InvalidCachedEvent(String),
    #[error("task join error: {0}")]
    Join(#[from] JoinError),
    #[error("claim error: {0}")]
    Claim(#[from] ModelError),
}

pub struct ResolverOutput {
    pub claims: ResolvedClaims,
    pub from_cache: bool,
    pub selection: Option<SelectionRecord>,
}

pub struct NnsResolver {
    storage: Arc<Storage>,
    relay_directory: RelayDirectory,
    client: NostrClient,
}

impl NnsResolver {
    pub fn new(
        storage: Arc<Storage>,
        relay_directory: RelayDirectory,
        client: NostrClient,
    ) -> Self {
        Self {
            storage,
            relay_directory,
            client,
        }
    }

    pub async fn resolve(&self, name: &str) -> Result<ResolverOutput, ResolverError> {
        let normalized = name.trim().to_ascii_lowercase();
        let now = unix_timestamp();

        let selection = self.storage.selection(&normalized)?;

        if let Some(output) = self.try_from_cache(&normalized, now, selection.clone())? {
            return Ok(ResolverOutput {
                claims: output,
                from_cache: true,
                selection,
            });
        }

        let filter = Filter::new()
            .kind(Kind::from(34256u16))
            .identifier(normalized.clone())
            .limit(50);

        let events = self
            .client
            .fetch_events(
                self.relay_directory.relays(),
                filter,
                Duration::from_secs(3),
            )
            .await?;

        let mut claims_map: HashMap<String, NnsClaim> = HashMap::new();
        for relay_event in events.iter() {
            match NnsClaim::from_event(&normalized, relay_event) {
                Ok(claim) => {
                    let key = claim.score_key();
                    if let Some(existing) = claims_map.get_mut(&key) {
                        for relay in &claim.relays {
                            existing.relays.insert(relay.clone());
                        }
                        if claim.created_at > existing.created_at {
                            existing.created_at = claim.created_at;
                            existing.event_id = claim.event_id;
                            existing.note = claim.note.clone();
                        }
                    } else {
                        claims_map.insert(key, claim);
                    }
                }
                Err(e) => {
                    tracing::warn!(name = %normalized, error = ?e, "failed to parse claim");
                }
            }
        }

        if claims_map.is_empty() {
            return Err(ResolverError::NoClaims(normalized));
        }

        let claims: Vec<NnsClaim> = claims_map.into_values().collect();

        self.persist_claims(&normalized, &claims, now).await?;

        let resolved = self.rank_claims(claims, selection.as_ref(), now)?;

        Ok(ResolverOutput {
            claims: resolved,
            from_cache: false,
            selection,
        })
    }

    fn try_from_cache(
        &self,
        name: &str,
        now: i64,
        selection: Option<SelectionRecord>,
    ) -> Result<Option<ResolvedClaims>, ResolverError> {
        let cached = self.storage.cached_claims(name)?;
        let fresh: Vec<_> = cached
            .into_iter()
            .filter(|record| now - record.fetched_at <= CACHE_TTL_SECONDS)
            .collect();
        if fresh.is_empty() {
            return Ok(None);
        }

        let mut claims = Vec::new();
        for record in fresh {
            if let Some(claim) = self.record_to_claim(&record)? {
                claims.push(claim);
            }
        }

        if claims.is_empty() {
            return Ok(None);
        }

        self.rank_claims(claims, selection.as_ref(), now).map(Some)
    }

    fn record_to_claim(&self, record: &ClaimRecord) -> Result<Option<NnsClaim>, ResolverError> {
        let pubkey = PublicKey::from_hex(&record.pubkey)
            .map_err(|_| ResolverError::InvalidCachedPubkey(record.pubkey.clone()))?;
        let pubkey_npub = pubkey
            .to_bech32()
            .map_err(|e| ResolverError::InvalidCachedPubkey(e.to_string()))?;
        let event_id = EventId::from_hex(&record.event_id)
            .map_err(|_| ResolverError::InvalidCachedEvent(record.event_id.clone()))?;
        let created_at = Timestamp::from(record.created_at.max(0) as u64);

        let relays = record
            .relays
            .iter()
            .filter_map(|relay| url::Url::parse(relay).ok())
            .collect();

        let socket_addr: SocketAddr = match record.ip.parse() {
            Ok(addr) => addr,
            Err(_) => return Ok(None),
        };

        Ok(Some(NnsClaim {
            name: record.name.clone(),
            socket_addr,
            pubkey_hex: record.pubkey.clone(),
            pubkey_npub,
            created_at,
            relays,
            note: None,
            event_id,
        }))
    }

    fn rank_claims(
        &self,
        mut claims: Vec<NnsClaim>,
        selection: Option<&SelectionRecord>,
        now: i64,
    ) -> Result<ResolvedClaims, ResolverError> {
        let selected_pubkey = selection.map(|sel| sel.pubkey.as_str());
        claims.sort_by(|a, b| {
            let score_a = score_claim(a, selected_pubkey, now);
            let score_b = score_claim(b, selected_pubkey, now);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut iter = claims.into_iter();
        if let Some(primary) = iter.next() {
            let alternates = iter.collect();
            Ok(ResolvedClaims {
                primary,
                alternates,
            })
        } else {
            Err(ResolverError::NoClaims(String::new()))
        }
    }

    pub async fn record_selection(&self, name: &str, pubkey: &str) -> Result<(), ResolverError> {
        let storage = Arc::clone(&self.storage);
        let name = name.trim().to_ascii_lowercase();
        let pubkey = pubkey.to_string();
        let record = SelectionRecord {
            name: name.to_string(),
            pubkey: pubkey.clone(),
            chosen_at: unix_timestamp(),
        };
        let result = task::spawn_blocking(move || storage.record_selection(&record)).await?;
        result?;
        Ok(())
    }

    async fn persist_claims(
        &self,
        name: &str,
        claims: &[NnsClaim],
        now: i64,
    ) -> Result<(), ResolverError> {
        let storage = Arc::clone(&self.storage);
        let name = name.to_string();
        let records: Vec<ClaimRecord> = claims
            .iter()
            .map(|claim| ClaimRecord {
                name: name.clone(),
                pubkey: claim.pubkey_hex.clone(),
                ip: claim.socket_addr.to_string(),
                relays: claim.relays.iter().map(|url| url.to_string()).collect(),
                created_at: claim.created_at.as_u64() as i64,
                fetched_at: now,
                event_id: claim.event_id.to_hex(),
            })
            .collect();

        let result = task::spawn_blocking(move || -> Result<(), StorageError> {
            for record in records {
                storage.save_claim(&record)?;
            }
            Ok(())
        })
        .await?;
        result?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{NostrClient, RelayDirectory};
    use crate::storage::Storage;
    use ::url::Url;
    use futures_util::{SinkExt, StreamExt};
    use nostr_sdk::prelude::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
    use serde_json::{json, Value};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener as TokioTcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::tungstenite::Message;

    #[tokio::test]
    async fn resolves_event_from_mock_relay() {
        let http_server = start_http_server().await;
        let event = build_nns_event(http_server.addr);
        let relay_server = start_mock_relay(event).await;

        let temp_dir = TempDir::new().expect("temp dir");
        std::env::set_var("FRONTIER_DATA_DIR", temp_dir.path());

        let storage = Arc::new(Storage::new().expect("storage"));

        let relay_config_path = temp_dir.path().join("relays.yaml");
        std::fs::write(
            &relay_config_path,
            format!(
                "relays:
  - {}
",
                relay_server.url
            ),
        )
        .unwrap();

        let relay_directory = RelayDirectory::load(Some(relay_config_path)).unwrap();
        let resolver = NnsResolver::new(storage, relay_directory, NostrClient::new());

        let output = resolver.resolve("testsite").await.unwrap();
        assert_eq!(
            output.claims.primary.socket_addr, http_server.addr,
            "expected resolver to return mock address",
        );

        let _ = http_server.shutdown.send(());
        let _ = relay_server.shutdown.send(());
        let _ = http_server.handle.await;
        let _ = relay_server.handle.await;

        std::env::remove_var("FRONTIER_DATA_DIR");
    }

    struct HttpServer {
        addr: SocketAddr,
        shutdown: oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    }

    async fn start_http_server() -> HttpServer {
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let handle = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    accept = listener.accept() => {
                        if let Ok((mut stream, _)) = accept {
                            let body = b"<html><body><h1>hello</h1></body></html>";
                            let response = format!(
                                "HTTP/1.1 200 OK
Content-Length: {}
Content-Type: text/html
Connection: close

",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.write_all(body).await;
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
        url: Url,
        shutdown: oneshot::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    }

    async fn start_mock_relay(event: Event) -> RelayServer {
        let listener = TokioTcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let handle = tokio::spawn(async move {
            let shutdown_rx = shutdown_rx;
            if let Ok((stream, _)) = listener.accept().await {
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut sub_id: Option<String> = None;
                while let Some(msg) = ws.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                if value.get(0) == Some(&serde_json::Value::String("REQ".into())) {
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
            url: Url::parse(&format!("ws://{}", addr)).unwrap(),
            shutdown: shutdown_tx,
            handle,
        }
    }

    fn build_nns_event(http_addr: SocketAddr) -> Event {
        let keys = Keys::generate();
        let identifier_tag = Tag::identifier("testsite");
        let socket = format!("{}:{}", http_addr.ip(), http_addr.port());
        let ip_tag = Tag::parse(&["ip", socket.as_str()]).unwrap();
        EventBuilder::new(Kind::from(34256u16), "", vec![identifier_tag, ip_tag])
            .custom_created_at(Timestamp::now())
            .to_event(&keys)
            .unwrap()
    }
}
