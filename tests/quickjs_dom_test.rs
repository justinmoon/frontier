use blitz_dom::{local_name, BaseDocument, Document, DocumentConfig, LocalName};
use blitz_html::{HtmlDocument, HtmlProvider};
use blitz_net::Provider;
use blitz_traits::events::{
    BlitzImeEvent, BlitzKeyEvent, BlitzMouseButtonEvent, DomEvent, DomEventData, KeyState,
    MouseEventButton, MouseEventButtons, UiEvent,
};
use blitz_traits::net::DummyNetCallback;
use frontier::blossom::BlossomFetcher;
use frontier::js::environment::JsDomEnvironment;
use frontier::js::processor;
use frontier::js::runtime_document::RuntimeDocument;
use frontier::js::session::JsPageRuntime;
use frontier::navigation::{self, FetchRequest, FetchSource, FetchedDocument};
use frontier::net::RelayDirectory;
use keyboard_types::{Code, Key, Location, Modifiers};
use std::ops::DerefMut;
use std::path::Path;
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

        let mut runtime = JsPageRuntime::new(&html, &scripts, None)
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
            blossom: None,
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
        let root_id = document.root_node().id;
        document.iter_subtree_mut(root_id, |node_id, doc| {
            if updated.is_some() {
                return;
            }
            if let Some(node) = doc.get_node(node_id) {
                if node.attr(local_name!("id")) == Some("message") {
                    updated = Some(node.text_content());
                }
            }
        });

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
        let chain = document.node_chain(button_id);

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
        let text_after = document
            .get_node(status_id)
            .expect("status node")
            .text_content();
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
        let text = document
            .get_node(root_id)
            .expect("root node")
            .text_content();
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
        let text = document
            .get_node(root_id)
            .expect("root node")
            .text_content();
        assert_eq!(text, "tick:3");

        // Ensure the interval no longer fires once cleared by letting
        // the runtime spin a little longer.
        sleep(Duration::from_millis(5)).await;
        environment.pump().expect("final pump");
        let text_after = document
            .get_node(root_id)
            .expect("root node")
            .text_content();
        assert_eq!(text_after, "tick:3");
    });
}

fn lookup_node_id<T>(document: &mut T, target: &str) -> Option<usize>
where
    T: DerefMut<Target = BaseDocument>,
{
    let mut result = None;
    let base: &mut BaseDocument = document.deref_mut();
    let root = base.root_node().id;
    base.iter_subtree_mut(root, |node_id, doc| {
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
        let root_id = document.root_node().id;
        document.iter_subtree_mut(root_id, |node_id, doc| {
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

        assert_eq!(
            found,
            Some(("Click me".to_string(), Some("increment".to_string())))
        );
    });
}

#[test]
fn comment_nodes_preserve_payload() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = "<!DOCTYPE html><html><body></body></html>";
        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());

        environment.attach_document(&mut document);
        environment
            .eval(
                r#"
                    const marker = document.createComment('react-root');
                    document.body.appendChild(marker);

                    if (marker.nodeValue !== 'react-root') {
                        throw new Error('comment nodeValue should round-trip');
                    }
                    if (marker.textContent !== 'react-root') {
                        throw new Error('comment textContent should round-trip');
                    }
                "#,
                "comment-payload.js",
            )
            .expect("create comment node");

        let serialized = environment.document_html().expect("serialize document");
        assert!(
            serialized.contains("<!--react-root-->"),
            "serialized DOM should include comment payload, got: {serialized}"
        );
    });
}

#[test]
fn comment_nodes_survive_inner_html_round_trip() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = "<!DOCTYPE html><html><body><div id=\"host\"></div></body></html>";
        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(
            html,
            DocumentConfig {
                html_parser_provider: Some(Arc::new(HtmlProvider)),
                ..Default::default()
            },
        );

        environment.attach_document(&mut document);
        environment
            .eval(
                r#"
                    const host = document.getElementById('host');
                    host.innerHTML = '<!--react-root--><span>keep</span>';
                "#,
                "comment-inner-html.js",
            )
            .expect("update host innerHTML");

        let node_type: i32 = environment
            .eval_with(
                "(() => { const host = document.getElementById('host'); const marker = host.firstChild; return marker ? marker.nodeType : -1; })()",
                "comment-inner-html-node-type.js",
            )
            .expect("query comment node type");
        assert_eq!(node_type, 8, "host.firstChild should be a comment node");

        let node_value: Option<String> = environment
            .eval_with(
                "(() => { const host = document.getElementById('host'); const marker = host.firstChild; return marker ? marker.nodeValue : null; })()",
                "comment-inner-html-node-value.js",
            )
            .expect("query comment node value");
        assert_eq!(node_value.as_deref(), Some("react-root"));

        let text_content: Option<String> = environment
            .eval_with(
                "(() => { const host = document.getElementById('host'); const marker = host.firstChild; return marker ? marker.textContent : null; })()",
                "comment-inner-html-text-content.js",
            )
            .expect("query comment text content");
        assert_eq!(text_content.as_deref(), Some("react-root"));

        let serialized = environment.document_html().expect("serialize document");
        assert!(
            serialized.contains("<!--react-root-->"),
            "serialized DOM should retain comment payload, got: {serialized}"
        );
    });
}

