use std::path::PathBuf;
use std::sync::Arc;

use ::url::Url;
use blitz_dom::net::Resource;
use blitz_net::Provider;
use blitz_traits::net::Request;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::input::{parse_input, ParseInputError, ParsedInput};
use crate::nns::{NnsClaim, NnsResolver, ResolverError, ResolverOutput};

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub fetch_url: Url,
    pub display_url: String,
}

#[derive(Debug, Clone)]
pub struct SelectionPrompt {
    pub name: String,
    pub display_url: String,
    pub options: Vec<NnsClaim>,
    pub default_index: usize,
    pub from_cache: bool,
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
}

pub async fn prepare_navigation(
    raw_input: &str,
    resolver: Arc<NnsResolver>,
) -> Result<NavigationPlan, NavigationError> {
    let parsed = parse_input(raw_input)?;

    match parsed {
        ParsedInput::Url(url) | ParsedInput::DirectIp(url) => {
            let display = raw_input.trim().to_string();
            let request = FetchRequest {
                fetch_url: url,
                display_url: if display.is_empty() {
                    String::from("about:blank")
                } else {
                    display
                },
            };
            Ok(NavigationPlan::Fetch(request))
        }
        ParsedInput::NnsName(name) => {
            let output = resolver.resolve(&name).await?;
            let SelectionData { request, prompt } =
                build_selection(name, output, Arc::clone(&resolver)).await?;

            if let Some(request) = request {
                Ok(NavigationPlan::Fetch(request))
            } else if let Some(prompt) = prompt {
                Ok(NavigationPlan::RequiresSelection(prompt))
            } else {
                Err(NavigationError::Unsupported)
            }
        }
    }
}

pub async fn execute_fetch(
    request: &FetchRequest,
    net_provider: Arc<Provider<Resource>>,
) -> Result<FetchedDocument, FetchError> {
    if request.fetch_url.scheme() == "file" {
        return fetch_file_url(request);
    }

    let (tx, rx) = oneshot::channel();
    let fetch_url = request.fetch_url.clone();

    let req = Request::get(fetch_url.clone());
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

    Ok(FetchedDocument {
        base_url: response_url,
        contents,
        file_path: None,
        display_url: request.display_url.clone(),
    })
}

fn fetch_file_url(request: &FetchRequest) -> Result<FetchedDocument, FetchError> {
    let path = request.fetch_url.to_file_path().map_err(|_| {
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

    let base_url = request.fetch_url.as_str().to_string();
    let contents = std::fs::read_to_string(&path)?;

    Ok(FetchedDocument {
        base_url,
        contents,
        file_path: Some(path),
        display_url: request.display_url.clone(),
    })
}

struct SelectionData {
    request: Option<FetchRequest>,
    prompt: Option<SelectionPrompt>,
}

async fn build_selection(
    name: String,
    output: ResolverOutput,
    resolver: Arc<NnsResolver>,
) -> Result<SelectionData, NavigationError> {
    let display_url = name.clone();
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
        let fetch_url = claim_url(&claim);
        return Ok(SelectionData {
            request: Some(FetchRequest {
                fetch_url,
                display_url,
            }),
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
            let fetch_url = claim_url(claim);
            return Ok(SelectionData {
                request: Some(FetchRequest {
                    fetch_url,
                    display_url,
                }),
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
            display_url,
            options,
            default_index,
            from_cache: output.from_cache,
        }),
    })
}

fn claim_url(claim: &NnsClaim) -> Url {
    Url::parse(&format!("http://{}", claim.socket_addr)).expect("valid socket addr")
}
