//! Selector matching results kept by matching key, so that an element that matches like an
//! earlier one takes the earlier one's rules without matching.
//!
//! The engine's sharing cache already skips matching for an element whose sibling or cousin it can
//! prove equal, but it holds a few elements of one tree level and compares candidates one by one.
//! A list of rows created from one template matches the same rules thousands of times. This table
//! holds one match per key for as long as the rule set stays the same, and finds it by hashing.
//!
//! # The key
//!
//! An element's matched rules depend on what the selectors that can reach it read. The key holds:
//!
//! - the element's local name, namespace, identifier, classes, state and custom states, which also
//!   decide the rule hash buckets its candidates come from;
//! - the same of every ancestor, which descendant and child combinators and inherited state read;
//! - the element's results for the rule set's revalidation selectors — every selector that reads an
//!   attribute, a position among siblings, emptiness, a sibling, `:has()` or a non-state
//!   pseudo-class, wherever in the selector it does so.
//!
//! The style attribute is not in the key. A kept match holds the rules without it, and a hit puts
//! the element's own style attribute in, as the engine does when only a style attribute changed.
//!
//! # What is never kept
//!
//! A match that read something the key does not hold: one that asked a container query, received
//! presentational hints, wrote a selector flag outside the element's ancestor chain, or produced
//! cascade input flags (container queries and `@scope`), for the element or one of its
//! pseudo-elements. Also a link and an element under a link, and an element with animations.
//!
//! The engine builds pseudo-element cascade inputs only from the styles an element holds, so a kept
//! match holds the pseudo-element styles its cascade produced, and a hit cascades its
//! pseudo-elements from their rule chains.
//!
//! # Selector flags
//!
//! Matching writes selector flags on the element and its ancestors, which later mutations read to
//! decide what to restyle. A kept match holds the flags its matching wrote, by distance up the
//! ancestor chain, and a hit writes them on the hit element and its ancestors.

use std::sync::{Mutex, OnceLock};

use rustc_hash::FxHashMap;
use selectors::matching::NeedsSelectorFlags;
use servo_arc::{Arc as ServoArc, ArcBorrow};
use smallvec::SmallVec;
use style::context::{CascadeInputs, ElementCascadeInputs, SharedStyleContext, StyleContext};
use style::data::{EagerPseudoStyles, ElementData};
use style::dom::TElement;
use style::properties::{ComputedValues, PropertyDeclarationBlock};
use style::rule_tree::{CascadeLevel, StrongRuleNode};
use style::selector_parser::PseudoElement;
use style::shared_lock::Locked;
use style::style_resolver::{PrimaryStyle, ResolvedElementStyles, ResolvedStyle};
use style::stylesheets::layer_rule::LayerOrder;
use style::values::AtomIdent;
use style::{Atom, LocalName, Namespace};
use zgui_css::MAX_STYLE_THREADS;
use zgui_dom::Node;
use zgui_dom::stylo::recorder::Recorded;

/// How many ancestors a key holds. An element deeper than this is not looked up.
const DEPTH: usize = 24;

/// How many matches one worker keeps before it forgets them all.
const KEPT: usize = 4096;

/// Whether a traversal uses the table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    /// Every element matches.
    Off,
    /// Every element matches, and the rules the table would have given must equal the match's.
    Shadow,
    /// A hit skips matching.
    On,
}

/// The mode, from `ZGUI_MATCH_CACHE` (`off`, `shadow` or `on`).
pub(super) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("ZGUI_MATCH_CACHE").as_deref() {
        Ok("off") => Mode::Off,
        Ok("shadow") => Mode::Shadow,
        _ => Mode::On,
    })
}

/// What selectors read of one element, other than its attributes and position.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Features {
    /// The local name.
    name: LocalName,
    /// The namespace.
    namespace: Namespace,
    /// The identifier.
    id: Option<Atom>,
    /// The classes, in document order.
    classes: SmallVec<[AtomIdent; 2]>,
    /// The element state bits.
    state: u64,
    /// The custom states.
    custom: SmallVec<[AtomIdent; 1]>,
}

impl Features {
    /// The features of `element`.
    fn of(element: Node<'_>) -> Self {
        let mut classes = SmallVec::new();
        element.each_class(|class| classes.push(class.clone()));
        let mut custom = SmallVec::new();
        element.each_custom_state(|state| custom.push(state.clone()));
        Self {
            name: style::values::GenericAtomIdent(element.local_name().clone()),
            namespace: style::values::GenericAtomIdent(element.namespace().clone()),
            id: element.id().cloned(),
            classes,
            state: element.state().bits(),
            custom,
        }
    }
}

