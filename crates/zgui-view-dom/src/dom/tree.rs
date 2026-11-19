//! Creating a whole subtree in one batch, and the shared declaration blocks it can carry.

use std::collections::HashMap;

use zgui_dom::NodeIndex;
use zgui_dom::side::inline_style::StyleBlock;
use zgui_view::{NodeId, StyleId, TemplateKind, TreeTemplate};

use crate::dom::DocumentDom;
use crate::id;

/// The parsed block of every [`StyleId`] this backend has applied, by the id's number.
#[derive(Default)]
pub(crate) struct StyleBlocks {
    held: HashMap<u64, StyleBlock>,
}

impl DocumentDom {
    /// The parsed block `style` names, parsed the first time it is asked for and shared after.
    pub(crate) fn style_block(&self, style: &StyleId) -> StyleBlock {
        if let Some(block) = self.style_blocks().borrow().held.get(&style.id()) {
            return block.clone();
        }
        let block = self.edit(|edit| edit.parse_style_block(style.css()));
        self.style_blocks()
            .borrow_mut()
            .held
            .insert(style.id(), block.clone());
        block
    }

    /// Creates every node of `template` and links them in one batch of changes.
    ///
    /// One batch rather than one per node: the document's per-batch work — working out which
    /// siblings a structural change can affect, handing each element its restyle hint, asking for
    /// a frame — runs once for the whole subtree.
    pub(crate) fn build_tree(&self, template: &TreeTemplate) -> Vec<NodeId> {
        assert!(
            template.is_well_formed(),
            "a template's parents come before their children"
        );
        let blocks: Vec<Option<StyleBlock>> = template
            .nodes
            .iter()
            .map(|node| node.style.as_ref().map(|style| self.style_block(style)))
            .collect();
        let indexes: Vec<NodeIndex> = self.edit(|edit| {
            let mut out: Vec<NodeIndex> = Vec::with_capacity(template.nodes.len());
            for (at, node) in template.nodes.iter().enumerate() {
                let index = match &node.kind {
                    TemplateKind::Element(name) => {
                        let el = if self.is_replaced_tag(name) {
                            edit.create_replaced_element(*name)
                        } else {
                            edit.create_element(*name)
                        };
                        if !node.classes.is_empty() {
                            edit.set_classes(el, &node.classes);
                        }
                        for (name, value) in &node.attributes {
                            edit.set_attribute(el, *name, Some(value.as_str().into()));
                        }
                        if let Some(block) = &blocks[at] {
                            edit.set_inline_style_block(el, Some(block.clone()));
                        }
                        el
                    }
                    TemplateKind::Text(text) => {
                        let created = edit.create_text("");
                        if !text.is_empty() {
                            edit.set_text(created, text);
                        }
                        created
                    }
                };
                if let Some(parent) = node.parent {
                    edit.insert_before(out[parent], index, None);
                }
                out.push(index);
            }
            out
        });
        // The attribute hook fires after the batch, as it does for one attribute at a time.
        for (node, index) in template.nodes.iter().zip(&indexes) {
            for (name, value) in &node.attributes {
                self.note_attribute(*index, *name, Some(value.as_str()));
            }
        }
        let document = self.document();
        indexes
            .into_iter()
            .map(|index| id::to_view(document.store().key_of(index)))
            .collect()
    }
}