#[test]
fn runtime_document_handles_keyboard_and_ime_events() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = r#"
            <!DOCTYPE html>
            <html>
                <body>
                    <input id="field" value="" />
                    <span id="key-output">?</span>
                    <span id="ime-output" data-state="idle">?</span>
                    <script>
                        const field = document.getElementById('field');
                        const keyOutput = document.getElementById('key-output');
                        const imeOutput = document.getElementById('ime-output');
                        field.addEventListener('keydown', (event) => {
                            keyOutput.textContent = event.key;
                        });
                        field.addEventListener('composition', (event) => {
                            imeOutput.textContent = event.value || '';
                            imeOutput.setAttribute('data-state', event.imeState || '');
                        });
                    </script>
                </body>
            </html>
        "#;

        let scripts = processor::collect_scripts(html).expect("collect scripts");
        let mut runtime = JsPageRuntime::new(html, &scripts, None)
            .expect("create runtime")
            .expect("runtime available");
        let mut html_doc = HtmlDocument::from_html(html, DocumentConfig::default());
        runtime.attach_document(&mut html_doc);
        runtime.run_blocking_scripts().expect("execute scripts");
        let environment = runtime.environment();
        let mut runtime_doc = RuntimeDocument::new(html_doc, environment);

        let field_id = lookup_node_id(&mut runtime_doc, "field").expect("field id");
        runtime_doc.set_focus_to(field_id);

        let key_event = BlitzKeyEvent {
            key: Key::Character("a".into()),
            code: Code::KeyA,
            modifiers: Modifiers::default(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: Some("a".into()),
        };

        runtime_doc.handle_ui_event(UiEvent::KeyDown(key_event));
        runtime.environment().pump().expect("pump after keydown");

        let key_output_id = lookup_node_id(&mut runtime_doc, "key-output").expect("key output");
        let key_text = runtime_doc
            .get_node(key_output_id)
            .expect("key output node")
            .text_content();
        assert_eq!(key_text, "a");

        runtime_doc.handle_ui_event(UiEvent::Ime(BlitzImeEvent::Commit("ねこ".into())));
        runtime.environment().pump().expect("pump after ime");

        let ime_output_id = lookup_node_id(&mut runtime_doc, "ime-output").expect("ime output");
        let ime_text = runtime_doc
            .get_node(ime_output_id)
            .expect("ime node")
            .text_content();

        assert_eq!(ime_text, "ねこ");
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
        let relay_directory = RelayDirectory::load(None).expect("relay directory");
        let blossom = Arc::new(BlossomFetcher::new(relay_directory).expect("blossom fetcher"));

        let document =
            navigation::execute_fetch(&fetch_request, Arc::clone(&net_provider), blossom)
                .await
                .expect("execute fetch");

        let scripts = document.scripts.clone();

        let mut runtime = JsPageRuntime::new(
            &document.contents,
            &scripts,
            Some(document.base_url.as_str()),
        )
        .expect("create runtime")
        .expect("runtime with scripts");
        let mut html_doc = HtmlDocument::from_html(&document.contents, DocumentConfig::default());
        runtime.attach_document(&mut html_doc);
        let summary = runtime
            .run_blocking_scripts()
            .expect("run blocking scripts")
            .expect("scripts executed");
        assert!(summary.executed_scripts > 0);
        runtime.environment().pump().expect("pump after render");

        let counter_id = lookup_node_id(&mut html_doc, "counter-value").expect("counter text id");
        let initial_text = html_doc
            .get_node(counter_id)
            .expect("counter node")
            .text_content();
        assert_eq!(initial_text, "Count: 0");

        let button_id = lookup_node_id(&mut html_doc, "increment").expect("button id");
        let chain = html_doc.node_chain(button_id);
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

        let updated_text = html_doc
            .get_node(counter_id)
            .expect("counter node")
            .text_content();
        assert_eq!(updated_text, "Count: 1");
    });
}