/// The key a match is kept under.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct Key {
    /// The element, then its ancestors, nearest first.
    chain: SmallVec<[Features; 8]>,
    /// The element's revalidation selector results, packed.
    revalidation: SmallVec<[u64; 2]>,
    /// How many revalidation selectors there were.
    revalidated: usize,
}

impl Key {
    /// The key of `element`, or `None` when the table cannot answer for it.
    ///
    /// Runs the rule set's revalidation selectors against the element.
    pub(super) fn of<'doc>(
        element: Node<'doc>,
        context: &mut StyleContext<Node<'doc>>,
    ) -> Option<Self> {
        if context.shared.traversal_flags.for_animation_only()
            || element.implemented_pseudo_element().is_some()
            || !element.matches_user_and_content_rules()
            || element.has_animations(context.shared)
        {
            return None;
        }
        element.traversal_parent()?;
        let mut chain = SmallVec::new();
        let mut at = Some(element);
        while let Some(node) = at {
            if chain.len() == DEPTH || node.matches_link() {
                return None;
            }
            chain.push(Features::of(node));
            at = node.traversal_parent();
        }
        let stylist = &context.shared.stylist;
        let thread = &mut context.thread_local;
        let result =
            zgui_dom::stylo::recorder::in_phase(zgui_dom::stylo::recorder::Phase::Keying, || {
                stylist.match_revalidation_selectors(
                    element,
                    Some(thread.bloom_filter.filter()),
                    &mut thread.selector_caches,
                    NeedsSelectorFlags::Yes,
                )
            });
        let mut revalidation = SmallVec::new();
        let mut revalidated = 0;
        for matched in result.selectors_matched.iter() {
            if revalidated % 64 == 0 {
                revalidation.push(0u64);
            }
            if matched {
                *revalidation.last_mut().expect("pushed above") |= 1 << (revalidated % 64);
            }
            revalidated += 1;
        }
        Some(Self {
            chain,
            revalidation,
            revalidated,
        })
    }
}

/// The last style a kept match was cascaded to, with what that cascade read besides the rules.
#[derive(Clone)]
struct Last {
    /// The parent style it was cascaded under.
    parent: ServoArc<ComputedValues>,
    /// The style attribute it was cascaded with.
    attribute: Option<ServoArc<Locked<PropertyDeclarationBlock>>>,
    /// The style.
    style: ServoArc<ComputedValues>,
    /// Whether the style was given through its rule node, as the engine records it.
    reused_via_rule_node: bool,
    /// The pseudo-element styles.
    pseudos: EagerPseudoStyles,
}

/// One kept match.
#[derive(Clone)]
pub(super) struct Kept {
    /// The matched rules, without a style attribute.
    rules: StrongRuleNode,
    /// The pseudo-element styles of the element the match was kept for, for their rule chains.
    pseudos: EagerPseudoStyles,
    /// The bits of the selector flags matching wrote, by distance above the element.
    flags: SmallVec<[(u8, usize); 2]>,
    /// The last style the match was cascaded to.
    last: Option<Last>,
}

impl Kept {
    /// The style a cascade of `element` under `parent` gives, when the last element that took this
    /// match was cascaded under the same parent style with the same style attribute.
    ///
    /// The same rules, parent style and style attribute cascade to the same style, so the element
    /// takes that style itself, as the engine's sharing cache gives one style to siblings. The key
    /// holds everything else the sharing cache compares. Writes the selector flags the match would
    /// have written.
    pub(super) fn reuse(
        &self,
        element: Node<'_>,
        parent: &ServoArc<ComputedValues>,
    ) -> Option<ResolvedElementStyles> {
        let last = self.last.as_ref()?;
        let same_attribute = match (&last.attribute, element.style_attribute()) {
            (None, None) => true,
            (Some(held), Some(own)) => core::ptr::eq(&**held, &*own),
            _ => false,
        };
        if !ServoArc::ptr_eq(&last.parent, parent) || !same_attribute {
            return None;
        }
        zgui_dom::stylo::recorder::replay(element, &self.flags);
        Some(ResolvedElementStyles {
            primary: PrimaryStyle {
                style: ResolvedStyle(ServoArc::clone(&last.style)),
                reused_via_rule_node: last.reused_via_rule_node,
            },
            pseudos: last.pseudos.clone(),
        })
    }

