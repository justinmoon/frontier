#![allow(clippy::disallowed_types)]

pub mod full_app;
pub(crate) mod headless;

pub use full_app::{
    AutomationArtifacts, AutomationCommand, AutomationEvent, AutomationReply, AutomationResponse,
    AutomationResult, AutomationStateHandle, ElementSelector, KeyboardAction, PointerAction,
    PointerButton, PointerTarget,
};
