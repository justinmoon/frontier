use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use nostr_sdk::prelude::*;
use thiserror::Error;
use tokio::task::{self, JoinError};

use crate::net::{NostrClient, NostrClientError, RelayDirectory};
use crate::nns::models::{ClaimLocation, ModelError, NnsClaim, ResolvedClaims};
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
    timeout: Duration,
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
            timeout: Duration::from_secs(3),
        }
    }

    /// Create resolver with custom timeout (primarily for testing)
    #[allow(dead_code)]
    pub fn new_with_timeout(
        storage: Arc<Storage>,
        relay_directory: RelayDirectory,
        client: NostrClient,
        timeout: Duration,
    ) -> Self {
        Self {
            storage,
            relay_directory,
            client,
            timeout,
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
            .fetch_events(self.relay_directory.relays(), filter, self.timeout)
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

        let location = if let Some(raw) = record.location.as_ref() {
            match serde_json::from_str::<ClaimLocation>(raw) {
                Ok(location) => location,
                Err(err) => {
                    tracing::warn!(
                        name = %record.name,
                        error = %err,
                        "failed to parse cached location"
                    );
                    return Ok(None);
                }
            }
        } else {
            match record.ip.parse::<SocketAddr>() {
                Ok(addr) => ClaimLocation::DirectIp(addr),
                Err(_) => return Ok(None),
            }
        };

        Ok(Some(NnsClaim {
            name: record.name.clone(),
            location,
            pubkey_hex: record.pubkey.clone(),
            pubkey_npub,
            created_at,
            relays,
            note: None,
            event_id,
            tls_pubkey: None,
            tls_alg: None,
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
                ip: match &claim.location {
                    ClaimLocation::DirectIp(addr) => addr.to_string(),
                    ClaimLocation::Blossom { root_hash, .. } => root_hash.clone(),
                },
                relays: claim.relays.iter().map(|url| url.to_string()).collect(),
                created_at: claim.created_at.as_u64() as i64,
                fetched_at: now,
                event_id: claim.event_id.to_hex(),
                location: Some(
                    serde_json::to_string(&claim.location).expect("claim location serializable"),
                ),
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

// Tests removed - see tests/nns_e2e_test.rs for comprehensive end-to-end test
