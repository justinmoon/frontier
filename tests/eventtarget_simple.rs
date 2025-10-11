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

#[test]
fn once_listener_not_reentered_on_nested_dispatch() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = "<!DOCTYPE html><html><body></body></html>";
        let environment = JsDomEnvironment::new(html).expect("environment");

        environment
            .eval(
                r#"
                    globalThis.__onceCount = 0;
                    const target = new EventTarget();
                    target.addEventListener('foo', () => {
                        __onceCount++;
                        target.dispatchEvent(new Event('foo'));
                    }, { once: true });
                    target.dispatchEvent(new Event('foo'));
                "#,
                "eventtarget-once-nested.js",
            )
            .expect("execute nested once listener script");

        let count: i32 = environment
            .eval_with("globalThis.__onceCount", "eventtarget-once-count.js")
            .expect("read once invocation count");
        assert_eq!(
            count, 1,
            "once listener should not be re-entered during nested dispatch"
        );
    });
}

#[test]
fn abort_signal_nested_listener_removed_without_error() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let html = "<!DOCTYPE html><html><body></body></html>";
        let environment = JsDomEnvironment::new(html).expect("environment");

        environment
            .eval(
                r#"
                    globalThis.__abortCount = 0;
                    globalThis.__abortError = null;
                    globalThis.__outerCount = 0;
                    const et = new EventTarget();
                    const ac = new AbortController();
                    function safeDispatch() {
                        try {
                            et.dispatchEvent(new Event('foo'));
                        } catch (error) {
                            __abortError = String(error);
                        }
                    }
                    et.addEventListener('foo', () => {
                        __outerCount++;
                        et.addEventListener('foo', () => {
                            __abortCount++;
                            if (__abortCount > 5) {
                                ac.abort();
                            }
                            safeDispatch();
                        }, { signal: ac.signal });
                        safeDispatch();
                    }, { once: true });
                    safeDispatch();
                "#,
                "eventtarget-signal-nested.js",
            )
            .expect("execute nested abort listener script");

        let count: i32 = environment
            .eval_with("globalThis.__abortCount", "eventtarget-signal-count.js")
            .expect("read abort invocation count");

        let outer_count: i32 = environment
            .eval_with("globalThis.__outerCount", "eventtarget-signal-outer.js")
            .expect("read outer listener invocation count");
        let error: Option<String> = environment
            .eval_with("globalThis.__abortError", "eventtarget-signal-error.js")
            .expect("read abort error state");

        assert_eq!(
            count, 6,
            "abort signal should remove listener before unbounded recursion"
        );
        assert_eq!(
            outer_count, 1,
            "once-wrapped listener should run exactly once"
        );

        assert!(
            error.as_deref().unwrap_or_default().is_empty(),
            "abort scenario should finish without errors"
        );
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
