use std::net::SocketAddr;

use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedInput {
    Url(Url),
    DirectIp(Url),
    NnsName(String),
}

#[derive(Debug, Error)]
pub enum ParseInputError {
    #[error("input is empty")]
    Empty,
    #[error("input could not be parsed as a URL")]
    InvalidUrl,
}

pub fn parse_input(raw: &str) -> Result<ParsedInput, ParseInputError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ParseInputError::Empty);
    }

    if let Ok(url) = Url::parse(trimmed) {
        match url.scheme() {
            "http" | "https" | "file" => {
                return Ok(ParsedInput::Url(url));
            }
            other if !other.is_empty() => {
                // If it has any other scheme treat as ordinary URL, we do not rewrite.
                return Ok(ParsedInput::Url(url));
            }
            _ => {}
        }
    }

    // Direct IP support - accept bare ip:port or ip without scheme.
    if trimmed.contains(':') {
        if let Ok(addr) = trimmed.parse::<SocketAddr>() {
            let url =
                Url::parse(&format!("http://{}", addr)).map_err(|_| ParseInputError::InvalidUrl)?;
            return Ok(ParsedInput::DirectIp(url));
        }
    }

    if trimmed.contains('.') || trimmed.contains('/') {
        let url =
            Url::parse(&format!("https://{trimmed}")).map_err(|_| ParseInputError::InvalidUrl)?;
        return Ok(ParsedInput::Url(url));
    }

    let lowered = trimmed.to_ascii_lowercase();
    if lowered.chars().all(is_valid_name_char) {
        Ok(ParsedInput::NnsName(lowered))
    } else {
        Err(ParseInputError::InvalidUrl)
    }
}

fn is_valid_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_url() {
        let parsed = parse_input("https://example.com").unwrap();
        match parsed {
            ParsedInput::Url(url) => assert_eq!(url.as_str(), "https://example.com/"),
            _ => panic!("expected URL"),
        }
    }

    #[test]
    fn parses_direct_ip() {
        let parsed = parse_input("127.0.0.1:8080").unwrap();
        match parsed {
            ParsedInput::DirectIp(url) => {
                assert_eq!(url.as_str(), "http://127.0.0.1:8080/")
            }
            _ => panic!("expected direct IP"),
        }
    }

    #[test]
    fn parses_nns_name() {
        let parsed = parse_input("JustInMoon").unwrap();
        match parsed {
            ParsedInput::NnsName(name) => assert_eq!(name, "justinmoon"),
            _ => panic!("expected nns"),
        }
    }

    #[test]
    fn rejects_invalid() {
        assert!(parse_input("???").is_err());
    }
}
