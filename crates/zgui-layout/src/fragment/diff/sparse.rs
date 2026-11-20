//! Entering a long list only where it owes work.
//!
//! A box whose own composition stood still hands its children what it handed them last time, so
//! every child that owes nothing takes its cached fold. With thousands of children the walk still
//! asks each of them, folds each of their inks into the box's subtree ink, and sorts every child
//! ink to decide whether any two overlap. All of that is proportional to the length of the list.
//!
//! So a box with many children keeps a [`ChildFold`]: what each child's subtree reported when the
//! box last folded them, and the aggregates over them in a form that one child's change can update
//! exactly. A later visit of the box enters only the children that may owe work, updates the
//! aggregates with what those children report, and folds the rest from the record. Where an update
//! cannot be proven exact, such as the last child at an edge of the ink union shrinking, the box
//! folds every child again from the fragments, which costs a read per child and no visit.
//!
//! # Which children are entered
//!
//! - The boxes of the children the document marks as owing work, found through the element's
//!   record of its dirty children. A child that generates no box, `display: contents`, is looked
//!   through to its own dirty children.
//! - The children whose layout result moved in the last layout pass.
//! - The children that read the box's own marks: anonymous boxes and the box's generated content.
//!
//! In a debug build, and whenever the layout invariants are checked, each sparse visit is checked
//! against the full walk: no child left out may owe work, and the aggregates must equal a fold of
//! every child.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use zgui_dom::side::BoxKey;
use zgui_geom::{Device, DevicePx, Point, Rect, Size};

use super::damage::overlaps;
use crate::fragment::FragmentFlags;
use crate::fragment::hit::HitOrder;

/// The fewest children a box needs before it keeps a [`ChildFold`].
pub(super) const SPARSE_FROM: usize = 64;

/// The height of one band of the overlap index, in device pixels.
const BAND: f32 = 64.0;

/// The most bands one child ink may span before the box stops keeping a record.
const MAX_SPAN: i64 = 64;

/// The most entries one band may hold before the box stops keeping a record.
const MAX_BAND: usize = 256;

/// Whether visits use the record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    /// Every child is asked.
    Off,
    /// Only the children that may owe work are entered, and each visit is checked against a fold
    /// of every child.
    Shadow,
    /// Only the children that may owe work are entered.
    On,
}

/// The mode, from `ZGUI_SPARSE_CHILDREN` (`off`, `shadow` or `on`), checked in debug builds and
/// whenever the layout invariants are.
pub(super) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("ZGUI_SPARSE_CHILDREN").as_deref() {
        Ok("off") => Mode::Off,
        Ok("shadow") => Mode::Shadow,
        _ if cfg!(debug_assertions) || crate::invariants::enabled() => Mode::Shadow,
        _ => Mode::On,
    })
}

/// What one child's subtree reported to the box, in the box's anchor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Entry {
    /// The union of the child's ink and everything below it.
    pub(super) ink: Rect<DevicePx, Device>,
    /// Whether nothing in the child's subtree overlaps anything else in it.
    pub(super) disjoint: bool,
    /// Whether every piece in the child's subtree moves with the box above it.
    pub(super) rigid: bool,
    /// Whether anything in the child's subtree blends with what is behind it.
    pub(super) blending: bool,
    /// Whether anything in the child's subtree reads what is behind it.
    pub(super) reads: bool,
    /// The bound flags of the child's subtree.
    pub(super) bound: FragmentFlags,
    /// The hit-order range of the child's subtree, `(MAX, 0)` when it indexes nothing.
    pub(super) order: (HitOrder, HitOrder),
}

/// One edge of the ink union, and how many children reach it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Extreme {
    /// The edge.
    at: f32,
    /// How many non-empty child inks have that edge.
    count: u32,
}

