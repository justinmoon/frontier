use crate::nns::NnsClaim;

pub fn score_claim(claim: &NnsClaim, selected_pubkey: Option<&str>, now: i64) -> f32 {
    let mut score = 1.0f32;
    if claim.relays.len() > 1 {
        score += 0.5;
    }
    if let Some(selected) = selected_pubkey {
        if selected == claim.pubkey_hex || selected == claim.pubkey_npub {
            score += 0.5;
        }
    }

    let age_seconds = now.saturating_sub(claim.created_at.as_u64() as i64);
    let decay = if age_seconds <= 0 {
        1.0
    } else {
        ((86_400f32 - age_seconds.min(86_400) as f32) / 86_400f32).max(0.0)
    };

    score + decay
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nns::{ClaimLocation, NnsClaim};
    use ::url::Url;
    use nostr_sdk::prelude::*;
    use std::collections::HashSet;
    use std::net::SocketAddr;

    fn claim(created_at: i64, relays: usize) -> NnsClaim {
        let mut relay_set = HashSet::new();
        for idx in 0..relays {
            relay_set.insert(Url::parse(&format!("wss://relay{idx}.example")).unwrap());
        }
        NnsClaim {
            name: "test".into(),
            location: ClaimLocation::DirectIp("127.0.0.1:8080".parse::<SocketAddr>().unwrap()),
            pubkey_hex: "deadbeef".into(),
            pubkey_npub: "npub1deadbeef".into(),
            created_at: Timestamp::from(created_at as u64),
            relays: relay_set,
            note: None,
            event_id: EventId::from_hex("f".repeat(64)).unwrap(),
            tls_pubkey: None,
            tls_alg: None,
        }
    }

    #[test]
    fn bonus_for_multiple_relays() {
        let c1 = claim(1, 1);
        let c2 = claim(1, 2);
        let now = 10;
        assert!(score_claim(&c2, None, now) > score_claim(&c1, None, now));
    }

    #[test]
    fn bonus_for_recent() {
        let new = claim(90, 1);
        let old = claim(1, 1);
        let now = 100;
        assert!(score_claim(&new, None, now) > score_claim(&old, None, now));
    }

    #[test]
    fn bonus_for_selection() {
        let c = claim(1, 1);
        let now = 2;
        assert!(score_claim(&c, Some("deadbeef"), now) > score_claim(&c, None, now));
    }
}
