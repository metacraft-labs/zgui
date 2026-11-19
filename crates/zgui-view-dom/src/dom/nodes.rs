//! Creating nodes, moving them, and taking them out again.

use zgui_dom::NodeKind;
use zgui_interned::ElementName;
use zgui_view::{DomError, NodeId};

use crate::dom::DocumentDom;
use crate::id;

impl DocumentDom {
    /// Creates a node of `kind` called `name`, outside the document.
    pub(crate) fn create(&self, kind: NodeKind, name: ElementName) -> NodeId {
        let replaced = kind == NodeKind::Element && self.is_replaced_tag(&name);
        let index = self.edit(|edit| match kind {
            NodeKind::Text => edit.create_text(""),
            NodeKind::Marker => edit.create_marker(),
            _ if replaced => edit.create_replaced_element(name),
            _ => edit.create_element(name),
        });
        id::to_view(self.document().store().key_of(index))
    }

    /// Puts `child` under `parent`, immediately before `before`.
    pub(crate) fn link(&self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        let parent = self.index_of(parent);
        let child = self.index_of(child);
        let before = before.map(|node| self.index_of(node));
        self.edit(|edit| edit.insert_before(parent, child, before));
    }

    /// Takes `node` out of the document.
    ///
    /// The record stays readable for the rest of the frame, and putting the node back before the
    /// frame ends keeps it — which is exactly what a list that moves a row does. What is still out
    /// when the frame ends is what the frame's end drops.
    ///
    /// A node that is already gone is left gone. A teardown can reach the same element twice —
    /// once from its own view and once from an ancestor whose whole subtree went — and the second
    /// arrival has nothing left to remove; unwinding over it would poison the document mid-batch.
    /// A poisoned document keeps the node.
    pub(crate) fn unlink(&self, node: NodeId) {
        let Some(node) = self.live_index_of(node) else {
            return;
        };
        self.edit_unless_poisoned(|edit| edit.remove(node));
    }

    /// The document's index of `node`, or why there is none.
    pub(crate) fn check_live(&self, node: NodeId) -> Result<zgui_dom::NodeIndex, DomError> {
        if !node.belongs_to(self.document_id()) {
            return Err(DomError::ForeignDocument(node));
        }
        let document = self.document();
        id::to_document(node)
            .and_then(|key| document.store().index_of(key))
            .ok_or(DomError::Gone(node))
    }

    /// Whether the document still accepts changes.
    pub(crate) fn check_writable(&self) -> Result<(), DomError> {
        if self.document().is_poisoned() {
            Err(DomError::Poisoned)
        } else {
            Ok(())
        }
    }

    /// Whether `node` is a live text node of this document.
    pub(crate) fn check_text(&self, node: NodeId) -> Result<(), DomError> {
        let index = self.check_live(node)?;
        self.check_writable()?;
        if self.document().store().core(index).kind() == NodeKind::Text {
            Ok(())
        } else {
            Err(DomError::NotText(node))
        }
    }

    /// Whether putting `child` under `parent` before `before` is an insertion the document can
    /// carry out.
    ///
    /// Everything the document would otherwise assert on inside the batch is checked here, before
    /// it opens: every node is live and belongs to this document, the parent can hold children,
    /// the reference node is a child of the parent, and the child is not the parent or one of its
    /// ancestors.
    pub(crate) fn check_insert(
        &self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), DomError> {
        let parent_index = self.check_live(parent)?;
        let child_index = self.check_live(child)?;
        let before_index = before.map(|node| self.check_live(node)).transpose()?;
        self.check_writable()?;
        let document = self.document();
        let store = document.store();
        if matches!(
            store.core(parent_index).kind(),
            NodeKind::Text | NodeKind::Marker
        ) {
            return Err(DomError::CannotHoldChildren(parent));
        }
        if let (Some(before), Some(index)) = (before, before_index)
            && (index == child_index || store.core(index).parent() != Some(parent_index))
        {
            return Err(DomError::NotAChild { parent, before });
        }
        let mut at = Some(parent_index);
        while let Some(index) = at {
            if index == child_index {
                return Err(DomError::Cycle { parent, child });
            }
            at = store.core(index).parent();
        }
        Ok(())
    }

    /// What `node` currently sits under, or nothing when it sits under the document itself.
    pub(crate) fn parent_of(&self, node: NodeId) -> Option<NodeId> {
        let document = self.document();
        let index = id::resolve(&document, node);
        let parent = document.store().core(index).parent()?;
        if parent == document.document_index() {
            return None;
        }
        Some(id::to_view(document.store().key_of(parent)))
    }

    /// The first child of `node`, or `None` when it has none or is gone.
    pub(crate) fn first_child_of(&self, node: NodeId) -> Option<NodeId> {
        let index = self.live_index_of(node)?;
        let document = self.document();
        let child = document.store().core(index).first_child()?;
        Some(id::to_view(document.store().key_of(child)))
    }

    /// The sibling after `node`, or `None` when it is the last child, detached or gone.
    pub(crate) fn next_sibling_of(&self, node: NodeId) -> Option<NodeId> {
        let index = self.live_index_of(node)?;
        let document = self.document();
        // The children of the document node are the window's own roots, which a view never sees
        // as siblings of each other.
        if document.store().core(index).parent()? == document.document_index() {
            return None;
        }
        let next = document.store().core(index).next_sibling()?;
        Some(id::to_view(document.store().key_of(next)))
    }

    /// Whether the live text node `node` holds exactly `data`. An absent text reads as empty.
    pub(crate) fn holds_text(&self, node: NodeId, data: &str) -> bool {
        let Some(index) = self.live_index_of(node) else {
            return false;
        };
        let document = self.document();
        zgui_dom::text::node::text_of(document.store(), index).unwrap_or_default() == data
    }

    /// Replaces the text `node` holds.
    ///
    /// A text binding is a writer like any attribute's and goes stale the same way — its effect
    /// can fire one frame after the node went — so a node that is gone is quietly left alone:
    /// there is no text left to replace, and unwinding would poison the document.
    pub(crate) fn write_text(&self, node: NodeId, data: &str) {
        let Some(node) = self.live_index_of(node) else {
            return;
        };
        self.edit(|edit| edit.set_text(node, data));
    }
}