/// What a box's children reported when it last folded them, kept between visits.
#[derive(Clone, Debug)]
pub(crate) struct ChildFold {
    /// The box's subtree ink after the fold. A record whose box now holds another was made stale by
    /// a move that did not visit the box.
    total: Rect<DevicePx, Device>,
    /// The box's own ink the fold was made against.
    own: Rect<DevicePx, Device>,
    /// How many of the box's own later pieces lead [`ChildFold::entries`], such as a scroll
    /// container's scrollbar. They fold like children with nothing below them.
    pieces: u32,
    /// What moved the children's inks into the box's anchor.
    into_anchor: Option<Size<DevicePx, Device>>,
    /// The children, in layout order.
    keys: Vec<BoxKey>,
    /// Each child's position in [`ChildFold::entries`].
    positions: FxHashMap<BoxKey, u32>,
    /// The reports of the box's own later pieces, then of each child.
    entries: Vec<Entry>,
    /// The children that read the box's own marks, by position in [`ChildFold::entries`].
    readers: Vec<u32>,
    /// The left, top, right and bottom edges of the union of the non-empty child inks.
    edges: [Extreme; 4],
    /// How many child inks are non-empty.
    non_empty: u32,
    /// How many children's subtrees are not disjoint.
    not_disjoint: u32,
    /// How many children's subtrees are not rigid.
    not_rigid: u32,
    /// How many children's subtrees blend.
    blending: u32,
    /// How many children's subtrees read what is behind them.
    reads: u32,
    /// For each bit of the bound flags, how many children's subtrees carry it.
    bound: [u32; 16],
    /// How many child inks overlap the box's own ink.
    own_overlaps: u32,
    /// How many pairs of child inks overlap.
    pairs: u32,
    /// The children whose inks reach each band of [`BAND`] device pixels, by position.
    bands: FxHashMap<i64, SmallVec<[u32; 4]>>,
    /// Whether an update could not be proven exact, so the box has to fold every child again.
    lost: bool,
}

/// The bands a non-empty ink reaches, or `None` when it spans too many.
fn bands_of(ink: Rect<DevicePx, Device>) -> Option<core::ops::RangeInclusive<i64>> {
    if ink.is_empty() {
        return Some(1..=0);
    }
    let first = (ink.top().0 / BAND).floor() as i64;
    let last = (ink.bottom().0 / BAND).floor() as i64;
    (last - first < MAX_SPAN).then_some(first..=last)
}

/// The edges of `ink`: left, top, right, bottom.
fn edges_of(ink: Rect<DevicePx, Device>) -> [f32; 4] {
    [ink.left().0, ink.top().0, ink.right().0, ink.bottom().0]
}

impl ChildFold {
    /// A record of `entries`, the reports of `keys`, folded after the box's own later `pieces`
    /// against its own ink `own`. `readers` are positions among the children.
    ///
    /// `None` when the children are arranged so that the overlap index would cost more than the
    /// fold it saves.
    pub(super) fn new(
        own: Rect<DevicePx, Device>,
        pieces: &[Rect<DevicePx, Device>],
        into_anchor: Option<Size<DevicePx, Device>>,
        keys: Vec<BoxKey>,
        entries: Vec<Entry>,
        readers: Vec<u32>,
    ) -> Option<Self> {
        let lead = pieces.len() as u32;
        let mut fold = Self {
            total: Rect::ZERO,
            own,
            pieces: lead,
            into_anchor,
            positions: keys
                .iter()
                .enumerate()
                .map(|(position, key)| (*key, lead + position as u32))
                .collect(),
            keys,
            entries: Vec::with_capacity(pieces.len() + entries.len()),
            readers: readers
                .into_iter()
                .map(|position| lead + position)
                .collect(),
            edges: [Extreme { at: 0.0, count: 0 }; 4],
            non_empty: 0,
            not_disjoint: 0,
            not_rigid: 0,
            blending: 0,
            reads: 0,
            bound: [0; 16],
            own_overlaps: 0,
            pairs: 0,
            bands: FxHashMap::default(),
            lost: false,
        };
        for entry in pieces.iter().map(|ink| Entry::piece(*ink)).chain(entries) {
            let position = fold.entries.len() as u32;
            fold.entries.push(entry);
            fold.count(position, entry, 1)?;
        }
        Some(fold)
    }

    /// The children the record was made for.
    pub(super) fn keys(&self) -> &[BoxKey] {
        &self.keys
    }

    /// The child at `position` in [`ChildFold::entries`].
    pub(super) fn child(&self, position: u32) -> BoxKey {
        self.keys[(position - self.pieces) as usize]
    }

