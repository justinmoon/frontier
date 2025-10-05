use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;

use ::url::Url;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::net::RelayEvent;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("missing d tag")]
    MissingName,
    #[error("name mismatch")]
    NameMismatch,
    #[error("missing location tag (ip or blossom)")]
    MissingLocation,
    #[error("invalid ip value: {0}")]
    InvalidIp(String),
    #[error("missing blossom servers")]
    MissingServers,
    #[error("invalid blossom server url: {0}")]
    InvalidServer(String),
    #[error("invalid endpoint value: {0}")]
    InvalidEndpoint(String),
    #[error("unsupported endpoint transport: {0}")]
    UnsupportedTransport(String),
    #[error("invalid endpoint priority: {0}")]
    InvalidEndpointPriority(String),
    #[error("invalid tls pubkey: {0}")]
    InvalidTlsPubkey(String),
    #[error("unsupported tls algorithm: {0}")]
    UnsupportedTlsAlgorithm(String),
    #[error("event signature invalid: {0}")]
    InvalidSignature(String),
    #[error("failed to convert pubkey to bech32: {0}")]
    Bech32(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceKind {
    Site,
    BlossomSite,
    BlossomServer,
    Relay,
    Api,
    Other(String),
}

impl ServiceKind {
    fn from_tag(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "site" => ServiceKind::Site,
            "blossom-site" => ServiceKind::BlossomSite,
            "blossom-server" => ServiceKind::BlossomServer,
            "relay" => ServiceKind::Relay,
            "api" => ServiceKind::Api,
            _ => ServiceKind::Other(value.to_string()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TransportKind {
    Https,
    Wss,
}

impl TransportKind {
    fn from_str(raw: &str) -> Result<Self, ModelError> {
        match raw.to_ascii_lowercase().as_str() {
            "https" => Ok(TransportKind::Https),
            "wss" => Ok(TransportKind::Wss),
            _ => Err(ModelError::UnsupportedTransport(raw.to_string())),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceEndpoint {
    pub transport: TransportKind,
    pub socket_addr: String,
    pub priority: u8,
}

impl ServiceEndpoint {
    fn new(
        transport: TransportKind,
        socket_addr: &str,
        priority_raw: Option<&str>,
    ) -> Result<Self, ModelError> {
        socket_addr
            .parse::<SocketAddr>()
            .map_err(|_| ModelError::InvalidEndpoint(socket_addr.to_string()))?;
        let priority = if let Some(raw) = priority_raw {
            raw.parse::<u8>()
                .map_err(|_| ModelError::InvalidEndpointPriority(raw.to_string()))?
        } else {
            0
        };

        Ok(ServiceEndpoint {
            transport,
            socket_addr: socket_addr.to_string(),
            priority,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TlsAlgorithm {
    Ed25519,
}

impl TlsAlgorithm {
    pub(crate) fn from_tag(value: Option<&str>) -> Result<Self, ModelError> {
        match value.unwrap_or("ed25519").to_ascii_lowercase().as_str() {
            "ed25519" => Ok(TlsAlgorithm::Ed25519),
            _ => Err(ModelError::UnsupportedTlsAlgorithm(
                value.unwrap_or("ed25519").to_string(),
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TlsAlgorithm::Ed25519 => "ed25519",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedTlsKey {
    pub algorithm: TlsAlgorithm,
    pub spki: Vec<u8>,
}

impl PublishedTlsKey {
    pub fn new(algorithm: TlsAlgorithm, spki_hex: &str) -> Result<Self, ModelError> {
        if spki_hex.is_empty()
            || !spki_hex.len().is_multiple_of(2)
            || !spki_hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(ModelError::InvalidTlsPubkey(spki_hex.to_string()));
        }
        let spki = hex::decode(spki_hex)
            .map_err(|_| ModelError::InvalidTlsPubkey(spki_hex.to_string()))?;
        Ok(PublishedTlsKey { algorithm, spki })
    }

    pub fn spki_hex(&self) -> String {
        hex::encode(&self.spki)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClaimLocation {
    DirectIp(SocketAddr),
    Blossom {
        root_hash: String,
        servers: Vec<Url>,
    },
    LegacyUrl(Url),
}

#[derive(Clone, Debug)]
pub struct NnsClaim {
    #[allow(dead_code)]
    pub name: String,
    pub service_kind: Option<ServiceKind>,
    pub location: ClaimLocation,
    pub endpoints: Vec<ServiceEndpoint>,
    pub pubkey_hex: String,
    pub pubkey_npub: String,
    pub created_at: Timestamp,
    pub relays: HashSet<Url>,
    pub note: Option<String>,
    pub event_id: EventId,
    pub tls_key: Option<PublishedTlsKey>,
}

impl NnsClaim {
    pub fn score_key(&self) -> String {
        match &self.location {
            ClaimLocation::DirectIp(addr) => format!("{}:direct:{}", self.pubkey_hex, addr),
            ClaimLocation::Blossom { root_hash, .. } => {
                format!("{}:blossom:{}", self.pubkey_hex, root_hash)
            }
            ClaimLocation::LegacyUrl(url) => format!("{}:legacy:{}", self.pubkey_hex, url),
        }
    }

    pub fn tls_algorithm(&self) -> Option<&str> {
        self.tls_key.as_ref().map(|key| key.algorithm.as_str())
    }

    pub fn tls_spki_hex(&self) -> Option<String> {
        self.tls_key.as_ref().map(|key| key.spki_hex())
    }

    pub fn from_event(expected_name: &str, relay_event: &RelayEvent) -> Result<Self, ModelError> {
        let event = &relay_event.event;

        let mut name: Option<String> = None;
        let mut ip: Option<String> = None;
        let mut blossom_hash: Option<String> = None;
        let mut servers: Vec<Url> = Vec::new();
        let mut note: Option<String> = None;
        let mut tls_pubkey_hex: Option<String> = None;
        let mut tls_alg_raw: Option<String> = None;
        let mut service_kind: Option<ServiceKind> = None;
        let mut endpoint_map: HashMap<(TransportKind, String), ServiceEndpoint> = HashMap::new();
        let mut legacy_url: Option<Url> = None;

        for tag in event.tags.iter() {
            let parts = tag.as_vec();
            if parts.is_empty() {
                continue;
            }
            match parts[0].as_str() {
                "d" => {
                    if let Some(value) = parts.get(1) {
                        name = Some(value.to_ascii_lowercase());
                    }
                }
                "ip" => {
                    if let Some(value) = parts.get(1) {
                        ip = Some(value.clone());
                    }
                }
                "blossom" | "root" => {
                    if let Some(value) = parts.get(1) {
                        blossom_hash = Some(value.clone());
                    }
                }
                "server" => {
                    if let Some(value) = parts.get(1) {
                        match Url::parse(value) {
                            Ok(url) => servers.push(url),
                            Err(_) => return Err(ModelError::InvalidServer(value.clone())),
                        }
                    }
                }
                "endpoint" => {
                    let transport_raw = parts.get(1).map(|s| s.as_str()).unwrap_or("");
                    let socket_raw = parts.get(2).map(|s| s.as_str()).unwrap_or("");
                    let priority_raw = parts.get(3).map(|s| s.as_str());

                    if transport_raw.is_empty() || socket_raw.is_empty() {
                        return Err(ModelError::InvalidEndpoint(parts.join(",")));
                    }

                    let transport = TransportKind::from_str(transport_raw)?;
                    let socket = socket_raw.to_string();
                    let endpoint = ServiceEndpoint::new(transport.clone(), &socket, priority_raw)?;
                    let key = (transport, socket);

                    match endpoint_map.get_mut(&key) {
                        Some(existing) => {
                            if endpoint.priority < existing.priority {
                                *existing = endpoint;
                            }
                        }
                        None => {
                            endpoint_map.insert(key, endpoint);
                        }
                    }
                }
                "note" => {
                    if let Some(value) = parts.get(1) {
                        note = Some(value.clone());
                    }
                }
                "tls-pubkey" => {
                    if let Some(value) = parts.get(1) {
                        tls_pubkey_hex = Some(value.clone());
                    }
                }
                "tls-alg" => {
                    if let Some(value) = parts.get(1) {
                        tls_alg_raw = Some(value.clone());
                    }
                }
                "svc" => {
                    if let Some(value) = parts.get(1) {
                        service_kind = Some(ServiceKind::from_tag(value));
                    }
                }
                "url" | "legacy-url" => {
                    if let Some(value) = parts.get(1) {
                        if let Ok(url) = Url::parse(value) {
                            legacy_url = Some(url);
                        }
                    }
                }
                _ => {}
            }
        }

        let mut endpoints: Vec<ServiceEndpoint> = endpoint_map.into_values().collect();
        endpoints.sort_by(|a, b| a.priority.cmp(&b.priority));

        let tls_key = if let Some(pubkey_hex) = tls_pubkey_hex {
            let algorithm = TlsAlgorithm::from_tag(tls_alg_raw.as_deref())?;
            Some(PublishedTlsKey::new(algorithm, &pubkey_hex)?)
        } else {
            None
        };

        let name = name.ok_or(ModelError::MissingName)?;
        if name != expected_name.to_ascii_lowercase() {
            return Err(ModelError::NameMismatch);
        }

        let location = if let Some(url) = legacy_url {
            ClaimLocation::LegacyUrl(url)
        } else if let Some(hash) = blossom_hash {
            if servers.is_empty() {
                return Err(ModelError::MissingServers);
            }
            ClaimLocation::Blossom {
                root_hash: hash,
                servers,
            }
        } else if let Some(ip) = ip {
            let socket_addr: SocketAddr =
                ip.parse().map_err(|_| ModelError::InvalidIp(ip.clone()))?;
            ClaimLocation::DirectIp(socket_addr)
        } else if let Some(endpoint) = endpoints
            .iter()
            .find(|endpoint| matches!(endpoint.transport, TransportKind::Https))
        {
            let socket_addr = endpoint
                .socket_addr
                .parse::<SocketAddr>()
                .map_err(|_| ModelError::InvalidIp(endpoint.socket_addr.clone()))?;
            ClaimLocation::DirectIp(socket_addr)
        } else {
            return Err(ModelError::MissingLocation);
        };

        event
            .verify()
            .map_err(|e| ModelError::InvalidSignature(e.to_string()))?;

        let pubkey_hex = event.pubkey.to_string();
        let pubkey_npub = event
            .pubkey
            .to_bech32()
            .map_err(|e| ModelError::Bech32(e.to_string()))?;

        Ok(Self {
            name,
            service_kind,
            location,
            endpoints,
            pubkey_hex,
            pubkey_npub,
            created_at: event.created_at,
            relays: relay_event.relays.clone(),
            note,
            event_id: event.id,
            tls_key,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedClaims {
    pub primary: NnsClaim,
    pub alternates: Vec<NnsClaim>,
}
