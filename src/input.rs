use std::net::SocketAddr;

use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedInput {
    Url(Url),
    DirectIp(Url),
    NnsName(String),
    NnsPath { name: String, path: String },
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

    if let Some((name_part, remainder)) = trimmed.split_once('/') {
        let normalized_name = name_part.to_ascii_lowercase();
        if normalized_name.chars().all(is_valid_name_char) {
            let normalized_path = if remainder.is_empty() {
                String::from('/')
            } else {
                normalize_path_component(remainder)
            };
            return Ok(ParsedInput::NnsPath {
                name: normalized_name,
                path: normalized_path,
            });
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

fn normalize_path_component(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    let mut normalized = path.trim().to_string();
    while normalized.starts_with('/') {
        normalized.remove(0);
    }
    format!("/{}", normalized)
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

    #[test]
    fn parses_nns_with_path() {
        let parsed = parse_input("justinmoon/about.html").unwrap();
        match parsed {
            ParsedInput::NnsPath { name, path } => {
                assert_eq!(name, "justinmoon");
                assert_eq!(path, "/about.html");
            }
            _ => panic!("expected nns path"),
        }
    }
}
