//! How much of the style engine's work a change actually needs redone.
//!
//! The engine takes a per-element *hint* saying how far to go: re-run selector matching for this
//! element, re-run it for this element and everything below it, or skip matching entirely and only
//! re-run the cascade. The difference is not marginal. A theme switch that changes one custom
//! property on the root element needs no selector matching anywhere in the document, and a `style`
//! attribute rewrite needs none on the element it changed; both arrive as a cascade-only hint and
//! cost a fraction of what a match would.
//!
//! A hint that is too wide is slow and a hint that is too narrow is wrong — the element keeps the
//! style it had, with nothing to report it — so every entry here is the narrowest hint that is
//! *provably* enough, and anything not proved takes the wide one.
//!
//! # Why the hints are logged rather than written where they are decided
//!
//! Hints accumulate by union, so writing one straight onto the element would be correct. They are
//! logged and applied together at the close of the batch for a different reason: the expansion of a
//! child-list change decides at that moment which siblings need restyling, and it must record its
//! hints through the same path as everything else. Two paths writing the same field is how one of
//! them ends up forgetting to mark the ancestors that lead to it.

use style::invalidation::element::restyle_hints::RestyleHint;
use zgui_bits::Dirty;

use crate::arena::store::DocumentStore;
use crate::dirty::propagate;
use crate::id::node_key::NodeIndex;

/// How many entries a log scans before it indexes them.
///
/// Most batches touch a handful of elements, where a scan beats a hash. A batch that builds a
/// list touches one element per row, where a scan per row costs the square of the list.
const SCAN: usize = 16;

/// The hints one batch decided on, waiting to be applied.
#[derive(Default)]
pub struct HintLog {
    /// One entry per element, unioned as more changes land on it.
    entries: Vec<(NodeIndex, RestyleHint)>,
    /// Each entry's position in [`HintLog::entries`], once there are more than [`SCAN`] of them.
    index: rustc_hash::FxHashMap<NodeIndex, u32>,
}

impl HintLog {
    /// A log with nothing in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many elements have a hint.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The hint recorded for `node`, if any.
    pub fn get(&self, node: NodeIndex) -> Option<RestyleHint> {
        self.position(node).map(|at| self.entries[at].1)
    }

    /// Where `node`'s entry is.
    fn position(&self, node: NodeIndex) -> Option<usize> {
        if self.entries.len() > SCAN {
            self.index.get(&node).map(|at| *at as usize)
        } else {
            self.entries.iter().position(|(index, _)| *index == node)
        }
    }

    /// Records `hint` for `node`, and that `node` and its ancestors owe restyle work.
    ///
    /// The mark is not separable from the hint: the engine's traversal descends only where
    /// something says there is work below, so a hint recorded on an element nothing leads to is
    /// never read.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of `store`.
    pub(crate) fn record(&mut self, store: &mut DocumentStore, node: NodeIndex, hint: RestyleHint) {
        match self.position(node) {
            Some(at) => self.entries[at].1.insert(hint),
            None => {
                self.entries.push((node, hint));
                if self.entries.len() == SCAN + 1 {
                    self.index = self
                        .entries
                        .iter()
                        .enumerate()
                        .map(|(at, (index, _))| (*index, at as u32))
                        .collect();
                } else if self.entries.len() > SCAN + 1 {
                    self.index.insert(node, self.entries.len() as u32 - 1);
                }
            }
        }
        propagate::mark(store, node, Dirty::RESTYLE);
    }

    /// Records that `node` alone must be matched against the rule set again.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of `store`.
    pub(crate) fn restyle_self(&mut self, store: &mut DocumentStore, node: NodeIndex) {
        self.record(store, node, RestyleHint::RESTYLE_SELF);
    }

    /// Hands every recorded hint to the elements it was recorded for, and empties the log.
    ///
    /// An element the batch has since removed is skipped: it has no style data to hand a hint to
    /// and nothing will traverse it again.
    pub(crate) fn apply(&mut self, store: &DocumentStore) {
        self.index.clear();
        for (node, hint) in self.entries.drain(..) {
            let Some(record) = store.try_core(node) else {
                continue;
            };
            let handle = crate::node::handle::Node::new(record);
            // An element with no data yet is styled from scratch by the next traversal, which is
            // strictly more than any hint asks for, so there is nothing to record on it.
            if let Some(mut data) = handle.mutate_style_data() {
                data.hint.insert(hint);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use style::invalidation::element::restyle_hints::RestyleHint;
    use zgui_bits::Dirty;
    use zgui_interned::ElementName;

    use super::HintLog;
    use crate::arena::document::Document;
    use crate::node::kind::NodeKind;

    #[test]
    fn two_changes_to_one_element_union_into_one_hint() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        let mut hints = HintLog::new();
        hints.record(document.store_mut(), root, RestyleHint::RESTYLE_SELF);
        hints.record(document.store_mut(), root, RestyleHint::RECASCADE_SELF);

        assert_eq!(hints.len(), 1);
        let held = hints.get(root).expect("a hint was recorded");
        assert!(held.contains(RestyleHint::RESTYLE_SELF));
        assert!(held.contains(RestyleHint::RECASCADE_SELF));
    }

    #[test]
    fn a_batch_that_builds_a_list_keeps_one_hint_per_row() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        let rows: Vec<_> = (0..200)
            .map(|_| document.append(root, NodeKind::Element, ElementName::new("row")))
            .collect();
        let mut hints = HintLog::new();
        for row in &rows {
            hints.record(document.store_mut(), *row, RestyleHint::RESTYLE_SELF);
        }
        for row in &rows {
            hints.record(document.store_mut(), *row, RestyleHint::RECASCADE_SELF);
        }

        assert_eq!(hints.len(), rows.len());
        for row in &rows {
            let held = hints.get(*row).expect("a hint was recorded");
            assert!(held.contains(RestyleHint::RESTYLE_SELF | RestyleHint::RECASCADE_SELF));
        }
        hints.apply(document.store());
        assert!(hints.is_empty());
        assert!(hints.get(rows[0]).is_none());
    }

    #[test]
    fn recording_a_hint_also_tells_the_ancestors_there_is_work_below() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        let child = document.append(root, NodeKind::Element, ElementName::new("item"));
        let mut hints = HintLog::new();
        hints.restyle_self(document.store_mut(), child);

        assert!(
            document
                .store()
                .core(root)
                .dirty()
                .subtree()
                .contains(Dirty::RESTYLE),
            "a hint on an element nothing descends to is never read"
        );
    }

    #[test]
    fn applying_a_hint_to_a_styled_element_reaches_its_style_data() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        document.node(root).establish_style_data();

        let mut hints = HintLog::new();
        hints.record(document.store_mut(), root, RestyleHint::RESTYLE_SELF);
        hints.apply(document.store());

        assert!(hints.is_empty());
        let data = document
            .node(root)
            .borrow_style_data()
            .expect("the element has data");
        assert!(data.hint.contains(RestyleHint::RESTYLE_SELF));
    }

    #[test]
    fn applying_a_hint_to_an_element_with_no_data_yet_is_not_an_error() {
        let mut document = Document::new();
        let root = document.append(
            document.document_index(),
            NodeKind::Element,
            ElementName::new("root"),
        );
        let mut hints = HintLog::new();
        hints.record(document.store_mut(), root, RestyleHint::RESTYLE_SELF);
        hints.apply(document.store());
        assert!(document.node(root).borrow_style_data().is_none());
    }
}
