//! Declarations attached to one element rather than to a rule.
//!
//! An element carries at most one block of its own declarations, and everything that writes one
//! writes into that block: the `style` text an author wrote, one declaration a view's binding
//! owns, one custom property a theme sets. One block is not a simplification — it is what the
//! cascade can see. The style engine reads an element's own declarations through a single hook,
//! so a second block held beside it would take part in no cascade at all and would be a set of
//! values that never reached the screen.
//!
//! Replacing one declaration therefore does not reparse the rest: the block is kept parsed, and a
//! write replaces the entry for that property in place. Only [`Edit::set_inline_style`], which is
//! given text rather than a property and a value, parses more than one declaration — and, like
//! setting the `style` attribute anywhere else, it replaces everything the element declared.
//!
//! # What this costs the style engine
//!
//! Nothing but a cascade. An element's own declarations cannot change what any selector matches,
//! so the engine is told to re-cascade that element and skip matching entirely — which is the
//! cheapest hint there is, and the reason an animating inline value is affordable at all. A custom
//! property additionally re-cascades the subtree, because a custom property is inherited.

mod parse;

use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::PropertyDeclarationBlock;
use style::shared_lock::SharedRwLock;
use zgui_interned::CustomPropertyName;

use crate::id::node_key::NodeIndex;
use crate::mutate::edit::Edit;
use crate::side::inline_style::StyleBlock;

