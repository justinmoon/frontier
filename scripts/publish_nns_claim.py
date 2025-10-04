#!/usr/bin/env python3
"""
Script to publish an NNS claim event to Nostr relays.
This creates a kind 34256 event claiming a name and mapping it to an IP:port.
"""

import json
import time
import hashlib
import argparse
from typing import Optional

# You'll need to install: pip install nostr-sdk
try:
    from nostr_sdk import Keys, Client, EventBuilder, Tag, Kind, Filter
except ImportError:
    print("Error: nostr-sdk not found. Install it with: pip install nostr-sdk")
    exit(1)

async def publish_nns_claim(
    name: str,
    ip_port: str,
    private_key: Optional[str] = None,
    relays: Optional[list] = None
):
    """
    Publish an NNS claim to Nostr relays.

    Args:
        name: The name being claimed (e.g., "mysite")
        ip_port: The IP:port to map to (e.g., "127.0.0.1:8080")
        private_key: Optional hex private key (will generate new one if not provided)
        relays: List of relay URLs to publish to
    """
    # Set up keys
    if private_key:
        keys = Keys.parse(private_key)
    else:
        keys = Keys.generate()
        print(f"Generated new keypair:")
        print(f"  Private key (hex): {keys.secret_key().to_hex()}")
        print(f"  Public key (hex): {keys.public_key().to_hex()}")
        print(f"  Save your private key to reuse it later!\n")

    # Create client
    client = Client(keys)

    # Add relays
    if not relays:
        relays = [
            "wss://relay.damus.io",
            "wss://nos.lol",
            "wss://relay.nostr.band",
        ]

    for relay in relays:
        await client.add_relay(relay)

    await client.connect()

    # Create the NNS event (kind 34256)
    # Using d tag for the name and ip tag for the IP:port
    tags = [
        Tag.parse(["d", name]),
        Tag.parse(["ip", ip_port]),
    ]

    event_builder = EventBuilder(
        Kind(34256),  # NNS kind
        "",  # Empty content
        tags
    )

    # Sign and publish
    event = client.sign_event_builder(event_builder)
    print(f"\n📝 Created NNS claim event:")
    print(f"  Event ID: {event.id().to_hex()}")
    print(f"  Name: {name}")
    print(f"  IP:Port: {ip_port}")
    print(f"  Pubkey: {event.author().to_hex()}\n")

    print("📡 Publishing to relays...")
    output = await client.send_event(event)
    print(f"✅ Published to {len(output)} relays")

    # Keep connection open briefly to ensure event is sent
    await asyncio.sleep(2)

    print("\n✨ Done! Your NNS claim is now published.")
    print(f"Try accessing it by entering '{name}' in the Frontier browser URL bar.")

if __name__ == '__main__':
    import asyncio

    parser = argparse.ArgumentParser(
        description="Publish an NNS claim to Nostr relays"
    )
    parser.add_argument(
        "name",
        help="The name to claim (e.g., 'mysite')"
    )
    parser.add_argument(
        "--ip",
        default="127.0.0.1:8080",
        help="The IP:port to map to (default: 127.0.0.1:8080)"
    )
    parser.add_argument(
        "--key",
        help="Your private key in hex format (will generate new one if not provided)"
    )
    parser.add_argument(
        "--relay",
        action="append",
        dest="relays",
        help="Relay URL to publish to (can be specified multiple times)"
    )

    args = parser.parse_args()

    asyncio.run(publish_nns_claim(
        name=args.name,
        ip_port=args.ip,
        private_key=args.key,
        relays=args.relays
    ))
