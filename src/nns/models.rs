use std::collections::HashSet;
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
    #[error("event signature invalid: {0}")]
    InvalidSignature(String),
    #[error("failed to convert pubkey to bech32: {0}")]
    Bech32(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClaimLocation {
    DirectIp(SocketAddr),
    Blossom {
        root_hash: String,
        servers: Vec<Url>,
    },
}

#[derive(Clone, Debug)]
pub struct NnsClaim {
    #[allow(dead_code)]
    pub name: String,
    pub location: ClaimLocation,
    pub pubkey_hex: String,
    pub pubkey_npub: String,
    pub created_at: Timestamp,
    pub relays: HashSet<Url>,
    pub note: Option<String>,
    pub event_id: EventId,
    pub tls_pubkey: Option<String>,
    #[allow(dead_code)]
    pub tls_alg: Option<String>,
}

impl NnsClaim {
    pub fn score_key(&self) -> String {
        match &self.location {
            ClaimLocation::DirectIp(addr) => {
                format!("{}:direct:{}", self.pubkey_hex, addr)
            }
            ClaimLocation::Blossom { root_hash, .. } => {
                format!("{}:blossom:{}", self.pubkey_hex, root_hash)
            }
        }
    }

    pub fn from_event(expected_name: &str, relay_event: &RelayEvent) -> Result<Self, ModelError> {
        let event = &relay_event.event;

        let mut name: Option<String> = None;
        let mut ip: Option<String> = None;
        let mut blossom_hash: Option<String> = None;
        let mut servers: Vec<Url> = Vec::new();
        let mut note: Option<String> = None;
        let mut tls_pubkey: Option<String> = None;
        let mut tls_alg: Option<String> = None;

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
                "blossom" => {
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
                "note" => {
                    if let Some(value) = parts.get(1) {
                        note = Some(value.clone());
                    }
                }
                "tls-pubkey" => {
                    if let Some(value) = parts.get(1) {
                        tls_pubkey = Some(value.clone());
                    }
                }
                "tls-alg" => {
                    if let Some(value) = parts.get(1) {
                        tls_alg = Some(value.clone());
                    }
                }
                _ => {}
            }
        }

        let name = name.ok_or(ModelError::MissingName)?;
        if name != expected_name.to_ascii_lowercase() {
            return Err(ModelError::NameMismatch);
        }
        let location = if let Some(hash) = blossom_hash {
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
            location,
            pubkey_hex,
            pubkey_npub,
            created_at: event.created_at,
            relays: relay_event.relays.clone(),
            note,
            event_id: event.id,
            tls_pubkey,
            tls_alg,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedClaims {
    pub primary: NnsClaim,
    pub alternates: Vec<NnsClaim>,
}