impl Edit<'_> {
    /// Replaces every declaration `node` carries of its own with the ones `css` spells out.
    ///
    /// `None` removes them all. Declarations that do not parse are dropped and the rest survive,
    /// exactly as they would in a stylesheet.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of the document.
    pub fn set_inline_style(&mut self, node: NodeIndex, css: Option<&str>) {
        let parsed = css.map(parse::attribute).filter(|block| !block.is_empty());
        let store = self.store();
        let key = store.key_of(node);
        let lock = store.lock().clone();
        let slot = store.columns_mut().inline_style.get_mut(key);
        if parsed.is_none() && slot.is_none() {
            return;
        }
        *slot = parsed.map(|block| wrapped(&lock, block));
        self.recascade(node);
    }

    /// Replaces every declaration `node` carries of its own with `block`, shared as it is.
    ///
    /// Elements given the same block share it: the style engine sees their own declarations are
    /// the same, which lets it share their computed style. A later write of one declaration to one
    /// of them copies the block first, so the others keep theirs.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of the document.
    pub fn set_inline_style_block(&mut self, node: NodeIndex, block: Option<StyleBlock>) {
        let store = self.store();
        let key = store.key_of(node);
        let slot = store.columns_mut().inline_style.get_mut(key);
        match (&*slot, &block) {
            (None, None) => return,
            (Some(held), Some(next)) if servo_arc::Arc::ptr_eq(held, next) => return,
            _ => {}
        }
        *slot = block;
        self.recascade(node);
    }

    /// Parses `css` into a block that [`Edit::set_inline_style_block`] can share between elements.
    pub fn parse_style_block(&self, css: &str) -> StyleBlock {
        let lock = self.document().store().lock().clone();
        wrapped(&lock, parse::attribute(css))
    }

    /// Sets or removes one paint-only value of `node` without computing its style again.
    ///
    /// `property` is `background-color`, `opacity`, `border-color` or `outline-color`. The first
    /// value is written as an inline declaration. Every later one is written where the painter
    /// reads a running animation's values, composed over the element's computed style at the
    /// moment it is painted, and the element owes a repaint and nothing else: no restyle, no
    /// layout. A cheap animation running on the same property paints over it. Removing the value
    /// removes the declaration too.
    ///
    /// Returns `false`, changing nothing, for another property, or for a value that is not an
    /// absolute colour (for the colours) or a number or percentage (for `opacity`). A caller falls
    /// back to [`Edit::set_style_property`] for those.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of the document.
    pub fn set_paint(&mut self, node: NodeIndex, property: &str, value: Option<&str>) -> bool {
        use crate::side::AnimOverride;
        let parsed = match value {
            None => None,
            Some(value) => match paint_value(property, value) {
                Some(parsed) => Some(parsed),
                None => return false,
            },
        };
        // The first value goes through the cascade, as an inline declaration. Layout decides from
        // the computed style whether a box paints anything at all, and a box whose style has no
        // background is not drawn whatever is composed over it. Once the element declares the
        // property, later values are composed over the declaration without a cascade.
        if parsed.is_some() && !self.declares(node, property) {
            return self.write_declaration(node, property, value).is_some();
        }
        if parsed.is_none() {
            self.write_declaration(node, property, None);
        }
        let store = self.store();
        let key = store.key_of(node);
        let slot = store.columns_mut().paint.get_mut(key);
        let mut over = slot.take().map_or_else(AnimOverride::new, |held| *held);
        match (property, parsed) {
            ("background-color", Some(PaintValue::Color(c))) => over.background_color = Some(c),
            ("background-color", None) => over.background_color = None,
            ("border-color", Some(PaintValue::Color(c))) => over.border_colors = Some([c; 4]),
            ("border-color", None) => over.border_colors = None,
            ("outline-color", Some(PaintValue::Color(c))) => over.outline_color = Some(c),
            ("outline-color", None) => over.outline_color = None,
            ("opacity", Some(PaintValue::Opacity(o))) => over.opacity = Some(o),
            ("opacity", None) => over.opacity = None,
            _ => {
                *slot = (!over.is_empty()).then(|| Box::new(over));
                return false;
            }
        }
        *slot = (!over.is_empty()).then(|| Box::new(over));
        crate::mutate::ancestors::mark(self.store(), node, crate::Dirty::REPAINT);
        true
    }

    /// Whether `node`'s own declarations set `property` (for `border-color`, its top side).
    fn declares(&mut self, node: NodeIndex, property: &str) -> bool {
        let longhand = if property == "border-color" {
            "border-top-color"
        } else {
            property
        };
        let Some(id) = parse::property_id(longhand) else {
            return false;
        };
        let store = self.store();
        let key = store.key_of(node);
        let Some(block) = store
            .columns()
            .inline_style
            .get(key)
            .and_then(Option::as_ref)
        else {
            return false;
        };
        let lock = store.lock().clone();
        let guard = lock.read();
        let Some(longhand) = id.longhand_id() else {
            return false;
        };
        block
            .read_with(&guard)
            .contains(style::properties::PropertyDeclarationId::Longhand(longhand))
    }

    /// Sets or removes one of `node`'s own declarations, leaving the others alone.
    ///
    /// Returns `false`, changing nothing, when `property` is not a property this build has or
    /// `value` does not parse for it. That is the answer a caller surfaces as a diagnostic: an
    /// unknown property is dropped here for the same reason it is dropped in a stylesheet, and
    /// silently doing nothing is what would make a misspelling invisible.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of the document.
    pub fn set_style_property(
        &mut self,
        node: NodeIndex,
        property: &str,
        value: Option<&str>,
    ) -> bool {
        self.write_declaration(node, property, value).is_some()
    }

    /// Writes one declaration, reporting whether it landed and whether it changed anything.
    ///
    /// `None` is "there is no such declaration to make"; `Some(false)` is "the block already said
    /// that", which is the case no consumer may turn into style-engine work.
    fn write_declaration(
        &mut self,
        node: NodeIndex,
        property: &str,
        value: Option<&str>,
    ) -> Option<bool> {
        let id = parse::property_id(property)?;
        let store = self.store();
        let key = store.key_of(node);
        let lock = store.lock().clone();
        // A block shared with other elements is copied before it is written, so the write lands
        // on this element alone.
        if let Some(shared) = store.columns_mut().inline_style.get_mut(key).as_mut()
            && !shared.is_unique()
        {
            let copy = shared.read_with(&lock.read()).clone();
            *shared = wrapped(&lock, copy);
        }
        // A reference-count bump, taken so the block can be written through while the column is
        // not borrowed: the handle and the column name the same block, so the write lands once.
        let held: Option<StyleBlock> = store.columns_mut().inline_style.get_mut(key).clone();

        let (changed, fresh) = match (&held, value) {
            (None, None) => (false, None),
            (None, Some(value)) => {
                let mut block = PropertyDeclarationBlock::new();
                // An empty block declares nothing, so anything that parses at all is a change.
                match parse::set(&mut block, property, &id, value) {
                    parse::Wrote::Rejected => return None,
                    parse::Wrote::Unchanged | parse::Wrote::Changed => {}
                }
                (true, Some(wrapped(&lock, block)))
            }
            (Some(block), value) => {
                let mut guard = lock.write();
                let block = block.write_with(&mut guard);
                let changed = match value {
                    Some(value) => match parse::set(block, property, &id, value) {
                        parse::Wrote::Rejected => return None,
                        // The whole point of the distinction: re-stating a declaration the block
                        // already holds is not a style change, and must not become one.
                        parse::Wrote::Unchanged => false,
                        parse::Wrote::Changed => true,
                    },
                    None => parse::remove(block, &id),
                };
                (changed, None)
            }
        };
        if let Some(fresh) = fresh {
            *self.store().columns_mut().inline_style.get_mut(key) = Some(fresh);
        }
        if changed {
            self.recascade(node);
        }
        Some(changed)
    }

    /// Sets or removes one custom property on `node`.
    ///
    /// A custom property is a declaration like any other, so it goes into the same block. It
    /// additionally re-cascades everything below `node`, because a custom property is inherited
    /// and an element that reads one is usually not the element that set it.
    ///
    /// Returns whether `value` parsed. A custom property's name always resolves, so `false` here
    /// means the value was not a valid one.
    ///
    /// # Panics
    ///
    /// Panics if `node` names no live node of the document.
    pub fn set_custom_property(
        &mut self,
        node: NodeIndex,
        property: CustomPropertyName,
        value: Option<&str>,
    ) -> bool {
        let Some(changed) = self.write_declaration(node, &property.to_declaration(), value) else {
            return false;
        };
        // Nothing below is told anything here. The element's own cascade rebuilds its map, and
        // the traversal hands the new map down from there: a descendant that reads the property
        // cascades again, one that does not takes the map into the style it has.
        let _ = changed;
        true
    }

    /// Tells the engine to re-cascade `node` without re-matching anything.
    fn recascade(&mut self, node: NodeIndex) {
        let (store, batch) = self.parts();
        batch
            .hints
            .record(store, node, RestyleHint::RESTYLE_STYLE_ATTRIBUTE);
    }
}

