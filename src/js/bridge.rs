use std::collections::HashMap;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use blitz_dom::node::NodeData;
use blitz_dom::{local_name, ns, BaseDocument, DocumentMutator, LocalName, QualName};
use html_escape::{encode_double_quoted_attribute, encode_text};

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

    fn with_document_mut<T>(
        &mut self,
        f: impl FnOnce(&mut BaseDocument, &mut HashMap<String, usize>) -> T,
    ) -> T {
        unsafe {
            let document = self.document.as_mut();
            f(document, &mut self.id_index)
        }
    }

    fn with_document_ref<T>(
        &self,
        f: impl FnOnce(&BaseDocument, &HashMap<String, usize>) -> T,
    ) -> T {
        unsafe { f(self.document.as_ref(), &self.id_index) }
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

    pub fn find_node_by_html_id(&mut self, id: &str) -> Option<usize> {
        self.with_document_mut(|document, index| Self::lookup_node_id_internal(document, index, id))
    }

    pub fn text_content(&self, node_id: usize) -> Option<String> {
        self.with_document_ref(|document, _| {
            document.get_node(node_id).map(|node| node.text_content())
        })
    }

    pub fn inner_html(&self, node_id: usize) -> Result<String> {
        self.with_document_ref(|document, _| {
            let mut output = String::new();
            self.serialize_children(document, node_id, &mut output)?;
            Ok(output)
        })
    }

    pub fn set_text_content(&mut self, node_id: usize, value: &str) -> Result<()> {
        self.with_document_mut(|document, index| {
            let Some(node) = document.get_node(node_id) else {
                return Err(anyhow!("missing node {node_id}"));
            };
            if node.text_content() == value {
                return Ok(());
            }

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.remove_and_drop_all_children(node_id);
                if !value.is_empty() {
                    let text_id = mutator.create_text_node(value);
                    mutator.append_children(node_id, &[text_id]);
                }
            }

            Self::refresh_node_index_internal(document, index, node_id);
            Ok(())
        })
    }

    pub fn set_inner_html(&mut self, node_id: usize, value: &str) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            {
                let mut mutator = DocumentMutator::new(document);
                mutator.set_inner_html(node_id, value);
            }
            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn set_attribute(&mut self, node_id: usize, name: &str, value: &str) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;

            let normalized = name.to_ascii_lowercase();
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

    pub fn get_attribute(&self, node_id: usize, name: &str) -> Option<String> {
        self.with_document_ref(|document, _| {
            let node = document.get_node(node_id)?;
            let normalized = name.to_ascii_lowercase();
            let local_name = LocalName::from(normalized.as_str());
            node.attr(local_name).map(|s| s.to_string())
        })
    }

    pub fn remove_attribute(&mut self, node_id: usize, name: &str) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;

            let normalized = name.to_ascii_lowercase();
            {
                let mut mutator = DocumentMutator::new(document);
                mutator.clear_attribute(node_id, Self::html_name(&normalized));
            }

            if normalized == "id" {
                Self::reindex_internal(document, index);
            } else {
                Self::refresh_node_index_internal(document, index, node_id);
            }

            Ok(())
        })
    }

    pub fn create_element(&mut self, tag_name: &str) -> Result<usize> {
        self.with_document_mut(|document, _| {
            let mut mutator = DocumentMutator::new(document);
            let normalized = tag_name.to_ascii_lowercase();
            let node_id = mutator.create_element(Self::html_name(&normalized), vec![]);
            Ok(node_id)
        })
    }

    pub fn create_text_node(&mut self, data: &str) -> usize {
        self.with_document_mut(|document, _| {
            let mut mutator = DocumentMutator::new(document);
            mutator.create_text_node(data)
        })
    }

    pub fn append_child(&mut self, parent_id: usize, child_id: usize) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            document
                .get_node(child_id)
                .ok_or_else(|| anyhow!("missing child node {child_id}"))?;

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.append_children(parent_id, &[child_id]);
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn insert_before(
        &mut self,
        parent_id: usize,
        new_node_id: usize,
        reference_node_id: Option<usize>,
    ) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            document
                .get_node(new_node_id)
                .ok_or_else(|| anyhow!("missing new node {new_node_id}"))?;

            if let Some(ref_id) = reference_node_id {
                document
                    .get_node(ref_id)
                    .ok_or_else(|| anyhow!("missing reference node {ref_id}"))?;

                let mut mutator = DocumentMutator::new(document);
                mutator.insert_nodes_before(ref_id, &[new_node_id]);
            } else {
                let mut mutator = DocumentMutator::new(document);
                mutator.append_children(parent_id, &[new_node_id]);
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn remove_child(&mut self, parent_id: usize, child_id: usize) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            document
                .get_node(child_id)
                .ok_or_else(|| anyhow!("missing child node {child_id}"))?;

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.remove_and_drop_node(child_id);
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn replace_child(
        &mut self,
        parent_id: usize,
        new_child_id: usize,
        old_child_id: usize,
    ) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            document
                .get_node(new_child_id)
                .ok_or_else(|| anyhow!("missing new child node {new_child_id}"))?;
            document
                .get_node(old_child_id)
                .ok_or_else(|| anyhow!("missing old child node {old_child_id}"))?;

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.insert_nodes_before(old_child_id, &[new_child_id]);
                mutator.remove_and_drop_node(old_child_id);
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn get_children(&self, node_id: usize) -> Option<Vec<usize>> {
        self.with_document_ref(|document, _| {
            let node = document.get_node(node_id)?;
            Some(node.children.clone())
        })
    }

    pub fn get_parent(&self, node_id: usize) -> Option<usize> {
        self.with_document_ref(|document, _| {
            let node = document.get_node(node_id)?;
            node.parent
        })
    }

    pub fn get_tag_name(&self, node_id: usize) -> Option<String> {
        self.with_document_ref(|document, _| {
            let node = document.get_node(node_id)?;
            match &node.data {
                NodeData::Element(data) => Some(data.name.local.to_string()),
                _ => None,
            }
        })
    }

    pub fn get_node_type(&self, node_id: usize) -> Option<u8> {
        self.with_document_ref(|document, _| {
            let node = document.get_node(node_id)?;
            Some(match &node.data {
                NodeData::Element(_) => 1,        // ELEMENT_NODE
                NodeData::Text(_) => 3,           // TEXT_NODE
                NodeData::Comment => 8,           // COMMENT_NODE
                NodeData::Document => 9,          // DOCUMENT_NODE
                NodeData::AnonymousBlock(_) => 1, // Treat as element
            })
        })
    }

    pub fn serialize_document(&self) -> Result<String> {
        self.with_document_ref(|document, _| {
            let mut output = String::new();
            output.push_str("<!DOCTYPE html>");
            self.serialize_children(document, document.root_node().id, &mut output)?;
            Ok(output)
        })
    }

    fn serialize_children(
        &self,
        doc: &BaseDocument,
        node_id: usize,
        output: &mut String,
    ) -> Result<()> {
        let node = doc
            .get_node(node_id)
            .ok_or_else(|| anyhow!("missing node {node_id}"))?;
        for child in &node.children {
            self.serialize_node(doc, *child, output)?;
        }
        Ok(())
    }

    fn serialize_node(
        &self,
        doc: &BaseDocument,
        node_id: usize,
        output: &mut String,
    ) -> Result<()> {
        let node = doc
            .get_node(node_id)
            .ok_or_else(|| anyhow!("missing node {node_id}"))?;

        match &node.data {
            NodeData::Document | NodeData::AnonymousBlock(_) => {
                self.serialize_children(doc, node_id, output)?;
            }
            NodeData::Element(data) => {
                output.push('<');
                output.push_str(data.name.local.as_ref());
                for attr in data.attrs.iter() {
                    output.push(' ');
                    output.push_str(attr.name.local.as_ref());
                    output.push_str("=\"");
                    output.push_str(&encode_double_quoted_attribute(&attr.value));
                    output.push('"');
                }
                output.push('>');
                self.serialize_children(doc, node_id, output)?;
                output.push_str("</");
                output.push_str(data.name.local.as_ref());
                output.push('>');
            }
            NodeData::Text(text) => {
                output.push_str(&encode_text(&text.content));
            }
            NodeData::Comment => {
                output.push_str("<!-- -->");
            }
        }

        Ok(())
    }
}
