use anyhow::{Context as AnyhowContext, Result as AnyResult};
use blitz_dom::{local_name, BaseDocument, DocumentConfig, LocalName};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_traits::events::{
    BlitzMouseButtonEvent, DomEvent, DomEventData, MouseEventButton, MouseEventButtons,
};
use blitz_traits::net::DummyNetCallback;
use frontier::js::environment::JsDomEnvironment;
use frontier::js::processor;
use frontier::js::script::{ScriptDescriptor, ScriptSource};
use frontier::js::session::JsPageRuntime;
use frontier::navigation::{self, FetchRequest, FetchSource, FetchedDocument};
use keyboard_types::Modifiers;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::time::sleep;
use url::Url;

#[test]
fn quickjs_demo_executes_script_and_mutates_dom() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = std::fs::read_to_string("assets/quickjs-demo.html").expect("demo asset");
        let scripts = processor::collect_scripts(&html).expect("collect scripts");
        assert_eq!(scripts.len(), 1, "demo asset contains one inline script");

        let mut runtime = JsPageRuntime::new(&html, &scripts)
            .expect("create runtime")
            .expect("runtime available for scripts");
        let mut runtime_doc = HtmlDocument::from_html(&html, DocumentConfig::default());
        runtime.attach_document(&mut runtime_doc);
        let runtime_summary = runtime
            .run_blocking_scripts()
            .expect("runtime execution")
            .expect("runtime executed script");
        assert!(runtime_summary.executed_scripts > 0);

        let mutated = runtime.document_html().expect("serialize runtime dom");
        assert!(mutated.contains("Hello from QuickJS!"));
        assert!(mutated.contains("data-origin=\"quickjs-demo\""));

        let mut document = FetchedDocument {
            base_url: "file://demo".into(),
            contents: html,
            file_path: None,
            display_url: "file://demo/quickjs-demo.html".into(),
            scripts: scripts.clone(),
        };
        let summary = processor::execute_inline_scripts(&mut document)
            .expect("processor execution")
            .expect("processor ran script");

        assert_eq!(summary.executed_scripts, runtime_summary.executed_scripts);
        assert!(document.contents.contains("Hello from QuickJS!"));
        assert!(document.contents.contains("data-origin=\"quickjs-demo\""));
    });
}

#[test]
fn dom_bridge_updates_live_document() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = "<!DOCTYPE html><html><body><h1 id=\"message\">Loading…</h1></body></html>";

        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());

        environment.attach_document(&mut document);
        environment
            .eval(
                "document.getElementById('message').textContent = 'Updated';",
                "bridge-test.js",
            )
            .expect("evaluate script");

        let mut updated = None;
        {
            let base: &mut BaseDocument = &mut document;
            let root_id = base.root_node().id;
            base.iter_subtree_mut(root_id, |node_id, doc| {
                if updated.is_some() {
                    return;
                }
                if let Some(node) = doc.get_node(node_id) {
                    if node.attr(local_name!("id")) == Some("message") {
                        updated = Some(node.text_content());
                    }
                }
            });
        }

        assert_eq!(updated.as_deref(), Some("Updated"));
    });
}

#[test]
fn dom_event_listener_runs_and_prevents_default() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = r#"
        <!DOCTYPE html>
        <html>
            <body>
                <button id="btn">Press</button>
                <span id="status">idle</span>
            </body>
        </html>
    "#;

        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
        environment.attach_document(&mut document);

        environment
            .eval(
                r#"
                const button = document.getElementById('btn');
                const status = document.getElementById('status');
                button.addEventListener('click', (event) => {
                    status.textContent = 'clicked';
                    event.preventDefault();
                });
            "#,
                "event-listener.js",
            )
            .expect("register listener");

        let button_id = lookup_node_id(&mut document, "btn").expect("button id");
        let chain = {
            let base: &BaseDocument = &document;
            base.node_chain(button_id)
        };

        let event = DomEvent::new(
            button_id,
            DomEventData::Click(BlitzMouseButtonEvent {
                x: 0.0,
                y: 0.0,
                button: MouseEventButton::Main,
                buttons: MouseEventButtons::Primary,
                mods: Modifiers::default(),
            }),
        );

        let outcome = environment
            .dispatch_dom_event(&event, &chain)
            .expect("dispatch result");
        assert!(outcome.default_prevented);

        let status_id = lookup_node_id(&mut document, "status").expect("status id");
        let text_after = {
            let base: &BaseDocument = &document;
            base.get_node(status_id)
                .expect("status node")
                .text_content()
        };
        assert_eq!(text_after, "clicked");
    });
}