    /// The cascade inputs of `element`, which matched as the element this was kept for did.
    ///
    /// Writes the selector flags the match would have written.
    pub(super) fn inputs(
        &self,
        element: Node<'_>,
        context: &SharedStyleContext<'_>,
        stand_in: &ServoArc<ComputedValues>,
    ) -> ElementCascadeInputs {
        zgui_dom::stylo::recorder::replay(element, &self.flags);
        let rules = with_style_attribute(&self.rules, element.style_attribute(), context);
        // The engine builds pseudo-element inputs only from held styles, so the inputs start from
        // a stand-in holding the kept pseudo-element styles, whose rule chains a hit shares.
        let mut held = ElementData::default();
        held.styles.primary = Some(ServoArc::clone(stand_in));
        held.styles.pseudos = self.pseudos.clone();
        let mut inputs = ElementCascadeInputs::new_from_element_data(&held);
        inputs.primary = CascadeInputs {
            rules: Some(rules),
            visited_rules: None,
            flags: Default::default(),
            included_cascade_flags: Default::default(),
        };
        inputs
    }
}

/// `rules` with any style attribute taken out.
fn without_style_attribute(
    rules: &StrongRuleNode,
    context: &SharedStyleContext<'_>,
) -> StrongRuleNode {
    replaced_style_attribute(rules, None, context, true)
}

/// `rules`, which hold no style attribute, with `attribute` put in where the cascade reads it.
///
/// Nothing is replaced for an element with no style attribute, and the important level only for a
/// block that declares something important: a replacement walks the chain and inserts it again.
fn with_style_attribute(
    rules: &StrongRuleNode,
    attribute: Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>>,
    context: &SharedStyleContext<'_>,
) -> StrongRuleNode {
    let Some(block) = attribute else {
        return rules.clone();
    };
    let important = block.read_with(context.guards.author).any_important();
    replaced_style_attribute(rules, Some(block), context, important)
}

/// `rules` with `attribute` at the normal level, and at the important level when `important`.
fn replaced_style_attribute(
    rules: &StrongRuleNode,
    attribute: Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>>,
    context: &SharedStyleContext<'_>,
    important: bool,
) -> StrongRuleNode {
    let tree = context.stylist.rule_tree();
    let mut path = rules.clone();
    let mut changed = false;
    let levels = [
        Some(CascadeLevel::same_tree_author_normal()),
        important.then(CascadeLevel::same_tree_author_important),
    ];
    for level in levels.into_iter().flatten() {
        if let Some(replaced) = tree.update_rule_at_level(
            level,
            LayerOrder::style_attribute(),
            attribute.clone(),
            &path,
            &context.guards,
            &mut changed,
        ) {
            path = replaced;
        }
    }
    path
}

/// One worker's kept matches.
type Table = FxHashMap<Key, Kept>;

/// The kept matches, one table per worker, held from one restyle to the next.
pub(crate) struct MatchCache {
    /// The rule set generation the tables were filled under.
    generation: Mutex<Option<usize>>,
    /// The tables, one per worker index the pool can produce plus one for the calling thread.
    workers: Vec<Mutex<Table>>,
}

impl Default for MatchCache {
    fn default() -> Self {
        Self {
            generation: Mutex::new(None),
            workers: (0..=MAX_STYLE_THREADS)
                .map(|_| Mutex::new(Table::default()))
                .collect(),
        }
    }
}

impl MatchCache {
    /// Forgets the styles the last traversal's matches were cascaded to, which only the elements
    /// of one traversal share, so that the shared styles give back what no element holds.
    pub(crate) fn end(&self) {
        for table in &self.workers {
            for kept in table
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .values_mut()
            {
                kept.last = None;
            }
        }
    }

    /// Forgets every match.
    pub(crate) fn clear(&self) {
        for table in &self.workers {
            table
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .clear();
        }
    }

    /// Forgets every match when the rule set was rebuilt since the tables were filled.
    pub(crate) fn begin(&self, rebuilds: usize) {
        let mut generation = self
            .generation
            .lock()
            .unwrap_or_else(|held| held.into_inner());
        if *generation != Some(rebuilds) {
            self.clear();
            *generation = Some(rebuilds);
        }
    }

    /// Runs `f` on the calling worker's table.
    fn with<R>(&self, f: impl FnOnce(&mut Table) -> R) -> R {
        let slot = rayon::current_thread_index()
            .map_or(MAX_STYLE_THREADS, |index| index.min(MAX_STYLE_THREADS));
        f(&mut self.workers[slot]
            .lock()
            .unwrap_or_else(|held| held.into_inner()))
    }

    /// The match kept under `key`.
    pub(super) fn find(&self, key: &Key) -> Option<Kept> {
        self.with(|table| table.get(key).cloned())
    }

