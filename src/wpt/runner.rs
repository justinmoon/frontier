use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use serde::Deserialize;
use tokio::time::sleep;

use crate::js::environment::JsDomEnvironment;

const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const BASE_HTML: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Frontier WPT</title></head><body></body></html>";
const BRIDGE_SCRIPT: &str = include_str!("bridge.js");
const WINDOW_POLYFILL: &str = include_str!("window_polyfill.js");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WptStatus {
    Pass,
    Fail,
    Timeout,
    NotRun,
    PreconditionFailed,
    Unknown(String),
}

impl WptStatus {
    fn from_raw(value: &str) -> Self {
        match value {
            "PASS" => WptStatus::Pass,
            "FAIL" => WptStatus::Fail,
            "TIMEOUT" => WptStatus::Timeout,
            "NOTRUN" => WptStatus::NotRun,
            "PRECONDITION_FAILED" => WptStatus::PreconditionFailed,
            other => WptStatus::Unknown(other.to_string()),
        }
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, WptStatus::Pass)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessOutcome {
    Ok,
    Error,
    Timeout,
    PreconditionFailed,
    Unknown(String),
}

impl HarnessOutcome {
    fn from_raw(value: &str) -> Self {
        match value {
            "OK" => HarnessOutcome::Ok,
            "ERROR" => HarnessOutcome::Error,
            "TIMEOUT" => HarnessOutcome::Timeout,
            "PRECONDITION_FAILED" => HarnessOutcome::PreconditionFailed,
            other => HarnessOutcome::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WptTestResult {
    pub name: String,
    pub status: WptStatus,
    pub message: Option<String>,
    pub stack: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WptHarnessStatus {
    pub status: HarnessOutcome,
    pub message: Option<String>,
    pub stack: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WptAssertTestRef {
    pub name: String,
    pub status: WptStatus,
}

#[derive(Debug, Clone)]
pub struct WptAssertRecord {
    pub name: Option<String>,
    pub status: Option<String>,
    pub stack: Option<String>,
    pub test: Option<WptAssertTestRef>,
}

#[derive(Debug, Clone)]
pub struct WptRun {
    pub tests: Vec<WptTestResult>,
    pub harness_status: Option<WptHarnessStatus>,
    pub asserts: Vec<WptAssertRecord>,
}

impl WptRun {
    pub fn success(&self) -> bool {
        let harness_ok = self
            .harness_status
            .as_ref()
            .map(|status| matches!(status.status, HarnessOutcome::Ok))
            .unwrap_or(true);
        harness_ok && self.tests.iter().all(|test| test.status.is_pass())
    }
}

#[derive(Debug)]
pub struct WptManifest {
    entries: Vec<PathBuf>,
}

impl WptManifest {
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let contents = fs::read_to_string(path.as_ref())
            .with_context(|| format!("reading manifest {}", path.as_ref().display()))?;
        let mut entries = Vec::new();
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            entries.push(PathBuf::from(trimmed));
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[PathBuf] {
        &self.entries
    }
}

#[derive(Debug)]
pub struct WptManifestResult {
    pub entry: PathBuf,
    pub outcome: WptRun,
}

pub struct WptRunner {
    root: PathBuf,
    harness_wrapped_js: String,
    harness_report_js: String,
    timeout: Duration,
}

impl WptRunner {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let harness_path = root.join("resources/testharness.js");
        let harness_report_path = root.join("resources/testharnessreport.js");

        let harness_js = fs::read_to_string(&harness_path)
            .with_context(|| format!("reading {}", harness_path.display()))?;
        let harness_report_js = fs::read_to_string(&harness_report_path)
            .with_context(|| format!("reading {}", harness_report_path.display()))?;
        let harness_wrapped_js = wrap_harness_for_shell(&harness_js);

        Ok(Self {
            root,
            harness_wrapped_js,
            harness_report_js,
            timeout: DEFAULT_TEST_TIMEOUT,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn run_inline(&self, source: &str, script_name: &str) -> Result<WptRun> {
        self.run_inner(source, script_name).await
    }

    pub async fn run_test(&self, relative_path: &Path) -> Result<WptRun> {
        let path = self.root.join(relative_path);
        let source = fs::read_to_string(&path)
            .with_context(|| format!("reading WPT test {}", path.display()))?;
        let display_name = relative_path.to_string_lossy().into_owned();
        self.run_inner(&source, &display_name).await
    }

    pub async fn run_manifest(&self, manifest: &WptManifest) -> Result<Vec<WptManifestResult>> {
        let mut results = Vec::with_capacity(manifest.entries().len());
        for entry in manifest.entries() {
            let run = self
                .run_test(entry)
                .await
                .with_context(|| format!("running manifest entry {}", entry.display()))?;
            results.push(WptManifestResult {
                entry: entry.clone(),
                outcome: run,
            });
        }
        Ok(results)
    }

    async fn run_inner(&self, source: &str, script_name: &str) -> Result<WptRun> {
        let environment =
            JsDomEnvironment::new(BASE_HTML).context("initialising QuickJS environment for WPT")?;
        let mut document = HtmlDocument::from_html(BASE_HTML, DocumentConfig::default());
        environment.attach_document(&mut document);

        environment
            .eval(WINDOW_POLYFILL, "frontier-wpt-window-polyfill.js")
            .context("installing window polyfills for WPT")?;

        environment
            .eval(&self.harness_wrapped_js, "testharness.js")
            .context("evaluating testharness.js")?;
        environment
            .eval(&self.harness_report_js, "testharnessreport.js")
            .context("evaluating testharnessreport.js")?;
        environment
            .eval(BRIDGE_SCRIPT, "frontier-wpt-bridge.js")
            .context("installing WPT bridge")?;
        environment
            .eval(source, script_name)
            .with_context(|| format!("executing WPT test {script_name}"))?;

        let callbacks_available: bool = environment
            .eval_with(
                "typeof add_completion_callback === 'function'",
                "frontier-wpt-check-callback.js",
            )
            .unwrap_or(false);
        if !callbacks_available {
            return Err(anyhow!(
                "WPT harness missing completion callback hooks for {script_name}"
            ));
        }

        // Signal to the harness that load has completed so tests may finish.
        environment
            .eval(
                "if (typeof window !== 'undefined' && typeof window.__frontierDispatchLoad === 'function') {\n  window.__frontierDispatchLoad();\n}",
                "frontier-wpt-load.js",
            )
            .ok();

        self.wait_for_completion(&environment, script_name).await?;

        let report_json: String = environment
            .eval_with(
                "(() => {\n  if (typeof globalThis.__frontierWptSerialize !== 'function') {\n    throw new Error('Frontier WPT bridge missing serializer');\n  }\n  return globalThis.__frontierWptSerialize();\n})()",
                "frontier-wpt-collect.js",
            )
            .context("collecting WPT report")?;

        let raw: RawReport =
            serde_json::from_str(&report_json).context("parsing WPT bridge output")?;

        WptRun::from_raw(raw, script_name)
    }

    async fn wait_for_completion(
        &self,
        environment: &JsDomEnvironment,
        script_name: &str,
    ) -> Result<()> {
        let start = Instant::now();
        loop {
            environment
                .pump()
                .with_context(|| format!("pumping QuickJS for {script_name}"))?;

            let done: bool = environment
                .eval_with(
                    "(() => {\n  const fn = globalThis.__frontierWptIsDone;\n  if (typeof fn !== 'function') {\n    return false;\n  }\n  try {\n    return Boolean(fn());\n  } catch (err) {\n    console.log('WPT completion probe failed', err);\n    return false;\n  }\n})()",
                    "frontier-wpt-check.js",
                )
                .context("checking WPT completion flag")?;

            if done {
                return Ok(());
            }

            if start.elapsed() > self.timeout {
                let harness_state: Option<String> = environment
                    .eval_with(
                        "(() => {\n  if (typeof tests !== 'undefined' && tests.status && typeof tests.status.message !== 'undefined') {\n    return String(tests.status.message);\n  }\n  return null;\n})()",
                        "frontier-wpt-timeout-debug.js",
                    )
                    .unwrap_or(None);
                let diagnostics: Option<String> = environment
                    .eval_with(
                        "(() => {\n  if (typeof tests === 'undefined' || !tests) {\n    return null;\n  }\n  try {\n    const summary = {\n      pending: tests.num_pending,\n      length: tests.tests ? tests.tests.length : 0,\n      allLoaded: !!tests.test_environment && !!tests.test_environment.all_loaded,\n      phase: tests.phase,\n      waitForFinish: !!tests.wait_for_finish,\n    };\n    return JSON.stringify(summary);\n  } catch (err) {\n    return String(err);\n  }\n})()",
                        "frontier-wpt-timeout-state.js",
                    )
                    .unwrap_or(None);
                return Err(anyhow!(
                    "WPT test {script_name} timed out after {:?}{}{}",
                    self.timeout,
                    harness_state
                        .as_deref()
                        .map(|msg| format!(" (last harness message: {msg})"))
                        .unwrap_or_default(),
                    diagnostics
                        .as_deref()
                        .map(|details| format!(" [state: {details}]"))
                        .unwrap_or_default()
                ));
            }

            sleep(POLL_INTERVAL).await;
        }
    }
}

impl WptRun {
    fn from_raw(raw: RawReport, script_name: &str) -> Result<Self> {
        if !raw.done {
            return Err(anyhow!(
                "WPT bridge reported incomplete results for {script_name}"
            ));
        }

        let tests = raw
            .tests
            .into_iter()
            .map(|test| WptTestResult {
                name: test.name,
                status: WptStatus::from_raw(&test.status),
                message: test.message,
                stack: test.stack,
            })
            .collect();

        let harness_status = raw.harness_status.map(|status| {
            let status_string = status.status.unwrap_or_else(|| "UNKNOWN".to_string());
            WptHarnessStatus {
                status: HarnessOutcome::from_raw(&status_string),
                message: status.message,
                stack: status.stack,
            }
        });

        let asserts = raw
            .asserts
            .into_iter()
            .map(|assert| WptAssertRecord {
                name: assert.name,
                status: assert.status,
                stack: assert.stack,
                test: assert.test.map(|test| WptAssertTestRef {
                    name: test.name,
                    status: WptStatus::from_raw(&test.status),
                }),
            })
            .collect();

        Ok(Self {
            tests,
            harness_status,
            asserts,
        })
    }
}

fn wrap_harness_for_shell(source: &str) -> String {
    let mut wrapped = String::with_capacity(source.len() + 256);
    wrapped.push_str(
        "(function(){\n  var __frontierHadDocument = typeof document !== 'undefined';\n  var __frontierDocument = __frontierHadDocument ? document : undefined;\n  if (__frontierHadDocument) {\n    try {\n      delete globalThis.document;\n    } catch (err) {\n      globalThis.document = undefined;\n    }\n  }\n\n",
    );
    wrapped.push_str(source);
    wrapped.push_str(
        "\n\n  if (__frontierHadDocument) {\n    globalThis.document = __frontierDocument;\n  }\n})();\n",
    );
    wrapped
}

#[derive(Debug, Deserialize)]
struct RawReport {
    done: bool,
    tests: Vec<RawTest>,
    #[serde(rename = "harnessStatus")]
    harness_status: Option<RawHarnessStatus>,
    #[serde(default)]
    asserts: Vec<RawAssert>,
}

#[derive(Debug, Deserialize)]
struct RawTest {
    name: String,
    status: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    stack: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawHarnessStatus {
    status: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    stack: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAssert {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    stack: Option<String>,
    #[serde(default)]
    test: Option<RawTest>,
}
