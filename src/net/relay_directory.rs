use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;
use url::Url;

const DEFAULT_RELAYS: &[&str] = &["wss://relay.damus.io", "wss://nos.lol"];

#[derive(Debug, Error)]
pub enum RelayDirectoryError {
    #[error("failed to read relay config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse relay URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("failed to parse YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[derive(Debug, Deserialize)]
struct RelayConfig {
    relays: Vec<String>,
}

#[derive(Clone)]
pub struct RelayDirectory {
    relays: Vec<Url>,
}

impl RelayDirectory {
    pub fn load(config_path: Option<PathBuf>) -> Result<Self, RelayDirectoryError> {
        let relays = if let Some(path) = config_path {
            if path.exists() {
                let contents = fs::read_to_string(path)?;
                let cfg: RelayConfig = serde_yaml::from_str(&contents)?;
                cfg.relays
            } else {
                DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
            }
        } else {
            DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
        };

        let relays = relays
            .into_iter()
            .map(|url| Url::parse(&url))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { relays })
    }

    pub fn relays(&self) -> &[Url] {
        &self.relays
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_default() {
        let directory = RelayDirectory::load(None).unwrap();
        assert!(!directory.relays().is_empty());
    }

    #[test]
    fn loads_from_file() {
        let mut file = NamedTempFile::new().unwrap();
        use std::io::Write;
        writeln!(
            file,
            "relays:\n  - wss://relay.example\n  - wss://relay.other"
        )
        .unwrap();
        let directory = RelayDirectory::load(Some(file.path().to_path_buf())).unwrap();
        assert_eq!(directory.relays().len(), 2);
        assert!(directory.relays()[0]
            .as_str()
            .starts_with("wss://relay.example"));
    }
}