    /// Notes that an element keyed `key` was cascaded under `parent`, with `attribute`, to
    /// `styles`, so the next one cascaded alike takes the same style.
    pub(super) fn note(
        &self,
        key: &Key,
        parent: ServoArc<ComputedValues>,
        attribute: Option<ServoArc<Locked<PropertyDeclarationBlock>>>,
        styles: &ResolvedElementStyles,
    ) {
        let style = styles.primary_style();
        // A value read from an attribute is not in what makes two cascades alike.
        if !style.attribute_references.is_none() {
            return;
        }
        self.with(|table| {
            if let Some(kept) = table.get_mut(key) {
                kept.last = Some(Last {
                    parent,
                    attribute,
                    style: ServoArc::clone(style),
                    reused_via_rule_node: styles.primary.reused_via_rule_node,
                    pseudos: styles.pseudos.clone(),
                });
            }
        });
    }

    /// Keeps the match that produced `styles`, if it read nothing the key leaves out.
    pub(super) fn keep(
        &self,
        key: Key,
        styles: &ResolvedElementStyles,
        recorded: Recorded,
        context: &SharedStyleContext<'_>,
    ) {
        let Some(kept) = keepable(styles, recorded, context) else {
            return;
        };
        self.with(|table| {
            if table.len() >= KEPT {
                table.clear();
            }
            table.insert(key, kept);
        });
    }
}

/// The match behind `styles` as it would be kept, or `None` when it read what a key leaves out.
fn keepable(
    styles: &ResolvedElementStyles,
    recorded: Recorded,
    context: &SharedStyleContext<'_>,
) -> Option<Kept> {
    let style = styles.primary_style();
    let read_outside = |style: &ComputedValues| {
        style.visited_style().is_some() || !style.flags.for_cascade_inputs().is_empty()
    };
    if recorded.tainted
        || read_outside(style)
        || styles
            .pseudos
            .as_array()
            .iter()
            .flatten()
            .any(|pseudo| read_outside(pseudo))
    {
        return None;
    }
    let rules = style.rules.as_ref()?;
    // Copies, so the record holds none of the allocations the shared styles sweep when nothing
    // else holds them: the copies are kept for their rule chains alone.
    let mut pseudos = EagerPseudoStyles::default();
    for (index, style) in styles.pseudos.as_array().iter().enumerate() {
        if let Some(style) = style {
            let pseudo = PseudoElement::from_eager_index(index);
            pseudos.set(&pseudo, style.clone_with_flags(style.flags, Some(&pseudo)));
        }
    }
    Some(Kept {
        rules: without_style_attribute(rules, context),
        pseudos,
        last: None,
        flags: recorded.flags,
    })
}

/// Stops the process when the rules `kept` gives `element` are not what its match produced.
pub(super) fn check(
    kept: &Kept,
    real: &ResolvedElementStyles,
    recorded: &Recorded,
    element: Node<'_>,
    context: &SharedStyleContext<'_>,
) {
    let style = real.primary_style();
    // Matching inserts a style attribute at the normal level even when it declares only
    // `!important` values, and a replacement inserts it only where it declares something. The two
    // chains cascade alike, so the matched one is compared after the same round trip.
    let real_rules = style.rules.as_ref().map(|rules| {
        let bare = without_style_attribute(rules, context);
        with_style_attribute(&bare, element.style_attribute(), context)
    });
    let given = with_style_attribute(&kept.rules, element.style_attribute(), context);
    assert!(
        real_rules.as_ref() == Some(&given),
        "the match cache gave <{:?}> rules its matching did not produce",
        element.local_name(),
    );
    let same_pseudos = kept
        .pseudos
        .as_array()
        .iter()
        .zip(real.pseudos.as_array())
        .all(|pair| match pair {
            (None, None) => true,
            (Some(kept), Some(real)) => kept.rules == real.rules,
            _ => false,
        });
    assert!(
        same_pseudos,
        "the match cache gave <{:?}> pseudo-element rules its matching did not produce",
        element.local_name(),
    );
    assert!(
        style.flags.for_cascade_inputs().is_empty() && style.visited_style().is_none(),
        "the match cache answered for <{:?}>, whose matching read a container or a link",
        element.local_name(),
    );
    assert!(
        !recorded.tainted,
        "the match cache answered for <{:?}>, whose matching read what the key leaves out",
        element.local_name(),
    );
    assert!(
        recorded.flags == kept.flags,
        "the match cache would write other selector flags than matching <{:?}> wrote",
        element.local_name(),
    );
}