    /// The positions of the children after the one at `position`.
    pub(super) fn after(&self, position: u32) -> core::ops::Range<u32> {
        position + 1..self.entries.len() as u32
    }

    /// The positions of the children in [`ChildFold::entries`].
    pub(super) fn children(&self) -> core::ops::Range<u32> {
        self.pieces..self.entries.len() as u32
    }

    /// The reports of the box's own later pieces.
    pub(super) fn pieces(&self) -> &[Entry] {
        &self.entries[..self.pieces as usize]
    }

    /// Whether the record still describes a box that now holds `total`, owns `own` and the later
    /// `pieces`, and moves its children's inks by `into_anchor`.
    pub(super) fn stands(
        &self,
        total: Rect<DevicePx, Device>,
        own: Rect<DevicePx, Device>,
        pieces: &[Rect<DevicePx, Device>],
        into_anchor: Option<Size<DevicePx, Device>>,
        children: &[BoxKey],
    ) -> bool {
        self.total == total
            && self.own == own
            && self.into_anchor == into_anchor
            && self.pieces().len() == pieces.len()
            && self
                .pieces()
                .iter()
                .zip(pieces)
                .all(|(held, ink)| held.ink == *ink)
            && self.keys == children
    }

    /// Notes the box's subtree ink after a fold.
    pub(super) fn set_total(&mut self, total: Rect<DevicePx, Device>) {
        self.total = total;
    }

    /// The position of `child`, if it is one of the box's children.
    pub(super) fn position(&self, child: BoxKey) -> Option<u32> {
        self.positions.get(&child).copied()
    }

    /// The children that read the box's own marks.
    pub(super) fn readers(&self) -> &[u32] {
        &self.readers
    }

    /// Replaces the report of the child at `position`.
    pub(super) fn update(&mut self, position: u32, entry: Entry) {
        let old = self.entries[position as usize];
        if old == entry {
            return;
        }
        if self.count(position, old, -1).is_none() {
            self.lost = true;
            return;
        }
        self.entries[position as usize] = entry;
        if self.count(position, entry, 1).is_none() {
            self.lost = true;
        }
    }

    /// Whether every update so far was exact.
    pub(super) fn exact(&self) -> bool {
        !self.lost
    }

    /// Adds (`sign` 1) or removes (`sign` -1) the report of the child at `position` from the
    /// aggregates, or `None` when the result cannot be known without the other children.
    fn count(&mut self, position: u32, entry: Entry, sign: i32) -> Option<()> {
        let step = |held: &mut u32, on: bool| {
            if on {
                *held = held.wrapping_add_signed(sign);
            }
        };
        step(&mut self.not_disjoint, !entry.disjoint);
        step(&mut self.not_rigid, !entry.rigid);
        step(&mut self.blending, entry.blending);
        step(&mut self.reads, entry.reads);
        for (bit, held) in self.bound.iter_mut().enumerate() {
            step(held, (entry.bound.0 & (1 << bit)) != 0);
        }
        step(&mut self.own_overlaps, overlaps(self.own, entry.ink));
        if entry.ink.is_empty() {
            return Some(());
        }
        // The pairs this child forms with every other child, counted before it joins the index or
        // after it leaves it.
        let bands = bands_of(entry.ink)?;
        if sign < 0 {
            for band in bands.clone() {
                if let Some(list) = self.bands.get_mut(&band) {
                    list.retain(|held| *held != position);
                }
            }
        }
        let mut seen: SmallVec<[u32; 8]> = SmallVec::new();
        let mut pairs = 0u32;
        for band in bands.clone() {
            for other in self.bands.get(&band).map_or(&[][..], |list| &list[..]) {
                if *other != position
                    && !seen.contains(other)
                    && overlaps(entry.ink, self.entries[*other as usize].ink)
                {
                    seen.push(*other);
                    pairs += 1;
                }
            }
        }
        self.pairs = if sign > 0 {
            self.pairs + pairs
        } else {
            self.pairs.checked_sub(pairs)?
        };
        if sign > 0 {
            for band in bands {
                let list = self.bands.entry(band).or_default();
                if list.len() >= MAX_BAND {
                    return None;
                }
                list.push(position);
            }
        }
        self.count_edges(entry.ink, sign)
    }

