use std::collections::HashMap;

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
    RemoveAttribute {
        handle: String,
        name: String,
    },
    AppendChild {
        parent: String,
        child: String,
    },
    InsertBefore {
        parent: String,
        child: String,
        reference: Option<String>,
    },
    RemoveChild {
        parent: String,
        child: String,
    },
    ReplaceChild {
        parent: String,
        new_node: String,
        old_node: String,
    },
    CreateElement {
        handle: String,
        name: String,
        namespace: Option<String>,
    },
    CreateText {
        handle: String,
        value: String,
    },
    CreateComment {
        handle: String,
        value: String,
    },
    CloneNode {
        source: String,
        handle: String,
        deep: bool,
    },
}

pub struct DomState {
    initial_html: String,
    mutations: Vec<DomPatch>,
    bridge: Option<BlitzJsBridge>,
    event_listener_counts: HashMap<String, usize>,
}

impl DomState {
    pub fn new(html: &str) -> Self {
        Self {
            initial_html: html.to_string(),
            mutations: Vec::new(),
            bridge: None,
            event_listener_counts: HashMap::new(),
        }
    }

    pub fn attach_document(&mut self, document: &mut BaseDocument) {
        if self.bridge.is_none() {
            self.bridge = Some(BlitzJsBridge::new(document));
        }
    }

    pub fn listen(&mut self, event_type: &str) {
        let key = normalize_event_name(event_type);
        *self.event_listener_counts.entry(key).or_default() += 1;
    }

    pub fn unlisten(&mut self, event_type: &str) {
        let key = normalize_event_name(event_type);
        if let Some(count) = self.event_listener_counts.get_mut(&key) {
            if *count > 1 {
                *count -= 1;
            } else {
                self.event_listener_counts.remove(&key);
            }
        }
    }

    pub fn is_listening(&self, event_type: &str) -> bool {
        let key = normalize_event_name(event_type);
        self.event_listener_counts.contains_key(&key)
    }

    fn bridge_mut(&mut self) -> Result<&mut BlitzJsBridge> {
        self.bridge
            .as_mut()
            .ok_or_else(|| anyhow!("DOM bridge not attached"))
    }

    fn bridge_ref(&self) -> Result<&BlitzJsBridge> {
        self.bridge
            .as_ref()
            .ok_or_else(|| anyhow!("DOM bridge not attached"))
    }

    fn record_mutation(&mut self, patch: DomPatch) {
        self.mutations.push(patch);
    }

    pub fn handle_from_element_id(&mut self, id: &str) -> Option<String> {
        let bridge = self.bridge.as_mut()?;
        bridge.find_node_by_html_id(id).map(format_handle)
    }

