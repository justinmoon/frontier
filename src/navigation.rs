use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use ::url::Url;
use blitz_dom::net::Resource;
use blitz_net::Provider;
use blitz_traits::net::Request;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::blossom::{BlossomError, BlossomFetcher};
use crate::input::{parse_input, ParseInputError, ParsedInput};
use crate::js::processor;
use crate::nns::{
    ClaimLocation, NnsClaim, NnsResolver, PublishedTlsKey, ResolverError, ResolverOutput,
    ServiceEndpoint, TransportKind,
};
use crate::tls::SecureHttpClient;

pub(crate) const DEFAULT_BLOSSOM_PATHS: &[&str] = &["/index.html", "/index.htm", "/index", "/"];

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub source: FetchSource,
    pub display_url: String,
}

#[derive(Debug, Clone)]
pub enum FetchSource {
    LegacyUrl(Url),
    SecureHttp(SecureHttpRequest),
    Blossom(BlossomFetchRequest),
}

#[derive(Debug, Clone)]
pub struct SecureHttpEndpoint {
    pub url: Url,
    pub priority: u8,
}

#[derive(Debug, Clone)]
pub struct SecureHttpRequest {
    pub endpoints: Vec<SecureHttpEndpoint>,
    pub tls_key: Option<PublishedTlsKey>,
}

#[derive(Debug, Clone)]
pub struct BlossomFetchRequest {
    pub name: String,
    pub pubkey_hex: String,
    pub root_hash: String,
    pub servers: Vec<Url>,
    pub relays: Vec<Url>,
    pub path: String,
    pub tls_key: Option<PublishedTlsKey>,
    pub endpoints: Vec<ServiceEndpoint>,
}

#[derive(Debug, Clone)]
pub struct SelectionPrompt {
    pub name: String,
    pub display_url: String,
    pub options: Vec<NnsClaim>,
    pub default_index: usize,
    pub from_cache: bool,
    pub preferred_path: Option<String>,
}

#[derive(Debug, Clone)]
pub enum NavigationPlan {
    Fetch(FetchRequest),
    RequiresSelection(SelectionPrompt),
}

#[derive(Debug, Clone)]
pub struct FetchedDocument {
    pub base_url: String,
    pub contents: String,
    pub file_path: Option<PathBuf>,
    pub display_url: String,
    pub blossom: Option<BlossomDocumentContext>,
}

#[derive(Debug, Clone)]
pub struct BlossomDocumentContext {
    pub name: String,
    pub pubkey_hex: String,
    pub root_hash: String,
    pub servers: Vec<Url>,
    pub relays: Vec<Url>,
    pub tls_key: Option<PublishedTlsKey>,
    pub endpoints: Vec<ServiceEndpoint>,
}

