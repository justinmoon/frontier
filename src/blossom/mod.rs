use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::url::Url;
use directories::ProjectDirs;
use nostr_sdk::prelude::*;
use rustls::ClientConfig;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::net::{NostrCertVerifier, NostrClient, NostrClientError, RelayDirectory};

const MANIFEST_TTL: Duration = Duration::from_secs(300);
const CACHE_SUBDIR: &str = "blossom-cache";

#[derive(Debug, Error)]
pub enum BlossomError {
    #[error("nostr error: {0}")]
    Nostr(#[from] NostrClientError),
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("invalid hash: {0}")]
    InvalidHash(String),
    #[error("path {0} not found in manifest")]
    MissingPath(String),
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),
    #[error("manifest error: {0}")]
    Manifest(String),
}

#[derive(Clone, Debug)]
pub struct BlossomManifestEntry {
    pub path: String,
    pub hash: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Default)]
pub struct BlossomManifest {
    entries: HashMap<String, BlossomManifestEntry>,
}

impl BlossomManifest {
    pub fn insert(&mut self, entry: BlossomManifestEntry) {
        match self.entries.get_mut(&entry.path) {
            Some(existing) => {
                if entry.created_at > existing.created_at {
                    *existing = entry;
                }
            }
            None => {
                self.entries.insert(entry.path.clone(), entry);
            }
        }
    }

    pub fn get(&self, path: &str) -> Option<&BlossomManifestEntry> {
        let normalized = normalize_path(path);
        self.entries.get(&normalized)
    }

    pub fn find_by_hash(&self, hash: &str) -> Option<&BlossomManifestEntry> {
        self.entries
            .values()
            .find(|entry| entry.hash.eq_ignore_ascii_case(hash))
    }
}

struct CachedManifest {
    manifest: BlossomManifest,
    fetched_at: Instant,
}

pub struct BlossomFetcher {
    client: NostrClient,
    relay_directory: RelayDirectory,
    http: reqwest::Client,
    cache_dir: PathBuf,
    manifest_cache: RwLock<HashMap<String, CachedManifest>>,
    timeout: Duration,
}

impl BlossomFetcher {
    pub fn new(relay_directory: RelayDirectory) -> Result<Self, BlossomError> {
        let cache_dir = resolve_cache_dir()?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self {
            client: NostrClient::new(),
            relay_directory,
            http,
            cache_dir,
            manifest_cache: RwLock::new(HashMap::new()),
            timeout: Duration::from_secs(5),
        })
    }

    #[allow(dead_code)]
    pub async fn fetch_document(
        &self,
        pubkey_hex: &str,
        relays: &[Url],
        servers: &[Url],
        path: &str,
    ) -> Result<(Vec<u8>, BlossomManifestEntry), BlossomError> {
        let manifest = self.manifest_for(pubkey_hex, relays).await?;
        let entry = manifest
            .get(path)
            .cloned()
            .ok_or_else(|| BlossomError::MissingPath(path.to_string()))?;
        let bytes = self.fetch_blob(servers, &entry.hash, None).await?;
        Ok((bytes, entry))
    }

    #[allow(dead_code)]
    pub async fn fetch_blob_by_hash(
        &self,
        servers: &[Url],
        hash: &str,
    ) -> Result<Vec<u8>, BlossomError> {
        self.fetch_blob(servers, hash, None).await
    }

    pub async fn fetch_blob_by_hash_with_tls(
        &self,
        servers: &[Url],
        hash: &str,
        tls_pubkey: Option<&str>,
    ) -> Result<Vec<u8>, BlossomError> {
        self.fetch_blob(servers, hash, tls_pubkey).await
    }

    pub async fn manifest_for(
        &self,
        pubkey_hex: &str,
        relays: &[Url],
    ) -> Result<BlossomManifest, BlossomError> {
        if let Some(manifest) = self.manifest_from_cache(pubkey_hex).await {
            return Ok(manifest);
        }

        let manifest = self.fetch_manifest_from_relays(pubkey_hex, relays).await?;
        self.store_manifest(pubkey_hex, manifest.clone()).await;
        Ok(manifest)
    }

    async fn manifest_from_cache(&self, pubkey_hex: &str) -> Option<BlossomManifest> {
        let guard = self.manifest_cache.read().await;
        guard.get(pubkey_hex).and_then(|cached| {
            if cached.fetched_at.elapsed() <= MANIFEST_TTL {
                Some(cached.manifest.clone())
            } else {
                None
            }
        })
    }

    async fn store_manifest(&self, pubkey_hex: &str, manifest: BlossomManifest) {
        let mut guard = self.manifest_cache.write().await;
        guard.insert(
            pubkey_hex.to_string(),
            CachedManifest {
                manifest,
                fetched_at: Instant::now(),
            },
        );
    }

    async fn fetch_manifest_from_relays(
        &self,
        pubkey_hex: &str,
        extra_relays: &[Url],
    ) -> Result<BlossomManifest, BlossomError> {
        let pubkey = PublicKey::from_hex(pubkey_hex)
            .map_err(|_| BlossomError::InvalidPubkey(pubkey_hex.to_string()))?;

        let mut relays: Vec<Url> = self.relay_directory.relays().to_vec();
        for relay in extra_relays {
            if !relays.iter().any(|url| url == relay) {
                relays.push(relay.clone());
            }
        }

        let filter = Filter::new()
            .kind(Kind::from(34128u16))
            .author(pubkey)
            .limit(500);

        let events = self
            .client
            .fetch_events(&relays, filter, self.timeout)
            .await?;

        let mut manifest = BlossomManifest::default();
        for relay_event in events {
            match parse_manifest_event(&relay_event.event) {
                Ok(Some(entry)) => {
                    manifest.insert(entry);
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(
                        pubkey = %pubkey_hex,
                        error = %err,
                        "skipping invalid blossom manifest event"
                    );
                }
            }
        }

        Ok(manifest)
    }

    async fn fetch_blob(
        &self,
        servers: &[Url],
        hash: &str,
        tls_pubkey: Option<&str>,
    ) -> Result<Vec<u8>, BlossomError> {
        validate_hash(hash)?;

        if let Some(bytes) = self.try_cache(hash)? {
            return Ok(bytes);
        }

        let mut last_error: Option<BlossomError> = None;
        for server in servers {
            match self.try_fetch_from_server(server, hash, tls_pubkey).await {
                Ok(bytes) => {
                    self.persist_cache(hash, &bytes).await?;
                    return Ok(bytes);
                }
                Err(err) => {
                    tracing::warn!(server = %server, hash, error = %err, "failed to fetch blossom blob");
                    last_error = Some(err);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| BlossomError::MissingPath(hash.to_string())))
    }

    fn cache_path(&self, hash: &str) -> PathBuf {
        let mut path = self.cache_dir.clone();
        let split = hash.len().min(2);
        let (prefix, suffix) = hash.split_at(split);
        path.push(prefix);
        path.push(suffix);
        path
    }

    fn try_cache(&self, hash: &str) -> Result<Option<Vec<u8>>, BlossomError> {
        let path = self.cache_path(hash);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        verify_hash(hash, &bytes)?;
        Ok(Some(bytes))
    }

    async fn persist_cache(&self, hash: &str, bytes: &[u8]) -> Result<(), BlossomError> {
        let path = self.cache_path(hash);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, bytes).await?;
        Ok(())
    }

    async fn try_fetch_from_server(
        &self,
        server: &Url,
        hash: &str,
        tls_pubkey: Option<&str>,
    ) -> Result<Vec<u8>, BlossomError> {
        let url = server
            .join(hash)
            .map_err(|e| BlossomError::Manifest(e.to_string()))?;

        // Use custom TLS verification if pubkey is provided
        let response = if let Some(pubkey) = tls_pubkey {
            let verifier = Arc::new(NostrCertVerifier::new(pubkey.to_string()));
            let config = ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth();

            let client = reqwest::Client::builder()
                .use_preconfigured_tls(config)
                .timeout(Duration::from_secs(10))
                .build()?;

            client.get(url).send().await?
        } else {
            self.http.get(url).send().await?
        };

        if !response.status().is_success() {
            return Err(BlossomError::Manifest(format!(
                "server {} returned status {}",
                server,
                response.status()
            )));
        }
        let bytes = response.bytes().await?.to_vec();
        verify_hash(hash, &bytes)?;
        Ok(bytes)
    }
}

