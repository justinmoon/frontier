use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use blitz_dom::BaseDocument;
use rquickjs::{Ctx, Function, IntoJs};

use super::dom::{DomPatch, DomState};
use super::runtime::QuickJsEngine;
use super::timers::TimerRegistry;

pub struct JsDomEnvironment {
    engine: QuickJsEngine,
    state: Rc<RefCell<DomState>>,
    #[allow(dead_code)]
    timers: Rc<TimerRegistry>,
    #[allow(dead_code)]
    pending_timer_callbacks: Rc<RefCell<Vec<String>>>,
    microtask_queue: Rc<RefCell<Vec<String>>>,
}

impl JsDomEnvironment {
    pub fn new(html: &str) -> Result<Self> {
        let state = Rc::new(RefCell::new(DomState::new(html)));
        let timers = Rc::new(TimerRegistry::new());
        let pending_timer_callbacks = Rc::new(RefCell::new(Vec::new()));
        let microtask_queue = Rc::new(RefCell::new(Vec::new()));
        let engine = QuickJsEngine::new()?;
        install_dom_bindings(&engine, Rc::clone(&state))?;
        install_timer_bindings(
            &engine,
            Rc::clone(&timers),
            Rc::clone(&pending_timer_callbacks),
        )?;
        install_microtask_bindings(&engine, Rc::clone(&microtask_queue))?;
        install_event_bindings(&engine)?;
        Ok(Self {
            engine,
            state,
            timers,
            pending_timer_callbacks,
            microtask_queue,
        })
    }

    pub fn eval(&self, source: &str, filename: &str) -> Result<()> {
        self.engine.eval(source, filename)?;
        // Process any queued microtasks after script execution
        self.process_microtasks()?;
        Ok(())
    }

    fn process_microtasks(&self) -> Result<()> {
        // Process all microtasks in the queue
        while !self.microtask_queue.borrow().is_empty() {
            let tasks: Vec<String> = self.microtask_queue.borrow_mut().drain(..).collect();
            for task_code in tasks {
                if let Err(e) = self.engine.eval(&task_code, "microtask.js") {
                    tracing::warn!(target = "quickjs", "Microtask error: {}", e);
                }
            }
        }
        Ok(())
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

    #[allow(dead_code)]
    pub fn poll_timers(&self) -> Result<usize> {
        let mut executed = 0;
        while let Some(timer_msg) = self.timers.try_recv_timer() {
            match timer_msg {
                super::timers::TimerMessage::Fire { timer_id } => {
                    // Look up the callback for this timer
                    let callback_code = format!("__frontier_timer_fire({})", timer_id);
                    self.pending_timer_callbacks
                        .borrow_mut()
                        .push(callback_code);
                    executed += 1;
                }
            }
        }

        // Execute all pending callbacks
        for callback in self.pending_timer_callbacks.borrow_mut().drain(..) {
            if let Err(e) = self.engine.eval(&callback, "timer-callback.js") {
                tracing::warn!(target = "quickjs", "Timer callback error: {}", e);
            }
        }

        Ok(executed)
    }
}

fn install_dom_bindings(engine: &QuickJsEngine, state: Rc<RefCell<DomState>>) -> Result<()> {
    engine.with_context(|ctx| {
        let global = ctx.globals();

        {
            let state_ref = Rc::clone(&state);
            let get_handle = Function::new(
                ctx.clone(),
                move |id: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow_mut().handle_from_element_id(&id))
                },
            )?
            .with_name("__frontier_dom_get_handle_by_id")?;
            global.set("__frontier_dom_get_handle_by_id", get_handle)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_text = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().text_content(&handle))
                },
            )?
            .with_name("__frontier_dom_get_text")?;
            global.set("__frontier_dom_get_text", get_text)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_html = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().inner_html(&handle))
                },
            )?
            .with_name("__frontier_dom_get_html")?;
            global.set("__frontier_dom_get_html", get_html)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let apply_patch = Function::new(
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
            global.set("__frontier_dom_apply_patch", apply_patch)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_attr = Function::new(
                ctx.clone(),
                move |handle: String, name: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().get_attribute(&handle, &name))
                },
            )?
            .with_name("__frontier_dom_get_attribute")?;
            global.set("__frontier_dom_get_attribute", get_attr)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_children = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<Vec<String>>> {
                    Ok(state_ref.borrow().get_children(&handle))
                },
            )?
            .with_name("__frontier_dom_get_children")?;
            global.set("__frontier_dom_get_children", get_children)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_parent = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().get_parent(&handle))
                },
            )?
            .with_name("__frontier_dom_get_parent")?;
            global.set("__frontier_dom_get_parent", get_parent)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_tag = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().get_tag_name(&handle))
                },
            )?
            .with_name("__frontier_dom_get_tag_name")?;
            global.set("__frontier_dom_get_tag_name", get_tag)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let get_type = Function::new(
                ctx.clone(),
                move |handle: String| -> rquickjs::Result<Option<u8>> {
                    Ok(state_ref.borrow().get_node_type(&handle))
                },
            )?
            .with_name("__frontier_dom_get_node_type")?;
            global.set("__frontier_dom_get_node_type", get_type)?;
        }

        {
            let state_ref = Rc::clone(&state);
            let alloc_id = Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>| -> rquickjs::Result<String> {
                    match state_ref.borrow_mut().allocate_node_id() {
                        Ok(id) => Ok(id),
                        Err(err) => dom_error(&ctx, err),
                    }
                },
            )?
            .with_name("__frontier_dom_allocate_node_id")?;
            global.set("__frontier_dom_allocate_node_id", alloc_id)?;
        }

        ctx.eval::<(), _>(DOM_BOOTSTRAP.as_bytes())?;
        Ok(())
    })
}

