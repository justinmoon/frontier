use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, Result};
use blitz_dom::BaseDocument;
use rquickjs::{Ctx, Function, IntoJs};

use super::dom::{DomPatch, DomState};
use super::runtime::QuickJsEngine;

pub struct JsDomEnvironment {
    engine: QuickJsEngine,
    state: Rc<RefCell<DomState>>,
}

impl JsDomEnvironment {
    pub fn new(html: &str) -> Result<Self> {
        let state = Rc::new(RefCell::new(DomState::new(html)));
        let engine = QuickJsEngine::new()?;
        install_dom_bindings(&engine, Rc::clone(&state))?;
        Ok(Self { engine, state })
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
            get(_, prop) {
                if (prop === HANDLE) {
                    return handle;
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
                if (prop === 'toString') {
                    return () => `[Node ${handle}]`;
                }
                return undefined;
            },
            set(_, prop, value) {
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
                return false;
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

    frontier.emitDomPatch = emitPatch;
})();
"#;
