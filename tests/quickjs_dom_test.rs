use blitz_dom::{local_name, BaseDocument, DocumentConfig, LocalName};
use blitz_html::HtmlDocument;
use frontier::js::environment::JsDomEnvironment;
use frontier::js::processor;
use frontier::js::session::JsPageRuntime;
use frontier::navigation::FetchedDocument;
use blitz_traits::events::{
    BlitzMouseButtonEvent, DomEvent, DomEventData, MouseEventButton, MouseEventButtons,
};
use keyboard_types::Modifiers;

#[test]
fn quickjs_demo_executes_script_and_mutates_dom() {
    let html = std::fs::read_to_string("assets/quickjs-demo.html").expect("demo asset");
    let scripts = processor::collect_scripts(&html).expect("collect scripts");
    assert_eq!(scripts.len(), 1, "demo asset contains one inline script");

    let mut runtime = JsPageRuntime::new(&html, &scripts)
        .expect("create runtime")
        .expect("runtime available for scripts");
    let mut runtime_doc = HtmlDocument::from_html(&html, DocumentConfig::default());
    runtime.attach_document(&mut *runtime_doc);
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
}

#[test]
fn dom_bridge_updates_live_document() {
    let html = "<!DOCTYPE html><html><body><h1 id=\"message\">Loading…</h1></body></html>";

    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());

    environment.attach_document(&mut *document);
    environment
        .eval(
            "document.getElementById('message').textContent = 'Updated';",
            "bridge-test.js",
        )
        .expect("evaluate script");

    let mut updated = None;
    {
        let base: &mut BaseDocument = &mut *document;
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
}

#[test]
fn dom_event_listener_runs_and_prevents_default() {
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
    environment.attach_document(&mut *document);

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
        let base: &BaseDocument = &*document;
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
        let base: &BaseDocument = &*document;
        base.get_node(status_id)
            .expect("status node")
            .text_content()
    };
    assert_eq!(text_after, "clicked");
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
    let html = "<!DOCTYPE html><html><body><div id=\"root\"></div></body></html>";
    let environment = JsDomEnvironment::new(html).expect("environment");
    let mut document = HtmlDocument::from_html(html, DocumentConfig::default());

    environment.attach_document(&mut *document);
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
        let base: &mut BaseDocument = &mut *document;
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
}