fn parse_manifest_event(event: &Event) -> Result<Option<BlossomManifestEntry>, BlossomError> {
    let mut path: Option<String> = None;
    let mut hash: Option<String> = None;

    for tag in &event.tags {
        let values = tag.as_vec();
        if values.is_empty() {
            continue;
        }
        match values[0].as_str() {
            "d" => {
                if let Some(value) = values.get(1) {
                    path = Some(normalize_path(value));
                }
            }
            "sha256" => {
                if let Some(value) = values.get(1) {
                    hash = Some(value.clone());
                }
            }
            _ => {}
        }
    }

    if path.is_none() || hash.is_none() {
        return Ok(None);
    }

    let hash = hash.unwrap();
    validate_hash(&hash)?;

    event
        .verify()
        .map_err(|e| BlossomError::Manifest(format!("invalid signature: {e}")))?;

    Ok(Some(BlossomManifestEntry {
        path: path.unwrap(),
        hash,
        created_at: event.created_at,
    }))
}

fn verify_hash(expected: &str, bytes: &[u8]) -> Result<(), BlossomError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let computed = hasher.finalize();
    let actual = format!("{:x}", computed);
    if expected == actual {
        Ok(())
    } else {
        Err(BlossomError::HashMismatch {
            expected: expected.to_string(),
            actual,
        })
    }
}

fn validate_hash(hash: &str) -> Result<(), BlossomError> {
    if hash.len() < 10
        || !hash.len().is_multiple_of(2)
        || !hash.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(BlossomError::InvalidHash(hash.to_string()));
    }
    Ok(())
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

fn resolve_cache_dir() -> Result<PathBuf, BlossomError> {
    if let Ok(dir) = std::env::var("FRONTIER_DATA_DIR") {
        let mut path = PathBuf::from(dir);
        path.push(CACHE_SUBDIR);
        return Ok(path);
    }

    if let Some(dirs) = ProjectDirs::from("org", "Frontier", "FrontierBrowser") {
        let mut data_dir = dirs.data_dir().to_path_buf();
        data_dir.push(CACHE_SUBDIR);
        Ok(data_dir)
    } else {
        Err(BlossomError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "data directory not available",
        )))
    }
}
