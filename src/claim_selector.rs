use crate::nns::NnsClaim;

/// Generate HTML for selecting between multiple NNS claims
#[allow(dead_code)]
pub fn generate_claim_selector_html(name: &str, claims: &[NnsClaim]) -> String {
    let mut claim_rows = String::new();

    for (idx, claim) in claims.iter().enumerate() {
        let pubkey_short = format!(
            "{}...{}",
            &claim.pubkey_npub[..8],
            &claim.pubkey_npub[claim.pubkey_npub.len() - 8..]
        );
        let timestamp = claim.created_at.as_u64();

        claim_rows.push_str(&format!(
            r#"
            <tr class="claim-row" data-idx="{idx}">
                <td class="claim-ip">{}</td>
                <td class="claim-pubkey">{}</td>
                <td class="claim-time">{}</td>
                <td class="claim-action">
                    <form method="GET" action="nns://select-claim" style="margin: 0;">
                        <input type="hidden" name="name" value="{}" />
                        <input type="hidden" name="pubkey" value="{}" />
                        <input type="hidden" name="ip" value="{}" />
                        <button type="submit" class="select-button">Select</button>
                    </form>
                </td>
            </tr>
            "#,
            claim.socket_addr, pubkey_short, timestamp, name, claim.pubkey_hex, claim.socket_addr
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Select NNS Claim for "{}"</title>
    <style>
        * {{
            box-sizing: border-box;
        }}

        html, body {{
            margin: 0;
            padding: 0;
            width: 100%;
            height: 100%;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
            background: #f6f8fa;
        }}

        .container {{
            max-width: 1000px;
            margin: 0 auto;
            padding: 40px 20px;
        }}

        h1 {{
            font-size: 32px;
            font-weight: 600;
            color: #24292f;
            margin-bottom: 8px;
        }}

        .subtitle {{
            font-size: 16px;
            color: #57606a;
            margin-bottom: 32px;
        }}

        .info-box {{
            background: #fff;
            border: 1px solid #d0d7de;
            border-radius: 6px;
            padding: 16px;
            margin-bottom: 24px;
        }}

        .info-box p {{
            margin: 8px 0;
            color: #24292f;
            line-height: 1.5;
        }}

        table {{
            width: 100%;
            background: white;
            border: 1px solid #d0d7de;
            border-radius: 6px;
            border-collapse: separate;
            border-spacing: 0;
        }}

        thead {{
            background: #f6f8fa;
        }}

        th {{
            padding: 12px 16px;
            text-align: left;
            font-weight: 600;
            font-size: 14px;
            color: #24292f;
            border-bottom: 1px solid #d0d7de;
        }}

        th:first-child {{
            border-top-left-radius: 6px;
        }}

        th:last-child {{
            border-top-right-radius: 6px;
        }}

        td {{
            padding: 12px 16px;
            font-size: 14px;
            color: #24292f;
            border-bottom: 1px solid #d0d7de;
        }}

        tr:last-child td {{
            border-bottom: none;
        }}

        .claim-row:hover {{
            background: #f6f8fa;
        }}

        .claim-ip {{
            font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
            color: #0969da;
        }}

        .claim-pubkey {{
            font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace;
            color: #57606a;
            font-size: 12px;
        }}

        .claim-time {{
            color: #57606a;
        }}

        .select-button {{
            background: #2da44e;
            color: white;
            border: 1px solid rgba(27, 31, 36, 0.15);
            border-radius: 6px;
            padding: 5px 16px;
            font-size: 14px;
            font-weight: 500;
            cursor: pointer;
            transition: background 0.2s;
        }}

        .select-button:hover {{
            background: #2c974b;
        }}

        .select-button:active {{
            background: #298e46;
        }}

        .count-badge {{
            display: inline-block;
            background: #0969da;
            color: white;
            padding: 2px 8px;
            border-radius: 12px;
            font-size: 12px;
            font-weight: 600;
            margin-left: 8px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Multiple claims found for "{}"<span class="count-badge">{}</span></h1>
        <p class="subtitle">Choose which claim to trust for this name</p>

        <div class="info-box">
            <p><strong>What is this?</strong></p>
            <p>Multiple people have published claims to the name "{}". This is normal in the Nostr Name System (NNS).</p>
            <p>Select the claim you want to trust. Your choice will be remembered for future visits.</p>
        </div>

        <table>
            <thead>
                <tr>
                    <th>IP Address</th>
                    <th>Publisher (Pubkey)</th>
                    <th>Published At (Unix Time)</th>
                    <th>Action</th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        name,
        name,
        claims.len(),
        name,
        claim_rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::prelude::*;
    use std::collections::HashSet;

    #[test]
    fn test_generate_claim_selector_html() {
        let keys1 = Keys::generate();
        let keys2 = Keys::generate();

        let claims = vec![
            NnsClaim {
                name: "testsite".to_string(),
                socket_addr: "192.168.1.100:8080".parse().unwrap(),
                pubkey_hex: keys1.public_key().to_hex(),
                pubkey_npub: keys1.public_key().to_bech32().unwrap(),
                created_at: Timestamp::from(1000),
                relays: HashSet::new(),
                note: None,
                event_id: EventId::all_zeros(),
            },
            NnsClaim {
                name: "testsite".to_string(),
                socket_addr: "10.0.0.5:8080".parse().unwrap(),
                pubkey_hex: keys2.public_key().to_hex(),
                pubkey_npub: keys2.public_key().to_bech32().unwrap(),
                created_at: Timestamp::from(2000),
                relays: HashSet::new(),
                note: None,
                event_id: EventId::all_zeros(),
            },
        ];

        let html = generate_claim_selector_html("testsite", &claims);

        // Verify the HTML contains expected elements
        assert!(html.contains("testsite"));
        assert!(html.contains("192.168.1.100:8080"));
        assert!(html.contains("10.0.0.5:8080"));
        assert!(html.contains("Select"));
    }
}