#[test]
fn timers_execute_after_delay() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = r#"
            <!DOCTYPE html>
            <html><body><div id="root">idle</div></body></html>
        "#;

        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
        environment.attach_document(&mut document);

        environment
            .eval(
                r#"
                    const root = document.getElementById('root');
                    setTimeout(() => {
                        root.textContent = 'done';
                    }, 5);
                "#,
                "timer.js",
            )
            .expect("evaluate script");

        environment.pump().expect("initial pump");
        sleep(Duration::from_millis(10)).await;
        environment.pump().expect("timer pump");

        let root_id = lookup_node_id(&mut document, "root").expect("root id");
        let text = {
            let base: &BaseDocument = &document;
            base.get_node(root_id).expect("root node").text_content()
        };
        assert_eq!(text, "done");
    });
}

#[test]
fn intervals_floor_zero_delay() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = r#"
            <!DOCTYPE html>
            <html><body><div id="root">idle</div></body></html>
        "#;

        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
        environment.attach_document(&mut document);

        environment
            .eval(
                r#"
                    const root = document.getElementById('root');
                    let ticks = 0;
                    const timerId = setInterval(() => {
                        ticks += 1;
                        root.textContent = `tick:${ticks}`;
                        if (ticks === 3) {
                            clearInterval(timerId);
                        }
                    }, 0);
                "#,
                "interval.js",
            )
            .expect("evaluate script");

        for _ in 0..6 {
            environment.pump().expect("pump interval queue");
            sleep(Duration::from_millis(2)).await;
        }

        let root_id = lookup_node_id(&mut document, "root").expect("root id");
        let text = {
            let base: &BaseDocument = &document;
            base.get_node(root_id).expect("root node").text_content()
        };
        assert_eq!(text, "tick:3");

        // Ensure the interval no longer fires once cleared by letting
        // the runtime spin a little longer.
        sleep(Duration::from_millis(5)).await;
        environment.pump().expect("final pump");
        let text_after = {
            let base: &BaseDocument = &document;
            base.get_node(root_id).expect("root node").text_content()
        };
        assert_eq!(text_after, "tick:3");
    });
}

fn lookup_node_id(document: &mut HtmlDocument, target: &str) -> Option<usize> {
    let mut result = None;
    let root = document.root_node().id;
    document.iter_subtree_mut(root, |node_id, doc| {
        if result.is_some() {
            return;
        }
        if let Some(node) = doc.get_node(node_id) {
            if node.attr(local_name!("id")) == Some(target) {
                result = Some(node_id);
            }
        }
    });
    result
}

#[test]
fn dom_api_supports_creating_elements() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = "<!DOCTYPE html><html><body><div id=\"root\"></div></body></html>";
        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());

        environment.attach_document(&mut document);
        environment
            .eval(
                r#"
                    const root = document.getElementById('root');
                    const button = document.createElement('button');
                    button.id = 'clicker';
                    button.setAttribute('data-action', 'increment');
                    button.textContent = 'Click me';
                    root.appendChild(button);
                "#,
                "dom-create.js",
            )
            .expect("evaluate script");

        let mut found: Option<(String, Option<String>)> = None;
        {
            let base: &mut BaseDocument = &mut document;
            let root_id = base.root_node().id;
            base.iter_subtree_mut(root_id, |node_id, doc| {
                if let Some(node) = doc.get_node(node_id) {
                    if node.attr(local_name!("id")) == Some("clicker") {
                        let text = node.text_content();
                        let data_attr = node
                            .attr(LocalName::from("data-action"))
                            .map(|value| value.to_string());
                        found = Some((text, data_attr));
                    }
                }
            });
        }

        assert_eq!(
            found,
            Some(("Click me".to_string(), Some("increment".to_string())))
        );
    });
}

