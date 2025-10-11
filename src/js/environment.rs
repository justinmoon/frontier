use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Waker;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use blitz_dom::BaseDocument;
use blitz_traits::events::{
    BlitzImeEvent, BlitzKeyEvent, BlitzMouseButtonEvent, DomEvent, DomEventData, MouseEventButton,
};
use futures_util::task::AtomicWaker;
use keyboard_types::{Location, Modifiers};
use rquickjs::function::{Args as FunctionArgs, Opt};
use rquickjs::{Ctx, Function, IntoJs, Value};
use serde_json::{json, to_string as to_json_string, Map as JsonMap, Value as JsonValue};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tracing::error;

use super::dom::{DomPatch, DomState};
use super::runtime::QuickJsEngine;

pub struct JsDomEnvironment {
    engine: QuickJsEngine,
    state: Rc<RefCell<DomState>>,
    timers: Rc<TimerManager>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DispatchOutcome {
    pub default_prevented: bool,
    pub redraw_requested: bool,
    pub propagation_stopped: bool,
}
impl JsDomEnvironment {
    pub fn new(html: &str) -> Result<Self> {
        let state = Rc::new(RefCell::new(DomState::new(html)));
        let engine = QuickJsEngine::new()?;
        let timers = Rc::new(TimerManager::new(Handle::current()));
        install_dom_bindings(&engine, Rc::clone(&state), Rc::clone(&timers))?;
        Ok(Self {
            engine,
            state,
            timers,
        })
    }

    pub fn is_listening(&self, event_type: &str) -> bool {
        self.state.borrow().is_listening(event_type)
    }

    pub fn dispatch_dom_event(&self, event: &DomEvent, chain: &[usize]) -> Result<DispatchOutcome> {
        let event_name = event.data.name();
        if !self.is_listening(event_name) {
            return Ok(DispatchOutcome::default());
        }

        let (target_handle, mut path_handles) = {
            let state = self.state.borrow();
            let target = match state.normalize_handle(event.target) {
                Ok(Some(handle)) => handle,
                Ok(None) => return Ok(DispatchOutcome::default()),
                Err(err) => {
                    error!(
                        target = "quickjs",
                        error = %err,
                        "failed to normalise event target handle"
                    );
                    return Ok(DispatchOutcome::default());
                }
            };

            let path = match state.normalize_chain(chain) {
                Ok(handles) => handles,
                Err(err) => {
                    error!(
                        target = "quickjs",
                        error = %err,
                        "failed to normalise event propagation chain"
                    );
                    Vec::new()
                }
            };

            (target, path)
        };

        if path_handles.is_empty() {
            path_handles.push(target_handle.clone());
        }

        let detail = build_event_detail(event);
        let detail_json = to_json_string(&detail).map_err(anyhow::Error::from)?;
        let event_name_owned = event_name.to_string();
        let target_handle_clone = target_handle.clone();
        let path_handles_clone = path_handles.clone();

        let result = self.engine.with_context(|ctx| {
            let global = ctx.globals();
            let frontier: rquickjs::Object = global.get("frontier")?;
            let dispatch: rquickjs::Function = frontier.get("__dispatchDomEvent")?;
            let detail_value = ctx.json_parse(detail_json.as_bytes())?;
            let js_result: rquickjs::Value = dispatch.call((
                target_handle_clone.clone(),
                event_name_owned.clone(),
                detail_value,
                path_handles_clone.clone(),
            ))?;
            let js_obj = js_result.into_object().ok_or(rquickjs::Error::Unknown)?;
            let default_prevented = js_obj
                .get::<_, Option<bool>>("defaultPrevented")?
                .unwrap_or(false);
            let redraw_requested = js_obj
                .get::<_, Option<bool>>("redrawRequested")?
                .unwrap_or(false);
            let propagation_stopped = js_obj
                .get::<_, Option<bool>>("propagationStopped")?
                .unwrap_or(false);
            Ok(DispatchOutcome {
                default_prevented,
                redraw_requested,
                propagation_stopped,
            })
        });

        let outcome = match result {
            Ok(outcome) => outcome,
            Err(err) => {
                error!(target = "quickjs", error = %err, "failed to dispatch DOM event");
                DispatchOutcome::default()
            }
        };

        if let Err(err) = self.pump() {
            error!(target = "quickjs", error = %err, "failed to pump timers after event");
        }

        Ok(outcome)
    }

    pub fn eval(&self, source: &str, filename: &str) -> Result<()> {
        self.engine.eval(source, filename)
    }

    #[allow(dead_code)]
    pub fn eval_with<V>(&self, source: &str, filename: &str) -> Result<V>
    where
        V: for<'js> rquickjs::FromJs<'js>,
    {
        self.engine.eval_with(source, filename)
    }

    pub fn drain_mutations(&self) -> Vec<DomPatch> {
        self.state.borrow_mut().drain_mutations()
    }

    pub fn document_html(&self) -> Result<String> {
        self.state.borrow().to_html()
    }

    pub fn attach_document(&self, document: &mut BaseDocument) {
        self.state.borrow_mut().attach_document(document);
        let _ = self.engine.with_context(|ctx| {
            let global = ctx.globals();
            if let Ok(frontier) = global.get::<_, rquickjs::Object>("frontier") {
                if let Ok(refresh) = frontier.get::<_, rquickjs::Function>("__refreshDocument") {
                    let _: Value = refresh.call(())?;
                }
            }
            Ok(())
        });
    }

    pub fn reattach_document(&self, document: &mut BaseDocument) {
        self.state.borrow_mut().reattach_document(document);
        let _ = self.engine.with_context(|ctx| {
            let global = ctx.globals();
            if let Ok(frontier) = global.get::<_, rquickjs::Object>("frontier") {
                if let Ok(refresh) = frontier.get::<_, rquickjs::Function>("__refreshDocument") {
                    let _: Value = refresh.call(())?;
                }
            }
            Ok(())
        });
    }

    pub fn pump(&self) -> Result<bool> {
        let mut did_work = false;
        loop {
            let timers_ran = self.timers.run_due(&self.engine)?;
            let jobs_ran = self.engine.drain_jobs()?;
            if timers_ran || jobs_ran {
                did_work = true;
            }
            if !timers_ran && !jobs_ran {
                break;
            }
        }
        Ok(did_work)
    }

    pub fn register_waker(&self, waker: &Waker) {
        self.timers.register_waker(waker);
    }

