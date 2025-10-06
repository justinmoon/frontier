use anyhow::{Context as AnyhowContext, Result};
use rquickjs::{Context, Ctx, Error as JsError, Function, Runtime, Value};

/// JavaScript runtime backed by QuickJS.
///
/// The engine owns the QuickJS runtime and context and provides helpers for evaluating
/// scripts. It also installs a minimal `console` implementation that forwards logs to
/// Rust tracing.
pub struct QuickJsEngine {
    runtime: Runtime,
    context: Context,
}

impl QuickJsEngine {
    /// Create a new QuickJS engine with `console.log` wired up to `tracing`.
    pub fn new() -> Result<Self> {
        let runtime = Runtime::new().context("failed to create QuickJS runtime")?;
        let context = Context::full(&runtime).context("failed to create QuickJS context")?;
        let engine = Self { runtime, context };
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
        let eval_result = self.context.with(|ctx| ctx.eval::<V, _>(script.clone()));

        let value = match eval_result {
            Ok(value) => Ok(value),
            Err(JsError::Exception) => {
                let message = self
                    .context
                    .with(|ctx| -> Result<Option<String>, JsError> {
                        Ok(capture_exception_message(&ctx))
                    })
                    .unwrap_or(None)
                    .unwrap_or_else(|| "QuickJS exception".to_string());
                Err(anyhow::anyhow!(message))
            }
            Err(err) => Err(anyhow::Error::from(err)),
        }?;

        // CRITICAL: Process all pending promise/microtask jobs
        // React 18's concurrent rendering uses promises internally
        // Without this, React's render() never completes
        self.execute_pending_jobs()?;

        Ok(value)
    }

    /// Execute all pending jobs in the QuickJS job queue.
    ///
    /// This processes promise continuations, microtasks, and other async work.
    /// Must be called after eval() to ensure promises and React's concurrent
    /// rendering complete properly.
    fn execute_pending_jobs(&self) -> Result<()> {
        let mut job_count = 0;
        const MAX_JOBS: usize = 1000; // Prevent infinite loops

        while self.runtime.is_job_pending() {
            match self.runtime.execute_pending_job() {
                Ok(true) => {
                    job_count += 1;
                    if job_count >= MAX_JOBS {
                        tracing::warn!(
                            target: "quickjs",
                            "Stopped processing jobs after {} iterations (possible infinite loop)",
                            MAX_JOBS
                        );
                        break;
                    }
                }
                Ok(false) => break, // Queue empty
                Err(job_exception) => {
                    // Log the exception but continue - don't fail the whole execution
                    tracing::error!(
                        target: "quickjs",
                        "Job execution error: {:?}",
                        job_exception
                    );
                    break;
                }
            }
        }

        if job_count > 0 {
            tracing::info!(target: "quickjs", "Executed {} pending jobs", job_count);
        } else {
            tracing::debug!(target: "quickjs", "No pending jobs to execute");
        }

        Ok(())
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

                // Add browser-like global polyfills
                // React UMD bundles expect 'self' to be defined
                ctx.eval::<(), _>(
                    "if (typeof self === 'undefined') { var self = globalThis; }".as_bytes(),
                )?;

                // Add DOM constructor stubs that React expects
                // React checks things like `x instanceof HTMLIFrameElement`
                // These need to be proper constructor functions
                ctx.eval::<(), _>(DOM_CONSTRUCTOR_POLYFILLS.as_bytes())?;

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

fn capture_exception_message(ctx: &Ctx<'_>) -> Option<String> {
    let exception: Value = ctx.catch();

    // Try to get detailed error information
    if let Some(obj) = exception.as_object() {
        if let Ok(message) = obj.get::<_, String>("message") {
            // Try to get stack trace if available
            if let Ok(stack) = obj.get::<_, String>("stack") {
                return Some(format!("Error: {}\nStack: {}", message, stack));
            }
            return Some(format!("Error: {}", message));
        }
    }

    Some(format!("{:?}", exception))
}

const DOM_CONSTRUCTOR_POLYFILLS: &str = r#"
(() => {
    // React expects these DOM constructors to exist as callable functions
    // for instanceof checks. We provide minimal stubs.
    const global = globalThis;

    // Base DOM constructors
    if (typeof global.Node === 'undefined') {
        global.Node = function Node() {};
    }
    if (typeof global.Element === 'undefined') {
        global.Element = function Element() {};
    }
    if (typeof global.HTMLElement === 'undefined') {
        global.HTMLElement = function HTMLElement() {};
    }
    if (typeof global.Document === 'undefined') {
        global.Document = function Document() {};
    }
    if (typeof global.Text === 'undefined') {
        global.Text = function Text() {};
    }
    if (typeof global.Comment === 'undefined') {
        global.Comment = function Comment() {};
    }

    // Specific HTML element constructors React might check
    if (typeof global.HTMLIFrameElement === 'undefined') {
        global.HTMLIFrameElement = function HTMLIFrameElement() {};
    }
    if (typeof global.HTMLInputElement === 'undefined') {
        global.HTMLInputElement = function HTMLInputElement() {};
    }
    if (typeof global.HTMLTextAreaElement === 'undefined') {
        global.HTMLTextAreaElement = function HTMLTextAreaElement() {};
    }
    if (typeof global.HTMLSelectElement === 'undefined') {
        global.HTMLSelectElement = function HTMLSelectElement() {};
    }
    if (typeof global.HTMLButtonElement === 'undefined') {
        global.HTMLButtonElement = function HTMLButtonElement() {};
    }
    if (typeof global.HTMLFormElement === 'undefined') {
        global.HTMLFormElement = function HTMLFormElement() {};
    }
    if (typeof global.HTMLAnchorElement === 'undefined') {
        global.HTMLAnchorElement = function HTMLAnchorElement() {};
    }
    if (typeof global.HTMLImageElement === 'undefined') {
        global.HTMLImageElement = function HTMLImageElement() {};
    }

    // Event constructors that React/JS might use
    // Note: The full Event implementation is in the DOM bridge (environment.rs)
    // These are just stubs for instanceof checks
    if (typeof global.Event === 'undefined') {
        global.Event = function Event(type, options) {
            this.type = type;
            this.bubbles = options && options.bubbles || false;
            this.cancelable = options && options.cancelable || false;
            this.defaultPrevented = false;
            this.propagationStopped = false;
        };
        global.Event.prototype.preventDefault = function() {
            this.defaultPrevented = true;
        };
        global.Event.prototype.stopPropagation = function() {
            this.propagationStopped = true;
        };
    }
    if (typeof global.MouseEvent === 'undefined') {
        global.MouseEvent = function MouseEvent(type, options) {
            this.type = type;
            this.bubbles = options && options.bubbles || false;
            this.cancelable = options && options.cancelable || false;
            this.defaultPrevented = false;
            this.propagationStopped = false;
        };
        global.MouseEvent.prototype = global.Event.prototype;
    }
})();
"#;

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

    // Make console methods REAL FUNCTIONS so apply/call work
    // React dev build calls console.error.apply()
    global.console.log = logImpl;
    global.console.error = logImpl;
    global.console.warn = logImpl;
    global.console.info = logImpl;
    global.console.debug = logImpl;
})();
"#;
