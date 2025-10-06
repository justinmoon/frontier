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
        &*self.inner
    }
}

impl DerefMut for RuntimeDocument {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.inner
    }
}

impl RuntimeDocument {
    pub fn html_mut(&mut self) -> &mut HtmlDocument {
        &mut self.inner
    }
}

impl Document for RuntimeDocument {
    fn handle_ui_event(&mut self, event: UiEvent) {
        let handler = JsEventHandler::new(Rc::clone(&self.environment));
        let mut driver = EventDriver::new(self.inner.mutate(), handler);
        driver.handle_ui_event(event);
    }

    fn poll(&mut self, task_context: Option<TaskContext>) -> bool {
        self.inner.poll(task_context)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.inner.as_any_mut()
    }

    fn id(&self) -> usize {
        self.inner.id()
    }
}