    /// Adds or removes one non-empty ink from the union's edges.
    fn count_edges(&mut self, ink: Rect<DevicePx, Device>, sign: i32) -> Option<()> {
        let values = edges_of(ink);
        if sign > 0 {
            for (side, value) in values.into_iter().enumerate() {
                let edge = &mut self.edges[side];
                // The left and top edges are the least, the right and bottom ones the greatest.
                let beyond = if side < 2 {
                    value < edge.at
                } else {
                    value > edge.at
                };
                if self.non_empty == 0 || beyond {
                    *edge = Extreme {
                        at: value,
                        count: 1,
                    };
                } else if value == edge.at {
                    edge.count += 1;
                }
            }
            self.non_empty += 1;
        } else {
            for (side, value) in values.into_iter().enumerate() {
                let edge = &mut self.edges[side];
                if value == edge.at {
                    edge.count = edge.count.checked_sub(1)?;
                    // The last child on an edge left it: where the edge now is depends on every
                    // other child.
                    if edge.count == 0 && self.non_empty > 1 {
                        return None;
                    }
                }
            }
            self.non_empty -= 1;
        }
        Some(())
    }

    /// What the children fold to: the union of their inks with `own`, and the rest of the
    /// aggregates, as the full walk computes them. `None` when every ink is empty, where the full
    /// walk's answer depends on the order of the empty ones.
    pub(super) fn folded(&self) -> Option<Aggregate> {
        let mut bound = FragmentFlags::EMPTY;
        for (bit, held) in self.bound.iter().enumerate() {
            if *held > 0 {
                bound = bound.union(FragmentFlags(1 << bit));
            }
        }
        let children = (self.non_empty > 0).then(|| {
            Rect::from_corners(
                Point::new(DevicePx(self.edges[0].at), DevicePx(self.edges[1].at)),
                Point::new(DevicePx(self.edges[2].at), DevicePx(self.edges[3].at)),
            )
        });
        let subtree_ink = match children {
            Some(children) => self.own.union(children),
            None if !self.own.is_empty() => self.own,
            None => return None,
        };
        Some(Aggregate {
            subtree_ink,
            blending: self.blending > 0,
            reads: self.reads > 0,
            disjoint: self.not_disjoint == 0 && self.own_overlaps == 0 && self.pairs == 0,
            rigid: self.not_rigid == 0,
            bound,
            // Hit orders are renumbered when the index is rebuilt, so the record does not keep them.
            // The visit reads the current range off the first and last children.
            order: None,
        })
    }
}

/// What a box's children fold to, as the full walk folds them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Aggregate {
    /// The union of the box's own ink and every child's subtree ink.
    pub(super) subtree_ink: Rect<DevicePx, Device>,
    /// Whether any child's subtree blends.
    pub(super) blending: bool,
    /// Whether any child's subtree reads what is behind it.
    pub(super) reads: bool,
    /// Whether every child's subtree is disjoint, no child ink overlaps the box's own ink, and no
    /// two child inks overlap.
    pub(super) disjoint: bool,
    /// Whether every child's subtree is rigid.
    pub(super) rigid: bool,
    /// The union of the children's bound flags.
    pub(super) bound: FragmentFlags,
    /// The hit-order range over the children that index anything.
    pub(super) order: Option<(HitOrder, HitOrder)>,
}

/// The children of one box a sparse visit enters, by position, and the record it updates.
pub(super) struct Plan {
    /// The record, taken out of the store for the visit.
    pub(super) fold: ChildFold,
    /// The positions of the children to enter, ascending.
    pub(super) positions: SmallVec<[u32; 8]>,
}

impl Entry {
    /// One of the box's own later pieces, which folds its ink and nothing else.
    pub(super) fn piece(ink: Rect<DevicePx, Device>) -> Self {
        Self {
            ink,
            disjoint: true,
            rigid: true,
            blending: false,
            reads: false,
            bound: FragmentFlags::EMPTY,
            order: (HitOrder::MAX, 0),
        }
    }

    /// What `folded` reported, its ink already moved into the box's anchor.
    pub(super) fn of(folded: &super::Folded) -> Self {
        Self {
            ink: folded.subtree_ink,
            disjoint: folded.disjoint,
            rigid: folded.rigid,
            blending: folded.blending,
            reads: folded.reads,
            bound: folded.bound,
            order: folded.order,
        }
    }
}

