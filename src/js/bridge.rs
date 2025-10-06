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
        Self::qualify_name(name, None)
    }

    fn qualify_name(name: &str, namespace: Option<&str>) -> QualName {
        let local = LocalName::from(name);
        match namespace {
            Some(ns_uri) if ns_uri.eq_ignore_ascii_case("http://www.w3.org/2000/svg") => {
                QualName::new(None, ns!(svg), local)
            }
            Some(ns_uri) if ns_uri.eq_ignore_ascii_case("http://www.w3.org/1998/Math/MathML") => {
                QualName::new(None, ns!(mathml), local)
            }
            Some(ns_uri) if ns_uri.eq_ignore_ascii_case("http://www.w3.org/1999/xhtml") => {
                QualName::new(None, ns!(html), local)
            }
            _ => QualName::new(None, ns!(html), local),
        }
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

    pub fn create_element(&mut self, tag: &str, namespace: Option<&str>) -> Result<usize> {
        self.with_document_mut(|document, index| {
            let qual = Self::qualify_name(tag, namespace);
            let node_id = {
                let mut mutator = DocumentMutator::new(document);
                mutator.create_element(qual, Vec::new())
            };

            Self::refresh_node_index_internal(document, index, node_id);
            Ok(node_id)
        })
    }

    pub fn create_text_node(&mut self, value: &str) -> Result<usize> {
        self.with_document_mut(|document, _| {
            let mut mutator = DocumentMutator::new(document);
            Ok(mutator.create_text_node(value))
        })
    }

    pub fn create_comment_node(&mut self, _value: &str) -> Result<usize> {
        self.with_document_mut(|document, _| {
            let mut mutator = DocumentMutator::new(document);
            Ok(mutator.create_comment_node())
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
        child_id: usize,
        reference_id: Option<usize>,
    ) -> Result<()> {
        self.with_document_mut(|document, index| {
            document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            document
                .get_node(child_id)
                .ok_or_else(|| anyhow!("missing child node {child_id}"))?;

            if let Some(reference) = reference_id {
                let reference_node = document
                    .get_node(reference)
                    .ok_or_else(|| anyhow!("missing reference node {reference}"))?;
                if reference_node.parent != Some(parent_id) {
                    return Err(anyhow!(
                        "reference node {reference} is not a child of parent {parent_id}"
                    ));
                }
            }

            {
                let mut mutator = DocumentMutator::new(document);
                if let Some(reference) = reference_id {
                    mutator.insert_nodes_before(reference, &[child_id]);
                } else {
                    mutator.append_children(parent_id, &[child_id]);
                }
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn remove_child(&mut self, parent_id: usize, child_id: usize) -> Result<()> {
        self.with_document_mut(|document, index| {
            let Some(node) = document.get_node(child_id) else {
                return Err(anyhow!("missing child node {child_id}"));
            };
            if node.parent != Some(parent_id) {
                return Err(anyhow!(
                    "node {child_id} is not a child of parent {parent_id}"
                ));
            }

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.remove_node(child_id);
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

            let Some(old_node) = document.get_node(old_child_id) else {
                return Err(anyhow!("missing existing child node {old_child_id}"));
            };
            if old_node.parent != Some(parent_id) {
                return Err(anyhow!(
                    "node {old_child_id} is not a child of parent {parent_id}"
                ));
            }

            document
                .get_node(new_child_id)
                .ok_or_else(|| anyhow!("missing replacement node {new_child_id}"))?;

            {
                let mut mutator = DocumentMutator::new(document);
                mutator.replace_node_with(old_child_id, &[new_child_id]);
            }

            Self::reindex_internal(document, index);
            Ok(())
        })
    }

    pub fn clone_node(&mut self, node_id: usize, deep: bool) -> Result<usize> {
        self.with_document_mut(|document, index| {
            document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;

            let cloned_id = {
                let mut mutator = DocumentMutator::new(document);
                let id = mutator.deep_clone_node(node_id);
                if !deep {
                    mutator.remove_and_drop_all_children(id);
                }
                id
            };

            Self::reindex_internal(document, index);
            Ok(cloned_id)
        })
    }

    pub fn parent_node(&self, node_id: usize) -> Result<Option<usize>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            Ok(node.parent)
        })
    }

    pub fn first_child(&self, node_id: usize) -> Result<Option<usize>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            Ok(node.children.first().copied())
        })
    }

    pub fn next_sibling(&self, node_id: usize) -> Result<Option<usize>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let parent_id = match node.parent {
                Some(id) => id,
                None => return Ok(None),
            };
            let parent = document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            let position = parent.children.iter().position(|id| *id == node_id);
            Ok(position
                .and_then(|idx| parent.children.get(idx + 1))
                .copied())
        })
    }

    pub fn previous_sibling(&self, node_id: usize) -> Result<Option<usize>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let parent_id = match node.parent {
                Some(id) => id,
                None => return Ok(None),
            };
            let parent = document
                .get_node(parent_id)
                .ok_or_else(|| anyhow!("missing parent node {parent_id}"))?;
            let position = parent.children.iter().position(|id| *id == node_id);
            Ok(position
                .and_then(|idx| {
                    if idx == 0 {
                        None
                    } else {
                        parent.children.get(idx - 1)
                    }
                })
                .copied())
        })
    }

    pub fn child_nodes(&self, node_id: usize) -> Result<Vec<usize>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            Ok(node.children.clone())
        })
    }

    pub fn node_name(&self, node_id: usize) -> Result<String> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let name = match &node.data {
                NodeData::Document => "#document".to_string(),
                NodeData::Element(data) => data.name.local.as_ref().to_ascii_uppercase(),
                NodeData::AnonymousBlock(data) => data.name.local.as_ref().to_ascii_uppercase(),
                NodeData::Text(_) => "#text".to_string(),
                NodeData::Comment => "#comment".to_string(),
            };
            Ok(name)
        })
    }

    pub fn node_type(&self, node_id: usize) -> Result<u16> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let ty = match node.data {
                NodeData::Document => 9,
                NodeData::Element(_) | NodeData::AnonymousBlock(_) => 1,
                NodeData::Text(_) => 3,
                NodeData::Comment => 8,
            };
            Ok(ty)
        })
    }

    pub fn node_value(&self, node_id: usize) -> Result<Option<String>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let value = match &node.data {
                NodeData::Text(text) => Some(text.content.clone()),
                NodeData::Comment => Some(String::new()),
                _ => None,
            };
            Ok(value)
        })
    }

    pub fn get_attribute(&self, node_id: usize, name: &str) -> Result<Option<String>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let attr_name = LocalName::from(name);
            let value = node.attr(attr_name).map(|s| s.to_string());
            Ok(value)
        })
    }

    pub fn namespace_uri(&self, node_id: usize) -> Result<Option<&'static str>> {
        self.with_document_ref(|document, _| {
            let node = document
                .get_node(node_id)
                .ok_or_else(|| anyhow!("missing node {node_id}"))?;
            let ns = match &node.data {
                NodeData::Element(data) | NodeData::AnonymousBlock(data) => {
                    if data.name.ns == ns!(html) {
                        Some("http://www.w3.org/1999/xhtml")
                    } else if data.name.ns == ns!(svg) {
                        Some("http://www.w3.org/2000/svg")
                    } else if data.name.ns == ns!(mathml) {
                        Some("http://www.w3.org/1998/Math/MathML")
                    } else {
                        None
                    }
                }
                _ => None,
            };
            Ok(ns)
        })
    }

    pub fn document_handle(&self) -> usize {
        self.with_document_ref(|document, _| document.root_node().id)
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
                    output.push_str(&encode_double_quoted_attribute(&attr.value).into_owned());
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
