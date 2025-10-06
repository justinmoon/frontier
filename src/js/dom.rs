use std::collections::HashMap;

use anyhow::{anyhow, Context as AnyhowContext, Result};
use blitz_dom::BaseDocument;
use kuchiki::traits::*;
use kuchiki::{parse_fragment, parse_html, NodeRef};
use serde::{Deserialize, Serialize};

use super::bridge::BlitzJsBridge;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomPatch {
    TextContent {
        id: String,
        value: String,
    },
    InnerHtml {
        id: String,
        value: String,
    },
    Attribute {
        id: String,
        name: String,
        value: String,
    },
}

#[derive(Debug)]
pub struct DomSnapshot {
    document: NodeRef,
    index: HashMap<String, NodeRef>,
}

impl DomSnapshot {
    pub fn parse(html: &str) -> Result<Self> {
        let document = parse_html().one(html);
        let mut snapshot = Self {
            document,
            index: HashMap::new(),
        };
        snapshot.reindex()?;
        Ok(snapshot)
    }

    pub fn has_element(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    pub fn text_content(&self, id: &str) -> Option<String> {
        self.index.get(id).map(|node| node.text_contents())
    }

    pub fn inner_html(&self, id: &str) -> Option<String> {
        let node = self.index.get(id)?.clone();
        let mut buf = Vec::new();
        for child in node.children() {
            if child.serialize(&mut buf).is_err() {
                return None;
            }
        }
        String::from_utf8(buf).ok()
    }

    pub fn set_text_content(&mut self, id: &str, value: &str) -> Result<bool> {
        let Some(node) = self.index.get(id).cloned() else {
            return Ok(false);
        };
        if self.text_content(id).as_deref() == Some(value) {
            return Ok(false);
        }

        let children: Vec<_> = node.children().collect();
        for child in children {
            child.detach();
        }
        if !value.is_empty() {
            node.append(NodeRef::new_text(value));
        }

        self.reindex()?;
        Ok(true)
    }

    pub fn set_inner_html(&mut self, id: &str, html: &str) -> Result<bool> {
        let Some(node) = self.index.get(id).cloned() else {
            return Ok(false);
        };
        if self.inner_html(id).as_deref() == Some(html) {
            return Ok(false);
        }

        let element = node
            .clone()
            .into_element_ref()
            .ok_or_else(|| anyhow!("node with id '{id}' is not an element"))?;

        let children: Vec<_> = node.children().collect();
        for child in children {
            child.detach();
        }

        let fragment = parse_fragment(element.name.clone(), Vec::new()).one(html);
        let new_children: Vec<_> = fragment.children().collect();
        for child in new_children {
            node.append(child);
        }

        self.reindex()?;
        Ok(true)
    }

    pub fn set_attribute(&mut self, id: &str, name: &str, value: &str) -> Result<bool> {
        let Some(node) = self.index.get(id).cloned() else {
            return Ok(false);
        };
        let element = node
            .into_element_ref()
            .ok_or_else(|| anyhow!("node with id '{id}' is not an element"))?;
        let mut attributes = element.attributes.borrow_mut();
        if attributes.get(name) == Some(value) {
            return Ok(false);
        }
        attributes.insert(name, value.to_string());
        drop(attributes);
        self.reindex()?;
        Ok(true)
    }

    pub fn to_html(&self) -> Result<String> {
        let mut buf = Vec::new();
        self.document
            .serialize(&mut buf)
            .context("failed to serialize document")?;
        String::from_utf8(buf).context("serialized HTML was not UTF-8")
    }

    pub fn apply_patch(&mut self, patch: &DomPatch) -> Result<bool> {
        match patch {
            DomPatch::TextContent { id, value } => self.set_text_content(id, value),
            DomPatch::InnerHtml { id, value } => self.set_inner_html(id, value),
            DomPatch::Attribute { id, name, value } => self.set_attribute(id, name, value),
        }
    }

    fn reindex(&mut self) -> Result<()> {
        self.index.clear();
        let matches = self
            .document
            .select("*[id]")
            .map_err(|_| anyhow!("invalid selector"))?;
        for element in matches {
            if let Some(id) = element.attributes.borrow().get("id") {
                self.index.insert(id.to_string(), element.as_node().clone());
            }
        }
        Ok(())
    }
}

pub struct DomState {
    snapshot: DomSnapshot,
    mutations: Vec<DomPatch>,
    bridge: Option<BlitzJsBridge>,
}

impl DomState {
    pub fn new(snapshot: DomSnapshot) -> Self {
        Self {
            snapshot,
            mutations: Vec::new(),
            bridge: None,
        }
    }

    pub fn has_element(&self, id: &str) -> bool {
        self.snapshot.has_element(id)
    }

    pub fn text_content(&self, id: &str) -> Option<String> {
        self.snapshot.text_content(id)
    }

    pub fn inner_html(&self, id: &str) -> Option<String> {
        self.snapshot.inner_html(id)
    }

    pub fn attach_document(&mut self, document: &mut BaseDocument) {
        self.bridge = Some(BlitzJsBridge::new(document));
    }

    pub fn apply_patch(&mut self, patch: DomPatch) -> Result<bool> {
        let changed = self.snapshot.apply_patch(&patch)?;

        if let Some(bridge) = self.bridge.as_mut() {
            bridge.apply_patch(&patch)?;
        }

        if changed {
            self.mutations.push(patch);
        }
        Ok(changed)
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
            self.snapshot.to_html()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DomPatch, DomSnapshot};

    #[test]
    fn set_text_updates_snapshot() {
        let html = r#"<!DOCTYPE html><html><body><h1 id="message">Loading…</h1></body></html>"#;
        let mut snapshot = DomSnapshot::parse(html).expect("parse");
        assert_eq!(
            snapshot.text_content("message").as_deref(),
            Some("Loading…")
        );

        assert!(snapshot
            .apply_patch(&DomPatch::TextContent {
                id: "message".into(),
                value: "Hello".into()
            })
            .unwrap());
        assert_eq!(snapshot.text_content("message").as_deref(), Some("Hello"));
    }

    #[test]
    fn set_html_replaces_children() {
        let html =
            r#"<!DOCTYPE html><html><body><div id="root"><span>Old</span></div></body></html>"#;
        let mut snapshot = DomSnapshot::parse(html).expect("parse");
        snapshot
            .apply_patch(&DomPatch::InnerHtml {
                id: "root".into(),
                value: "<strong>New</strong>".into(),
            })
            .unwrap();
        let inner = snapshot.inner_html("root").unwrap();
        assert!(inner.contains("<strong>New</strong>"));
    }
}