/// What `entries` fold to under a box whose own ink is `own`, folded in order as the full walk
/// folds them.
pub(super) fn fold_entries(own: Rect<DevicePx, Device>, entries: &[Entry]) -> Aggregate {
    let mut aggregate = Aggregate {
        subtree_ink: own,
        blending: false,
        reads: false,
        disjoint: true,
        rigid: true,
        bound: FragmentFlags::EMPTY,
        order: None,
    };
    for entry in entries {
        aggregate.blending |= entry.blending;
        aggregate.reads |= entry.reads;
        aggregate.disjoint &= entry.disjoint;
        aggregate.rigid &= entry.rigid;
        aggregate.bound = aggregate.bound.union(entry.bound);
        aggregate.subtree_ink = aggregate.subtree_ink.union(entry.ink);
        if entry.order.1 > 0 {
            aggregate.order = Some(match aggregate.order {
                None => entry.order,
                Some(held) => (held.0.min(entry.order.0), held.1.max(entry.order.1)),
            });
        }
    }
    let inks: Vec<Rect<DevicePx, Device>> = entries.iter().map(|entry| entry.ink).collect();
    aggregate.disjoint &= !inks.iter().any(|child| overlaps(own, *child));
    aggregate.disjoint &= super::damage::pairwise_disjoint(&inks);
    aggregate
}

/// What a sparse visit of a box's children produced.
pub(super) struct Sparse {
    /// The children's aggregates, the box's own ink included.
    pub(super) aggregate: Aggregate,
    /// Whether any child's folded ink, anchor or set of pieces changed.
    pub(super) moved: bool,
}

impl<D: super::FrameDirty> super::Pass<'_, '_, D> {
    /// The children of `key` a visit has to enter, with the record it updates, or `None` when the
    /// box has to ask every child.
    ///
    /// `generator` is the box's element, `total` the subtree ink its first piece held before this
    /// visit, `own` its own ink after it, and `into_anchor` what moves its children's inks into its
    /// anchor.
    pub(super) fn sparse_plan(
        &mut self,
        key: BoxKey,
        generator: Option<zgui_dom::NodeKey>,
        total: Rect<DevicePx, Device>,
        own: Rect<DevicePx, Device>,
        pieces: &[Rect<DevicePx, Device>],
        into_anchor: Option<Size<DevicePx, Device>>,
    ) -> Option<Plan> {
        if mode() == Mode::Off {
            return None;
        }
        let generator = generator?;
        if self.store.get(key)?.children.len() < SPARSE_FROM
            || !crate::fragment::stacking::children_in_layout_order(self.store, key)
        {
            return None;
        }
        use zgui_profile::{Counter, counter};
        let Some(fold) = self.store.child_folds.remove(&key) else {
            counter::bump(Counter::SparseNoRecord);
            return None;
        };
        if !fold.stands(
            total,
            own,
            pieces,
            into_anchor,
            &self.store.get(key)?.children,
        ) {
            counter::bump(Counter::SparseStale);
            return None;
        }
        let Some(positions) = self.owing(key, generator, &fold) else {
            counter::bump(Counter::SparseUnnamed);
            return None;
        };
        Some(Plan { fold, positions })
    }

    /// The positions in `fold` of the children of `key` that may owe work, ascending, or `None` when
    /// the document cannot name them. See the module documentation for which they are.
    pub(super) fn owing(
        &self,
        key: BoxKey,
        generator: zgui_dom::NodeKey,
        fold: &ChildFold,
    ) -> Option<SmallVec<[u32; 8]>> {
        let mut positions: SmallVec<[u32; 8]> = fold.readers().iter().copied().collect();
        let mut elements = self.dirty.dirty_children(generator)?;
        let mut looked = 0usize;
        while let Some(element) = elements.pop() {
            looked += 1;
            if looked > SPARSE_FROM * 16 {
                return None;
            }
            let boxes = self.store.boxes_of(element);
            if boxes.is_empty() {
                // An element that generates no box hands its children to the box above it.
                elements.extend(self.dirty.dirty_children(element)?);
                continue;
            }
            for &first in boxes {
                let mut at = first;
                while let Some(parent) = self.store.get(at).and_then(|node| node.parent) {
                    if parent == key {
                        positions.extend(fold.position(at));
                        break;
                    }
                    at = parent;
                }
            }
        }
        if let Some(moved) = self.layout_moved.get(&key) {
            positions.extend(moved.iter().filter_map(|child| fold.position(*child)));
        }
        positions.sort_unstable();
        positions.dedup();
        Some(positions)
    }

