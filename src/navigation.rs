use std::path::PathBuf;
use std::sync::Arc;

use ::url::Url;
use blitz_dom::net::Resource;
use blitz_net::Provider;
use blitz_traits::net::Request;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::input::{parse_input, ParseInputError, ParsedInput};
use crate::js::processor;
use crate::js::script::{ScriptDescriptor, ScriptExecution, ScriptKind, ScriptSource};

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub source: FetchSource,
    pub display_url: String,
}

#[derive(Debug, Clone)]
pub enum FetchSource {
    Url(Url),
}

#[derive(Debug, Clone)]
pub enum NavigationPlan {
    Fetch(FetchRequest),
}

#[derive(Debug, Clone)]
pub struct FetchedDocument {
    pub base_url: String,
    pub contents: String,
    pub file_path: Option<PathBuf>,
    pub display_url: String,
    pub scripts: Vec<ScriptDescriptor>,
}

#[derive(Debug, Error)]
pub enum NavigationError {
    #[error("failed to parse input: {0}")]
    Parse(#[from] ParseInputError),
    #[error("unsupported input")]
    #[allow(dead_code)]
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

pub async fn prepare_navigation(raw_input: &str) -> Result<NavigationPlan, NavigationError> {
    let trimmed = raw_input.trim().to_string();
    let parsed = parse_input(raw_input)?;

    match parsed {
        ParsedInput::Url(url) | ParsedInput::DirectIp(url) => {
            let request = FetchRequest {
                source: FetchSource::Url(url),
                display_url: if trimmed.is_empty() {
                    String::from("about:blank")
                } else {
                    trimmed
                },
            };
            Ok(NavigationPlan::Fetch(request))
        }
    }
}

pub async fn execute_fetch(
    request: &FetchRequest,
    net_provider: Arc<Provider<Resource>>,
) -> Result<FetchedDocument, FetchError> {
    let mut document = match &request.source {
        FetchSource::Url(url) => {
            fetch_url(url, &request.display_url, Arc::clone(&net_provider)).await?
        }
    };

    hydrate_blocking_scripts(&mut document, net_provider).await;

    Ok(document)
}

async fn fetch_url(
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
        scripts: Vec::new(),
    };
    collect_document_scripts(&mut document);

    Ok(document)
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
        scripts: Vec::new(),
    };
    collect_document_scripts(&mut document);

    Ok(document)
}

fn collect_document_scripts(document: &mut FetchedDocument) {
    let scripts = match processor::collect_scripts(&document.contents) {
        Ok(scripts) => scripts,
        Err(err) => {
            tracing::error!(
                target = "quickjs",
                url = %document.base_url,
                error = %err,
                "failed to collect scripts"
            );
            return;
        }
    };

    document.scripts = scripts;
}

async fn hydrate_blocking_scripts(
    document: &mut FetchedDocument,
    net_provider: Arc<Provider<Resource>>,
) {
    if document.scripts.is_empty() {
        return;
    }

    let base_url = Url::parse(&document.base_url).ok();

    for descriptor in document.scripts.iter_mut() {
        if descriptor.execution != ScriptExecution::Blocking
            || descriptor.kind != ScriptKind::Classic
        {
            continue;
        }

        let src = match &descriptor.source {
            ScriptSource::Inline { .. } => continue,
            ScriptSource::External { src } => src.clone(),
        };

        let resolved = match resolve_script_url(&src, base_url.as_ref()) {
            Ok(url) => url,
            Err(err) => {
                tracing::error!(
                    target = "quickjs",
                    src = %src,
                    error = %err,
                    "failed to resolve external script URL"
                );
                continue;
            }
        };

        match fetch_script_source(&resolved, Arc::clone(&net_provider)).await {
            Ok(code) => {
                descriptor.source = ScriptSource::Inline { code };
            }
            Err(err) => {
                tracing::error!(
                    target = "quickjs",
                    url = %resolved,
                    error = %err,
                    "failed to fetch blocking script"
                );
            }
        }
    }
}

fn resolve_script_url(src: &str, base: Option<&Url>) -> Result<Url, url::ParseError> {
    match Url::parse(src) {
        Ok(url) => Ok(url),
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            if let Some(base) = base {
                base.join(src)
            } else {
                Err(url::ParseError::RelativeUrlWithoutBase)
            }
        }
        Err(err) => Err(err),
    }
}

async fn fetch_script_source(
    url: &Url,
    net_provider: Arc<Provider<Resource>>,
) -> Result<String, FetchError> {
    let (_final_url, bytes) = net_provider
        .fetch_async(Request::get(url.clone()))
        .await
        .map_err(|err| FetchError::Network(format!("{err:?}")))?;
    let code = std::str::from_utf8(&bytes)?.to_string();
    Ok(code)
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

        assert_eq!(document.scripts.len(), 1);
        assert!(matches!(
            document.scripts[0].source,
            crate::js::script::ScriptSource::Inline { .. }
        ));
        assert!(document.contents.contains("<script>"));
    }
}