fn install_timer_bindings(
    engine: &QuickJsEngine,
    timers: Rc<TimerRegistry>,
    _pending_callbacks: Rc<RefCell<Vec<String>>>,
) -> Result<()> {
    engine.with_context(|ctx| {
        let global = ctx.globals();

        {
            let timers_ref = Rc::clone(&timers);
            let set_timeout =
                Function::new(ctx.clone(), move |delay_ms: u32| -> rquickjs::Result<u32> {
                    Ok(timers_ref.set_timeout(delay_ms))
                })?
                .with_name("__frontier_set_timeout")?;
            global.set("__frontier_set_timeout", set_timeout)?;
        }

        {
            let timers_ref = Rc::clone(&timers);
            let set_interval =
                Function::new(ctx.clone(), move |delay_ms: u32| -> rquickjs::Result<u32> {
                    Ok(timers_ref.set_interval(delay_ms))
                })?
                .with_name("__frontier_set_interval")?;
            global.set("__frontier_set_interval", set_interval)?;
        }

        {
            let timers_ref = Rc::clone(&timers);
            let clear_timer =
                Function::new(ctx.clone(), move |timer_id: u32| -> rquickjs::Result<()> {
                    timers_ref.clear_timer(timer_id);
                    Ok(())
                })?
                .with_name("__frontier_clear_timer")?;
            global.set("__frontier_clear_timer", clear_timer)?;
        }

        ctx.eval::<(), _>(TIMER_BOOTSTRAP.as_bytes())?;
        Ok(())
    })
}

fn install_microtask_bindings(
    engine: &QuickJsEngine,
    microtask_queue: Rc<RefCell<Vec<String>>>,
) -> Result<()> {
    engine.with_context(|ctx| {
        let global = ctx.globals();

        {
            let queue_ref = Rc::clone(&microtask_queue);
            let queue_microtask = Function::new(
                ctx.clone(),
                move |callback_code: String| -> rquickjs::Result<()> {
                    queue_ref.borrow_mut().push(callback_code);
                    Ok(())
                },
            )?
            .with_name("__frontier_queue_microtask")?;
            global.set("__frontier_queue_microtask", queue_microtask)?;
        }

        ctx.eval::<(), _>(MICROTASK_BOOTSTRAP.as_bytes())?;
        Ok(())
    })
}