    /// The children of `key` that may owe work, in painting order, when the box keeps a record
    /// that still names its children; `None` when every child has to be asked.
    pub(super) fn owing_children(
        &self,
        key: BoxKey,
        generator: Option<zgui_dom::NodeKey>,
    ) -> Option<SmallVec<[BoxKey; 8]>> {
        if mode() == Mode::Off {
            return None;
        }
        let node = self.store.get(key)?;
        if node.children.len() < SPARSE_FROM
            || !crate::fragment::stacking::children_in_layout_order(self.store, key)
        {
            return None;
        }
        let fold = self.store.child_folds.get(&key)?;
        if fold.keys() != node.children.as_slice() {
            return None;
        }
        let positions = self.owing(key, generator?, fold)?;
        Some(positions.into_iter().map(|at| fold.child(at)).collect())
    }

    /// Enters the children `plan` names, as the full walk enters a box's children when the box
    /// stood still, and folds the rest from the record.
    ///
    /// `own_cursor` is where a child born before every sibling goes in painting order, and `after`
    /// the first order key painted after the box.
    pub(super) fn visit_sparse(
        &mut self,
        key: BoxKey,
        plan: Plan,
        placed: &crate::fragment::build::Placed,
        first_fragment: Option<crate::fragment::FragKey>,
        generator: Option<zgui_dom::NodeKey>,
        (own_cursor, after): (HitOrder, HitOrder),
        previous_shift: (f32, f32),
        own: Rect<DevicePx, Device>,
    ) -> Sparse {
        let Plan {
            mut fold,
            positions,
        } = plan;
        let into_anchor = fold.into_anchor;
        let mut moved = false;
        let mut entered: SmallVec<[(u32, Entry); 8]> = SmallVec::new();
        for position in positions {
            let child = fold.child(position);
            zgui_profile::counter::bump(zgui_profile::Counter::FragmentChildrenAsked);
            let clean = self.can_skip(child, generator);
            let before = (!clean).then(|| self.folded_reading(child));
            let cursor = (fold.pieces().len() as u32..position)
                .rev()
                .map(|before| self.current_order(fold.child(before)).1)
                .find(|order| *order > 0)
                .map_or(own_cursor, |order| order.max(own_cursor));
            let next = fold
                .after(position)
                .find_map(|later| self.indexed_first(fold.child(later)))
                .unwrap_or(after);
            let mut folded = if clean {
                self.cached(child)
            } else {
                match self.layout_shift(child, placed, previous_shift, generator) {
                    Some((shift, snapped)) => {
                        let state = self.store.state_mut(child);
                        state.composed = state.unrounded;
                        state.snapped = snapped;
                        if shift.by == (0.0, 0.0) && shift.scrolled == (0.0, 0.0) {
                            self.cached(child)
                        } else {
                            self.translate(child, shift)
                        }
                    }
                    None => self.visit(
                        child,
                        placed.descent,
                        first_fragment,
                        generator,
                        (cursor, next),
                    ),
                }
            };
            if let Some(before) = before {
                moved |= self.folded_reading(child) != before;
            }
            match folded.anchor {
                Some(anchor) if anchor != placed.descent.anchor => {
                    folded.subtree_ink = self.reanchor(folded.subtree_ink, anchor, placed.anchor);
                }
                _ => {
                    if let Some(by) = into_anchor {
                        folded.subtree_ink = folded.subtree_ink.translate(by);
                    }
                }
            }
            let entry = Entry::of(&folded);
            fold.update(position, entry);
            entered.push((position, entry));
        }
        let kept = if fold.exact() { fold.folded() } else { None };
        if kept.is_none() {
            zgui_profile::counter::bump(zgui_profile::Counter::SparseRefolds);
        }
        // Orders grow in painting order, which is layout order here, so the children's range runs
        // from the first child that indexes anything to the last.
        let children = fold.children();
        let least = children
            .clone()
            .map(|at| self.current_order(fold.child(at)))
            .find(|order| order.1 > 0);
        let most = children
            .rev()
            .map(|at| self.current_order(fold.child(at)))
            .find(|order| order.1 > 0);
        let kept = kept.map(|aggregate| Aggregate {
            order: least.zip(most).map(|(least, most)| (least.0, most.1)),
            ..aggregate
        });
        let check = mode() == Mode::Shadow;
        let (aggregate, fold) = match kept {
            Some(aggregate) if !check => (aggregate, Some(fold)),
            kept => {
                let entries = self.entries_of(&fold, &entered, placed, into_anchor);
                let full = fold_entries(own, &entries);
                if let Some(aggregate) = kept {
                    assert_eq!(
                        aggregate, full,
                        "a sparse visit of {key:?} folded its children to another answer"
                    );
                    for (index, child) in fold.keys().iter().enumerate() {
                        let position = fold.pieces().len() + index;
                        assert!(
                            entered.iter().any(|(at, _)| *at as usize == position)
                                || self.can_skip(*child, generator),
                            "a sparse visit of {key:?} left out {child:?}, which owes work"
                        );
                    }
                    (aggregate, Some(fold))
                } else {
                    let lead = fold.pieces().len();
                    let pieces: Vec<_> = fold.pieces().iter().map(|piece| piece.ink).collect();
                    let keys = fold.keys().to_vec();
                    let readers = fold.readers().iter().map(|at| at - lead as u32).collect();
                    let children = entries[lead..].to_vec();
                    (
                        full,
                        ChildFold::new(own, &pieces, into_anchor, keys, children, readers),
                    )
                }
            }
        };
        if let Some(mut fold) = fold {
            fold.set_total(aggregate.subtree_ink);
            self.store.child_folds.insert(key, fold);
        }
        Sparse { aggregate, moved }
    }

