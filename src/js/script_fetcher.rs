use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use blitz_dom::net::Resource;
use blitz_net::Provider;
use blitz_traits::net::Request;
use tokio::sync::{oneshot, RwLock};
use url::Url;

use super::script::{ScriptDescriptor, ScriptSource};

/// Cache key for scripts: combination of origin and content hash
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
#[allow(dead_code)] // Used in tests
struct ScriptCacheKey {
    origin: String,
    path: String,
}

/// Cached script content
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used in tests
struct CachedScript {
    content: String,
}

/// Manages fetching and caching of external scripts
#[allow(dead_code)] // Used in tests
pub struct ScriptFetcher {
    net_provider: Arc<Provider<Resource>>,
    cache: Arc<RwLock<HashMap<ScriptCacheKey, CachedScript>>>,
}

impl ScriptFetcher {
    #[allow(dead_code)] // Used in tests
    pub fn new(net_provider: Arc<Provider<Resource>>) -> Self {
        Self {
            net_provider,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Fetch a script from a URL, using cache if available
    #[allow(dead_code)] // Used in tests
    pub async fn fetch_script(&self, script_url: &str, base_url: &str) -> Result<String> {
        // Resolve relative URLs
        let absolute_url = resolve_url(script_url, base_url)?;
        let parsed_url = Url::parse(&absolute_url)
            .with_context(|| format!("invalid script URL: {}", absolute_url))?;

        // Create cache key
        let cache_key = ScriptCacheKey {
            origin: format!(
                "{}://{}",
                parsed_url.scheme(),
                parsed_url.host_str().unwrap_or("")
            ),
            path: parsed_url.path().to_string(),
        };

        // Check cache first
        {
            let cache_read = self.cache.read().await;
            if let Some(cached) = cache_read.get(&cache_key) {
                tracing::debug!(target: "script_fetch", url = %absolute_url, "cache hit");
                return Ok(cached.content.clone());
            }
        }

        // Fetch from network
        tracing::debug!(target: "script_fetch", url = %absolute_url, "fetching");
        let content = self.fetch_from_network(&parsed_url).await?;

        // Store in cache
        {
            let mut cache_write = self.cache.write().await;
            cache_write.insert(
                cache_key,
                CachedScript {
                    content: content.clone(),
                },
            );
        }

        Ok(content)
    }

    /// Fetch all external scripts for a list of descriptors
    #[allow(dead_code)] // Used in tests
    pub async fn fetch_all_external(
        &self,
        scripts: &[ScriptDescriptor],
        base_url: &str,
    ) -> Result<Vec<(usize, String)>> {
        let mut fetched = Vec::new();

        for descriptor in scripts {
            if let ScriptSource::External { src } = &descriptor.source {
                match self.fetch_script(src, base_url).await {
                    Ok(content) => {
                        fetched.push((descriptor.index, content));
                    }
                    Err(e) => {
                        tracing::error!(
                            target: "script_fetch",
                            src = %src,
                            error = %e,
                            "failed to fetch external script"
                        );
                        // Continue with other scripts even if one fails
                    }
                }
            }
        }

        Ok(fetched)
    }

    #[allow(dead_code)] // Used in tests
    async fn fetch_from_network(&self, url: &Url) -> Result<String> {
        let (tx, rx) = oneshot::channel();
        let fetch_url = url.clone();

        let req = Request::get(fetch_url);
        self.net_provider.fetch_with_callback(
            req,
            Box::new(move |result| match result {
                Ok((_url, bytes)) => {
                    tx.send(Ok(bytes)).ok();
                }
                Err(err) => {
                    tx.send(Err(format!("{err:?}"))).ok();
                }
            }),
        );

        let received = rx.await.map_err(|_| anyhow!("network provider dropped"))?;
        let bytes = received.map_err(|e| anyhow!("network error: {}", e))?;

        let text = String::from_utf8(bytes.to_vec())
            .with_context(|| format!("script at {} is not valid UTF-8", url))?;

        Ok(text)
    }
}

#[allow(dead_code)] // Used in tests
fn resolve_url(script_url: &str, base_url: &str) -> Result<String> {
    // If it's already an absolute URL, use it as-is
    if script_url.starts_with("http://")
        || script_url.starts_with("https://")
        || script_url.starts_with("file://")
    {
        return Ok(script_url.to_string());
    }

    // Parse base URL and join with script URL
    let base = Url::parse(base_url).with_context(|| format!("invalid base URL: {}", base_url))?;

    let resolved = base.join(script_url).with_context(|| {
        format!(
            "failed to resolve URL: {} relative to {}",
            script_url, base_url
        )
    })?;

    Ok(resolved.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_url_absolute() {
        let result = resolve_url(
            "https://example.com/script.js",
            "https://base.com/page.html",
        );
        assert_eq!(result.unwrap(), "https://example.com/script.js");
    }

    #[test]
    fn test_resolve_url_relative() {
        let result = resolve_url("script.js", "https://base.com/page.html");
        assert_eq!(result.unwrap(), "https://base.com/script.js");
    }

    #[test]
    fn test_resolve_url_relative_path() {
        let result = resolve_url("../lib/script.js", "https://base.com/app/page.html");
        assert_eq!(result.unwrap(), "https://base.com/lib/script.js");
    }

    #[test]
    fn test_resolve_url_absolute_path() {
        let result = resolve_url("/assets/script.js", "https://base.com/app/page.html");
        assert_eq!(result.unwrap(), "https://base.com/assets/script.js");
    }
}