#[derive(Debug, Error)]
pub enum NavigationError {
    #[error("failed to parse input: {0}")]
    Parse(#[from] ParseInputError),
    #[error("resolver error: {0}")]
    Resolver(#[from] ResolverError),
    #[error("unsupported input")]
    Unsupported,
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("network error: {0}")]
    Network(String),
    #[error("utf-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("file error: {0}")]
    File(#[from] std::io::Error),
    #[error("blossom error: {0}")]
    Blossom(#[from] BlossomError),
}

pub async fn prepare_navigation(
    raw_input: &str,
    resolver: Arc<NnsResolver>,
) -> Result<NavigationPlan, NavigationError> {
    let trimmed = raw_input.trim().to_string();
    let parsed = parse_input(raw_input)?;

    match parsed {
        ParsedInput::Url(url) | ParsedInput::DirectIp(url) => {
            let request = FetchRequest {
                source: FetchSource::LegacyUrl(url),
                display_url: if trimmed.is_empty() {
                    String::from("about:blank")
                } else {
                    trimmed
                },
            };
            Ok(NavigationPlan::Fetch(request))
        }
        ParsedInput::NnsName(name) => handle_nns_navigation(name, None, trimmed, resolver).await,
        ParsedInput::NnsPath { name, path } => {
            handle_nns_navigation(name, Some(path), trimmed, resolver).await
        }
    }
}

async fn handle_nns_navigation(
    name: String,
    preferred_path: Option<String>,
    display_url: String,
    resolver: Arc<NnsResolver>,
) -> Result<NavigationPlan, NavigationError> {
    let resolved = resolver.resolve(&name).await?;
    let SelectionData { request, prompt } = build_selection(
        name,
        display_url,
        preferred_path,
        resolved,
        Arc::clone(&resolver),
    )
    .await?;

    if let Some(request) = request {
        Ok(NavigationPlan::Fetch(request))
    } else if let Some(prompt) = prompt {
        Ok(NavigationPlan::RequiresSelection(prompt))
    } else {
        Err(NavigationError::Unsupported)
    }
}

pub async fn execute_fetch(
    request: &FetchRequest,
    net_provider: Arc<Provider<Resource>>,
    blossom: Arc<BlossomFetcher>,
) -> Result<FetchedDocument, FetchError> {
    match &request.source {
        FetchSource::LegacyUrl(url) => {
            fetch_legacy_url(url, &request.display_url, net_provider).await
        }
        FetchSource::SecureHttp(http_request) => {
            fetch_secure_http(http_request, &request.display_url).await
        }
        FetchSource::Blossom(blossom_request) => {
            fetch_blossom_document(blossom_request, &request.display_url, Arc::clone(&blossom))
                .await
        }
    }
}

async fn fetch_legacy_url(
    url: &Url,
    display_url: &str,
    net_provider: Arc<Provider<Resource>>,
) -> Result<FetchedDocument, FetchError> {
    if url.scheme() == "file" {
        return fetch_file_url(url, display_url);
    }

    let (tx, rx) = oneshot::channel();
    let fetch_url = url.clone();

    let req = Request::get(fetch_url);
    net_provider.fetch_with_callback(
        req,
        Box::new(move |result| match result {
            Ok((url, bytes)) => {
                tx.send(Ok((url, bytes))).ok();
            }
            Err(err) => {
                tx.send(Err(format!("{err:?}"))).ok();
            }
        }),
    );

    let received = rx.await.map_err(|e| FetchError::Network(e.to_string()))?;
    let (response_url, bytes) = received.map_err(FetchError::Network)?;

    let contents = std::str::from_utf8(&bytes)?.to_string();

    let mut document = FetchedDocument {
        base_url: response_url,
        contents,
        file_path: None,
        display_url: display_url.to_string(),
        blossom: None,
    };
    process_document_inline_scripts(&mut document);

    Ok(document)
}

async fn fetch_secure_http(
    request: &SecureHttpRequest,
    display_url: &str,
) -> Result<FetchedDocument, FetchError> {
    if request.endpoints.is_empty() {
        return Err(FetchError::Network("no endpoints".to_string()));
    }

    let client = SecureHttpClient::new(request.tls_key.as_ref())
        .map_err(|err| FetchError::Network(err.to_string()))?
        .client()
        .clone();

    let mut last_error: Option<reqwest::Error> = None;

    for endpoint in &request.endpoints {
        match client.get(endpoint.url.clone()).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(success) => {
                    let final_url = success.url().to_string();
                    let bytes = success
                        .bytes()
                        .await
                        .map_err(|e| FetchError::Network(e.to_string()))?;
                    let contents = std::str::from_utf8(&bytes)?.to_string();

                    let mut document = FetchedDocument {
                        base_url: final_url,
                        contents,
                        file_path: None,
                        display_url: display_url.to_string(),
                        blossom: None,
                    };
                    process_document_inline_scripts(&mut document);

                    return Ok(document);
                }
                Err(err) => {
                    last_error = Some(err);
                }
            },
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    let error = last_error
        .map(|err| err.to_string())
        .unwrap_or_else(|| "all endpoints failed".to_string());
    Err(FetchError::Network(error))
}

fn fetch_file_url(url: &Url, display_url: &str) -> Result<FetchedDocument, FetchError> {
    let path = url.to_file_path().map_err(|_| {
        FetchError::File(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid file URL",
        ))
    })?;

    if path.is_dir() {
        return Err(FetchError::File(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is a directory",
        )));
    }

    let base_url = url.as_str().to_string();
    let contents = std::fs::read_to_string(&path)?;

    let mut document = FetchedDocument {
        base_url,
        contents,
        file_path: Some(path),
        display_url: display_url.to_string(),
        blossom: None,
    };
    process_document_inline_scripts(&mut document);

    Ok(document)
}

struct SelectionData {
    request: Option<FetchRequest>,
    prompt: Option<SelectionPrompt>,
}

async fn build_selection(
    name: String,
    display_url: String,
    preferred_path: Option<String>,
    output: ResolverOutput,
    resolver: Arc<NnsResolver>,
) -> Result<SelectionData, NavigationError> {
    let display = if display_url.is_empty() {
        name.clone()
    } else {
        display_url
    };
    let mut options = Vec::with_capacity(1 + output.claims.alternates.len());
    options.push(output.claims.primary.clone());
    options.extend(output.claims.alternates.clone());

    let normalized_name = name.to_ascii_lowercase();

    if options.is_empty() {
        return Err(NavigationError::Unsupported);
    }

    if options.len() == 1 {
        let claim = options[0].clone();
        resolver
            .record_selection(&normalized_name, &claim.pubkey_hex)
            .await
            .map_err(NavigationError::Resolver)?;
        let fetch_request = claim_fetch_request_with_path(
            &claim,
            display.clone(),
            &normalized_name,
            preferred_path.as_deref(),
        )?;
        return Ok(SelectionData {
            request: Some(fetch_request),
            prompt: None,
        });
    }

    if let Some(selection) = output.selection.as_ref() {
        if let Some(claim) = options.iter().find(|claim| {
            claim.pubkey_hex == selection.pubkey || claim.pubkey_npub == selection.pubkey
        }) {
            resolver
                .record_selection(&normalized_name, &claim.pubkey_hex)
                .await
                .map_err(NavigationError::Resolver)?;
            let fetch_request = claim_fetch_request_with_path(
                claim,
                display.clone(),
                &normalized_name,
                preferred_path.as_deref(),
            )?;
            return Ok(SelectionData {
                request: Some(fetch_request),
                prompt: None,
            });
        }
    }

    let default_index = output
        .selection
        .as_ref()
        .and_then(|selection| {
            options.iter().position(|claim| {
                claim.pubkey_hex == selection.pubkey || claim.pubkey_npub == selection.pubkey
            })
        })
        .unwrap_or(0);
    Ok(SelectionData {
        request: None,
        prompt: Some(SelectionPrompt {
            name: normalized_name,
            display_url: display,
            options,
            default_index,
            from_cache: output.from_cache,
            preferred_path,
        }),
    })
}

fn build_secure_http_request(claim: &NnsClaim) -> Result<SecureHttpRequest, NavigationError> {
    let mut endpoints = Vec::new();
    for endpoint in &claim.endpoints {
        if matches!(endpoint.transport, TransportKind::Https) {
            let url = Url::parse(&format!("https://{}", endpoint.socket_addr))
                .map_err(|_| NavigationError::Unsupported)?;
            endpoints.push(SecureHttpEndpoint {
                url,
                priority: endpoint.priority,
            });
        }
    }

    if endpoints.is_empty() {
        let ClaimLocation::DirectIp(addr) = &claim.location else {
            return Err(NavigationError::Unsupported);
        };
        let scheme = if claim.tls_key.is_some() {
            "https"
        } else {
            "http"
        };
        let url =
            Url::parse(&format!("{scheme}://{addr}")).map_err(|_| NavigationError::Unsupported)?;
        endpoints.push(SecureHttpEndpoint { url, priority: 0 });
    }

    endpoints.sort_by(|a, b| a.priority.cmp(&b.priority));

    Ok(SecureHttpRequest {
        endpoints,
        tls_key: claim.tls_key.clone(),
    })
}

pub(crate) fn claim_fetch_request_with_path(
    claim: &NnsClaim,
    display_url: String,
    name: &str,
    preferred_path: Option<&str>,
) -> Result<FetchRequest, NavigationError> {
    let source = match &claim.location {
        ClaimLocation::DirectIp(_) => {
            let request = build_secure_http_request(claim)?;
            FetchSource::SecureHttp(request)
        }
        ClaimLocation::Blossom { root_hash, servers } => {
            let servers = servers.clone();
            let relays = claim.relays.iter().cloned().collect();
            let path = preferred_path
                .map(normalize_blossom_path)
                .unwrap_or_else(|| DEFAULT_BLOSSOM_PATHS[0].to_string());
            FetchSource::Blossom(BlossomFetchRequest {
                name: name.to_string(),
                pubkey_hex: claim.pubkey_hex.clone(),
                root_hash: root_hash.clone(),
                servers,
                relays,
                path,
                tls_key: claim.tls_key.clone(),
                endpoints: claim.endpoints.clone(),
            })
        }
        ClaimLocation::LegacyUrl(url) => {
            let url = if let Some(path) = preferred_path {
                let normalized = normalize_http_path(path);
                url.join(&normalized)
                    .map_err(|_| NavigationError::Unsupported)?
            } else {
                url.clone()
            };
            FetchSource::LegacyUrl(url)
        }
    };
    Ok(FetchRequest {
        source,
        display_url,
    })
}

async fn fetch_blossom_document(
    request: &BlossomFetchRequest,
    display_url: &str,
    blossom: Arc<BlossomFetcher>,
) -> Result<FetchedDocument, FetchError> {
    let manifest = blossom
        .manifest_for(&request.pubkey_hex, &request.relays)
        .await?;

    let mut raw_candidates: Vec<String> = Vec::new();
    raw_candidates.push(request.path.clone());
    if let Some(entry) = manifest.find_by_hash(&request.root_hash) {
        raw_candidates.push(entry.path.clone());
    }
    raw_candidates.extend(DEFAULT_BLOSSOM_PATHS.iter().map(|s| s.to_string()));
    let candidates = dedupe_candidates(raw_candidates);

    let mut last_error: Option<BlossomError> = None;

    for path in candidates {
        let Some(entry) = manifest.get(&path).cloned() else {
            continue;
        };

        match blossom
            .fetch_blob_by_hash_with_tls(&request.servers, &entry.hash, request.tls_key.as_ref())
            .await
        {
            Ok(bytes) => {
                let contents = std::str::from_utf8(&bytes)?.to_string();
                let normalized = entry.path.clone();
                let base_url = format!(
                    "blossom://{pubkey}{path}",
                    pubkey = request.pubkey_hex,
                    path = normalized
                );
                let context = BlossomDocumentContext {
                    name: request.name.clone(),
                    pubkey_hex: request.pubkey_hex.clone(),
                    root_hash: request.root_hash.clone(),
                    servers: request.servers.clone(),
                    relays: request.relays.clone(),
                    tls_key: request.tls_key.clone(),
                    endpoints: request.endpoints.clone(),
                };
                let mut document = FetchedDocument {
                    base_url,
                    contents,
                    file_path: None,
                    display_url: display_url.to_string(),
                    blossom: Some(context),
                };
                process_document_inline_scripts(&mut document);
                return Ok(document);
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    match last_error {
        Some(err) => Err(FetchError::from(err)),
        None => Err(FetchError::Blossom(BlossomError::MissingPath(
            request.path.clone(),
        ))),
    }
}

fn normalize_blossom_path(path: &str) -> String {
    if path.trim().is_empty() {
        return "/".to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

fn normalize_http_path(path: &str) -> String {
    if path.trim().is_empty() {
        return "/".to_string();
    }
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    }
}

fn dedupe_candidates(initial: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(initial.len());
    for path in initial {
        let normalized = normalize_blossom_path(&path);
        if seen.insert(normalized.clone()) {
            deduped.push(normalized);
        }
    }
    deduped
}

fn process_document_inline_scripts(document: &mut FetchedDocument) {
    match processor::execute_inline_scripts(document) {
        Ok(Some(summary)) => {
            tracing::info!(
                target = "quickjs",
                scripts = summary.executed_scripts,
                dom_mutations = summary.dom_mutations,
                url = %document.base_url,
                "processed inline scripts"
            );
        }
        Ok(None) => {}
        Err(err) => {
            tracing::error!(
                target = "quickjs",
                url = %document.base_url,
                error = %err,
                "failed to process inline scripts"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::url::Url;

    #[test]
    fn file_fetch_executes_inline_scripts() {
        let asset_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/quickjs-demo.html");
        let file_url = Url::from_file_path(&asset_path).expect("file url");

        let document = fetch_file_url(&file_url, file_url.as_str()).expect("file fetch");

        assert!(document.contents.contains("Hello from QuickJS!"));
        assert!(document.contents.contains("data-origin=\"quickjs-demo\""));
    }
}
