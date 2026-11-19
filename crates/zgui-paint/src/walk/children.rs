//! Which children of a box the damage can reach, without asking each of them.
//!
//! The emit walk asks every child of an entered box whether its subtree ink reaches the damage.
//! For a box with a long run of children — the rows of a list — that question is asked of every
//! row on every frame, though a damaged label reaches one or two of them. When the children are
//! laid out one below another, their tops rise with their order, so the ones a damaged band can
//! reach form one contiguous run that two binary searches find.
//!
//! [`ChildIndex`] records, per child, the top of its subtree ink and the lowest bottom any child
//! up to it reaches, in the coordinate system the children's ink is measured in. It is built once
//! and kept until the layout says a child's ink moved
//! ([`LayoutStore::child_ink_revision`]), or the number of children changes.
//!
//! The index narrows; the walk still asks each child it lets through. A box whose children do not
//! satisfy the index's conditions — a child drawn in its own coordinate system, children out of
//! vertical order, a child with no ink — gets no index and is walked as before.

use rustc_hash::FxHashMap;
use zgui_bits::DamageSet;
use zgui_dom::side::BoxKey;
use zgui_geom::{Device, DevicePx, Point, Rect, Size};
use zgui_layout::LayoutStore;
use zgui_scene::{Placements, SpatialId};

/// How many children a box needs before an index is worth building.
pub(crate) const INDEXED_FROM: usize = 32;

/// Where each child of one box paints, ordered for a range search.
#[derive(Debug)]
pub(crate) struct ChildIndex {
    /// The coordinate system every child's ink is measured in.
    anchor: SpatialId,
    /// Each child's ink top, rising with the child's position.
    tops: Vec<f32>,
    /// For each position, the lowest ink bottom of any child up to and including it.
    reach: Vec<f32>,
}

/// The indexes of the boxes with enough children, kept between frames.
#[derive(Debug, Default)]
pub(crate) struct ChildIndexes {
    held: FxHashMap<BoxKey, Entry>,
}

/// What was built for one box: its index, or nothing when its children refused one.
#[derive(Debug)]
struct Entry {
    revision: u64,
    len: usize,
    index: Option<ChildIndex>,
}

impl ChildIndexes {
    /// The range of `parent`'s children the damage can reach, or `None` when it cannot be
    /// narrowed: too few children, children the index refused, or damage it cannot be asked
    /// about.
    pub(crate) fn candidates(
        &mut self,
        store: &LayoutStore,
        parent: BoxKey,
        children: &[BoxKey],
        damage: &DamageSet,
        surface: Option<Rect<i32, Device>>,
        placements: Option<&Placements>,
    ) -> Option<core::ops::Range<usize>> {
        if children.len() < INDEXED_FROM {
            return None;
        }
        let revision = store.child_ink_revision(parent);
        let stale = self
            .held
            .get(&parent)
            .is_none_or(|entry| entry.revision != revision || entry.len != children.len());
        if stale {
            self.held.insert(
                parent,
                Entry {
                    revision,
                    len: children.len(),
                    index: ChildIndex::build(store, children),
                },
            );
        }
        self.held
            .get(&parent)?
            .index
            .as_ref()?
            .range(damage, surface, placements)
    }

    /// Forgets the indexes of boxes that no longer exist.
    pub(crate) fn retain_live(&mut self, store: &LayoutStore) {
        self.held.retain(|key, _| store.get(*key).is_some());
    }
}

impl ChildIndex {
    /// Builds the index of `children`, or `None` when they do not satisfy its conditions.
    fn build(store: &LayoutStore, children: &[BoxKey]) -> Option<Self> {
        let mut anchor = None;
        let mut tops = Vec::with_capacity(children.len());
        let mut reach = Vec::with_capacity(children.len());
        let mut lowest = f32::NEG_INFINITY;
        let mut previous_top = f32::NEG_INFINITY;
        for child in children {
            let frags = store.fragments_of_box(*child);
            let mut ink: Option<Rect<DevicePx, Device>> = None;
            for frag in frags {
                let fragment = store.fragment(*frag)?;
                // The two ways a piece is put on the device by its anchor's matrix alone: the
                // viewport with no transform of its own, or a content space it is drawn in.
                let plain = if fragment.anchor == SpatialId::VIEWPORT {
                    fragment.transform.is_none()
                } else {
                    fragment.transform == Some(fragment.anchor)
                };
                if !plain || *anchor.get_or_insert(fragment.anchor) != fragment.anchor {
                    return None;
                }
                ink = Some(match ink {
                    Some(held) => held.union(fragment.subtree_ink),
                    None => fragment.subtree_ink,
                });
            }
            // A child with no piece, or with empty ink, is reached by rules the index does not
            // model; such a box is walked child by child.
            let ink = ink.filter(|ink| !ink.is_empty())?;
            let top = ink.top().0;
            if top < previous_top {
                return None;
            }
            previous_top = top;
            lowest = lowest.max(ink.bottom().0);
            tops.push(top);
            reach.push(lowest);
        }
        Some(Self {
            anchor: anchor?,
            tops,
            reach,
        })
    }

    /// The children a damaged region can reach, as a range of positions.
    fn range(
        &self,
        damage: &DamageSet,
        surface: Option<Rect<i32, Device>>,
        placements: Option<&Placements>,
    ) -> Option<core::ops::Range<usize>> {
        // How far the anchor puts the children's ink from where it is measured. Only a pure
        // translation can be undone on a damage rectangle; anything else is walked in full.
        let offset = match placements {
            Some(placements) if self.anchor != SpatialId::VIEWPORT => {
                let probe = Rect::new(
                    Point::new(DevicePx(0.0), DevicePx(0.0)),
                    Size::new(DevicePx(1.0), DevicePx(1.0)),
                );
                let placed = placements.place(self.anchor, probe);
                if (placed.size.width.0 - 1.0).abs() > 1e-3
                    || (placed.size.height.0 - 1.0).abs() > 1e-3
                {
                    return None;
                }
                placed.origin.y.0
            }
            _ => 0.0,
        };
        let mut bands: smallvec::SmallVec<[(f32, f32); 8]> = smallvec::SmallVec::new();
        if damage.is_full() {
            let surface = surface?;
            bands.push((
                surface.origin.y as f32,
                (surface.origin.y + surface.size.height) as f32,
            ));
        } else {
            for rect in damage.rects() {
                bands.push((
                    rect.origin.y as f32,
                    (rect.origin.y + rect.size.height) as f32,
                ));
            }
        }
        let mut lo = self.tops.len();
        let mut hi = 0;
        for (top, bottom) in bands {
            // Measured where the children's ink is, and widened by a pixel on each side for the
            // rounding of ink to whole pixels the walk's own test does.
            let top = top - offset - 1.0;
            let bottom = bottom - offset + 1.0;
            let first = self.reach.partition_point(|reach| *reach < top);
            let end = self.tops.partition_point(|t| *t <= bottom);
            if first < end {
                lo = lo.min(first);
                hi = hi.max(end);
            }
        }
        Some(if lo < hi { lo..hi } else { 0..0 })
    }
}
