#![allow(dead_code)]
#![allow(clippy::disallowed_types)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElementSelector {
    Css {
        selector: String,
    },
    Role {
        role: String,
        #[serde(default)]
        name: Option<String>,
    },
}

impl ElementSelector {
    pub fn css(selector: impl Into<String>) -> Self {
        Self::Css {
            selector: selector.into(),
        }
    }

    pub fn role(role: impl Into<String>, name: Option<String>) -> Self {
        Self::Role {
            role: role.into(),
            name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum PointerTarget {
    Element {
        selector: ElementSelector,
        #[serde(default, skip_serializing_if = "PointerOffset::is_zero")]
        offset: Option<PointerOffset>,
    },
    Viewport {
        x: f64,
        y: f64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct PointerOffset {
    pub x: f64,
    pub y: f64,
}

impl PointerOffset {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn is_zero(offset: &Option<Self>) -> bool {
        matches!(offset, None | Some(PointerOffset { x: 0.0, y: 0.0 }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PointerAction {
    Move {
        to: PointerTarget,
    },
    Down {
        button: PointerButton,
    },
    Up {
        button: PointerButton,
    },
    Scroll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        origin: Option<PointerTarget>,
        delta_x: f64,
        delta_y: f64,
    },
    Pause {
        duration_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Primary,
    Secondary,
    Auxiliary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeyboardAction {
    Text {
        value: String,
    },
    Shortcut {
        key: String,
        #[serde(default)]
        modifiers: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AutomationCommand {
    Click {
        selector: ElementSelector,
    },
    TypeText {
        selector: ElementSelector,
        text: String,
    },
    GetText {
        selector: ElementSelector,
    },
    ElementExists {
        selector: ElementSelector,
    },
    Pump {
        duration_ms: u64,
    },
    Navigate {
        target: String,
    },
    PointerSequence {
        actions: Vec<PointerAction>,
    },
    KeyboardSequence {
        actions: Vec<KeyboardAction>,
    },
    Focus {
        selector: ElementSelector,
    },
    ScrollIntoView {
        selector: ElementSelector,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutomationArtifacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dom_html: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AutomationResponse {
    None,
    Text(String),
    Bool(bool),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationReply {
    pub response: AutomationResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<AutomationArtifacts>,
}

pub type AutomationResult = Result<AutomationReply>;

pub struct AutomationTask {
    pub command: AutomationCommand,
    responder: Option<oneshot::Sender<AutomationResult>>,
}

impl AutomationTask {
    pub fn new(command: AutomationCommand, responder: oneshot::Sender<AutomationResult>) -> Self {
        Self {
            command,
            responder: Some(responder),
        }
    }

    pub fn into_parts(mut self) -> (AutomationCommand, oneshot::Sender<AutomationResult>) {
        let responder = self.responder.take().expect("automation responder missing");
        (self.command, responder)
    }
}

pub struct AutomationState {
    queue: Mutex<VecDeque<AutomationTask>>,
}

impl AutomationState {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
        }
    }

    pub fn enqueue(&self, task: AutomationTask) {
        let mut queue = self.queue.lock().unwrap();
        queue.push_back(task);
    }

    pub fn pop(&self) -> Option<AutomationTask> {
        let mut queue = self.queue.lock().unwrap();
        queue.pop_front()
    }
}

impl Default for AutomationState {
    fn default() -> Self {
        Self::new()
    }
}

pub type AutomationStateHandle = Arc<AutomationState>;

#[derive(Debug)]
pub struct AutomationEvent;
