use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use blitz_dom::BaseDocument;
use blitz_traits::events::{
    BlitzImeEvent, BlitzKeyEvent, BlitzMouseButtonEvent, DomEvent, DomEventData, MouseEventButton,
};
use keyboard_types::{Location, Modifiers};
use rquickjs::{Ctx, Function, IntoJs};
use serde_json::{json, to_string as to_json_string, Map as JsonMap, Value as JsonValue};
use tracing::error;

use super::dom::{DomPatch, DomState};
use super::runtime::QuickJsEngine;

pub struct JsDomEnvironment {
    engine: QuickJsEngine,
    state: Rc<RefCell<DomState>>,
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
        install_dom_bindings(&engine, Rc::clone(&state))?;
        Ok(Self { engine, state })
    }

    pub fn is_listening(&self, event_type: &str) -> bool {
        self.state.borrow().is_listening(event_type)
    }

    pub fn dispatch_dom_event(&self, event: &DomEvent, chain: &[usize]) -> Result<DispatchOutcome> {
        let event_name = event.data.name();
        if !self.is_listening(event_name) {
            return Ok(DispatchOutcome::default());
        }

        let (target_handle, path_handles) = {
            let state = self.state.borrow();
            let target = state.handle_to_string(event.target);
            let path = chain
                .iter()
                .map(|node_id| state.handle_to_string(*node_id))
                .collect::<Vec<_>>();
            (target, path)
        };

        let detail = build_event_detail(event);
        let detail_json = to_json_string(&detail).map_err(anyhow::Error::from)?;
        let event_name_owned = event_name.to_string();
        let target_handle_clone = target_handle.clone();
        let path_handles_clone = path_handles.clone();

        let result = self
            .engine
            .with_context(|ctx| {
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
            })
            .map_err(anyhow::Error::from);

        match result {
            Ok(outcome) => Ok(outcome),
            Err(err) => {
                error!(target = "quickjs", error = %err, "failed to dispatch DOM event");
                Ok(DispatchOutcome::default())
            }
        }
    }

    pub fn eval(&self, source: &str, filename: &str) -> Result<()> {
        self.engine.eval(source, filename)
    }

    pub fn drain_mutations(&self) -> Vec<DomPatch> {
        self.state.borrow_mut().drain_mutations()
    }

    pub fn document_html(&self) -> Result<String> {
        self.state.borrow().to_html()
    }

    pub fn attach_document(&self, document: &mut BaseDocument) {
        self.state.borrow_mut().attach_document(document);
    }
}

fn install_dom_bindings(engine: &QuickJsEngine, state: Rc<RefCell<DomState>>) -> Result<()> {
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

        ctx.eval::<(), _>(DOM_BOOTSTRAP.as_bytes())?;
        Ok(())
    })
}

