use std::rc::Rc;

use blitz_dom::{DocumentMutator, EventHandler};
use blitz_traits::events::{DomEvent, EventState};
use tracing::error;

use super::environment::{DispatchOutcome, JsDomEnvironment};

pub struct JsEventHandler {
    environment: Rc<JsDomEnvironment>,
}

impl JsEventHandler {
    pub fn new(environment: Rc<JsDomEnvironment>) -> Self {
        Self { environment }
    }
}

impl EventHandler for JsEventHandler {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        _mutr: &mut DocumentMutator<'_>,
        event_state: &mut EventState,
    ) {
        if !self.environment.is_listening(event.data.name()) {
            return;
        }

        match self.environment.dispatch_dom_event(event, chain) {
            Ok(DispatchOutcome {
                default_prevented,
                redraw_requested,
                propagation_stopped,
            }) => {
                if default_prevented {
                    event_state.prevent_default();
                }
                if redraw_requested {
                    event_state.request_redraw();
                }
                if propagation_stopped {
                    event_state.stop_propagation();
                }
            }
            Err(err) => {
                error!(target = "quickjs", error = %err, "failed to dispatch event to JS");
            }
        }
    }
}
