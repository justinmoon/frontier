use serde::{Deserialize, Serialize};

/// How a script should be scheduled relative to HTML parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptExecution {
    /// Classic blocking scripts that run immediately and block HTML parsing.
    Blocking,
    /// Scripts marked as `async`, which download in parallel and run asap.
    Async,
    /// Scripts marked as `defer`, which run after document parsing before DOMContentLoaded.
    Defer,
}

impl Default for ScriptExecution {
    fn default() -> Self {
        Self::Blocking
    }
}

/// Minimal classification of the script language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptKind {
    /// Traditional classic scripts (JavaScript).
    Classic,
    /// `<script type="module">`.
    Module,
    /// Unknown/unsupported type; preserved for completeness.
    Unknown,
}

impl Default for ScriptKind {
    fn default() -> Self {
        Self::Classic
    }
}

/// Where the script source comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScriptSource {
    Inline { code: String },
    External { src: String },
}

/// Descriptor capturing everything we need to evaluate a script.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptDescriptor {
    pub index: usize,
    pub kind: ScriptKind,
    pub execution: ScriptExecution,
    pub source: ScriptSource,
}

impl ScriptDescriptor {
    pub fn inline(index: usize, code: String, kind: ScriptKind) -> Self {
        Self {
            index,
            kind,
            execution: ScriptExecution::Blocking,
            source: ScriptSource::Inline { code },
        }
    }
}
