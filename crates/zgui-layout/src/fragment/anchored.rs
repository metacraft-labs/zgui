//! Which boxes a scroll carries with it, and which stay where they are.
//!
//! A scroll is composed rather than laid out: nothing about the boxes inside a scroll container
//! changes when it is scrolled, so the offset is subtracted once, at the container, and carried
//! down as a shift that every descendant's origin is written with. That is right for every box
//! whose containing block is inside the container — which is nearly all of them.
//!
//! It is wrong for exactly one kind. A `position: fixed` box's containing block is the viewport
//! (unless a transformed or filtered ancestor contains it), so scrolling an ancestor must not move
//! it: that is the whole of what `fixed` means, and it is what a floating surface, a masthead
//! pinned to the top of the window and a modal's scrim are all built out of. A shift applied to one of those carries it off the screen at
//! exactly the rate the page is scrolled, and every measurement taken inside the process still
//! agrees with itself — the box has a box, it has the size it asked for, it is in the display list.
//! It is simply somewhere nobody can see.

use zgui_css::values::size::PositionValue;
use zgui_dom::side::BoxKey;

use crate::tree::store::LayoutStore;

/// Whether the box `key` is in the viewport rather than in whatever is scrolling around it.
///
/// The one question a caller composing an origin has to ask: a box that answers `true` takes no
/// part of the accumulated scroll shift, and neither does anything inside it.
///
/// A fixed box is laid out by its containing block (see
/// [`establishes_fixed_containing_block`](crate::boxtree::absolute::establishes_fixed_containing_block)).
/// That is the root's box, or a transformed box, which has a coordinate system the fixed box hangs
/// from, or a filtered box, which has none. A fixed box that a filtered box contains moves with it
/// as an absolute box does, so it answers `false`.
pub(crate) fn ignores_scroll(store: &LayoutStore, key: BoxKey) -> bool {
    let Some(node) = store.get(key) else {
        return false;
    };
    if node.style.get_box().position != PositionValue::Fixed {
        return false;
    }
    !node
        .parent
        .and_then(|parent| store.get(parent))
        .is_some_and(|parent| {
            parent.parent.is_some()
                && crate::boxtree::absolute::establishes_fixed_containing_block(
                    &parent.style,
                    parent.fc,
                )
                && !crate::fragment::transform::is_transformed(parent.style.get_box())
        })
}
