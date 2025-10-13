use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use blitz_dom::net::Resource;
use blitz_dom::{local_name, qual_name, BaseDocument, Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_traits::events::{
    BlitzImeEvent, BlitzInputEvent, BlitzMouseButtonEvent, DomEvent, DomEventData,
    MouseEventButton, MouseEventButtons, UiEvent,
};
use blitz_traits::net::DummyNetCallback;
use tokio::time::sleep;
use url::Url;

use crate::js::runtime_document::RuntimeDocument;
use crate::js::script::{ScriptExecution, ScriptKind, ScriptSource};
use crate::js::session::JsPageRuntime;
use crate::navigation::{self, FetchError, FetchRequest, FetchSource};

/// Utility for creating headless DOM sessions backed by the QuickJS runtime.
#[derive(Clone)]
pub struct HeadlessSessionBuilder {
    base_dir: PathBuf,
}

impl Default for HeadlessSessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessSessionBuilder {
    pub fn new() -> Self {
        Self {
            base_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        }
    }

    pub fn with_base_dir(mut self, base: PathBuf) -> Self {
        self.base_dir = base;
        self
    }

    pub async fn open_file(self, path: impl AsRef<Path>) -> Result<HeadlessSession> {
        let joined = self.base_dir.join(path);
        let url = Url::from_file_path(&joined)
            .map_err(|_| anyhow!("invalid file path: {}", joined.display()))?;
        HeadlessSession::navigate_url(url).await
    }
}

pub struct HeadlessSession {
    runtime: JsPageRuntime,
    document: RuntimeDocument,
    net_provider: Arc<Provider<Resource>>,
    current_url: Url,
}

impl HeadlessSession {
    pub async fn navigate(url: &str) -> Result<Self> {
        let parsed = Url::parse(url).context("invalid url for headless session")?;
        Self::navigate_url(parsed).await
    }

    async fn navigate_url(url: Url) -> Result<Self> {
        let net = Arc::new(Provider::new(Arc::new(DummyNetCallback)));
        let request = FetchRequest {
            source: FetchSource::Url(url.clone()),
            display_url: url.to_string(),
        };
        let mut fetched = navigation::execute_fetch(&request, Arc::clone(&net))
            .await
            .context("execute fetch")?;

        hydrate_blocking_scripts(&mut fetched, Arc::clone(&net)).await;

        let scripts = fetched.scripts.clone();
        let mut runtime =
            JsPageRuntime::new(&fetched.contents, &scripts, Some(fetched.base_url.as_str()))
                .context("create js runtime")?
                .ok_or_else(|| anyhow!("document contained no executable scripts"))?;

        let mut html_doc = HtmlDocument::from_html(
            &fetched.contents,
            DocumentConfig {
                base_url: Some(fetched.base_url.clone()),
                ..Default::default()
            },
        );
        runtime.attach_document(&mut html_doc);
        if let Some(summary) = runtime
            .run_blocking_scripts()
            .context("execute inline scripts")?
        {
            tracing::info!(
                target = "automation",
                url = %fetched.base_url,
                scripts = summary.executed_scripts,
                dom_mutations = summary.dom_mutations,
                "headless executed scripts"
            );
        }
        runtime.environment().pump().context("initial pump")?;

        let environment = runtime.environment();
        let runtime_document = RuntimeDocument::new(html_doc, environment.clone());

        Ok(Self {
            runtime,
            document: runtime_document,
            net_provider: net,
            current_url: url,
        })
    }

    pub fn document_html(&self) -> Result<String> {
        self.runtime
            .environment()
            .document_html()
            .context("serialize document")
    }

    pub async fn click(&mut self, selector: &str) -> Result<()> {
        let node_id = self.node_id(selector)?;
        let chain = self.document.node_chain(node_id);
        let event = DomEvent::new(
            node_id,
            DomEventData::Click(BlitzMouseButtonEvent {
                x: 0.0,
                y: 0.0,
                button: MouseEventButton::Main,
                buttons: MouseEventButtons::Primary,
                mods: Default::default(),
            }),
        );
        let environment = self.runtime.environment();
        environment.reattach_document(&mut self.document);
        environment
            .dispatch_dom_event(&event, &chain)
            .context("dispatch click")?;
        self.pump_for(Duration::from_millis(25)).await;
        Ok(())
    }

    pub fn inner_text(&mut self, selector: &str) -> Result<String> {
        let node_id = self.node_id(selector)?;
        Ok(self
            .document
            .get_node(node_id)
            .map(|node| node.text_content())
            .unwrap_or_default())
    }

    pub async fn send_keys(&mut self, selector: &str, text: &str) -> Result<()> {
        let node_id = self.node_id(selector)?;
        self.document.set_focus_to(node_id);

        for ch in text.chars() {
            self.document
                .handle_ui_event(UiEvent::Ime(BlitzImeEvent::Commit(ch.to_string())));
            self.pump_for(Duration::from_millis(10)).await;
        }

        Ok(())
    }

    pub async fn type_text(&mut self, selector: &str, text: &str) -> Result<()> {
        self.send_keys(selector, text).await
    }

    pub async fn clear(&mut self, selector: &str) -> Result<()> {
        self.set_input_value(selector, "")?;
        self.pump_for(Duration::from_millis(10)).await;
        Ok(())
    }

    pub fn element_value(&mut self, selector: &str) -> Result<String> {
        let element_id = selector
            .strip_prefix('#')
            .ok_or_else(|| anyhow!("only id selectors are supported (got {selector})"))?;

        let script = format!(
            "(function() {{\n                const el = document.getElementById({element_id:?});\n                if (!el) return '';\n                const value = el.value;\n                return typeof value === 'string' ? value : '';\n            }})()"
        );

        self.runtime
            .environment()
            .eval_with::<Option<String>>(&script, "webdriver-element-value.js")
            .context("evaluate element value script")?
            .ok_or_else(|| anyhow!("element id not found: {selector}"))
    }

    pub fn element_attribute(&mut self, selector: &str, attribute: &str) -> Result<Option<String>> {
        let node_id = self.node_id(selector)?;
        Ok(self
            .document
            .get_node(node_id)
            .and_then(|node| node.element_data())
            .and_then(|element| {
                element
                    .attrs()
                    .iter()
                    .find(|attr| attr.name.local.as_ref() == attribute)
                    .map(|attr| attr.value.to_string())
            }))
    }

    pub fn eval(&mut self, script: &str) -> Result<()> {
        let environment = self.runtime.environment();
        environment.reattach_document(&mut self.document);
        environment
            .eval(script, "headless-eval.js")
            .context("eval script")
    }

    pub fn eval_with<V>(&mut self, script: &str) -> Result<V>
    where
        V: for<'js> rquickjs::FromJs<'js>,
    {
        let environment = self.runtime.environment();
        environment.reattach_document(&mut self.document);
        environment
            .eval_with(script, "headless-eval.js")
            .context("eval script")
    }

    pub(crate) fn ensure_selector(&mut self, selector: &str) -> Result<()> {
        let _ = self.node_id(selector)?;
        Ok(())
    }

    pub async fn navigate_to(&mut self, url: &str) -> Result<()> {
        let mut session = HeadlessSession::navigate(url).await?;
        std::mem::swap(self, &mut session);
        Ok(())
    }

    pub async fn navigate_relative(&mut self, relative: &str) -> Result<()> {
        let joined = self
            .current_url
            .join(relative)
            .context("join relative url")?;
        *self = HeadlessSession::navigate_url(joined).await?;
        Ok(())
    }

    pub async fn pump_for(&mut self, duration: Duration) {
        let iterations = (duration.as_millis() / 10).max(1) as usize;
        for _ in 0..iterations {
            if let Err(err) = self.runtime.environment().pump() {
                tracing::error!(target = "automation", error = %err, "pump failure");
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    fn node_id(&mut self, selector: &str) -> Result<usize> {
        let id = selector
            .strip_prefix('#')
            .ok_or_else(|| anyhow!("only id selectors are supported (got {selector})"))?;
        lookup_node_id(&mut self.document, id).ok_or_else(|| anyhow!("element id not found: {id}"))
    }

    fn set_input_value(&mut self, selector: &str, value: &str) -> Result<()> {
        let node_id = self.node_id(selector)?;
        {
            let mut mutator = self.document.mutate();
            mutator.set_attribute(node_id, qual_name!("value", html), value);
        }
        let chain = self.document.node_chain(node_id);
        let event = DomEvent::new(
            node_id,
            DomEventData::Input(BlitzInputEvent {
                value: value.to_string(),
            }),
        );
        let environment = self.runtime.environment();
        environment.reattach_document(&mut self.document);
        environment
            .dispatch_dom_event(&event, &chain)
            .context("dispatch input event")?;
        Ok(())
    }

    pub fn current_url(&self) -> &Url {
        &self.current_url
    }

    pub fn net_provider(&self) -> Arc<Provider<Resource>> {
        Arc::clone(&self.net_provider)
    }
}

fn lookup_node_id<T>(document: &mut T, target_id: &str) -> Option<usize>
where
    T: std::ops::DerefMut<Target = BaseDocument>,
{
    let mut result = None;
    let root = document.root_node().id;
    document.iter_subtree_mut(root, |node_id, doc| {
        if result.is_some() {
            return;
        }
        if let Some(node) = doc.get_node(node_id) {
            if node.attr(local_name!("id")) == Some(target_id) {
                result = Some(node_id);
            }
        }
    });
    result
}

async fn hydrate_blocking_scripts(
    document: &mut navigation::FetchedDocument,
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
            Ok(code) => descriptor.source = ScriptSource::Inline { code },
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
        .fetch_async(blitz_traits::net::Request::get(url.clone()))
        .await
        .map_err(|err| FetchError::Network(format!("{err:?}")))?;
    let code = std::str::from_utf8(&bytes)?.to_string();
    Ok(code)
}
