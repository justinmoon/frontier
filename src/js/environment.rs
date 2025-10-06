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

    function coercePatch(patch) {
        if (!patch || typeof patch !== 'object') {
            throw new TypeError('frontier.emitDomPatch expects an object');
        }
        if (typeof patch.type !== 'string') {
            throw new TypeError('Patch requires a string "type" field');
        }
        let rawHandle = patch.handle;
        if (rawHandle === undefined && typeof patch.id === 'string') {
            rawHandle = global.__frontier_dom_get_handle_by_id(patch.id);
        }
        if (rawHandle === undefined) {
            throw new TypeError('Patch requires a "handle" field');
        }
        const formatted = { type: patch.type, handle: String(rawHandle) };
        if (patch.value !== undefined) {
            formatted.value = String(patch.value);
        }
        if (patch.name !== undefined) {
            formatted.name = String(patch.name);
        }
        return formatted;
    }

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

    function createElementProxy(handle) {
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
                if (prop === 'setAttribute') {
                    return (name, value) => {
                        frontier.emitDomPatch({
                            type: 'attribute',
                            handle,
                            name: String(name),
                            value: value == null ? '' : String(value),
                        });
                    };
                }
                if (prop === 'toString') {
                    return () => `[Element ${handle}]`;
                }
                return undefined;
            },
            set(_, prop, value) {
                if (prop === 'textContent') {
                    frontier.emitDomPatch({
                        type: 'text_content',
                        handle,
                        value: value == null ? '' : String(value),
                    });
                    return true;
                }
                if (prop === 'innerHTML') {
                    frontier.emitDomPatch({
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
        return createElementProxy(handle);
    };

    frontier.emitDomPatch = function emitDomPatch(patch) {
        const formatted = coercePatch(patch);
        return global.__frontier_dom_apply_patch(JSON.stringify(formatted));
    };
})();
"#;