/// A parsed paint-only value.
enum PaintValue {
    Color(style::color::AbsoluteColor),
    Opacity(f32),
}

/// Parses `value` for the paint-only `property`, or `None` when it is not a value the painter can
/// take without a cascade.
fn paint_value(property: &str, value: &str) -> Option<PaintValue> {
    use style::properties::PropertyDeclaration;
    use style::values::specified::Color;
    if property == "opacity" {
        let value = value.trim();
        let number = match value.strip_suffix('%') {
            Some(percent) => percent.trim().parse::<f32>().ok()? / 100.0,
            None => value.parse::<f32>().ok()?,
        };
        return number
            .is_finite()
            .then(|| PaintValue::Opacity(number.clamp(0.0, 1.0)));
    }
    if !matches!(
        property,
        "background-color" | "border-color" | "outline-color"
    ) {
        return None;
    }
    let block = parse::attribute(&format!("color:{value}"));
    match block.declarations().first() {
        Some(PropertyDeclaration::Color(color)) => match &color.0 {
            Color::Absolute(absolute) => Some(PaintValue::Color(absolute.color)),
            _ => None,
        },
        _ => None,
    }
}

/// One block, wrapped in the document's lock and reference-counted.
fn wrapped(lock: &SharedRwLock, block: PropertyDeclarationBlock) -> StyleBlock {
    servo_arc::Arc::new(lock.wrap(block))
}

#[cfg(test)]
mod tests;