    pub fn handle_to_string(&self, handle: usize) -> String {
        format_handle(handle)
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

    pub fn set_text_content_direct(&mut self, handle: &str, value: &str) -> Result<()> {
        self.apply_patch(DomPatch::TextContent {
            handle: handle.to_string(),
            value: value.to_string(),
        })?;
        Ok(())
    }

    pub fn set_inner_html_direct(&mut self, handle: &str, value: &str) -> Result<()> {
        self.apply_patch(DomPatch::InnerHtml {
            handle: handle.to_string(),
            value: value.to_string(),
        })?;
        Ok(())
    }

    pub fn set_attribute_direct(&mut self, handle: &str, name: &str, value: &str) -> Result<()> {
        self.apply_patch(DomPatch::Attribute {
            handle: handle.to_string(),
            name: name.to_string(),
            value: value.to_string(),
        })?;
        Ok(())
    }

    pub fn remove_attribute_direct(&mut self, handle: &str, name: &str) -> Result<()> {
        self.apply_patch(DomPatch::RemoveAttribute {
            handle: handle.to_string(),
            name: name.to_string(),
        })?;
        Ok(())
    }

    pub fn create_element(&mut self, name: &str, namespace: Option<&str>) -> Result<String> {
        let node_id = self.bridge_mut()?.create_element(name, namespace)?;
        let handle = format_handle(node_id);
        self.record_mutation(DomPatch::CreateElement {
            handle: handle.clone(),
            name: name.to_string(),
            namespace: namespace.map(|ns| ns.to_string()),
        });
        Ok(handle)
    }

    pub fn create_text_node(&mut self, value: &str) -> Result<String> {
        let node_id = self.bridge_mut()?.create_text_node(value)?;
        let handle = format_handle(node_id);
        self.record_mutation(DomPatch::CreateText {
            handle: handle.clone(),
            value: value.to_string(),
        });
        Ok(handle)
    }

    pub fn create_comment_node(&mut self, value: &str) -> Result<String> {
        let node_id = self.bridge_mut()?.create_comment_node(value)?;
        let handle = format_handle(node_id);
        self.record_mutation(DomPatch::CreateComment {
            handle: handle.clone(),
            value: value.to_string(),
        });
        Ok(handle)
    }

    pub fn append_child(&mut self, parent: &str, child: &str) -> Result<()> {
        let parent_id = parse_handle(parent)?;
        let child_id = parse_handle(child)?;
        self.bridge_mut()?.append_child(parent_id, child_id)?;
        self.record_mutation(DomPatch::AppendChild {
            parent: parent.to_string(),
            child: child.to_string(),
        });
        Ok(())
    }

    pub fn insert_before(
        &mut self,
        parent: &str,
        child: &str,
        reference: Option<&str>,
    ) -> Result<()> {
        let parent_id = parse_handle(parent)?;
        let child_id = parse_handle(child)?;
        let reference_owned = reference.map(|s| s.to_string());
        let reference_id = match reference_owned.as_deref() {
            Some(value) => Some(parse_handle(value)?),
            None => None,
        };
        self.bridge_mut()?
            .insert_before(parent_id, child_id, reference_id)?;
        self.record_mutation(DomPatch::InsertBefore {
            parent: parent.to_string(),
            child: child.to_string(),
            reference: reference_owned,
        });
        Ok(())
    }

    pub fn remove_child(&mut self, parent: &str, child: &str) -> Result<()> {
        let parent_id = parse_handle(parent)?;
        let child_id = parse_handle(child)?;
        self.bridge_mut()?.remove_child(parent_id, child_id)?;
        self.record_mutation(DomPatch::RemoveChild {
            parent: parent.to_string(),
            child: child.to_string(),
        });
        Ok(())
    }

    pub fn replace_child(&mut self, parent: &str, new_child: &str, old_child: &str) -> Result<()> {
        let parent_id = parse_handle(parent)?;
        let new_child_id = parse_handle(new_child)?;
        let old_child_id = parse_handle(old_child)?;
        self.bridge_mut()?
            .replace_child(parent_id, new_child_id, old_child_id)?;
        self.record_mutation(DomPatch::ReplaceChild {
            parent: parent.to_string(),
            new_node: new_child.to_string(),
            old_node: old_child.to_string(),
        });
        Ok(())
    }

    pub fn clone_node(&mut self, handle: &str, deep: bool) -> Result<String> {
        let node_id = parse_handle(handle)?;
        let cloned_id = self.bridge_mut()?.clone_node(node_id, deep)?;
        let cloned_handle = format_handle(cloned_id);
        self.record_mutation(DomPatch::CloneNode {
            source: handle.to_string(),
            handle: cloned_handle.clone(),
            deep,
        });
        Ok(cloned_handle)
    }

    pub fn parent_handle(&self, handle: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        let parent = self.bridge_ref()?.parent_node(node_id)?;
        Ok(optional_handle(parent))
    }

    pub fn first_child_handle(&self, handle: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        let child = self.bridge_ref()?.first_child(node_id)?;
        Ok(optional_handle(child))
    }

    pub fn next_sibling_handle(&self, handle: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        let sibling = self.bridge_ref()?.next_sibling(node_id)?;
        Ok(optional_handle(sibling))
    }

    pub fn previous_sibling_handle(&self, handle: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        let sibling = self.bridge_ref()?.previous_sibling(node_id)?;
        Ok(optional_handle(sibling))
    }

    pub fn child_handles(&self, handle: &str) -> Result<Vec<String>> {
        let node_id = parse_handle(handle)?;
        let children = self.bridge_ref()?.child_nodes(node_id)?;
        Ok(children.into_iter().map(format_handle).collect())
    }

    pub fn node_name(&self, handle: &str) -> Result<String> {
        let node_id = parse_handle(handle)?;
        self.bridge_ref()?.node_name(node_id)
    }

    pub fn node_type(&self, handle: &str) -> Result<u16> {
        let node_id = parse_handle(handle)?;
        self.bridge_ref()?.node_type(node_id)
    }

    pub fn node_value(&self, handle: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        self.bridge_ref()?.node_value(node_id)
    }

    pub fn get_attribute(&self, handle: &str, name: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        self.bridge_ref()?.get_attribute(node_id, name)
    }

    pub fn namespace_uri(&self, handle: &str) -> Result<Option<String>> {
        let node_id = parse_handle(handle)?;
        let ns = self.bridge_ref()?.namespace_uri(node_id)?;
        Ok(ns.map(|value| value.to_string()))
    }

    pub fn document_handle(&self) -> Result<String> {
        let handle = self.bridge_ref()?.document_handle();
        Ok(format_handle(handle))
    }

    pub fn apply_patch(&mut self, patch: DomPatch) -> Result<bool> {
        let bridge = self.bridge_mut()?;

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
            DomPatch::RemoveAttribute { handle, name } => {
                bridge.remove_attribute(parse_handle(handle)?, name)?;
            }
            other => {
                // Record-only variants (created outside the patch API).
                self.record_mutation((*other).clone());
                return Ok(true);
            }
        }

        self.record_mutation(patch);
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

fn format_handle(handle: usize) -> String {
    handle.to_string()
}

fn optional_handle(handle: Option<usize>) -> Option<String> {
    handle.map(format_handle)
}

fn normalize_event_name(name: &str) -> String {
    let trimmed = name.trim();
    let without_on = trimmed.strip_prefix("on").unwrap_or(trimmed);
    without_on.to_ascii_lowercase()
}
