use std::collections::HashSet;
use std::net::SocketAddr;

use ::url::Url;
use nostr_sdk::prelude::*;
use serde::Serialize;
use thiserror::Error;

use crate::net::RelayEvent;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("missing d tag")]
    MissingName,
    #[error("name mismatch")]
    NameMismatch,
    #[error("missing ip tag")]
    MissingIp,
    #[error("invalid ip value: {0}")]
    InvalidIp(String),
    #[error("event signature invalid: {0}")]
    InvalidSignature(String),
    #[error("failed to convert pubkey to bech32: {0}")]
    Bech32(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct NnsClaim {
    pub name: String,
    pub socket_addr: SocketAddr,
    pub pubkey_hex: String,
    pub pubkey_npub: String,
    pub created_at: Timestamp,
    pub relays: HashSet<Url>,
    pub note: Option<String>,
    pub event_id: EventId,
}

impl NnsClaim {
    pub fn score_key(&self) -> String {
        format!("{}:{}", self.pubkey_hex, self.socket_addr)
    }

    pub fn from_event(expected_name: &str, relay_event: &RelayEvent) -> Result<Self, ModelError> {
        let event = &relay_event.event;

        let mut name: Option<String> = None;
        let mut ip: Option<String> = None;
        let mut note: Option<String> = None;

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
                "note" => {
                    if let Some(value) = parts.get(1) {
                        note = Some(value.clone());
                    }
                }
                _ => {}
            }
        }

        let name = name.ok_or(ModelError::MissingName)?;
        if name != expected_name.to_ascii_lowercase() {
            return Err(ModelError::NameMismatch);
        }
        let ip = ip.ok_or(ModelError::MissingIp)?;
        let socket_addr: SocketAddr = ip.parse().map_err(|_| ModelError::InvalidIp(ip.clone()))?;

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
            socket_addr,
            pubkey_hex,
            pubkey_npub,
            created_at: event.created_at,
            relays: relay_event.relays.clone(),
            note,
            event_id: event.id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedClaims {
    pub primary: NnsClaim,
    pub alternates: Vec<NnsClaim>,
}
