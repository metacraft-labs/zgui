//! Where an out-of-flow box would have been, had it been in flow.
//!
//! When an absolutely positioned box has `auto` for both insets in an axis, CSS places it at its
//! *static position* in that axis: where its margin box would have started had its `position` been
//! `static` (CSS 2.1 §10.3.7 and §10.6.4; css-position-3 §4.6, the static-position rectangle).
//!
//! The box tree holds an out-of-flow box as a layout child of its containing block, appended after
//! that block's in-flow children (see [`crate::boxtree::absolute`]). Asked where such a child would
//! have been, the containing block's own layout answers "after all of my content", which is its
//! bottom. That is right only for a box written last and directly inside its containing block.
//! jsTree's full-width row highlight is a `top: auto` box written first in each list item, inside a
//! list that contains them all, and every one of them landed at the list's bottom instead of on
//! its own row.
//!
//! The answer here comes from where the box was written instead: the [`Declared`] record names the
//! box it was written inside and the in-flow sibling it followed, and the layouts of both are
//! written by the time the containing block lays out its out-of-flow boxes. What it answers, in the
//! coordinates of the containing block's border box:
//!
//! - written before any in-flow sibling: the top of its parent's content box;
//! - after a block-level sibling: that sibling's bottom margin edge;
//! - after inline-level content: the bottom of the line that content ends on, because the box is
//!   block-level (an absolutely positioned box is blockified) and so would have started a new line.
//!
//! In the inline axis it spans its parent's content box, aligned the way the containing block
//! already aligns it there. Chromium 141 measures each of these the same way.
//!
//! When the record cannot be followed — a parent that is a flex or grid container, whose static
//! position follows other rules, a parent that is an inline box, a sibling with no layout yet —
//! the containing block's own answer is kept, which is what every box got before this existed.

use cephal::compute::oof::AxisStaticPosition;
use cephal::geometry::{Line, Point};
use zgui_dom::side::BoxKey;

use crate::boxtree::absolute::Declared;
use crate::measure::MeasureContent;
use crate::node::kind::FormattingContext;
use crate::tree::LayoutTree;

/// The static position of `node`, an out-of-flow box laid out by `cb`, given what `cb` recorded.
pub(crate) fn static_position<C: MeasureContent>(
    tree: &LayoutTree<'_, C>,
    cb: BoxKey,
    node: BoxKey,
    recorded: Point<AxisStaticPosition>,
) -> Point<AxisStaticPosition> {
    let Some(declared) = tree.structure().declared(node) else {
        return recorded;
    };
    let Some((inline_area, block_start)) = where_written(tree, cb, declared) else {
        return recorded;
    };
    Point {
        x: AxisStaticPosition {
            area: inline_area,
            align: recorded.x.align,
        },
        y: AxisStaticPosition {
            area: Line {
                start: block_start,
                end: block_start,
            },
            align: recorded.y.align,
        },
    }
}

/// The inline extent of the parent's content box and the block-axis start of the hypothetical
/// box, both relative to `cb`'s border box.
fn where_written<C: MeasureContent>(
    tree: &LayoutTree<'_, C>,
    cb: BoxKey,
    declared: Declared,
) -> Option<(Line<f32>, f32)> {
    let structure = tree.structure();
    let parent = declared.parent;
    let parent_node = structure.get(parent)?;
    // A block container lays its children out one after another, which is what the answer below
    // follows. A flex or grid container places an out-of-flow child as if it were its only item,
    // and an inline box has no box of its own to measure from.
    if !parent_node.block_level
        || !matches!(
            parent_node.fc,
            FormattingContext::Block | FormattingContext::Inline
        )
    {
        return None;
    }
    let origin = origin_within(tree, parent, cb)?;
    let layout = tree.state(parent)?.unrounded;
    let content_top = layout.border.top + layout.padding.top;
    let content_left = layout.border.left + layout.padding.left;
    let content_right = layout.size.width - layout.border.right - layout.padding.right;
    let inline_area = Line {
        start: origin.x + content_left,
        end: origin.x + content_right.max(content_left),
    };
    let block_start = match declared.after.filter(|&after| follows_in(tree, after, parent)) {
        None => content_top,
        Some(after) => below(tree, after, parent)?,
    };
    Some((inline_area, origin.y + block_start))
}

/// Whether `after` is still an in-flow box inside `parent`: its child, or the child of an
/// anonymous block wrapping a run of `parent`'s inline-level children. A patch can leave the
/// record naming a box that has since gone.
fn follows_in<C: MeasureContent>(tree: &LayoutTree<'_, C>, after: BoxKey, parent: BoxKey) -> bool {
    let structure = tree.structure();
    let Some(holder) = structure.get(after).and_then(|node| node.parent) else {
        return false;
    };
    holder == parent
        || structure.get(holder).is_some_and(|wrapper| {
            wrapper.parent == Some(parent)
                && matches!(
                    wrapper.kind,
                    crate::node::kind::BoxKind::AnonymousBlock
                        | crate::node::kind::BoxKind::AnonymousInlineRoot
                )
        })
}

/// Where a block-level box written right after `after` would start, relative to `parent`'s border
/// box.
fn below<C: MeasureContent>(tree: &LayoutTree<'_, C>, after: BoxKey, parent: BoxKey) -> Option<f32> {
    let structure = tree.structure();
    let after_node = structure.get(after)?;
    let holder = after_node.parent?;
    if holder == parent && after_node.block_level {
        // The margin edge: a hypothetical box is placed where its own margin box would start.
        let layout = tree.state(after)?.unrounded;
        return Some(layout.location.y + layout.size.height + layout.margin.bottom);
    }
    // Inline-level content: the lines of the formatting context it is on, which is `parent` itself
    // or the anonymous block that wraps the run. A block-level box breaks the line, so it starts
    // below the line that `after` ends on.
    let resolution = tree.inline_resolution_of(holder)?;
    let line = resolution
        .placements
        .iter()
        .find(|placement| placement.box_ == after)
        .map(|placement| placement.line)
        // A run of text has no placement of its own. Its last line is not recorded either, so
        // this takes the context's last line, which is right when the text ends the run.
        .or_else(|| resolution.lines.len().checked_sub(1))?;
    let line = resolution.lines.get(line)?;
    let holder_layout = tree.state(holder)?.unrounded;
    // The lines are measured from the formatting context's content box.
    let content_top = holder_layout.border.top + holder_layout.padding.top;
    let holder_top = if holder == parent {
        0.0
    } else {
        holder_layout.location.y
    };
    Some(holder_top + content_top + line.top + line.height())
}

/// Where `key`'s border box sits within `cb`'s, by the layout parents between them.
fn origin_within<C: MeasureContent>(
    tree: &LayoutTree<'_, C>,
    key: BoxKey,
    cb: BoxKey,
) -> Option<Point<f32>> {
    let structure = tree.structure();
    let mut origin = Point { x: 0.0, y: 0.0 };
    let mut at = key;
    while at != cb {
        let node = structure.get(at)?;
        // An inline box has no location of its own: its fragments sit on lines.
        if !node.block_level && node.fc == FormattingContext::Inline {
            return None;
        }
        let location = tree.state(at)?.unrounded.location;
        origin.x += location.x;
        origin.y += location.y;
        at = node.parent?;
    }
    Some(origin)
}