fn install_event_bindings(engine: &QuickJsEngine) -> Result<()> {
    engine.with_context(|ctx| {
        ctx.eval::<(), _>(EVENT_BOOTSTRAP.as_bytes())?;
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

const DOM_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;
    const HANDLE = Symbol('frontierHandle');

    function ensureFrontier() {
        if (typeof global.frontier !== 'object' || global.frontier === null) {
            global.frontier = {};
        }
        return global.frontier;
    }

    function ensureDocument() {
        if (typeof global.document !== 'object' || global.document === null) {
            global.document = {};
        }
    }

    ensureDocument();
    const frontier = ensureFrontier();

    // Set up window object that React expects
    if (typeof global.window !== 'object' || global.window === null) {
        global.window = global;
    }

    // Add navigator object for browser detection
    if (typeof global.navigator !== 'object' || global.navigator === null) {
        global.navigator = {
            userAgent: 'Frontier/1.0',
            platform: 'Frontier',
            language: 'en-US'
        };
    }

    function emitPatch(patch) {
        // Backward compatibility: convert 'id' field to 'handle'
        if (patch && typeof patch === 'object') {
            if (patch.id !== undefined && patch.handle === undefined) {
                const handle = global.__frontier_dom_get_handle_by_id(String(patch.id));
                if (handle) {
                    patch = { ...patch, handle };
                    delete patch.id;
                }
            }
            // For operations on children, convert id fields too
            if (patch.parent_id && !patch.parent_handle) {
                const handle = global.__frontier_dom_get_handle_by_id(String(patch.parent_id));
                if (handle) patch.parent_handle = handle;
            }
            if (patch.child_id && !patch.child_handle) {
                const handle = global.__frontier_dom_get_handle_by_id(String(patch.child_id));
                if (handle) patch.child_handle = handle;
            }
        }
        return global.__frontier_dom_apply_patch(JSON.stringify(patch));
    }

    function createNodeProxy(handle) {
        const target = { [HANDLE]: handle };
        return new Proxy(target, {
            get(target, prop) {
                if (prop === HANDLE) {
                    return handle;
                }
                // Check if this property was previously set by JavaScript code
                if (prop in target && prop !== HANDLE) {
                    return target[prop];
                }
                if (prop === 'textContent') {
                    const value = global.__frontier_dom_get_text(handle);
                    return value == null ? null : value;
                }
                if (prop === 'innerHTML') {
                    const value = global.__frontier_dom_get_html(handle);
                    return value == null ? null : value;
                }
                if (prop === 'tagName') {
                    const value = global.__frontier_dom_get_tag_name(handle);
                    return value == null ? null : value.toUpperCase();
                }
                if (prop === 'nodeType') {
                    return global.__frontier_dom_get_node_type(handle) || 1;
                }
                if (prop === 'parentNode') {
                    const parentHandle = global.__frontier_dom_get_parent(handle);
                    return parentHandle ? createNodeProxy(parentHandle) : null;
                }
                if (prop === 'childNodes' || prop === 'children') {
                    const handles = global.__frontier_dom_get_children(handle);
                    if (!handles) return [];
                    return handles.map(h => createNodeProxy(h));
                }
                if (prop === 'firstChild') {
                    const handles = global.__frontier_dom_get_children(handle);
                    if (!handles || handles.length === 0) return null;
                    return createNodeProxy(handles[0]);
                }
                if (prop === 'style') {
                    // Return a cached style object for this element
                    // Store it on the target so it's consistent across accesses
                    if (!target._style) {
                        target._style = {};
                    }
                    return target._style;
                }
                if (prop === 'ownerDocument') {
                    // Elements belong to the global document
                    return global.document;
                }
                if (prop === 'getAttribute') {
                    return (name) => {
                        return global.__frontier_dom_get_attribute(handle, String(name)) || null;
                    };
                }
                if (prop === 'setAttribute') {
                    return (name, value) => {
                        emitPatch({
                            type: 'attribute',
                            handle,
                            name: String(name),
                            value: value == null ? '' : String(value),
                        });
                    };
                }
                if (prop === 'removeAttribute') {
                    return (name) => {
                        emitPatch({
                            type: 'remove_attribute',
                            handle,
                            name: String(name),
                        });
                    };
                }
                if (prop === 'appendChild') {
                    return (child) => {
                        if (!child || typeof child !== 'object') {
                            throw new TypeError('appendChild expects a node');
                        }
                        const childHandle = child[HANDLE];
                        if (typeof childHandle !== 'string') {
                            throw new TypeError('child must be a Frontier DOM node');
                        }
                        emitPatch({
                            type: 'append_child',
                            parent_handle: handle,
                            child_handle: childHandle,
                        });
                        return child;
                    };
                }
                if (prop === 'removeChild') {
                    return (child) => {
                        if (!child || typeof child !== 'object') {
                            throw new TypeError('removeChild expects a node');
                        }
                        const childHandle = child[HANDLE];
                        if (typeof childHandle !== 'string') {
                            throw new TypeError('child must be a Frontier DOM node');
                        }
                        emitPatch({
                            type: 'remove_child',
                            parent_handle: handle,
                            child_handle: childHandle,
                        });
                        return child;
                    };
                }
                if (prop === 'insertBefore') {
                    return (newNode, refNode) => {
                        if (!newNode || typeof newNode !== 'object') {
                            throw new TypeError('insertBefore expects a node');
                        }
                        const newHandle = newNode[HANDLE];
                        if (typeof newHandle !== 'string') {
                            throw new TypeError('newNode must be a Frontier DOM node');
                        }
                        let refHandle = null;
                        if (refNode !== null && refNode !== undefined) {
                            refHandle = refNode[HANDLE];
                            if (typeof refHandle !== 'string') {
                                throw new TypeError('refNode must be a Frontier DOM node');
                            }
                        }
                        emitPatch({
                            type: 'insert_before',
                            parent_handle: handle,
                            new_handle: newHandle,
                            reference_handle: refHandle,
                        });
                        return newNode;
                    };
                }
                if (prop === 'replaceChild') {
                    return (newChild, oldChild) => {
                        if (!newChild || typeof newChild !== 'object') {
                            throw new TypeError('replaceChild expects nodes');
                        }
                        if (!oldChild || typeof oldChild !== 'object') {
                            throw new TypeError('replaceChild expects nodes');
                        }
                        const newHandle = newChild[HANDLE];
                        const oldHandle = oldChild[HANDLE];
                        if (typeof newHandle !== 'string' || typeof oldHandle !== 'string') {
                            throw new TypeError('children must be Frontier DOM nodes');
                        }
                        emitPatch({
                            type: 'replace_child',
                            parent_handle: handle,
                            new_handle: newHandle,
                            old_handle: oldHandle,
                        });
                        return oldChild;
                    };
                }
                if (prop === 'addEventListener') {
                    return (type, listener, options) => {
                        if (typeof listener !== 'function') {
                            throw new TypeError('Event listener must be a function');
                        }
                        global.__frontier_add_event_listener(handle, type, listener, options);
                    };
                }
                if (prop === 'removeEventListener') {
                    return (type, listener, options) => {
                        global.__frontier_remove_event_listener(handle, type, listener, options);
                    };
                }
                if (prop === 'dispatchEvent') {
                    return (event) => {
                        return global.__frontier_dispatch_event(handle, event);
                    };
                }
                if (prop === 'toString') {
                    return () => `[Node ${handle}]`;
                }
                return undefined;
            },
            set(target, prop, value) {
                if (prop === 'textContent') {
                    emitPatch({
                        type: 'text_content',
                        handle,
                        value: value == null ? '' : String(value),
                    });
                    return true;
                }
                if (prop === 'innerHTML') {
                    emitPatch({
                        type: 'inner_html',
                        handle,
                        value: value == null ? '' : String(value),
                    });
                    return true;
                }
                // Allow setting arbitrary properties on the element
                // (React and other libraries store internal data this way)
                target[prop] = value;
                return true;
            },
            has(target, prop) {
                // Support the 'in' operator for property checking
                // Check target first (for properties set via JavaScript)
                if (prop in target) {
                    return true;
                }
                // Check well-known DOM properties
                const domProps = ['textContent', 'innerHTML', 'tagName', 'nodeType', 'parentNode',
                                   'childNodes', 'children', 'firstChild', 'style', 'ownerDocument',
                                   'getAttribute', 'setAttribute', 'removeAttribute', 'appendChild',
                                   'insertBefore', 'removeChild', 'replaceChild', 'addEventListener',
                                   'removeEventListener', 'dispatchEvent'];
                return domProps.includes(prop);
            },
        });
    }

    global.document.getElementById = function getElementById(rawId) {
        if (typeof rawId !== 'string') {
            return null;
        }
        const handle = global.__frontier_dom_get_handle_by_id(String(rawId));
        if (typeof handle !== 'string') {
            return null;
        }
        return createNodeProxy(handle);
    };

    global.document.createElement = function createElement(tagName) {
        const resultHandle = global.__frontier_dom_allocate_node_id();
        emitPatch({
            type: 'create_element',
            result_handle: resultHandle,
            tag_name: String(tagName),
        });
        return createNodeProxy(resultHandle);
    };

    global.document.createTextNode = function createTextNode(data) {
        const resultHandle = global.__frontier_dom_allocate_node_id();
        emitPatch({
            type: 'create_text_node',
            result_handle: resultHandle,
            data: String(data),
        });
        return createNodeProxy(resultHandle);
    };

    // Add event listener methods to document
    global.document.addEventListener = function addEventListener(type, listener, options) {
        if (typeof listener !== 'function') {
            throw new TypeError('Event listener must be a function');
        }
        global.__frontier_add_event_listener('document', type, listener, options);
    };

    global.document.removeEventListener = function removeEventListener(type, listener, options) {
        global.__frontier_remove_event_listener('document', type, listener, options);
    };

    global.document.dispatchEvent = function dispatchEvent(event) {
        return global.__frontier_dispatch_event('document', event);
    };

    frontier.emitDomPatch = emitPatch;
})();
"#;

const TIMER_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;
    const timerCallbacks = new Map();
    let nextCallbackId = 1;

    global.__frontier_timer_fire = function(timerId) {
        const callback = timerCallbacks.get(timerId);
        if (callback) {
            try {
                callback();
            } catch (err) {
                console.log('Timer callback error: ' + err);
            }
            // For setTimeout, remove the callback after execution
            if (callback.__isTimeout) {
                timerCallbacks.delete(timerId);
            }
        }
    };

    global.setTimeout = function(callback, delay) {
        if (typeof callback !== 'function') {
            throw new TypeError('setTimeout callback must be a function');
        }
        const delayMs = Math.max(0, delay || 0);
        const timerId = global.__frontier_set_timeout(delayMs);
        callback.__isTimeout = true;
        timerCallbacks.set(timerId, callback);
        return timerId;
    };

    global.setInterval = function(callback, delay) {
        if (typeof callback !== 'function') {
            throw new TypeError('setInterval callback must be a function');
        }
        const delayMs = Math.max(0, delay || 0);
        const timerId = global.__frontier_set_interval(delayMs);
        timerCallbacks.set(timerId, callback);
        return timerId;
    };

    global.clearTimeout = function(timerId) {
        if (timerId !== undefined && timerId !== null) {
            global.__frontier_clear_timer(timerId);
            timerCallbacks.delete(timerId);
        }
    };

    global.clearInterval = function(timerId) {
        if (timerId !== undefined && timerId !== null) {
            global.__frontier_clear_timer(timerId);
            timerCallbacks.delete(timerId);
        }
    };
})();
"#;