    /// The hit-order range `child`'s subtree spans now, `(MAX, 0)` when it indexes nothing, as
    /// [`super::Pass::cached`] reads it.
    fn current_order(&self, child: BoxKey) -> (HitOrder, HitOrder) {
        let resolved = self
            .store
            .fragments_of_box(child)
            .iter()
            .any(|frag| self.store.fragment(*frag).is_some());
        if resolved {
            self.store.subtree_order(child)
        } else {
            (HitOrder::MAX, 0)
        }
    }

    /// Every child's report: what the visit entered and, for the rest, what their pieces hold.
    fn entries_of(
        &self,
        fold: &ChildFold,
        entered: &[(u32, Entry)],
        placed: &crate::fragment::build::Placed,
        into_anchor: Option<Size<DevicePx, Device>>,
    ) -> Vec<Entry> {
        let lead = fold.pieces().len();
        let children = fold.keys().iter().enumerate().map(|(index, child)| {
            let position = lead + index;
            if let Some((_, entry)) = entered.iter().find(|(at, _)| *at as usize == position) {
                return *entry;
            }
            let mut folded = self.cached(*child);
            match folded.anchor {
                Some(anchor) if anchor != placed.descent.anchor => {
                    folded.subtree_ink = self.reanchor(folded.subtree_ink, anchor, placed.anchor);
                }
                _ => {
                    if let Some(by) = into_anchor {
                        folded.subtree_ink = folded.subtree_ink.translate(by);
                    }
                }
            }
            Entry::of(&folded)
        });
        fold.pieces().iter().copied().chain(children).collect()
    }

    /// The positions of `key`'s children that read the box's own marks: anonymous boxes, the
    /// box's generated content, and out-of-flow boxes placed under it from deeper in the document.
    pub(super) fn readers_of(&self, key: BoxKey, generator: Option<zgui_dom::NodeKey>) -> Vec<u32> {
        let Some(node) = self.store.get(key) else {
            return Vec::new();
        };
        node.children
            .iter()
            .enumerate()
            .filter(|(_, child)| {
                let source = self.store.get(**child).and_then(|child| child.source);
                source.is_none() || source == generator || self.store.declared(**child).is_some()
            })
            .map(|(position, _)| position as u32)
            .collect()
    }
}
