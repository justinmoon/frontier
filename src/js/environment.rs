use std::cell::RefCell;
use std::rc::Rc;

use anyhow::{anyhow, Context as AnyhowContext, Result};
use blitz_dom::BaseDocument;
use rquickjs::{Ctx, Function, IntoJs};

use super::dom::{DomPatch, DomSnapshot, DomState};
use super::runtime::QuickJsEngine;

pub struct JsDomEnvironment {
    engine: QuickJsEngine,
    state: Rc<RefCell<DomState>>,
}

impl JsDomEnvironment {
    pub fn new(html: &str) -> Result<Self> {
        let snapshot =
            DomSnapshot::parse(html).context("failed to parse HTML for QuickJS snapshot")?;
        let state = Rc::new(RefCell::new(DomState::new(snapshot)));
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

        // Element existence check
        {
            let state_ref = Rc::clone(&state);
            let exists_fn =
                Function::new(ctx.clone(), move |id: String| -> rquickjs::Result<bool> {
                    Ok(state_ref.borrow().has_element(&id))
                })?
                .with_name("__frontier_dom_element_exists")?;
            global.set("__frontier_dom_element_exists", exists_fn)?;
        }

        // text getter
        {
            let state_ref = Rc::clone(&state);
            let get_text = Function::new(
                ctx.clone(),
                move |id: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().text_content(&id))
                },
            )?
            .with_name("__frontier_dom_get_text")?;
            global.set("__frontier_dom_get_text", get_text)?;
        }

        // inner HTML getter
        {
            let state_ref = Rc::clone(&state);
            let get_html = Function::new(
                ctx.clone(),
                move |id: String| -> rquickjs::Result<Option<String>> {
                    Ok(state_ref.borrow().inner_html(&id))
                },
            )?
            .with_name("__frontier_dom_get_html")?;
            global.set("__frontier_dom_get_html", get_html)?;
        }

        // apply patch helper
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
    let message = format!("DOM mutation failed: {err}");
    let value = message.into_js(ctx)?;
    Err(ctx.throw(value))
}

const DOM_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;

    function coercePatch(patch) {
        if (!patch || typeof patch !== 'object') {
            throw new TypeError('frontier.emitDomPatch expects an object');
        }
        if (typeof patch.type !== 'string') {
            throw new TypeError('Patch requires a string "type" field');
        }
        if (typeof patch.id !== 'string') {
            throw new TypeError('Patch requires a string "id" field');
        }
        const result = { type: patch.type, id: patch.id };
        if (patch.value !== undefined) {
            result.value = String(patch.value);
        }
        if (patch.name !== undefined) {
            result.name = String(patch.name);
        }
        return result;
    }

    function ensureFrontier() {
        if (typeof global.frontier !== 'object' || global.frontier === null) {
            global.frontier = {};
        }
        return global.frontier;
    }

    function ensureDocument() {
        if (typeof global.document === 'object' && global.document !== null) {
            return;
        }
        global.document = {};
    }

    ensureDocument();

    global.document.getElementById = function getElementById(rawId) {
        if (typeof rawId !== 'string') {
            return null;
        }
        const id = rawId;
        if (!global.__frontier_dom_element_exists(id)) {
            return null;
        }
        const element = { id };
        return new Proxy(element, {
            get(target, prop) {
                if (prop === 'textContent') {
                    return global.__frontier_dom_get_text(target.id) ?? null;
                }
                if (prop === 'innerHTML') {
                    return global.__frontier_dom_get_html(target.id) ?? null;
                }
                if (prop === 'setAttribute') {
                    return (name, value) => {
                        frontier.emitDomPatch({
                            type: 'attribute',
                            id: target.id,
                            name: String(name),
                            value: value == null ? '' : String(value),
                        });
                    };
                }
                if (prop === 'id') {
                    return target.id;
                }
                if (prop === 'toString') {
                    return () => `[Element ${target.id}]`;
                }
                return undefined;
            },
            set(target, prop, value) {
                if (prop === 'textContent') {
                    frontier.emitDomPatch({
                        type: 'text_content',
                        id: target.id,
                        value: value == null ? '' : String(value),
                    });
                    return true;
                }
                if (prop === 'innerHTML') {
                    frontier.emitDomPatch({
                        type: 'inner_html',
                        id: target.id,
                        value: value == null ? '' : String(value),
                    });
                    return true;
                }
                return false;
            },
        });
    };

    const frontier = ensureFrontier();
    frontier.emitDomPatch = function emitDomPatch(patch) {
        const formatted = coercePatch(patch);
        return global.__frontier_dom_apply_patch(JSON.stringify(formatted));
    };
})();
"#;
