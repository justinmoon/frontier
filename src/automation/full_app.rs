use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::oneshot;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AutomationCommand {
    Click { selector: String },
    TypeText { selector: String, text: String },
    GetText { selector: String },
    Pump { duration_ms: u64 },
    Navigate { target: String },
    Shutdown,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AutomationResponse {
    None,
    Text(String),
}

pub type AutomationResult = Result<AutomationResponse>;

pub struct AutomationTask {
    pub command: AutomationCommand,
    responder: Option<oneshot::Sender<AutomationResult>>,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
