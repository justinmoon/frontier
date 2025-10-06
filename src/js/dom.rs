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
    CreateElement {
        result_handle: String,
        tag_name: String,
    },
    CreateTextNode {
        result_handle: String,
        data: String,
    },
    AppendChild {
        parent_handle: String,
        child_handle: String,
    },
    InsertBefore {
        parent_handle: String,
        new_handle: String,
        reference_handle: Option<String>,
    },
    RemoveChild {
        parent_handle: String,
        child_handle: String,
    },
    ReplaceChild {
        parent_handle: String,
        new_handle: String,
        old_handle: String,
    },
}

pub struct DomState {
    initial_html: String,
    mutations: Vec<DomPatch>,
    bridge: Option<BlitzJsBridge>,
    next_allocated_id: usize,
    node_id_map: std::collections::HashMap<String, usize>,
    node_handle_map: std::collections::HashMap<usize, String>,
}

impl DomState {
    pub fn new(html: &str) -> Self {
        Self {
            initial_html: html.to_string(),
            mutations: Vec::new(),
            bridge: None,
            next_allocated_id: 1_000_000, // Start high to avoid conflicts with parsed nodes
            node_id_map: std::collections::HashMap::new(),
            node_handle_map: std::collections::HashMap::new(),
        }
    }

    fn remember_handle(&mut self, handle: &str, node_id: usize) {
        let handle_string = handle.to_string();
        self.node_id_map.insert(handle_string.clone(), node_id);
        self.node_handle_map.insert(node_id, handle_string);
    }

    fn forget_handle(&mut self, node_id: usize) {
        if let Some(handle) = self.node_handle_map.remove(&node_id) {
            self.node_id_map.remove(&handle);
        }
    }

    fn public_handle_for(&self, node_id: usize) -> String {
        self.node_handle_map
            .get(&node_id)
            .cloned()
            .unwrap_or_else(|| node_id.to_string())
    }

    pub fn normalize_public_handle(&self, handle: &str) -> Option<String> {
        if handle == "document" {
            return Some("document".to_string());
        }

        if self.node_id_map.contains_key(handle) {
            return Some(handle.to_string());
        }

        handle
            .parse::<usize>()
            .ok()
            .map(|node_id| self.public_handle_for(node_id))
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
            .map(|node_id| self.public_handle_for(node_id))
    }

    pub fn text_content(&self, handle: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge.text_content(node_id)
    }

    pub fn inner_html(&self, handle: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge.inner_html(node_id).ok()
    }

    pub fn apply_patch(&mut self, patch: DomPatch) -> Result<bool> {
        // Resolve handles before borrowing bridge
        let resolved_ids = match &patch {
            DomPatch::TextContent { handle, .. } => {
                vec![self.resolve_handle(handle)?]
            }
            DomPatch::InnerHtml { handle, .. } => {
                vec![self.resolve_handle(handle)?]
            }
            DomPatch::Attribute { handle, .. } => {
                vec![self.resolve_handle(handle)?]
            }
            DomPatch::RemoveAttribute { handle, .. } => {
                vec![self.resolve_handle(handle)?]
            }
            DomPatch::AppendChild {
                parent_handle,
                child_handle,
            } => {
                vec![
                    self.resolve_handle(parent_handle)?,
                    self.resolve_handle(child_handle)?,
                ]
            }
            DomPatch::InsertBefore {
                parent_handle,
                new_handle,
                reference_handle,
            } => {
                let mut ids = vec![
                    self.resolve_handle(parent_handle)?,
                    self.resolve_handle(new_handle)?,
                ];
                if let Some(ref_h) = reference_handle {
                    ids.push(self.resolve_handle(ref_h)?);
                }
                ids
            }
            DomPatch::RemoveChild {
                parent_handle,
                child_handle,
            } => {
                vec![
                    self.resolve_handle(parent_handle)?,
                    self.resolve_handle(child_handle)?,
                ]
            }
            DomPatch::ReplaceChild {
                parent_handle,
                new_handle,
                old_handle,
            } => {
                vec![
                    self.resolve_handle(parent_handle)?,
                    self.resolve_handle(new_handle)?,
                    self.resolve_handle(old_handle)?,
                ]
            }
            _ => vec![],
        };

        let bridge = self
            .bridge
            .as_mut()
            .ok_or_else(|| anyhow!("DOM bridge not attached"))?;

        match &patch {
            DomPatch::TextContent { value, .. } => {
                bridge.set_text_content(resolved_ids[0], value)?;
            }
            DomPatch::InnerHtml { value, .. } => {
                bridge.set_inner_html(resolved_ids[0], value)?;
            }
            DomPatch::Attribute { name, value, .. } => {
                bridge.set_attribute(resolved_ids[0], name, value)?;
            }
            DomPatch::RemoveAttribute { name, .. } => {
                bridge.remove_attribute(resolved_ids[0], name)?;
            }
            DomPatch::CreateElement {
                result_handle,
                tag_name,
            } => {
                let node_id = bridge.create_element(tag_name)?;
                self.remember_handle(result_handle, node_id);
            }
            DomPatch::CreateTextNode {
                result_handle,
                data,
            } => {
                let node_id = bridge.create_text_node(data);
                self.remember_handle(result_handle, node_id);
            }
            DomPatch::AppendChild { .. } => {
                bridge.append_child(resolved_ids[0], resolved_ids[1])?;
            }
            DomPatch::InsertBefore {
                reference_handle, ..
            } => {
                let ref_id = if reference_handle.is_some() {
                    Some(resolved_ids[2])
                } else {
                    None
                };
                bridge.insert_before(resolved_ids[0], resolved_ids[1], ref_id)?;
            }
            DomPatch::RemoveChild { .. } => {
                bridge.remove_child(resolved_ids[0], resolved_ids[1])?;
                self.forget_handle(resolved_ids[1]);
            }
            DomPatch::ReplaceChild { .. } => {
                bridge.replace_child(resolved_ids[0], resolved_ids[1], resolved_ids[2])?;
                self.forget_handle(resolved_ids[2]);
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

    pub fn get_attribute(&self, handle: &str, name: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge.get_attribute(node_id, name)
    }

    pub fn get_children(&self, handle: &str) -> Option<Vec<String>> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge
            .get_children(node_id)
            .map(|ids| ids.iter().map(|id| self.public_handle_for(*id)).collect())
    }

    pub fn get_parent(&self, handle: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge
            .get_parent(node_id)
            .map(|id| self.public_handle_for(id))
    }

    pub fn get_tag_name(&self, handle: &str) -> Option<String> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge.get_tag_name(node_id)
    }

    pub fn get_node_type(&self, handle: &str) -> Option<u8> {
        let bridge = self.bridge.as_ref()?;
        let node_id = self.resolve_handle(handle).ok()?;
        bridge.get_node_type(node_id)
    }

    pub fn allocate_node_id(&mut self) -> Result<String> {
        let allocated = format!("alloc_{}", self.next_allocated_id);
        self.next_allocated_id += 1;
        Ok(allocated)
    }

    fn resolve_handle(&self, handle: &str) -> Result<usize> {
        // First try direct parse (for handles from getElementById etc)
        if let Ok(id) = handle.parse::<usize>() {
            return Ok(id);
        }

        // Otherwise lookup in the map (for allocated handles)
        self.node_id_map
            .get(handle)
            .copied()
            .ok_or_else(|| anyhow!("unknown handle '{handle}'"))
    }
}
