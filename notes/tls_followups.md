# TLS follow-ups

- NostrClient still uses nostr-sdk's default TLS connector. Wire up the new `tls::websocket_connector` helper so runtime relay subscriptions benefit from pinning.
- Secure HTTP fetchers currently build fresh clients per request. We could pool clients by key to reuse connections when we start layering caching.
- Consider exposing helper conversions so UI can display TLS fingerprint strings without recomputing in multiple places.
