use std::collections::HashMap;
use std::ptr::NonNull;

use anyhow::Result;
use blitz_dom::{local_name, ns, BaseDocument, DocumentMutator, LocalName, QualName};

use super::dom::DomPatch;

/// Applies DOM mutations emitted from QuickJS to the live Blitz document.
pub struct BlitzJsBridge {
    document: NonNull<BaseDocument>,
    id_index: HashMap<String, usize>,
}

impl BlitzJsBridge {
    pub fn new(document: &mut BaseDocument) -> Self {
        let pointer = NonNull::new(document as *mut BaseDocument).expect("document pointer");
        let mut id_index = HashMap::new();
        Self::reindex_internal(document, &mut id_index);
        Self {
            document: pointer,
            id_index,
        }
    }

    fn with_document<T>(
        &mut self,
        f: impl FnOnce(&mut BaseDocument, &mut HashMap<String, usize>) -> T,
    ) -> T {
        unsafe {
            let document = self.document.as_mut();
            f(document, &mut self.id_index)
        }
    }

    fn reindex_internal(document: &mut BaseDocument, index: &mut HashMap<String, usize>) {
        index.clear();
        let root_id = document.root_node().id;
        document.iter_subtree_mut(root_id, |node_id, doc| {
            if let Some(node) = doc.get_node(node_id) {
                if let Some(id_attr) = node.attr(local_name!("id")) {
                    index.insert(id_attr.to_string(), node_id);
                }
            }
        });
    }

    fn lookup_node_id_internal(
        document: &mut BaseDocument,
        index: &mut HashMap<String, usize>,
        id: &str,
    ) -> Option<usize> {
        if let Some(node_id) = index.get(id).copied() {
            return Some(node_id);
        }
        Self::reindex_internal(document, index);
        index.get(id).copied()
    }

    fn refresh_node_index_internal(
        document: &mut BaseDocument,
        index: &mut HashMap<String, usize>,
        target_id: usize,
    ) {
        index.retain(|_, mapped| *mapped != target_id);
        if let Some(node) = document.get_node(target_id) {
            if let Some(id_attr) = node.attr(local_name!("id")) {
                index.insert(id_attr.to_string(), target_id);
            }
        }
    }

    fn html_name(name: &str) -> QualName {
        QualName::new(None, ns!(html), LocalName::from(name))
    }

    fn set_text_content(&mut self, id: &str, value: &str) -> Result<()> {
        self.with_document(|document, index| {
            let Some(node_id) = Self::lookup_node_id_internal(document, index, id) else {
                return Ok(());
            };

            let current = document
                .get_node(node_id)
                .map(|node| node.text_content())
                .unwrap_or_default();
            if current == value {
                return Ok(());
            }

            let mut mutator = DocumentMutator::new(document);
            mutator.remove_and_drop_all_children(node_id);
            if !value.is_empty() {
                let text_id = mutator.create_text_node(value);
                mutator.append_children(node_id, &[text_id]);
            }

            Ok(())
        })
    }

    fn set_inner_html(&mut self, id: &str, value: &str) -> Result<()> {
        self.with_document(|document, index| {
            let Some(node_id) = Self::lookup_node_id_internal(document, index, id) else {
                return Ok(());
            };

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.set_inner_html(node_id, value);
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    fn set_attribute(&mut self, id: &str, name: &str, value: &str) -> Result<()> {
        self.with_document(|document, index| {
            let Some(node_id) = Self::lookup_node_id_internal(document, index, id) else {
                return Ok(());
            };

            let normalized = name.to_ascii_lowercase();
            let attr_local = LocalName::from(&*normalized);
            let existing = document
                .get_node(node_id)
                .and_then(|node| node.attr(attr_local.clone()));

            if existing.map(|current| current == value).unwrap_or(false) {
                return Ok(());
            }

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.set_attribute(node_id, Self::html_name(&normalized), value);
            }

            if normalized == "id" {
                Self::reindex_internal(document, index);
            } else {
                Self::refresh_node_index_internal(document, index, node_id);
            }

            Ok(())
        })
    }

    pub fn apply_patch(&mut self, patch: &DomPatch) -> Result<()> {
        match patch {
            DomPatch::TextContent { id, value } => self.set_text_content(id, value),
            DomPatch::InnerHtml { id, value } => self.set_inner_html(id, value),
            DomPatch::Attribute { id, name, value } => self.set_attribute(id, name, value),
        }
    }
}