const MICROTASK_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;
    const microtaskCallbacks = new Map();
    let nextMicrotaskId = 1;

    global.__frontier_execute_microtask = function(microtaskId) {
        const callback = microtaskCallbacks.get(microtaskId);
        if (callback) {
            microtaskCallbacks.delete(microtaskId);
            try {
                callback();
            } catch (err) {
                console.log('Microtask error: ' + err);
            }
        }
    };

    global.queueMicrotask = function(callback) {
        if (typeof callback !== 'function') {
            throw new TypeError('queueMicrotask callback must be a function');
        }
        const microtaskId = nextMicrotaskId++;
        microtaskCallbacks.set(microtaskId, callback);
        // Queue the execution code
        global.__frontier_queue_microtask(`__frontier_execute_microtask(${microtaskId})`);
    };
})();
"#;

const EVENT_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;

    // Store event listeners per element: Map<handle, Map<eventType, Set<listener>>>
    const eventListeners = new Map();

    global.__frontier_add_event_listener = function(handle, type, listener, options) {
        if (!eventListeners.has(handle)) {
            eventListeners.set(handle, new Map());
        }
        const handleListeners = eventListeners.get(handle);

        if (!handleListeners.has(type)) {
            handleListeners.set(type, new Set());
        }
        handleListeners.get(type).add(listener);
    };

    global.__frontier_remove_event_listener = function(handle, type, listener, options) {
        const handleListeners = eventListeners.get(handle);
        if (!handleListeners) return;

        const typeListeners = handleListeners.get(type);
        if (!typeListeners) return;

        typeListeners.delete(listener);

        if (typeListeners.size === 0) {
            handleListeners.delete(type);
        }
        if (handleListeners.size === 0) {
            eventListeners.delete(handle);
        }
    };

    global.__frontier_dispatch_event = function(handle, event) {
        const handleListeners = eventListeners.get(handle);
        if (!handleListeners) return true;

        const typeListeners = handleListeners.get(event.type);
        if (!typeListeners) return true;

        let defaultPrevented = false;
        for (const listener of typeListeners) {
            try {
                listener(event);
                if (event.defaultPrevented) {
                    defaultPrevented = true;
                }
            } catch (err) {
                console.log('Event listener error: ' + err);
            }
        }

        return !defaultPrevented;
    };

    // Simple Event constructor
    global.Event = function(type, eventInitDict) {
        this.type = type;
        this.bubbles = eventInitDict && eventInitDict.bubbles || false;
        this.cancelable = eventInitDict && eventInitDict.cancelable || false;
        this.defaultPrevented = false;
        this.propagationStopped = false;
    };

    global.Event.prototype.preventDefault = function() {
        if (this.cancelable) {
            this.defaultPrevented = true;
        }
    };

    global.Event.prototype.stopPropagation = function() {
        this.propagationStopped = true;
    };
})();
"#;
