use anyhow::{Context as AnyhowContext, Result};
use rquickjs::{Context, Ctx, Function, Runtime};

/// JavaScript runtime backed by QuickJS.
///
/// The engine owns the QuickJS runtime and context and provides helpers for evaluating
/// scripts. It also installs a minimal `console` implementation that forwards logs to
/// Rust tracing.
pub struct QuickJsEngine {
    _runtime: Runtime,
    context: Context,
}

impl QuickJsEngine {
    /// Create a new QuickJS engine with `console.log` wired up to `tracing`.
    pub fn new() -> Result<Self> {
        let runtime = Runtime::new().context("failed to create QuickJS runtime")?;
        let context = Context::full(&runtime).context("failed to create QuickJS context")?;
        let engine = Self {
            _runtime: runtime,
            context,
        };
        engine.init_console()?;
        Ok(engine)
    }

    /// Evaluate a script and discard the result.
    pub fn eval(&self, source: &str, filename: &str) -> Result<()> {
        self.eval_with::<()>(source, filename)
    }

    /// Evaluate a script and deserialize the result into `V`.
    pub fn eval_with<V>(&self, source: &str, filename: &str) -> Result<V>
    where
        V: for<'js> rquickjs::FromJs<'js>,
    {
        let script = Self::with_source_url(source, filename);
        self.context
            .with(move |ctx| ctx.eval::<V, _>(script))
            .map_err(anyhow::Error::from)
    }

    /// Provide access to the underlying QuickJS context for advanced integrations.
    pub fn with_context<T, F>(&self, f: F) -> Result<T>
    where
        F: for<'js> FnOnce(Ctx<'js>) -> rquickjs::Result<T>,
    {
        self.context.with(f).map_err(anyhow::Error::from)
    }

    fn init_console(&self) -> Result<()> {
        self.context
            .with(|ctx| {
                let global = ctx.globals();
                let log_fn =
                    Function::new(ctx.clone(), log_from_js)?.with_name("__frontier_log")?;
                global.set("__frontier_log", log_fn)?;
                ctx.eval::<(), _>(CONSOLE_BOOTSTRAP.as_bytes())
            })
            .map_err(anyhow::Error::from)
    }

    fn with_source_url(source: &str, filename: &str) -> Vec<u8> {
        let mut script = String::with_capacity(source.len() + filename.len() + 32);
        script.push_str(source);
        if !source.ends_with('\n') {
            script.push('\n');
        }
        script.push_str("//# sourceURL=");
        script.push_str(filename);
        script.push('\n');
        script.into_bytes()
    }
}

fn log_from_js(message: String) -> rquickjs::Result<()> {
    tracing::info!(target = "quickjs", message = %message);
    Ok(())
}

const CONSOLE_BOOTSTRAP: &str = r#"
(() => {
    const global = globalThis;
    const stringify = (value) => {
        try {
            if (typeof value === 'string') {
                return value;
            }
            if (value === undefined) {
                return 'undefined';
            }
            if (value === null) {
                return 'null';
            }
            return String(value);
        } catch (err) {
            return '[unprintable]';
        }
    };

    const logImpl = (...args) => {
        try {
            const joined = args.map(stringify).join(' ');
            global.__frontier_log(joined);
        } catch (err) {
            // Swallow logging errors; console must never throw.
        }
    };

    if (typeof global.console !== 'object' || global.console === null) {
        global.console = {};
    }

    global.console.log = logImpl;
})();
"#;
