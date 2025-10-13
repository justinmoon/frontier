use std::any::Any;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::task::Context as TaskContext;

use blitz_dom::{BaseDocument, Document, EventDriver};
use blitz_html::HtmlDocument;
use blitz_traits::events::UiEvent;

use super::environment::JsDomEnvironment;
use super::events::JsEventHandler;

/// Wraps an [`HtmlDocument`] and forwards UI events into the JS runtime so DOM event
/// listeners can observe user input.
pub struct RuntimeDocument {
    inner: HtmlDocument,
    environment: Rc<JsDomEnvironment>,
}

impl RuntimeDocument {
    pub fn new(inner: HtmlDocument, environment: Rc<JsDomEnvironment>) -> Self {
        Self { inner, environment }
    }
}

impl Deref for RuntimeDocument {
    type Target = BaseDocument;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for RuntimeDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl RuntimeDocument {
    #[allow(dead_code)]
    pub fn html_mut(&mut self) -> &mut HtmlDocument {
        &mut self.inner
    }
}

impl Document for RuntimeDocument {
    fn handle_ui_event(&mut self, event: UiEvent) {
        let handler = JsEventHandler::new(Rc::clone(&self.environment));
        let mutator = self.inner.mutate();
        self.environment.reattach_document(mutator.doc);
        let mut driver = EventDriver::new(mutator, handler);
        driver.handle_ui_event(event);
        if let Err(err) = self.environment.pump() {
            tracing::error!(target = "quickjs", error = %err, "failed to pump timers after UI event");
        }
    }

    fn poll(&mut self, task_context: Option<TaskContext>) -> bool {
        if let Some(cx) = task_context.as_ref() {
            let waker = cx.waker().clone();
            self.environment.register_waker(&waker);
        }

        let mut needs_redraw = self.inner.poll(task_context);

        match self.environment.pump() {
            Ok(did_work) => {
                if did_work {
                    needs_redraw = true;
                }
            }
            Err(err) => {
                tracing::error!(
                    target = "quickjs",
                    error = %err,
                    "failed to pump timers inside poll"
                );
                needs_redraw = true;
            }
        }

        needs_redraw
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.inner.as_any_mut()
    }

    fn id(&self) -> usize {
        self.inner.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js::session::JsPageRuntime;
    use crate::navigation::{self, FetchRequest, FetchSource};
    use blitz_dom::{local_name, DocumentConfig};
    use blitz_html::HtmlDocument;
    use blitz_net::Provider;
    use blitz_traits::events::{
        BlitzMouseButtonEvent, DomEvent, DomEventData, MouseEventButton, MouseEventButtons,
    };
    use blitz_traits::net::DummyNetCallback;
    use futures_util::task::ArcWake;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::runtime::Builder;
    use tokio::time::sleep;
    use url::Url;

    struct CountingWaker {
        hits: Arc<AtomicUsize>,
    }

    impl ArcWake for CountingWaker {
        fn wake_by_ref(arc_self: &Arc<Self>) {
            arc_self.hits.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn timer_poll_relies_on_timer_waker() {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let asset_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos");
            let timer_path = asset_root.join("timer.html");
            let timer_url = Url::from_file_path(&timer_path).expect("file url");

            let net = Arc::new(Provider::new(Arc::new(DummyNetCallback)));
            let request = FetchRequest {
                source: FetchSource::Url(timer_url.clone()),
                display_url: timer_url.to_string(),
            };

            let fetched = navigation::execute_fetch(&request, Arc::clone(&net))
                .await
                .expect("fetch timer document");

            let scripts = fetched.scripts.clone();
            let mut runtime =
                JsPageRuntime::new(&fetched.contents, &scripts, Some(fetched.base_url.as_str()))
                    .expect("create runtime")
                    .expect("runtime available");

            let mut document = HtmlDocument::from_html(
                &fetched.contents,
                DocumentConfig {
                    base_url: Some(fetched.base_url.clone()),
                    ..Default::default()
                },
            );

            runtime.attach_document(&mut document);
            runtime
                .run_blocking_scripts()
                .expect("run blocking scripts");
            runtime.environment().pump().expect("initial pump");

            let environment = runtime.environment();
            let mut runtime_document = RuntimeDocument::new(document, environment.clone());
            environment.reattach_document(&mut runtime_document);

            let start_id = lookup_node_id(&mut runtime_document, "start-timer").expect("start id");
            let chain = runtime_document.node_chain(start_id);
            let event = DomEvent::new(
                start_id,
                DomEventData::Click(BlitzMouseButtonEvent {
                    x: 0.0,
                    y: 0.0,
                    button: MouseEventButton::Main,
                    buttons: MouseEventButtons::Primary,
                    mods: Default::default(),
                }),
            );
            environment
                .dispatch_dom_event(&event, &chain)
                .expect("dispatch click");
            environment.pump().expect("pump after click");

            let hits = Arc::new(AtomicUsize::new(0));
            let waker = futures_util::task::waker(Arc::new(CountingWaker {
                hits: Arc::clone(&hits),
            }));

            let cx = std::task::Context::from_waker(&waker);
            runtime_document.poll(Some(cx));
            assert_eq!(
                hits.load(Ordering::SeqCst),
                0,
                "poll should not trigger an immediate wake while waiting for timer"
            );

            sleep(Duration::from_millis(150)).await;
            assert!(
                hits.load(Ordering::SeqCst) > 0,
                "timer should wake the document once the interval fires"
            );

            let cx = std::task::Context::from_waker(&waker);
            runtime_document.poll(Some(cx));

            let value_id = lookup_node_id(&mut runtime_document, "timer-value").expect("value id");
            let value_text = runtime_document
                .get_node(value_id)
                .map(|node| node.text_content())
                .unwrap_or_default();

            assert!(
                value_text.starts_with("Elapsed: ") && value_text != "Elapsed: 0.0s",
                "timer value should advance after interval tick (got {value_text})"
            );
        });
    }

    fn lookup_node_id(document: &mut RuntimeDocument, target_id: &str) -> Option<usize> {
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
}
