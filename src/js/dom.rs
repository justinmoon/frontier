use anyhow::{anyhow, Result};
use blitz_dom::BaseDocument;
use serde::{Deserialize, Serialize};

use super::bridge::BlitzJsBridge;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomPatch {
    TextContent {
        handle: String,
        value: String,
    },
    InnerHtml {
        handle: String,
        value: String,
    },
    Attribute {
        handle: String,
        name: String,
        value: String,
    },
}

pub struct DomState {
    initial_html: String,
    mutations: Vec<DomPatch>,
    bridge: Option<BlitzJsBridge>,
}

impl DomState {
    pub fn new(html: &str) -> Self {
        Self {
            initial_html: html.to_string(),
            mutations: Vec::new(),
            bridge: None,
        }
    }

    pub fn attach_document(&mut self, document: &mut BaseDocument) {
        if self.bridge.is_none() {
            self.bridge = Some(BlitzJsBridge::new(document));
        }
    }

    pub fn handle_from_element_id(&mut self, id: &str) -> Option<String> {
        let bridge = self.bridge.as_mut()?;
        bridge
            .find_node_by_html_id(id)
            .map(|node_id| node_id.to_string())
    }

    pub fn text_content(&self, handle: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = parse_handle(handle).ok()?;
        bridge.text_content(node_id)
    }

    pub fn inner_html(&self, handle: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = parse_handle(handle).ok()?;
        bridge.inner_html(node_id).ok()
    }

    pub fn apply_patch(&mut self, patch: DomPatch) -> Result<bool> {
        let bridge = self
            .bridge
            .as_mut()
            .ok_or_else(|| anyhow!("DOM bridge not attached"))?;

        match &patch {
            DomPatch::TextContent { handle, value } => {
                bridge.set_text_content(parse_handle(handle)?, value)?;
            }
            DomPatch::InnerHtml { handle, value } => {
                bridge.set_inner_html(parse_handle(handle)?, value)?;
            }
            DomPatch::Attribute {
                handle,
                name,
                value,
            } => {
                bridge.set_attribute(parse_handle(handle)?, name, value)?;
            }
        }

        self.mutations.push(patch);
        Ok(true)
    }

    pub fn drain_mutations(&mut self) -> Vec<DomPatch> {
        let mut drained = Vec::new();
        std::mem::swap(&mut drained, &mut self.mutations);
        drained
    }

    pub fn to_html(&self) -> Result<String> {
        if let Some(bridge) = self.bridge.as_ref() {
            bridge.serialize_document()
        } else {
            Ok(self.initial_html.clone())
        }
    }
}

fn parse_handle(handle: &str) -> Result<usize> {
    handle
        .parse::<usize>()
        .map_err(|err| anyhow!("invalid handle '{handle}': {err}"))
}
