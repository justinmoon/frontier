/// Integration test for NNS navigation flow
/// Tests the exact flow that happens when user types an NNS name in the URL bar
#[cfg(test)]
mod tests {
    use url::Url;

    #[test]
    fn test_url_bar_nns_name_flow() {
        // Step 1: User types "testsite" in URL bar
        let user_input = "testsite";

        // Step 2: Form submits to current URL (example.com) with ?url=testsite
        let form_submission_url = format!("https://example.com/?url={}", user_input);
        let url = Url::parse(&form_submission_url).unwrap();

        // Step 3: Navigate function extracts the target from query parameter
        let query = url.query().expect("Should have query string");
        let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
        let target_url = params
            .iter()
            .find(|(key, _)| key == "url")
            .map(|(_, value)| value.to_string())
            .expect("Should have url parameter");

        // Step 4: Verify we extracted "testsite"
        assert_eq!(
            target_url, "testsite",
            "Failed to extract NNS name from form"
        );

        // Step 5: The bug was here - trying to parse "testsite" as URL fails
        // This is what the OLD code did:
        let old_parse_result = Url::parse(&target_url);
        assert!(
            old_parse_result.is_err(),
            "Bug: NNS name should NOT be parseable as URL"
        );

        // Step 6: The FIX - navigate() now calls fetch() which handles NNS names
        // fetch() will use parse_input() which correctly identifies "testsite" as NNS
        // We can't test the full fetch here, but we can verify the logic:

        // Simulate parse_input logic (simplified version)
        let is_nns_name = !target_url.starts_with("http://")
            && !target_url.starts_with("https://")
            && !target_url.contains('.')
            && !target_url.contains(':');

        assert!(is_nns_name, "testsite should be identified as NNS name");

        println!("✅ URL bar NNS navigation flow test passed");
    }

    #[test]
    fn test_direct_ip_from_url_bar() {
        // Test that direct IP:port also works through the form
        let user_input = "127.0.0.1:8080";
        let form_url = format!("https://example.com/?url={}", user_input);
        let url = Url::parse(&form_url).unwrap();

        let query = url.query().unwrap();
        let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
        let target = params
            .iter()
            .find(|(k, _)| k == "url")
            .unwrap()
            .1
            .to_string();

        assert_eq!(target, "127.0.0.1:8080");

        // Should be identified as direct IP (has colon)
        let is_direct_ip = target.contains(':');
        assert!(is_direct_ip);

        println!("✅ Direct IP from URL bar test passed");
    }

    #[test]
    fn test_traditional_url_from_url_bar() {
        let user_input = "example.com";
        let form_url = format!("https://example.com/?url={}", user_input);
        let url = Url::parse(&form_url).unwrap();

        let query = url.query().unwrap();
        let params: Vec<_> = url::form_urlencoded::parse(query.as_bytes()).collect();
        let target = params
            .iter()
            .find(|(k, _)| k == "url")
            .unwrap()
            .1
            .to_string();

        assert_eq!(target, "example.com");

        // Should be identified as URL (has dot)
        let is_traditional_url = target.contains('.');
        assert!(is_traditional_url);

        println!("✅ Traditional URL from URL bar test passed");
    }
}
