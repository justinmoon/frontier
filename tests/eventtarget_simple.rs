use blitz_dom::{local_name, BaseDocument, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_traits::events::{
    BlitzMouseButtonEvent, DomEvent, DomEventData, MouseEventButton, MouseEventButtons,
};
use frontier::js::environment::JsDomEnvironment;
use keyboard_types::Modifiers;
use std::ops::DerefMut;
use tokio::runtime::Builder;

#[test]
fn bubbling_reaches_parent_without_target_listener() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = r#"
            <!DOCTYPE html>
            <html>
                <body>
                    <div id="parent">
                        <button id="child">Click</button>
                    </div>
                </body>
            </html>
        "#;

        let environment = JsDomEnvironment::new(html).expect("environment");
        let mut document = HtmlDocument::from_html(html, DocumentConfig::default());
        environment.attach_document(&mut document);

        environment
            .eval(
                r#"
                    globalThis.__bubbled = false;
                    const parent = document.getElementById('parent');
                    parent.addEventListener('click', () => {
                        __bubbled = true;
                    });
                "#,
                "eventtarget-simple.js",
            )
            .expect("register parent listener");

        let child_id = lookup_node_id(&mut document, "child").expect("child id");
        let chain = document.node_chain(child_id);

        let event = DomEvent::new(
            child_id,
            DomEventData::Click(BlitzMouseButtonEvent {
                x: 0.0,
                y: 0.0,
                button: MouseEventButton::Main,
                buttons: MouseEventButtons::Primary,
                mods: Modifiers::default(),
            }),
        );

        environment
            .dispatch_dom_event(&event, &chain)
            .expect("dispatch bubbled event");

        let bubbled: bool = environment
            .eval_with("globalThis.__bubbled", "check-bubble.js")
            .expect("read bubble flag");
        assert!(bubbled, "parent listener should fire for bubbling event");
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