fn dom_error<T>(ctx: &Ctx<'_>, err: anyhow::Error) -> rquickjs::Result<T> {
    tracing::error!(target = "quickjs", "DOM mutation failed: {err}");
    println!("dom_error: {err}");
    let message = format!("DOM mutation failed: {err}");
    let value = message.into_js(ctx)?;
    Err(ctx.throw(value))
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
    const HANDLE = Symbol('frontierHandle');
    const NODE_CACHE = new Map();

    function ensureFrontier() {
        if (typeof global.frontier !== 'object' || global.frontier === null) {
            global.frontier = {};
        }
        return global.frontier;
    }

    const frontier = ensureFrontier();
    const listenerStore = new Map();

    function normalizeEventType(type) {
        return String(type).trim().toLowerCase();
    }

    function getListenerBuckets(handle, type, create) {
        const key = String(handle);
        let typeMap = listenerStore.get(key);
        if (!typeMap) {
            if (!create) {
                return null;
            }
            typeMap = new Map();
            listenerStore.set(key, typeMap);
        }
        let buckets = typeMap.get(type);
        if (!buckets && create) {
            buckets = { capture: [], bubble: [] };
            typeMap.set(type, buckets);
        }
        return buckets;
    }

    function registerListener(handle, type) {
        global.__frontier_dom_listen(String(handle), type);
    }

    function unregisterListener(handle, type) {
        global.__frontier_dom_unlisten(String(handle), type);
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
        Object.defineProperty(proto, 'constructor', { value: ctor });
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
            if (typeof listener !== 'function') {
                return;
            }
            const normalizedType = normalizeEventType(type);
            const handle = toHandle(this);
            let capture = false;
            let once = false;
            if (options === true) {
                capture = true;
            } else if (options && typeof options === 'object') {
                capture = !!options.capture;
                once = !!options.once;
            }
            const buckets = getListenerBuckets(handle, normalizedType, true);
            const bucket = capture ? buckets.capture : buckets.bubble;
            if (bucket.some((entry) => entry.callback === listener)) {
                return;
            }
            bucket.push({ callback: listener, once });
            registerListener(handle, normalizedType);
        },
        removeEventListener(type, listener, options) {
            if (typeof listener !== 'function') {
                return;
            }
            const normalizedType = normalizeEventType(type);
            const handle = toHandle(this);
            const key = String(handle);
            const typeMap = listenerStore.get(key);
            if (!typeMap) {
                return;
            }
            let capture = false;
            if (options === true) {
                capture = true;
            } else if (options && typeof options === 'object') {
                capture = !!options.capture;
            }
            const buckets = typeMap.get(normalizedType);
            if (!buckets) {
                return;
            }
            const bucket = capture ? buckets.capture : buckets.bubble;
            const index = bucket.findIndex((entry) => entry.callback === listener);
            if (index === -1) {
                return;
            }
            bucket.splice(index, 1);
            if (buckets.capture.length === 0 && buckets.bubble.length === 0) {
                typeMap.delete(normalizedType);
                unregisterListener(handle, normalizedType);
                if (typeMap.size === 0) {
                    listenerStore.delete(key);
                }
            }
        },
        dispatchEvent(event) {
            if (!event || typeof event.type !== 'string') {
                return false;
            }
            const result = frontier.__dispatchDomEvent(
                toHandle(this),
                event.type,
                Object.assign({}, event),
                []
            );
            return !(result && result.defaultPrevented);
        },
    };

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

    function ensureDocument() {
        const docHandle = global.__frontier_dom_document_handle();
        let document = global.document;
        if (typeof document !== 'object' || document === null) {
            document = {};
        }
        Object.setPrototypeOf(document, DocumentProto);
        document[HANDLE] = String(docHandle);
        global.document = document;
        NODE_CACHE.set(String(docHandle), document);
    }

    function seedDocumentCache() {
        const documentHandle = global.document[HANDLE];
        const children = mapHandles(global.__frontier_dom_child_nodes(documentHandle));
        for (const handle of children) {
            wrapHandle(handle);
        }
    }

    ensureDocument();
    seedDocumentCache();

    frontier.wrapHandle = wrapHandle;
    frontier.collectDescendants = collectDescendants;

    const CAPTURING_PHASE = 1;
    const AT_TARGET = 2;
    const BUBBLING_PHASE = 3;

    function buildPropagationPath(targetNode, providedHandles) {
        if (Array.isArray(providedHandles) && providedHandles.length > 0) {
            const path = providedHandles.map((handle) => wrapHandle(handle));
            if (path[path.length - 1] !== global.document) {
                path.push(global.document);
            }
            return path;
        }
        const path = [];
        let current = targetNode;
        while (current) {
            path.push(current);
            current = current.parentNode;
        }
        if (path[path.length - 1] !== global.document) {
            path.push(global.document);
        }
        return path;
    }

    function createEvent(type, target, detail) {
        const event = {
            type: String(type),
            target,
            currentTarget: null,
            eventPhase: 0,
            bubbles: true,
            cancelable: true,
            defaultPrevented: false,
            isTrusted: true,
            timeStamp: Date.now(),
            _propagationStopped: false,
            _immediatePropagationStopped: false,
            _redrawRequested: false,
            preventDefault() {
                if (this.cancelable) {
                    this.defaultPrevented = true;
                }
            },
            stopPropagation() {
                this._propagationStopped = true;
            },
            stopImmediatePropagation() {
                this._propagationStopped = true;
                this._immediatePropagationStopped = true;
            },
            requestRedraw() {
                this._redrawRequested = true;
            },
        };
        if (detail && typeof detail === 'object') {
            for (const key of Object.keys(detail)) {
                event[key] = detail[key];
            }
            if (detail.bubbles != null) {
                event.bubbles = !!detail.bubbles;
            }
            if (detail.cancelable != null) {
                event.cancelable = !!detail.cancelable;
            }
        }
        event.altKey = !!event.altKey;
        event.ctrlKey = !!event.ctrlKey;
        event.metaKey = !!event.metaKey;
        event.shiftKey = !!event.shiftKey;
        return event;
    }

    function invokeListenersOnNode(handle, node, type, event, capturePhase) {
        const typeMap = listenerStore.get(String(handle));
        if (!typeMap) {
            return true;
        }
        const buckets = typeMap.get(type);
        if (!buckets) {
            return true;
        }
        const bucket = capturePhase ? buckets.capture : buckets.bubble;
        if (!bucket || bucket.length === 0) {
            return true;
        }
        const callbacks = bucket.slice();
        for (const entry of callbacks) {
            if (event._immediatePropagationStopped) {
                break;
            }
            try {
                entry.callback.call(node, event);
            } catch (err) {
                console.error(err);
            }
            if (entry.once) {
                const index = bucket.indexOf(entry);
                if (index !== -1) {
                    bucket.splice(index, 1);
                }
            }
        }
        if (buckets.capture.length === 0 && buckets.bubble.length === 0) {
            typeMap.delete(type);
            unregisterListener(handle, type);
            if (typeMap.size === 0) {
                listenerStore.delete(String(handle));
            }
        }
        return !event._propagationStopped;
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
        const normalizedType = normalizeEventType(type);
        const path = buildPropagationPath(target, pathHandles);
        const event = createEvent(normalizedType, target, detail || {});

        const capturePath = path.slice().reverse();
        for (let i = 0; i < capturePath.length; i++) {
            const node = capturePath[i];
            event.currentTarget = node;
            event.eventPhase = CAPTURING_PHASE;
            if (!invokeListenersOnNode(node[HANDLE], node, normalizedType, event, true)) {
                break;
            }
        }

        if (!event._propagationStopped) {
            for (let i = 0; i < path.length; i++) {
                const node = path[i];
                event.currentTarget = node;
                event.eventPhase = i === 0 ? AT_TARGET : BUBBLING_PHASE;
                if (!invokeListenersOnNode(node[HANDLE], node, normalizedType, event, false)) {
                    break;
                }
            }
        }

        return {
            defaultPrevented: event.defaultPrevented,
            redrawRequested: event._redrawRequested,
            propagationStopped: event._propagationStopped,
        };
    };

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