#[test]
fn react_counter_sample_executes() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let asset_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/react-counter/index.html");
        let file_url = Url::from_file_path(&asset_path).expect("file url");

        let fetch_request = FetchRequest {
            source: FetchSource::LegacyUrl(file_url.clone()),
            display_url: file_url.to_string(),
        };

        let net_callback = Arc::new(DummyNetCallback);
        let net_provider = Arc::new(Provider::new(net_callback));
        let document = navigation::execute_fetch(&fetch_request, Arc::clone(&net_provider))
            .await
            .expect("execute fetch");

        let scripts = document.scripts.clone();

        let mut runtime = JsPageRuntime::new(&document.contents, &scripts)
            .expect("create runtime")
            .expect("runtime with scripts");
        let mut html_doc = HtmlDocument::from_html(&document.contents, DocumentConfig::default());
        runtime.attach_document(&mut html_doc);
        let env_rc = runtime.environment();
        load_external_scripts(
            env_rc.as_ref(),
            &scripts,
            asset_path.parent().unwrap_or_else(|| Path::new(".")),
        )
        .expect("load external scripts");
        let summary = runtime
            .run_blocking_scripts()
            .expect("run blocking scripts")
            .expect("scripts executed");
        assert!(summary.executed_scripts > 0);
        runtime.environment().pump().expect("pump after render");

        let counter_id = lookup_node_id(&mut html_doc, "counter-value").expect("counter text id");
        let initial_text = {
            let base: &BaseDocument = &html_doc;
            base.get_node(counter_id)
                .expect("counter node")
                .text_content()
        };
        assert_eq!(initial_text, "Count: 0");

        let button_id = lookup_node_id(&mut html_doc, "increment").expect("button id");
        let chain = {
            let base: &BaseDocument = &html_doc;
            base.node_chain(button_id)
        };
        let click_event = DomEvent::new(
            button_id,
            DomEventData::Click(BlitzMouseButtonEvent {
                x: 0.0,
                y: 0.0,
                button: MouseEventButton::Main,
                buttons: MouseEventButtons::Primary,
                mods: Modifiers::default(),
            }),
        );

        runtime
            .environment()
            .dispatch_dom_event(&click_event, &chain)
            .expect("dispatch click");

        for _ in 0..5 {
            runtime.environment().pump().expect("pump after click");
            sleep(Duration::from_millis(5)).await;
        }

        let updated_text = {
            let base: &BaseDocument = &html_doc;
            base.get_node(counter_id)
                .expect("counter node")
                .text_content()
        };
        assert_eq!(updated_text, "Count: 1");
    });
}

fn load_external_scripts(
    environment: &JsDomEnvironment,
    scripts: &[ScriptDescriptor],
    base_dir: &Path,
) -> AnyResult<()> {
    for descriptor in scripts {
        if let ScriptSource::External { src } = &descriptor.source {
            if src.starts_with("http://") || src.starts_with("https://") {
                continue;
            }
            let path = if Path::new(src).is_absolute() {
                PathBuf::from(src)
            } else {
                base_dir.join(src)
            };
            let code = std::fs::read_to_string(&path)
                .with_context(|| format!("reading external script {}", path.display()))?;
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("external-script.js");
            environment
                .eval(&code, filename)
                .with_context(|| format!("executing external script {}", path.display()))?;
        }
    }

    Ok(())
}