    pub fn has_pending_timers(&self) -> bool {
        self.timers.has_active_timers()
    }
}

fn install_dom_bindings(
    engine: &QuickJsEngine,
    state: Rc<RefCell<DomState>>,
    timers: Rc<TimerManager>,
) -> Result<()> {
    engine.with_context(|ctx| {
        let global = ctx.globals();

        // Lookup helpers
        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |id: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow_mut().handle_from_element_id(&id))
                },
            )?
            .with_name("__frontier_dom_get_handle_by_id")?;
            global.set("__frontier_dom_get_handle_by_id", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().text_content(&handle))
                },
            )?
            .with_name("__frontier_dom_get_text")?;
            global.set("__frontier_dom_get_text", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().inner_html(&handle))
                },
            )?
            .with_name("__frontier_dom_get_html")?;
            global.set("__frontier_dom_get_html", func)?;
        }

        // Mutation helpers
        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      handle: String,
                      value: Option<String>|
                      -> rquickjs::Result<()> {
                    let text = value.unwrap_or_default();
                    match state_ref
                        .borrow_mut()
                        .set_text_content_direct(&handle, &text)
                    {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_set_text")?;
            global.set("__frontier_dom_set_text", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      handle: String,
                      value: Option<String>|
                      -> rquickjs::Result<()> {
                    let html = value.unwrap_or_default();
                    match state_ref.borrow_mut().set_inner_html_direct(&handle, &html) {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_set_inner_html")?;
            global.set("__frontier_dom_set_inner_html", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      handle: String,
                      name: String,
                      value: Option<String>|
                      -> rquickjs::Result<()> {
                    let attr_value = value.unwrap_or_default();
                    match state_ref
                        .borrow_mut()
                        .set_attribute_direct(&handle, &name, &attr_value)
                    {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_set_attribute")?;
            global.set("__frontier_dom_set_attribute", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String, name: String| -> rquickjs::Result<()> {
                    match state_ref
                        .borrow_mut()
                        .remove_attribute_direct(&handle, &name)
                    {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_remove_attribute")?;
            global.set("__frontier_dom_remove_attribute", func)?;
        }

        // Node creation
        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, name: String| -> rquickjs::Result<String> {
                    match state_ref.borrow_mut().create_element(&name, None) {
                        Ok(handle) => Ok(handle),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_create_element")?;
            global.set("__frontier_dom_create_element", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      namespace: Option<String>,
                      name: String|
                      -> rquickjs::Result<String> {
                    let ns = namespace.as_deref();
                    match state_ref.borrow_mut().create_element(&name, ns) {
                        Ok(handle) => Ok(handle),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_create_element_ns")?;
            global.set("__frontier_dom_create_element_ns", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, value: Option<String>| -> rquickjs::Result<String> {
                    let text = value.unwrap_or_default();
                    match state_ref.borrow_mut().create_text_node(&text) {
                        Ok(handle) => Ok(handle),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_create_text")?;
            global.set("__frontier_dom_create_text", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, value: Option<String>| -> rquickjs::Result<String> {
                    let text = value.unwrap_or_default();
                    match state_ref.borrow_mut().create_comment_node(&text) {
                        Ok(handle) => Ok(handle),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_create_comment")?;
            global.set("__frontier_dom_create_comment", func)?;
        }

        // Tree manipulation
        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, parent: String, child: String| -> rquickjs::Result<()> {
                    match state_ref.borrow_mut().append_child(&parent, &child) {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_append_child")?;
            global.set("__frontier_dom_append_child", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      parent: String,
                      child: String,
                      reference: Option<String>|
                      -> rquickjs::Result<()> {
                    match state_ref.borrow_mut().insert_before(
                        &parent,
                        &child,
                        reference.as_deref(),
                    ) {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_insert_before")?;
            global.set("__frontier_dom_insert_before", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, parent: String, child: String| -> rquickjs::Result<()> {
                    match state_ref.borrow_mut().remove_child(&parent, &child) {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_remove_child")?;
            global.set("__frontier_dom_remove_child", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      parent: String,
                      new_child: String,
                      old_child: String|
                      -> rquickjs::Result<()> {
                    match state_ref
                        .borrow_mut()
                        .replace_child(&parent, &new_child, &old_child)
                    {
                        Ok(()) => Ok(()),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_replace_child")?;
            global.set("__frontier_dom_replace_child", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      handle: String,
                      deep: Option<bool>|
                      -> rquickjs::Result<String> {
                    let deep = deep.unwrap_or(false);
                    match state_ref.borrow_mut().clone_node(&handle, deep) {
                        Ok(new_handle) => Ok(new_handle),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_clone_node")?;
            global.set("__frontier_dom_clone_node", func)?;
        }

        // Tree reads
        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().parent_handle(&handle) {
                        Ok(parent) => Ok(parent),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_parent")?;
            global.set("__frontier_dom_parent", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().first_child_handle(&handle) {
                        Ok(child) => Ok(child),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_first_child")?;
            global.set("__frontier_dom_first_child", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().next_sibling_handle(&handle) {
                        Ok(next) => Ok(next),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_next_sibling")?;
            global.set("__frontier_dom_next_sibling", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().previous_sibling_handle(&handle) {
                        Ok(prev) => Ok(prev),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_previous_sibling")?;
            global.set("__frontier_dom_previous_sibling", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Vec<String>> {
                    match state_ref.borrow().child_handles(&handle) {
                        Ok(children) => Ok(children),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_child_nodes")?;
            global.set("__frontier_dom_child_nodes", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<String> {
                    match state_ref.borrow().node_name(&handle) {
                        Ok(name) => Ok(name),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_node_name")?;
            global.set("__frontier_dom_node_name", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<i32> {
                    match state_ref.borrow().node_type(&handle) {
                        Ok(ty) => Ok(ty as i32),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_node_type")?;
            global.set("__frontier_dom_node_type", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().node_value(&handle) {
                        Ok(value) => Ok(value),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_node_value")?;
            global.set("__frontier_dom_node_value", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      handle: String,
                      name: String|
                      -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().get_attribute(&handle, &name) {
                        Ok(value) => Ok(value),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_get_attribute")?;
            global.set("__frontier_dom_get_attribute", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, handle: String| -> rquickjs::Result<Option<String>> {
                    match state_ref.borrow().namespace_uri(&handle) {
                        Ok(ns) => Ok(ns),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_namespace_uri")?;
            global.set("__frontier_dom_namespace_uri", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, _handle: String, event_type: String| -> rquickjs::Result<()> {
                    state_ref.borrow_mut().listen(&event_type);
                    Ok(())
                },
            )?
            .with_name("__frontier_dom_listen")?;
            global.set("__frontier_dom_listen", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, _handle: String, event_type: String| -> rquickjs::Result<()> {
                    state_ref.borrow_mut().unlisten(&event_type);
                    Ok(())
                },
            )?
            .with_name("__frontier_dom_unlisten")?;
            global.set("__frontier_dom_unlisten", func)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>| -> rquickjs::Result<String> {
                    match state_ref.borrow().document_handle() {
                        Ok(handle) => Ok(handle),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_document_handle")?;
            global.set("__frontier_dom_document_handle", func)?;
        }

        // Timer helpers
        {
            let timers_ref = Rc::clone(&timers);
            let func = Function::new(
                ctx.clone(),
                move |kind: String, delay: Opt<f64>, repeating: bool| -> rquickjs::Result<u32> {
                    let timer_kind = match kind.as_str() {
                        "timeout" => TimerKind::Timeout,
                        "interval" => TimerKind::Interval,
                        "animationFrame" => TimerKind::AnimationFrame,
                        other => {
                            return Err(rquickjs::Error::new_from_js_message(
                                "timer",
                                "supported kind",
                                format!("unsupported timer kind: {other}"),
                            ))
                        }
                    };
                    let delay_ms = delay.0.unwrap_or(0.0).max(0.0);
                    Ok(timers_ref.register_timer(delay_ms, timer_kind, repeating))
                },
            )?
            .with_name("__frontier_schedule_timer")?;
            global.set("__frontier_schedule_timer", func)?;
        }

        {
            let timers_ref = Rc::clone(&timers);
            let func = Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, id: Value<'_>| -> rquickjs::Result<()> {
                    let timer_id = id.as_int().unwrap_or_default() as u32;
                    timers_ref.clear_timer(timer_id);
                    Ok(())
                },
            )?
            .with_name("__frontier_cancel_timer")?;
            global.set("__frontier_cancel_timer", func)?;
        }

        // Legacy patch interface retained for compatibility
        {
            let state_ref = Rc::clone(&state);
            let func = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, json: String| -> rquickjs::Result<bool> {
                    let dom_patch: DomPatch = match serde_json::from_str(&json) {
                        Ok(patch) => patch,
                        Err(err) => {
                            return dom_error(&ctx, anyhow!("invalid DOM patch payload: {err}"))
                        }
                    };
                    tracing::debug!(target = "quickjs", patch = %json, "apply_dom_patch");
                    match state_ref.borrow_mut().apply_patch(dom_patch) {
                        Ok(changed) => Ok(changed),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_apply_patch")?;
            global.set("__frontier_dom_apply_patch", func)?;
        }

        match ctx.eval::<(), _>(DOM_BOOTSTRAP.as_bytes()) {
            Ok(()) => Ok(()),
            Err(err) => {
                if let rquickjs::Error::Exception = err {
                    let value: Value<'_> = ctx.catch();
                    tracing::error!(target = "quickjs", "DOM bootstrap failed: {:?}", value);
                }
                Err(err)
            }
        }
    })
}

fn dom_error<T>(ctx: &Ctx<'_>, err: anyhow::Error) -> rquickjs::Result<T> {
    tracing::error!(target = "quickjs", "DOM mutation failed: {err}");
    let message = format!("DOM mutation failed: {err}");
    let value = message.into_js(ctx)?;
    Err(ctx.throw(value))
}

#[derive(Clone, Copy)]
enum TimerKind {
    Timeout,
    Interval,
    AnimationFrame,
}

struct TimerEntry {
    kind: TimerKind,
    repeating: bool,
    task: Option<JoinHandle<()>>,
}

struct TimerManager {
    handle: Handle,
    start: Instant,
    next_id: RefCell<u32>,
    timers: RefCell<HashMap<u32, TimerEntry>>,
    fired_rx: RefCell<UnboundedReceiver<u32>>,
    fired_tx: UnboundedSender<u32>,
    waker: Arc<AtomicWaker>,
}

impl TimerManager {
    fn new(handle: Handle) -> Self {
        let (tx, rx) = unbounded_channel();
        Self {
            handle,
            start: Instant::now(),
            next_id: RefCell::new(1),
            timers: RefCell::new(HashMap::new()),
            fired_rx: RefCell::new(rx),
            fired_tx: tx,
            waker: Arc::new(AtomicWaker::new()),
        }
    }

    fn next_id(&self) -> u32 {
        let mut id_ref = self.next_id.borrow_mut();
        let id = *id_ref;
        *id_ref = id.wrapping_add(1).max(1);
        id
    }

    fn register_waker(&self, waker: &Waker) {
        self.waker.register(waker);
    }

    fn wake(&self) {
        self.waker.wake();
    }

    fn has_active_timers(&self) -> bool {
        !self.timers.borrow().is_empty()
    }

    fn register_timer(&self, delay_ms: f64, kind: TimerKind, repeating: bool) -> u32 {
        let id = self.next_id();
        let mut duration = if delay_ms <= 0.0 {
            Duration::from_millis(0)
        } else {
            Duration::from_secs_f64(delay_ms / 1_000.0)
        };

        if matches!(kind, TimerKind::AnimationFrame) && duration.is_zero() {
            duration = Duration::from_millis(16);
        }

        if repeating && duration.is_zero() {
            duration = Duration::from_millis(1);
        }

        let tx = self.fired_tx.clone();
        let waker = Arc::clone(&self.waker);
        let join = if repeating {
            self.handle.spawn(async move {
                let interval = duration;
                loop {
                    sleep(interval).await;
                    if tx.send(id).is_err() {
                        break;
                    }
                    waker.wake();
                }
            })
        } else {
            self.handle.spawn(async move {
                sleep(duration).await;
                if tx.send(id).is_ok() {
                    waker.wake();
                }
            })
        };

        let entry = TimerEntry {
            kind,
            repeating,
            task: Some(join),
        };

        self.timers.borrow_mut().insert(id, entry);
        self.wake();
        id
    }

    fn clear_timer(&self, id: u32) {
        if let Some(entry) = self.timers.borrow_mut().remove(&id) {
            if let Some(task) = entry.task {
                task.abort();
            }
        }
        self.wake();
    }

    fn run_due(&self, engine: &QuickJsEngine) -> Result<bool> {
        let mut fired = Vec::new();
        {
            let mut rx = self.fired_rx.borrow_mut();
            while let Ok(id) = rx.try_recv() {
                fired.push(id);
            }
        }

        let mut ran = false;
        for id in fired {
            let kind = {
                let timers = self.timers.borrow();
                timers.get(&id).map(|entry| entry.kind)
            };

            let Some(kind) = kind else {
                continue;
            };

            self.invoke(engine, id, kind)?;
            ran = true;

            let should_remove = {
                let timers = self.timers.borrow();
                timers
                    .get(&id)
                    .map(|entry| !entry.repeating)
                    .unwrap_or(true)
            };

            if should_remove {
                if let Some(entry) = self.timers.borrow_mut().remove(&id) {
                    if let Some(handle) = entry.task {
                        handle.abort();
                    }
                }
            }
        }

        Ok(ran)
    }

    fn invoke(&self, engine: &QuickJsEngine, id: u32, kind: TimerKind) -> Result<()> {
        engine.with_context(|ctx| {
            let global = ctx.globals();
            let frontier: rquickjs::Object = global.get("frontier")?;
            let invoke: rquickjs::Function = frontier.get("__invokeTimer")?;
            let arg_count = if matches!(kind, TimerKind::AnimationFrame) {
                2
            } else {
                1
            };
            let mut builder = FunctionArgs::new(ctx.clone(), arg_count);
            builder.push_arg(id)?;
            if matches!(kind, TimerKind::AnimationFrame) {
                let timestamp = self.start.elapsed().as_secs_f64() * 1_000.0;
                builder.push_arg(timestamp)?;
            }

            match invoke.call_arg::<Value<'_>>(builder) {
                Ok(_) => Ok(()),
                Err(err) => {
                    if let rquickjs::Error::Exception = err {
                        let value: Value<'_> = ctx.catch();
                        let message = ctx
                            .globals()
                            .get::<_, rquickjs::Function>("String")
                            .ok()
                            .and_then(|string_fn| {
                                string_fn.call::<_, rquickjs::String>((value.clone(),)).ok()
                            })
                            .and_then(|js_string| js_string.to_string().ok())
                            .unwrap_or_else(|| "<unknown>".to_string());
                        return Err(rquickjs::Error::new_from_js_message(
                            "timer",
                            "callback",
                            format!("timer {id} threw: {message}"),
                        ));
                    }
                    Err(err)
                }
            }
        })
    }
}

fn build_event_detail(event: &DomEvent) -> JsonValue {
    let mut map = JsonMap::new();
    map.insert("bubbles".to_string(), JsonValue::Bool(event.bubbles));
    map.insert("cancelable".to_string(), JsonValue::Bool(event.cancelable));

    match &event.data {
        DomEventData::MouseMove(data)
        | DomEventData::MouseDown(data)
        | DomEventData::MouseUp(data)
        | DomEventData::Click(data) => insert_mouse_event(&mut map, data),
        DomEventData::KeyDown(data) | DomEventData::KeyUp(data) | DomEventData::KeyPress(data) => {
            insert_key_event(&mut map, data)
        }
        DomEventData::Input(data) => {
            map.insert("value".to_string(), JsonValue::String(data.value.clone()));
        }
        DomEventData::Ime(data) => insert_ime_event(&mut map, data),
    }

    JsonValue::Object(map)
}

fn insert_mouse_event(map: &mut JsonMap<String, JsonValue>, event: &BlitzMouseButtonEvent) {
    map.insert("clientX".to_string(), json!(event.x));
    map.insert("clientY".to_string(), json!(event.y));
    map.insert("x".to_string(), json!(event.x));
    map.insert("y".to_string(), json!(event.y));
    map.insert("button".to_string(), json!(mouse_button_code(event.button)));
    map.insert("buttons".to_string(), json!(event.buttons.bits()));
    insert_modifier_flags(map, &event.mods);
}

fn insert_key_event(map: &mut JsonMap<String, JsonValue>, event: &BlitzKeyEvent) {
    insert_modifier_flags(map, &event.modifiers);
    map.insert("key".to_string(), JsonValue::String(event.key.to_string()));
    map.insert(
        "code".to_string(),
        JsonValue::String(event.code.to_string()),
    );
    map.insert("location".to_string(), json!(location_code(event.location)));
    map.insert(
        "repeat".to_string(),
        JsonValue::Bool(event.is_auto_repeating),
    );
    map.insert(
        "isComposing".to_string(),
        JsonValue::Bool(event.is_composing),
    );
    if let Some(text) = &event.text {
        map.insert("text".to_string(), JsonValue::String(text.to_string()));
    }
}

fn insert_ime_event(map: &mut JsonMap<String, JsonValue>, event: &BlitzImeEvent) {
    match event {
        BlitzImeEvent::Enabled => {
            map.insert("imeState".to_string(), JsonValue::String("enabled".into()));
        }
        BlitzImeEvent::Disabled => {
            map.insert("imeState".to_string(), JsonValue::String("disabled".into()));
        }
        BlitzImeEvent::Commit(value) => {
            map.insert("imeState".to_string(), JsonValue::String("commit".into()));
            map.insert("value".to_string(), JsonValue::String(value.clone()));
        }
        BlitzImeEvent::Preedit(value, cursor) => {
            map.insert("imeState".to_string(), JsonValue::String("preedit".into()));
            map.insert("value".to_string(), JsonValue::String(value.clone()));
            if let Some((start, end)) = cursor {
                map.insert("preeditStart".to_string(), json!(*start));
                map.insert("preeditEnd".to_string(), json!(*end));
            }
        }
    }
}

fn insert_modifier_flags(map: &mut JsonMap<String, JsonValue>, mods: &Modifiers) {
    map.insert("altKey".to_string(), JsonValue::Bool(mods.alt()));
    map.insert("ctrlKey".to_string(), JsonValue::Bool(mods.ctrl()));
    map.insert("metaKey".to_string(), JsonValue::Bool(mods.meta()));
    map.insert("shiftKey".to_string(), JsonValue::Bool(mods.shift()));
}

fn mouse_button_code(button: MouseEventButton) -> i32 {
    match button {
        MouseEventButton::Main => 0,
        MouseEventButton::Auxiliary => 1,
        MouseEventButton::Secondary => 2,
        MouseEventButton::Fourth => 3,
        MouseEventButton::Fifth => 4,
    }
}

fn location_code(location: Location) -> i32 {
    match location {
        Location::Standard => 0,
        Location::Left => 1,
        Location::Right => 2,
        Location::Numpad => 3,
    }
}

const DOM_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;
    if (typeof global.self !== 'object' || global.self === null) {
        global.self = global;
    }
    if (typeof global.window !== 'object' || global.window === null) {
        global.window = global;
    }
    if (typeof global.global !== 'object' || global.global === null) {
        global.global = global;
    }
    const HANDLE = Symbol('frontierHandle');
    const NODE_CACHE = new Map();

    function ensureFrontier() {
        if (typeof global.frontier !== 'object' || global.frontier === null) {
            global.frontier = {};
        }
        return global.frontier;
    }

    const frontier = ensureFrontier();

    const EVENT_TARGET_DATA = new WeakMap();
    const ABORT_SIGNAL_FLAG = Symbol('frontierAbortSignal');
    const DOM_EXCEPTION_CODES = {
        IndexSizeError: 1,
        HierarchyRequestError: 3,
        WrongDocumentError: 4,
        InvalidCharacterError: 5,
        NoModificationAllowedError: 7,
        NotFoundError: 8,
        NotSupportedError: 9,
        InUseAttributeError: 10,
        InvalidStateError: 11,
        SyntaxError: 12,
        InvalidModificationError: 13,
        NamespaceError: 14,
        InvalidAccessError: 15,
        TypeMismatchError: 17,
        SecurityError: 18,
        NetworkError: 19,
        AbortError: 20,
        URLMismatchError: 21,
        QuotaExceededError: 22,
        TimeoutError: 23,
        InvalidNodeTypeError: 24,
        DataCloneError: 25,
    };

    let EventCtorRef = null;
    let CustomEventCtorRef = null;
    let MessageEventCtorRef = null;

    function normalizeEventType(type) {
        return String(type ?? '').trim().toLowerCase();
    }

    function normalizeEventTargetReceiver(target) {
        if (target == null) {
            return global;
        }
        if (typeof target === 'object' || typeof target === 'function') {
            return target;
        }
        return Object(target);
    }

    function getEventTargetRecord(target, create) {
        if (target == null || (typeof target !== 'object' && typeof target !== 'function')) {
            if (create) {
                throw new TypeError('EventTarget records require objects');
            }
            return null;
        }
        let record = EVENT_TARGET_DATA.get(target);
        if (!record && create) {
            record = {
                listeners: new Map(),
                handle: typeof target[HANDLE] === 'string' ? String(target[HANDLE]) : null,
                counts: new Map(),
                blocked: new Map(),
            };
            EVENT_TARGET_DATA.set(target, record);
        } else if (record && record.handle == null && typeof target[HANDLE] === 'string') {
            record.handle = String(target[HANDLE]);
        }
        return record ?? null;
    }

    function ensureEventTargetRecord(target) {
        const record = getEventTargetRecord(target, true);
        return record;
    }

    function associateEventTargetHandle(target, handle) {
        if (target == null || (typeof target !== 'object' && typeof target !== 'function')) {
            return;
        }
        const record = ensureEventTargetRecord(target);
        const normalized = handle == null ? null : String(handle);
        const previous = record.handle;
        if (previous === normalized) {
            return;
        }
        if (previous) {
            record.counts.forEach((count, type) => {
                if (count > 0) {
                    global.__frontier_dom_unlisten(previous, type);
                }
            });
        }
        record.handle = normalized;
        if (normalized) {
            record.counts.forEach((count, type) => {
                if (count > 0) {
                    global.__frontier_dom_listen(normalized, type);
                }
            });
        }
    }

    function getListenerBuckets(record, type, create) {
        let buckets = record.listeners.get(type);
        if (!buckets && create) {
            buckets = { capture: [], bubble: [] };
            record.listeners.set(type, buckets);
        }
        return buckets ?? null;
    }

    function incrementDomListener(record, type) {
        const handle = record.handle;
        const counts = record.counts;
        const current = counts.get(type) ?? 0;
        if (handle && current === 0) {
            global.__frontier_dom_listen(String(handle), type);
        }
        counts.set(type, current + 1);
    }

    function decrementDomListener(record, type) {
        const handle = record.handle;
        const counts = record.counts;
        const current = counts.get(type);
        if (current == null) {
            return;
        }
        if (handle && current === 1) {
            global.__frontier_dom_unlisten(String(handle), type);
            counts.delete(type);
        } else if (current > 1) {
            counts.set(type, current - 1);
        } else {
            counts.delete(type);
        }
    }

    function normalizeAddOptions(options) {
        let capture = false;
        let once = false;
        let passive = false;
        let signal;
        let signalProvided = false;
        if (options === true || options === false) {
            capture = !!options;
        } else if (options && typeof options === 'object') {
            capture = !!options.capture;
            once = !!options.once;
            passive = !!options.passive;
            if ('signal' in options) {
                signal = options.signal;
                signalProvided = true;
            }
        }
        return { capture, once, passive, signal, signalProvided };
    }

    function normalizeRemoveOptions(options) {
        let capture = false;
        if (options === true || options === false) {
            capture = !!options;
        } else if (options && typeof options === 'object') {
            capture = !!options.capture;
        }
        return { capture };
    }

    function normalizeCallback(callback) {
        if (typeof callback === 'function') {
            return {
                original: callback,
                call(target, event) {
                    callback.call(target, event);
                },
            };
        }
        if (callback && typeof callback.handleEvent === 'function') {
            return {
                original: callback,
                call(_target, event) {
                    callback.handleEvent.call(callback, event);
                },
            };
        }
        return null;
    }

    function isAbortSignal(value) {
        return value && typeof value === 'object' && value[ABORT_SIGNAL_FLAG] === true;
    }

    function removeListenerEntry(record, type, entry, bucket, indexHint) {
        if (!entry || entry.removed) {
            return;
        }
        entry.removed = true;
        if (entry.signal && typeof entry.abortListener === 'function') {
            entry.signal.removeEventListener('abort', entry.abortListener);
            entry.abortListener = null;
        }
        const buckets = getListenerBuckets(record, type, false);
        if (!buckets) {
            return;
        }
        const targetBucket = bucket ?? (entry.capture ? buckets.capture : buckets.bubble);
        if (!targetBucket) {
            return;
        }
        if (typeof indexHint === 'number' && targetBucket[indexHint] === entry) {
            targetBucket.splice(indexHint, 1);
        } else {
            let found = false;
            for (let i = targetBucket.length - 1; i >= 0; i--) {
                if (targetBucket[i] === entry) {
                    targetBucket.splice(i, 1);
                    found = true;
                    break;
                }
            }
            if (!found) {
                // Rebuild bucket without the entry as a fallback.
                for (let i = targetBucket.length - 1; i >= 0; i--) {
                    if (targetBucket[i] && targetBucket[i].removed) {
                        targetBucket.splice(i, 1);
                    }
                }
            }
        }
        decrementDomListener(record, type);
        if (entry.signal && entry.abortListener && !entry.signal.aborted) {
            entry.signal.removeEventListener('abort', entry.abortListener);
        }
        entry.abortListener = null;
        entry.listener = {
            call() {},
        };
        entry.originalCallback = null;
    }

    function blockEventType(record, type) {
        const blocked = record.blocked;
        const current = blocked.get(type) ?? 0;
        blocked.set(type, current + 1);
        queueMicrotask(() => {
            const remaining = blocked.get(type);
            if (remaining == null) {
                return;
            }
            if (remaining <= 1) {
                blocked.delete(type);
            } else {
                blocked.set(type, remaining - 1);
            }
        });
    }

    function addEventListenerInternal(target, type, listener, options) {
        target = normalizeEventTargetReceiver(target);
        const normalizedType = normalizeEventType(type);
        if (!normalizedType) {
            return;
        }
        const { capture, once, passive, signal, signalProvided } = normalizeAddOptions(options);
        const handler = normalizeCallback(listener);
        if (!handler && !signalProvided) {
            return;
        }
        if (signalProvided) {
            if (!isAbortSignal(signal)) {
                throw new TypeError('The "signal" option must be an instance of AbortSignal');
            }
            if (signal.aborted) {
                return;
            }
        }
        if (!handler) {
            return;
        }
        const record = ensureEventTargetRecord(target);
        const buckets = getListenerBuckets(record, normalizedType, true);
        const bucket = capture ? buckets.capture : buckets.bubble;
        for (const existing of bucket) {
            if (existing.originalCallback === listener && existing.capture === capture) {
                return;
            }
        }
        let effectiveListener = handler;
        if (signalProvided && signal) {
            effectiveListener = {
                call(target, event) {
                    if (signal.aborted) {
                        return;
                    }
                    handler.call(target, event);
                },
            };
        }
        const entry = {
            listener: effectiveListener,
            originalCallback: listener,
            capture,
            once,
            passive: !!passive,
            signal: signalProvided ? signal : null,
            removed: false,
            ownerRecord: record,
            eventType: normalizedType,
            abortListener: null,
        };
        bucket.push(entry);
        incrementDomListener(record, normalizedType);
        if (signalProvided && signal) {
            const abortListener = () => {
                removeListenerEntry(record, normalizedType, entry);
                blockEventType(record, normalizedType);
            };
            entry.abortListener = abortListener;
            signal.addEventListener('abort', abortListener, { once: true });
        }
    }

    function removeEventListenerInternal(target, type, listener, options) {
        target = normalizeEventTargetReceiver(target);
        const normalizedType = normalizeEventType(type);
        if (!normalizedType) {
            return;
        }
        const { capture } = normalizeRemoveOptions(options);
        const record = getEventTargetRecord(target, false);
        if (!record) {
            return;
        }
        const buckets = getListenerBuckets(record, normalizedType, false);
        if (!buckets) {
            return;
        }
        const bucket = capture ? buckets.capture : buckets.bubble;
        if (!bucket) {
            return;
        }
        for (let i = 0; i < bucket.length; i++) {
            const entry = bucket[i];
            if (entry.originalCallback === listener && entry.capture === capture) {
                removeListenerEntry(record, normalizedType, entry, bucket, i);
                break;
            }
        }
    }

    const EventTargetCtor = function EventTarget() {
        if (!(this instanceof EventTargetCtor)) {
            throw new TypeError('Constructor EventTarget requires "new"');
        }
        ensureEventTargetRecord(this);
    };

    const EventTargetProto = EventTargetCtor.prototype;

    Object.defineProperty(EventTargetProto, 'constructor', {
        value: EventTargetCtor,
        configurable: true,
        writable: true,
    });

    EventTargetProto.addEventListener = function (type, listener, options) {
        addEventListenerInternal(this, type, listener, options);
    };

    EventTargetProto.removeEventListener = function (type, listener, options) {
        removeEventListenerInternal(this, type, listener, options);
    };

    EventTargetProto.dispatchEvent = function (event) {
        const target = normalizeEventTargetReceiver(this);
        const result = dispatchEventInternal(target, event, null);
        return !result.defaultPrevented;
    };

    Object.defineProperty(EventTargetProto, Symbol.toStringTag, {
        value: 'EventTarget',
        configurable: true,
    });

    ensureEventTargetRecord(global);
    global.EventTarget = EventTargetCtor;
    global.addEventListener = EventTargetProto.addEventListener;
    global.removeEventListener = EventTargetProto.removeEventListener;
    global.dispatchEvent = EventTargetProto.dispatchEvent;

    function ensureDomException() {
        if (typeof global.DOMException === 'function') {
            return;
        }
        const DOMExceptionCtor = function DOMException(message = '', name = 'Error') {
            this.message = String(message);
            this.name = String(name);
            this.code = DOM_EXCEPTION_CODES[this.name] ?? 0;
        };
        DOMExceptionCtor.prototype = Object.create(Error.prototype);
        Object.defineProperty(DOMExceptionCtor.prototype, 'constructor', {
            value: DOMExceptionCtor,
            configurable: true,
            writable: true,
        });
        Object.defineProperty(DOMExceptionCtor.prototype, 'toString', {
            value() {
                return `${this.name}: ${this.message}`;
            },
            configurable: true,
        });
        global.DOMException = DOMExceptionCtor;
    }

    function domException(name, message) {
        ensureDomException();
        return new global.DOMException(message, name);
    }

    function initializeEventInstance(event, type, init, trusted) {
        if (type == null) {
            throw new TypeError('Failed to construct "Event": 1 argument required');
        }
        const typeString = String(type);
        if (typeString === '') {
            throw new TypeError('Failed to construct "Event": The event type cannot be the empty string');
        }
        const options = init && typeof init === 'object' ? init : {};
        event.type = typeString;
        event.bubbles = !!options.bubbles;
        event.cancelable = !!options.cancelable;
        event.composed = !!options.composed;
        event.defaultPrevented = !!options.defaultPrevented;
        event.isTrusted = !!trusted;
        event.target = null;
        event.currentTarget = null;
        event.srcElement = null;
        event.eventPhase = 0;
        event.timeStamp = Date.now();
        event._propagationStopped = false;
        event._immediatePropagationStopped = false;
        event._passiveListener = false;
        event._redrawRequested = false;
        event._dispatchFlag = false;
        event._initialized = true;
        event._path = [];
    }

    function prepareEventForDispatch(event, target, path) {
        event._dispatchFlag = true;
        event._propagationStopped = false;
        event._immediatePropagationStopped = false;
        event._redrawRequested = false;
        event._passiveListener = false;
        event._path = path.slice();
        event.target = target;
        event.srcElement = target;
        event.currentTarget = null;
        event.eventPhase = 0;
    }

    function finalizeEventAfterDispatch(event) {
        event._dispatchFlag = false;
        event.currentTarget = null;
        event.eventPhase = 0;
        event._path = [];
        event._passiveListener = false;
    }

    function activeListeners(record, type, capture) {
        const buckets = getListenerBuckets(record, type, false);
        if (!buckets) {
            return [];
        }
        const bucket = capture ? buckets.capture : buckets.bubble;
        if (!bucket || bucket.length === 0) {
            return [];
        }
        const result = [];
        for (const entry of bucket) {
            if (!entry || entry.removed) {
                continue;
            }
            if (entry.signal && entry.signal.aborted) {
                removeListenerEntry(record, type, entry);
                continue;
            }
            result.push(entry);
        }
        return result;
    }

    function invokeListenerList(target, type, event, listeners, phase) {
        if (listeners.length === 0) {
            return;
        }
        const record = getEventTargetRecord(target, false);
        if (!record) {
            return;
        }
        const snapshot = listeners.slice();

        for (const entry of snapshot) {
            if (!entry || entry.removed) {
                continue;
            }
            if (entry.signal && entry.signal.aborted) {
                removeListenerEntry(record, type, entry);
                continue;
            }

            event.currentTarget = target;
            event.eventPhase = phase;
            event._passiveListener = !!entry.passive;

            try {
                entry.listener.call(target, event);
            } catch (error) {
                const descriptor = `listener failure: ${error instanceof Error ? error.message : error} | listenerType=${typeof entry.listener} | callType=${typeof (entry.listener && entry.listener.call)}`;
                throw new Error(descriptor);
            }

            event._passiveListener = false;

            if (entry.once) {
                removeListenerEntry(record, type, entry);
            }

            if (event._immediatePropagationStopped) {
                break;
            }
        }
    }

    function buildPropagationPath(targetNode, providedHandles) {
        if (Array.isArray(providedHandles) && providedHandles.length > 0) {
            const path = providedHandles
                .map((handle) => wrapHandle(handle))
                .filter((node) => node != null);
            if (path.length === 0) {
                return [targetNode];
            }
            if (path[0] !== targetNode) {
                path.unshift(targetNode);
            }
            const last = path[path.length - 1];
            if (last !== global.document) {
                path.push(global.document);
            }
            return path;
        }

        const path = [];
        let current = targetNode;
        while (current) {
            path.push(current);
            if (!current.parentNode || current === global.document) {
                break;
            }
            current = current.parentNode;
        }
        const last = path[path.length - 1];
        const shouldAppendDocument =
            global.document &&
            last !== global.document &&
            targetNode &&
            typeof targetNode === 'object' &&
            targetNode !== global &&
            targetNode !== global.document &&
            (HANDLE in targetNode);
        if (shouldAppendDocument) {
            path.push(global.document);
        }
        return path;
    }

    function dispatchEventInternal(target, event, providedPath) {
        if (event == null || (typeof event !== 'object' && typeof event !== 'function')) {
            throw new TypeError(
                'Failed to execute "dispatchEvent" on "EventTarget": parameter 1 is not of type "Event"',
            );
        }
        const typeValue = event.type;
        if (typeof typeValue !== 'string') {
            throw new TypeError('Failed to execute "dispatchEvent": The event.type property must be a string');
        }
        if (event._dispatchFlag) {
            throw domException('InvalidStateError', 'The event is already being dispatched');
        }
        if (event._initialized === false) {
            throw domException('InvalidStateError', 'The event has not been initialized');
        }
        if (typeValue.length === 0) {
            throw new TypeError('Failed to execute "dispatchEvent": The event type cannot be the empty string');
        }

        const normalizedType = normalizeEventType(typeValue);
        const targetRecordInitial = getEventTargetRecord(target, false);
        if (targetRecordInitial) {
            const blockedCount = targetRecordInitial.blocked.get(normalizedType) ?? 0;
            if (blockedCount > 0) {
                return {
                    defaultPrevented: false,
                    redrawRequested: false,
                    propagationStopped: false,
                };
            }
        }
        const path = providedPath ?? buildPropagationPath(target, null);
        prepareEventForDispatch(event, target, path);

        const ancestors = path.slice(1);
        const captureTargets = ancestors.slice().reverse();

        for (const node of captureTargets) {
            if (event._propagationStopped) {
                break;
            }
            const record = getEventTargetRecord(node, false);
            if (!record) {
                continue;
            }
            const listeners = activeListeners(record, normalizedType, true);
            invokeListenerList(node, normalizedType, event, listeners, CAPTURING_PHASE);
        }

        if (!event._propagationStopped) {
            const targetRecord = targetRecordInitial;
            if (targetRecord) {
                const captureListeners = activeListeners(targetRecord, normalizedType, true);
                if (captureListeners.length > 0) {
                    invokeListenerList(target, normalizedType, event, captureListeners, AT_TARGET);
                }
                if (!event._propagationStopped) {
                    const bubbleListeners = activeListeners(targetRecord, normalizedType, false);
                    if (bubbleListeners.length > 0) {
                        invokeListenerList(target, normalizedType, event, bubbleListeners, AT_TARGET);
                    }
                }
            }
        }

        if (!event._propagationStopped && event.bubbles) {
            for (const node of ancestors) {
                if (event._propagationStopped) {
                    break;
                }
                const record = getEventTargetRecord(node, false);
                if (!record) {
                    continue;
                }
                const bubbleListeners = activeListeners(record, normalizedType, false);
                if (bubbleListeners.length === 0) {
                    continue;
                }
                invokeListenerList(node, normalizedType, event, bubbleListeners, BUBBLING_PHASE);
            }
        }

        const result = {
            defaultPrevented: !!event.defaultPrevented,
            redrawRequested: !!event._redrawRequested,
            propagationStopped: !!event._propagationStopped,
        };

        finalizeEventAfterDispatch(event);

        return result;
    }

    function toHandle(node) {
        if (node == null) {
            return null;
        }
        const handle = node[HANDLE];
        if (handle == null) {
            throw new TypeError('Node is not managed by Frontier');
        }
        return String(handle);
    }

    function mapHandles(handles) {
        if (!handles) {
            return [];
        }
        return handles.map((handle) => String(handle));
    }

    function collectDescendants(handle) {
        const result = [];
        const stack = [String(handle)];
        while (stack.length > 0) {
            const current = stack.pop();
            const children = mapHandles(global.__frontier_dom_child_nodes(current));
            for (const child of children) {
                result.push(child);
                stack.push(child);
            }
        }
        return result;
    }

    function defineConstructor(name, proto) {
        const ctor = function () {};
        ctor.prototype = proto;
        Object.defineProperty(proto, 'constructor', {
            value: ctor,
            configurable: true,
            writable: true,
        });
        global[name] = ctor;
    }

    function isFragment(node) {
        return node && node.nodeType === 11 && typeof node.__flush === 'function';
    }

    function wrapHandle(handle, typeHint) {
        if (handle == null) {
            return null;
        }
        const key = String(handle);
        if (NODE_CACHE.has(key)) {
            return NODE_CACHE.get(key);
        }
        const wrapper = createWrapper(key, typeHint);
        NODE_CACHE.set(key, wrapper);
        return wrapper;
    }

    function createWrapper(handle, typeHint) {
        const type = typeHint ?? global.__frontier_dom_node_type(handle);
        let proto;
        switch (type) {
            case 1:
                proto = ElementProto;
                break;
            case 3:
                proto = TextProto;
                break;
            case 8:
                proto = CommentProto;
                break;
            case 9:
                proto = DocumentProto;
                break;
            default:
                proto = NodeProto;
                break;
        }
        const node = Object.create(proto);
        node[HANDLE] = String(handle);
        associateEventTargetHandle(node, handle);
        return node;
    }

    const NodeProto = {
        get nodeType() {
            return global.__frontier_dom_node_type(this[HANDLE]);
        },
        get nodeName() {
            return global.__frontier_dom_node_name(this[HANDLE]);
        },
        get ownerDocument() {
            return global.document;
        },
        get parentNode() {
            const handle = global.__frontier_dom_parent(this[HANDLE]);
            return wrapHandle(handle);
        },
        get firstChild() {
            const handle = global.__frontier_dom_first_child(this[HANDLE]);
            return wrapHandle(handle);
        },
        get nextSibling() {
            const handle = global.__frontier_dom_next_sibling(this[HANDLE]);
            return wrapHandle(handle);
        },
        get previousSibling() {
            const handle = global.__frontier_dom_previous_sibling(this[HANDLE]);
            return wrapHandle(handle);
        },
        get childNodes() {
            const handles = mapHandles(global.__frontier_dom_child_nodes(this[HANDLE]));
            return handles.map((handle) => wrapHandle(handle));
        },
        hasChildNodes() {
            return (global.__frontier_dom_child_nodes(this[HANDLE]) || []).length > 0;
        },
        appendChild(node) {
            if (isFragment(node)) {
                node.__flush(this, null);
                return node;
            }
            global.__frontier_dom_append_child(this[HANDLE], toHandle(node));
            return node;
        },
        insertBefore(node, reference) {
            if (isFragment(node)) {
                node.__flush(this, reference || null);
                return node;
            }
            const referenceHandle = reference == null ? null : toHandle(reference);
            global.__frontier_dom_insert_before(this[HANDLE], toHandle(node), referenceHandle);
            return node;
        },
        removeChild(node) {
            global.__frontier_dom_remove_child(this[HANDLE], toHandle(node));
            return node;
        },
        replaceChild(newNode, oldNode) {
            global.__frontier_dom_replace_child(this[HANDLE], toHandle(newNode), toHandle(oldNode));
            return oldNode;
        },
        cloneNode(deep = false) {
            const handle = global.__frontier_dom_clone_node(this[HANDLE], !!deep);
            return wrapHandle(handle);
        },
        get textContent() {
            const value = global.__frontier_dom_get_text(this[HANDLE]);
            return value == null ? null : value;
        },
        set textContent(value) {
            const stale = collectDescendants(this[HANDLE]);
            global.__frontier_dom_set_text(this[HANDLE], value == null ? '' : String(value));
            for (const handle of stale) {
                NODE_CACHE.delete(handle);
            }
        },
        contains(node) {
            if (!node) {
                return false;
            }
            let current = node;
            while (current) {
                if (current === this) {
                    return true;
                }
                current = current.parentNode;
            }
            return false;
        },
        get isConnected() {
            let current = this;
            while (current) {
                if (current === global.document) {
                    return true;
                }
                current = current.parentNode;
            }
            return false;
        },
        normalize() {},
        addEventListener(type, listener, options) {
            EventTargetProto.addEventListener.call(this, type, listener, options);
        },
        removeEventListener(type, listener, options) {
            EventTargetProto.removeEventListener.call(this, type, listener, options);
        },
        dispatchEvent(event) {
            return EventTargetProto.dispatchEvent.call(this, event);
        },
    };

    Object.setPrototypeOf(NodeProto, EventTargetProto);

    const CharacterDataProto = Object.create(NodeProto);
    Object.defineProperty(CharacterDataProto, 'data', {
        get() {
            const value = global.__frontier_dom_get_text(this[HANDLE]);
            return value == null ? '' : value;
        },
        set(value) {
            const stale = collectDescendants(this[HANDLE]);
            global.__frontier_dom_set_text(this[HANDLE], value == null ? '' : String(value));
            for (const handle of stale) {
                NODE_CACHE.delete(handle);
            }
        },
    });
    Object.defineProperty(CharacterDataProto, 'nodeValue', {
        get() {
            return this.data;
        },
        set(value) {
            this.data = value;
        },
    });

    const TextProto = Object.create(CharacterDataProto);
    const CommentProto = Object.create(CharacterDataProto);

    const ElementProto = Object.create(NodeProto);
    Object.defineProperty(ElementProto, 'tagName', {
        get() {
            return this.nodeName;
        },
    });
    Object.defineProperty(ElementProto, 'localName', {
        get() {
            return this.nodeName.toLowerCase();
        },
    });
    Object.defineProperty(ElementProto, 'id', {
        get() {
            return this.getAttribute('id') ?? '';
        },
        set(value) {
            this.setAttribute('id', value);
        },
    });
    Object.defineProperty(ElementProto, 'className', {
        get() {
            return this.getAttribute('class') ?? '';
        },
        set(value) {
            this.setAttribute('class', value);
        },
    });
    Object.defineProperty(ElementProto, 'classList', {
        get() {
            if (!this.__classList) {
                this.__classList = createClassList(this);
            }
            return this.__classList;
        },
    });
    Object.defineProperty(ElementProto, 'namespaceURI', {
        get() {
            return global.__frontier_dom_namespace_uri(this[HANDLE]) ?? null;
        },
    });
    Object.defineProperty(ElementProto, 'innerHTML', {
        get() {
            return global.__frontier_dom_get_html(this[HANDLE]) ?? '';
        },
        set(value) {
            const stale = collectDescendants(this[HANDLE]);
            global.__frontier_dom_set_inner_html(this[HANDLE], value == null ? '' : String(value));
            for (const handle of stale) {
                NODE_CACHE.delete(handle);
            }
        },
    });
    Object.defineProperty(ElementProto, 'children', {
        get() {
            return this.childNodes.filter((node) => node && node.nodeType === 1);
        },
    });
    Object.defineProperty(ElementProto, 'firstElementChild', {
        get() {
            return this.children[0] ?? null;
        },
    });
    Object.defineProperty(ElementProto, 'lastElementChild', {
        get() {
            const children = this.children;
            return children[children.length - 1] ?? null;
        },
    });
    Object.defineProperty(ElementProto, 'nextElementSibling', {
        get() {
            let sibling = this.nextSibling;
            while (sibling && sibling.nodeType !== 1) {
                sibling = sibling.nextSibling;
            }
            return sibling ?? null;
        },
    });
    Object.defineProperty(ElementProto, 'previousElementSibling', {
        get() {
            let sibling = this.previousSibling;
            while (sibling && sibling.nodeType !== 1) {
                sibling = sibling.previousSibling;
            }
            return sibling ?? null;
        },
    });
    ElementProto.getAttribute = function (name) {
        const value = global.__frontier_dom_get_attribute(this[HANDLE], String(name));
        return value == null ? null : value;
    };
    ElementProto.setAttribute = function (name, value) {
        global.__frontier_dom_set_attribute(this[HANDLE], String(name), value == null ? '' : String(value));
    };
    ElementProto.setAttributeNS = function (_ns, name, value) {
        this.setAttribute(name, value);
    };
    ElementProto.removeAttribute = function (name) {
        global.__frontier_dom_remove_attribute(this[HANDLE], String(name));
    };
    ElementProto.removeAttributeNS = function (_ns, name) {
        this.removeAttribute(name);
    };
    ElementProto.hasAttribute = function (name) {
        return this.getAttribute(name) != null;
    };
    ElementProto.append = function (...nodes) {
        nodes.forEach((node) => {
            if (typeof node === 'string') {
                this.appendChild(global.document.createTextNode(node));
            } else {
                this.appendChild(node);
            }
        });
    };
    ElementProto.prepend = function (...nodes) {
        let reference = this.firstChild;
        nodes.forEach((node) => {
            if (typeof node === 'string') {
                node = global.document.createTextNode(node);
            }
            this.insertBefore(node, reference);
        });
    };
    ElementProto.matches = function () {
        return false;
    };
    ElementProto.closest = function () {
        return null;
    };
    ElementProto.focus = function () {};
    ElementProto.blur = function () {};

    function createStyleProxy(element) {
        const cache = Object.create(null);
        function write() {
            const entries = Object.entries(cache)
                .filter(([, value]) => value != null && value !== '')
                .map(([name, value]) => `${name}: ${value}`);
            element.setAttribute('style', entries.join('; '));
        }
        return new Proxy(cache, {
            get(target, prop) {
                if (prop === 'setProperty') {
                    return (name, value) => {
                        target[String(name)] = value == null ? '' : String(value);
                        write();
                    };
                }
                if (prop === 'removeProperty') {
                    return (name) => {
                        delete target[String(name)];
                        write();
                    };
                }
                if (prop === 'cssText') {
                    return element.getAttribute('style') ?? '';
                }
                return target[prop];
            },
            set(target, prop, value) {
                target[prop] = value == null ? '' : String(value);
                write();
                return true;
            },
            deleteProperty(target, prop) {
                delete target[prop];
                write();
                return true;
            },
        });
    }

    function createClassList(element) {
        function readTokens() {
            const value = element.className;
            if (!value) {
                return new Set();
            }
            return new Set(value.trim().split(/\s+/));
        }
        function writeTokens(tokens) {
            element.className = Array.from(tokens).join(' ');
        }
        return {
            add(...tokens) {
                const set = readTokens();
                for (const token of tokens) {
                    set.add(String(token));
                }
                writeTokens(set);
            },
            remove(...tokens) {
                const set = readTokens();
                for (const token of tokens) {
                    set.delete(String(token));
                }
                writeTokens(set);
            },
            toggle(token, force) {
                const set = readTokens();
                const value = String(token);
                const has = set.has(value);
                const shouldAdd = force ?? !has;
                if (shouldAdd) {
                    set.add(value);
                } else {
                    set.delete(value);
                }
                writeTokens(set);
                return shouldAdd;
            },
            contains(token) {
                return readTokens().has(String(token));
            },
            get value() {
                return element.className;
            },
        };
    }

    Object.defineProperty(ElementProto, 'style', {
        get() {
            if (!this.__styleProxy) {
                this.__styleProxy = createStyleProxy(this);
            }
            return this.__styleProxy;
        },
    });

    Object.defineProperty(ElementProto, 'dataset', {
        get() {
            if (!this.__datasetProxy) {
                this.__datasetProxy = new Proxy(
                    {},
                    {
                        get: (_, prop) => this.getAttribute(`data-${String(prop)}`) ?? undefined,
                        set: (_, prop, value) => {
                            this.setAttribute(`data-${String(prop)}`, value);
                            return true;
                        },
                        deleteProperty: (_, prop) => {
                            this.removeAttribute(`data-${String(prop)}`);
                            return true;
                        },
                        has: (_, prop) => this.hasAttribute(`data-${String(prop)}`),
                        ownKeys: () => [],
                        getOwnPropertyDescriptor: () => ({ configurable: true, enumerable: true }),
                    },
                );
            }
            return this.__datasetProxy;
        },
    });

    const DocumentProto = Object.create(NodeProto);
    DocumentProto.createElement = function (name) {
        const handle = global.__frontier_dom_create_element(String(name));
        return wrapHandle(handle, 1);
    };
    DocumentProto.createElementNS = function (namespace, name) {
        const handle = global.__frontier_dom_create_element_ns(
            namespace == null ? null : String(namespace),
            String(name),
        );
        return wrapHandle(handle, 1);
    };
    DocumentProto.createTextNode = function (value) {
        const handle = global.__frontier_dom_create_text(value == null ? '' : String(value));
        return wrapHandle(handle, 3);
    };
    DocumentProto.createComment = function (value) {
        const handle = global.__frontier_dom_create_comment(value == null ? '' : String(value));
        return wrapHandle(handle, 8);
    };
    DocumentProto.createDocumentFragment = function () {
        return createDocumentFragment();
    };
    DocumentProto.createEvent = function (interfaceName) {
        const name = String(interfaceName ?? '');
        const event = createLegacyEvent(name);
        if (name === 'CustomEvent' && CustomEventCtorRef) {
            Object.setPrototypeOf(event, CustomEventCtorRef.prototype);
            event.detail = null;
        } else if ((name === 'MessageEvent' || name === 'MessageEvents') && MessageEventCtorRef) {
            Object.setPrototypeOf(event, MessageEventCtorRef.prototype);
            event.data = null;
            event.origin = '';
            event.lastEventId = '';
            event.source = null;
            event.ports = [];
        } else if (!EventCtorRef || Object.getPrototypeOf(event) !== EventCtorRef.prototype) {
            if (EventCtorRef) {
                Object.setPrototypeOf(event, EventCtorRef.prototype);
            }
        }
        return event;
    };
    DocumentProto.getElementById = function (id) {
        const handle = global.__frontier_dom_get_handle_by_id(String(id));
        return wrapHandle(handle, 1);
    };
    Object.defineProperty(DocumentProto, 'documentElement', {
        get() {
            const handles = mapHandles(global.__frontier_dom_child_nodes(this[HANDLE]));
            for (const handle of handles) {
                const node = wrapHandle(handle);
                if (node && node.nodeType === 1) {
                    return node;
                }
            }
            return null;
        },
    });
    Object.defineProperty(DocumentProto, 'body', {
        get() {
            const root = this.documentElement;
            if (!root) {
                return null;
            }
            const nodes = root.childNodes;
            for (const node of nodes) {
                if (node && node.nodeType === 1 && node.nodeName === 'BODY') {
                    return node;
                }
            }
            return null;
        },
    });
    Object.defineProperty(DocumentProto, 'head', {
        get() {
            const root = this.documentElement;
            if (!root) {
                return null;
            }
            const nodes = root.childNodes;
            for (const node of nodes) {
                if (node && node.nodeType === 1 && node.nodeName === 'HEAD') {
                    return node;
                }
            }
            return null;
        },
    });
    Object.defineProperty(DocumentProto, 'defaultView', {
        get() {
            return global;
        },
    });
    DocumentProto.contains = function (node) {
        return this === node || this.body?.contains(node) || false;
    };

    const FragmentProto = {
        nodeType: 11,
        nodeName: '#document-fragment',
        appendChild(node) {
            if (isFragment(node)) {
                node.__flush(this, null);
                return node;
            }
            this.__children.push(node);
            return node;
        },
        insertBefore(node, reference) {
            if (isFragment(node)) {
                node.__flush(this, reference || null);
                return node;
            }
            if (!reference) {
                this.__children.push(node);
                return node;
            }
            const index = this.__children.indexOf(reference);
            if (index === -1) {
                this.__children.push(node);
            } else {
                this.__children.splice(index, 0, node);
            }
            return node;
        },
        removeChild(node) {
            const index = this.__children.indexOf(node);
            if (index !== -1) {
                this.__children.splice(index, 1);
            }
            return node;
        },
        replaceChild(newNode, oldNode) {
            const index = this.__children.indexOf(oldNode);
            if (index !== -1) {
                this.__children.splice(index, 1, newNode);
            }
            return oldNode;
        },
        cloneNode(deep = false) {
            const fragment = createDocumentFragment();
            if (deep) {
                this.__children.forEach((child) => fragment.appendChild(child.cloneNode(true)));
            }
            return fragment;
        },
        __flush(target, reference) {
            const children = this.__children.slice();
            this.__children.length = 0;
            for (const child of children) {
                if (reference) {
                    target.insertBefore(child, reference);
                } else {
                    target.appendChild(child);
                }
            }
        },
    };
    Object.setPrototypeOf(FragmentProto, EventTargetProto);
    Object.defineProperty(FragmentProto, 'firstChild', {
        get() {
            return this.__children[0] ?? null;
        },
    });
    Object.defineProperty(FragmentProto, 'childNodes', {
        get() {
            return this.__children.slice();
        },
    });
    Object.defineProperty(FragmentProto, 'textContent', {
        get() {
            return this.__children.map((child) => child.textContent ?? '').join('');
        },
        set(value) {
            this.__children.length = 0;
            if (value && value !== '') {
                this.__children.push(global.document.createTextNode(String(value)));
            }
        },
    });

    function createDocumentFragment() {
        const fragment = Object.create(FragmentProto);
        fragment.__children = [];
        fragment.ownerDocument = global.document;
        ensureEventTargetRecord(fragment);
        return fragment;
    }

    const DocumentFragmentCtor = function DocumentFragment() {};
    DocumentFragmentCtor.prototype = FragmentProto;
    Object.defineProperty(FragmentProto, 'constructor', { value: DocumentFragmentCtor });
    global.DocumentFragment = DocumentFragmentCtor;

    defineConstructor('Node', NodeProto);
    defineConstructor('Element', ElementProto);
    defineConstructor('Text', TextProto);
    defineConstructor('Comment', CommentProto);
    defineConstructor('Document', DocumentProto);
    global.HTMLElement = global.Element;

    function ensureDocument() {
        try {
            const docHandle = global.__frontier_dom_document_handle();
            let document = global.document;
            if (typeof document !== 'object' || document === null) {
                document = {};
            }
            Object.setPrototypeOf(document, DocumentProto);
            document[HANDLE] = String(docHandle);
            associateEventTargetHandle(document, docHandle);
            global.document = document;
            NODE_CACHE.set(String(docHandle), document);
            return true;
        } catch (err) {
            return false;
        }
    }

    function seedDocumentCache() {
        const documentHandle = global.document && global.document[HANDLE];
        if (!documentHandle) {
            return;
        }
        const children = mapHandles(global.__frontier_dom_child_nodes(documentHandle));
        for (const handle of children) {
            wrapHandle(handle);
        }
    }

    function refreshDocument() {
        if (ensureDocument()) {
            seedDocumentCache();
        }
    }

    refreshDocument();
    installEventConstructors();
    installMessagingPolyfills();
    installMutationObserverStub();
    installHtmlElementConstructors();

    frontier.wrapHandle = wrapHandle;
    frontier.collectDescendants = collectDescendants;
    frontier.__refreshDocument = refreshDocument;

    const CAPTURING_PHASE = 1;
    const AT_TARGET = 2;
    const BUBBLING_PHASE = 3;

    function createEvent(type, target, detail, trusted = false) {
        const init = detail && typeof detail === 'object' ? detail : {};
        let proto = EventCtorRef ? EventCtorRef.prototype : null;
        if (!proto && typeof global.Event === 'function') {
            proto = global.Event.prototype;
        }
        const event = proto ? Object.create(proto) : {};
        initializeEventInstance(event, type, init, !!trusted);
        for (const key of Object.keys(init)) {
            if (key === 'bubbles' || key === 'cancelable' || key === 'composed' || key === 'defaultPrevented') {
                continue;
            }
            event[key] = init[key];
        }
        if (init.defaultPrevented) {
            event.defaultPrevented = true;
        }
        if (target) {
            event.target = target;
            event.srcElement = target;
        }
        return event;
    }

    function createLegacyEvent(_interfaceName) {
        const event = EventCtorRef ? Object.create(EventCtorRef.prototype) : {};
        event.type = '';
        event.bubbles = false;
        event.cancelable = false;
        event.composed = false;
        event.defaultPrevented = false;
        event.isTrusted = false;
        event.target = null;
        event.currentTarget = null;
        event.srcElement = null;
        event.eventPhase = 0;
        event.timeStamp = Date.now();
        event._propagationStopped = false;
        event._immediatePropagationStopped = false;
        event._passiveListener = false;
        event._redrawRequested = false;
        event._dispatchFlag = false;
        event._initialized = false;
        event._path = [];
        return event;
    }

    function installEventConstructors() {
        const EventCtor = function Event(type, init = {}) {
            if (!(this instanceof EventCtor)) {
                throw new TypeError('Constructor Event requires "new"');
            }
            initializeEventInstance(this, type, init, false);
        };

        EventCtor.prototype = {
            constructor: EventCtor,
            preventDefault() {
                if (!this.cancelable || this._passiveListener) {
                    return;
                }
                this.defaultPrevented = true;
            },
            stopPropagation() {
                this._propagationStopped = true;
            },
            stopImmediatePropagation() {
                this._propagationStopped = true;
                this._immediatePropagationStopped = true;
            },
            composedPath() {
                return Array.isArray(this._path) ? this._path.slice() : [];
            },
            requestRedraw() {
                this._redrawRequested = true;
            },
            initEvent(type, bubbles = false, cancelable = false) {
                if (this._dispatchFlag) {
                    return;
                }
                const value = String(type ?? '');
                this.type = value;
                this.bubbles = !!bubbles;
                this.cancelable = !!cancelable;
                this.defaultPrevented = false;
                this._propagationStopped = false;
                this._immediatePropagationStopped = false;
                this._initialized = value.length > 0;
            },
        };

        Object.defineProperty(EventCtor.prototype, Symbol.toStringTag, {
            value: 'Event',
            configurable: true,
        });

        Object.defineProperty(EventCtor.prototype, 'cancelBubble', {
            get() {
                return !!this._propagationStopped;
            },
            set(value) {
                if (value) {
                    this.stopPropagation();
                }
            },
            configurable: true,
        });

        Object.defineProperty(EventCtor.prototype, 'returnValue', {
            get() {
                return !this.defaultPrevented;
            },
            set(value) {
                if (value === false) {
                    this.preventDefault();
                }
            },
            configurable: true,
        });

        EventCtorRef = EventCtor;
        global.Event = EventCtor;

        const MessageEventCtor = function MessageEvent(type, init = {}) {
            if (!(this instanceof MessageEventCtor)) {
                throw new TypeError('Constructor MessageEvent requires "new"');
            }
            initializeEventInstance(this, type, init, false);
            this.data = Object.prototype.hasOwnProperty.call(init ?? {}, 'data') ? init.data : null;
            this.origin = Object.prototype.hasOwnProperty.call(init ?? {}, 'origin') ? init.origin : '';
            this.lastEventId = Object.prototype.hasOwnProperty.call(init ?? {}, 'lastEventId')
                ? init.lastEventId
                : '';
            this.source = Object.prototype.hasOwnProperty.call(init ?? {}, 'source') ? init.source : null;
            this.ports = Object.prototype.hasOwnProperty.call(init ?? {}, 'ports') ? init.ports : [];
        };
        MessageEventCtor.prototype = Object.create(EventCtor.prototype);
        Object.defineProperty(MessageEventCtor.prototype, 'constructor', {
            value: MessageEventCtor,
            configurable: true,
            writable: true,
        });
        Object.defineProperty(MessageEventCtor.prototype, Symbol.toStringTag, {
            value: 'MessageEvent',
            configurable: true,
        });
        MessageEventCtorRef = MessageEventCtor;
        global.MessageEvent = MessageEventCtor;

        const CustomEventCtor = function CustomEvent(type, init = {}) {
            if (!(this instanceof CustomEventCtor)) {
                throw new TypeError('Constructor CustomEvent requires "new"');
            }
            initializeEventInstance(this, type, init, false);
            this.detail = Object.prototype.hasOwnProperty.call(init ?? {}, 'detail') ? init.detail : null;
        };
        CustomEventCtor.prototype = Object.create(EventCtor.prototype);
        Object.defineProperty(CustomEventCtor.prototype, 'constructor', {
            value: CustomEventCtor,
            configurable: true,
            writable: true,
        });
        CustomEventCtor.prototype.initCustomEvent = function (type, bubbles, cancelable, detail) {
            if (this._dispatchFlag) {
                return;
            }
            const value = String(type ?? '');
            this.type = value;
            this.bubbles = !!bubbles;
            this.cancelable = !!cancelable;
            this.detail = detail;
            this.defaultPrevented = false;
            this._initialized = value.length > 0;
        };
        Object.defineProperty(CustomEventCtor.prototype, Symbol.toStringTag, {
            value: 'CustomEvent',
            configurable: true,
        });
        CustomEventCtorRef = CustomEventCtor;
        global.CustomEvent = CustomEventCtor;
    }

    function abortSignalInternal(signal, reason) {
        if (signal._aborted) {
            return;
        }
        signal._aborted = true;
        signal._reason = reason ?? domException('AbortError', 'The operation was aborted.');
        const abortEvent = createEvent('abort', signal, { bubbles: false, cancelable: false }, false);
        EventTargetProto.dispatchEvent.call(signal, abortEvent);
    }

    const AbortSignalCtor = function AbortSignal() {
        throw new TypeError('Illegal constructor');
    };
    AbortSignalCtor.prototype = Object.create(EventTargetProto);
    Object.defineProperty(AbortSignalCtor.prototype, 'constructor', {
        value: AbortSignalCtor,
        configurable: true,
        writable: true,
    });
    Object.defineProperty(AbortSignalCtor.prototype, Symbol.toStringTag, {
        value: 'AbortSignal',
        configurable: true,
    });
    Object.defineProperty(AbortSignalCtor.prototype, 'aborted', {
        get() {
            return !!this._aborted;
        },
        configurable: true,
    });
    Object.defineProperty(AbortSignalCtor.prototype, 'reason', {
        get() {
            return this._reason;
        },
        configurable: true,
    });
    AbortSignalCtor.prototype.throwIfAborted = function () {
        if (this.aborted) {
            throw this._reason ?? domException('AbortError', 'The operation was aborted.');
        }
    };

    AbortSignalCtor.abort = function (reason) {
        const signal = Object.create(AbortSignalCtor.prototype);
        ensureEventTargetRecord(signal);
        signal._aborted = true;
        signal._reason = reason ?? domException('AbortError', 'The operation was aborted.');
        signal[ABORT_SIGNAL_FLAG] = true;
        return signal;
    };

    AbortSignalCtor.timeout = function (milliseconds) {
        const controller = new AbortControllerCtor();
        const ms = Number(milliseconds);
        if (Number.isFinite(ms) && ms >= 0) {
            setTimeout(() => {
                if (!controller.signal._aborted) {
                    abortSignalInternal(
                        controller.signal,
                        domException('TimeoutError', 'The operation timed out.'),
                    );
                }
            }, ms);
        }
        return controller.signal;
    };

    const AbortControllerCtor = function AbortController() {
        if (!(this instanceof AbortControllerCtor)) {
            throw new TypeError('Constructor AbortController requires "new"');
        }
        const signal = Object.create(AbortSignalCtor.prototype);
        ensureEventTargetRecord(signal);
        signal._aborted = false;
        signal._reason = undefined;
        signal[ABORT_SIGNAL_FLAG] = true;
        this.signal = signal;
    };
    AbortControllerCtor.prototype.abort = function (reason) {
        if (!this.signal || this.signal._aborted) {
            return;
        }
        abortSignalInternal(this.signal, reason ?? domException('AbortError', 'The operation was aborted.'));
    };
    Object.defineProperty(AbortControllerCtor.prototype, Symbol.toStringTag, {
        value: 'AbortController',
        configurable: true,
    });

    global.AbortSignal = AbortSignalCtor;
    global.AbortController = AbortControllerCtor;

    function installMessagingPolyfills() {
        if (typeof global.MessageChannel !== 'function') {
            function FrontierMessagePort() {
                this.onmessage = null;
                this._entangled = null;
            }
            FrontierMessagePort.prototype = {
                constructor: FrontierMessagePort,
                postMessage(message) {
                    const target = this._entangled;
                    if (!target) {
                        return;
                    }
                    Promise.resolve().then(() => {
                        if (typeof target.onmessage === 'function') {
                            try {
                                const event = createEvent('message', target, { data: message, source: this });
                                target.onmessage.call(target, event);
                            } catch (error) {
                                throw error;
                            }
                        }
                    });
                },
                start() {},
                close() {
                    if (this._entangled) {
                        this._entangled._entangled = null;
                        this._entangled = null;
                    }
                },
            };

            function FrontierMessageChannel() {
                const port1 = new FrontierMessagePort();
                const port2 = new FrontierMessagePort();
                port1._entangled = port2;
                port2._entangled = port1;
                this.port1 = port1;
                this.port2 = port2;
            }
            FrontierMessageChannel.prototype = {
                constructor: FrontierMessageChannel,
            };

            global.MessageChannel = FrontierMessageChannel;
            global.MessagePort = FrontierMessagePort;
        }
    }

    function installMutationObserverStub() {
        if (typeof global.MutationObserver !== 'function') {
            const MutationObserverCtor = function MutationObserver(callback) {
                if (typeof callback !== 'function') {
                    throw new TypeError('MutationObserver constructor requires a callback function');
                }
                this._callback = callback;
            };
            MutationObserverCtor.prototype = {
                constructor: MutationObserverCtor,
                observe(_target, _options) {},
                disconnect() {},
                takeRecords() {
                    return [];
                },
            };
            global.MutationObserver = MutationObserverCtor;
        }
    }

    function installHtmlElementConstructors() {
        const elementBase = typeof global.HTMLElement === 'function' ? global.HTMLElement : global.Element;
        if (typeof global.HTMLElement !== 'function' && typeof global.Element === 'function') {
            global.HTMLElement = global.Element;
        }
        if (typeof global.HTMLIFrameElement !== 'function') {
            const IFrameCtor = function HTMLIFrameElement() {};
            if (typeof elementBase === 'function' && elementBase.prototype) {
                IFrameCtor.prototype = Object.create(elementBase.prototype);
                Object.defineProperty(IFrameCtor.prototype, 'constructor', {
                    value: IFrameCtor,
                    configurable: true,
                    writable: true,
                });
            }
            global.HTMLIFrameElement = IFrameCtor;
        }
    }

    frontier.__dispatchDomEvent = function (handle, type, detail, pathHandles) {
        const target = wrapHandle(handle);
        if (!target) {
            return {
                defaultPrevented: false,
                redrawRequested: false,
                propagationStopped: false,
            };
        }
        const event = createEvent(type, target, detail || {}, true);
        const path = buildPropagationPath(target, pathHandles);
        const result = dispatchEventInternal(target, event, path);
        return result;
    };

    const TIMER_STORE = new Map();

    function toTimerId(value) {
        const num = Number(value);
        if (!Number.isFinite(num) || num <= 0) {
            return 0;
        }
        return Math.trunc(num);
    }

    function normalizeDelay(value) {
        const num = Number(value);
        if (!Number.isFinite(num) || num < 0) {
            return 0;
        }
        return num;
    }

    function ensureNativeTimer(name) {
        const fn = global[name];
        if (typeof fn !== 'function') {
            throw new Error(`${name} bridge is missing`);
        }
        return fn;
    }

    const scheduleNativeTimer = ensureNativeTimer('__frontier_schedule_timer');
    const cancelNativeTimer = ensureNativeTimer('__frontier_cancel_timer');

    function scheduleTimer(kind, delay, repeating, callback, args) {
        if (typeof callback !== 'function') {
            throw new TypeError('Timer callback must be a function');
        }
        const id = scheduleNativeTimer(kind, normalizeDelay(delay), !!repeating);
        TIMER_STORE.set(id, { callback, args, kind, repeating: !!repeating });
        return id;
    }

    frontier.__invokeTimer = function (id, timestamp) {
        const entry = TIMER_STORE.get(id);
        if (!entry) {
            return;
        }
        if (entry.kind === 'animationFrame' && typeof timestamp === 'number') {
            entry.callback.call(global, timestamp);
        } else {
            entry.callback.apply(global, entry.args);
        }
        if (!entry.repeating) {
            TIMER_STORE.delete(id);
        }
    };

    function cancelTimer(id) {
        const timerId = toTimerId(id);
        if (!timerId) {
            return;
        }
        TIMER_STORE.delete(timerId);
        cancelNativeTimer(timerId);
    }

    global.setTimeout = function (callback, delay, ...args) {
        return scheduleTimer('timeout', delay ?? 0, false, callback, args);
    };

    global.setInterval = function (callback, delay, ...args) {
        return scheduleTimer('interval', delay ?? 0, true, callback, args);
    };

    global.clearTimeout = cancelTimer;
    global.clearInterval = cancelTimer;

    global.requestAnimationFrame = function (callback) {
        if (typeof callback !== 'function') {
            throw new TypeError('requestAnimationFrame callback must be a function');
        }
        return scheduleTimer('animationFrame', 16, false, callback, []);
    };

    global.cancelAnimationFrame = cancelTimer;

    if (typeof global.queueMicrotask !== 'function') {
        global.queueMicrotask = function (callback) {
            if (typeof callback !== 'function') {
                throw new TypeError('callback must be a function');
            }
            Promise.resolve()
                .then(callback)
                .catch((error) => {
                    setTimeout(() => {
                        throw error;
                    }, 0);
                });
        };
    }

    frontier.emitDomPatch = function (patch) {
        if (!patch || typeof patch !== 'object') {
            throw new TypeError('frontier.emitDomPatch expects an object');
        }
        const handle =
            patch.handle ??
            (typeof patch.id === 'string'
                ? global.__frontier_dom_get_handle_by_id(patch.id)
                : undefined);
        if (handle == null) {
            throw new TypeError('Patch requires a "handle" field');
        }
        const normalizedHandle = String(handle);
        switch (patch.type) {
            case 'text_content': {
                const stale = collectDescendants(normalizedHandle);
                global.__frontier_dom_set_text(
                    normalizedHandle,
                    patch.value == null ? '' : String(patch.value),
                );
                for (const staleHandle of stale) {
                    NODE_CACHE.delete(staleHandle);
                }
                break;
            }
            case 'inner_html': {
                const stale = collectDescendants(normalizedHandle);
                global.__frontier_dom_set_inner_html(
                    normalizedHandle,
                    patch.value == null ? '' : String(patch.value),
                );
                for (const staleHandle of stale) {
                    NODE_CACHE.delete(staleHandle);
                }
                break;
            }
            case 'attribute': {
                global.__frontier_dom_set_attribute(
                    normalizedHandle,
                    String(patch.name),
                    patch.value == null ? '' : String(patch.value),
                );
                break;
            }
            case 'remove_attribute': {
                global.__frontier_dom_remove_attribute(normalizedHandle, String(patch.name));
                break;
            }
            default:
                throw new TypeError(`Unknown patch type: ${patch.type}`);
        }
    };
})();
"#;
