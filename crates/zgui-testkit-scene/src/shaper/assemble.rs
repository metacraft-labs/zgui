//! Turning the cluster ranges of the last break into the geometry a layout engine reads.

use smallvec::SmallVec;
use zgui_geom::{Css, CssPx, Point, Size};
use zgui_text::{
    BreakRequest, BrokenParagraph, InlineBoxGeometry, InlineBoxPlacement, LineGeometry,
    ShapedParagraph, TextGeometry,
};
use zgui_text_style::{Direction, TextAlign};

use crate::shaper::cluster::MonoLayout;

/// Builds the broken geometry from the lines the last break produced.
pub fn geometry(
    shaped: &ShapedParagraph<MonoLayout>,
    request: &BreakRequest<'_>,
) -> BrokenParagraph {
    let strut = shaped.strut();
    let clusters = &shaped.engine.clusters;
    let mut lines = Vec::new();
    let mut boxes: SmallVec<[InlineBoxPlacement; 2]> = SmallVec::new();
    let mut top = CssPx::ZERO;
    let mut widest: f32 = 0.0;

    for (index, (start, end)) in shaped.engine.lines.iter().copied().enumerate() {
        let on_line: Vec<(&InlineBoxGeometry, f32)> = shaped.engine.placed[index]
            .iter()
            .map(|&(at, before)| (&shaped.boxes()[at], before))
            .collect();

        // A line box is at least as tall as the strut, and taller where an inline box reaches past
        // it. The shifted height is what an inline box occupies above the baseline, which is where
        // `vertical-align` has already been folded in.
        let ascent = on_line
            .iter()
            .fold(strut.ascent(), |tallest, (geometry, _)| {
                CssPx(tallest.0.max(geometry.shaper_height().0))
            });
        let descent = on_line
            .iter()
            .fold(strut.descent(), |deepest, (geometry, _)| {
                CssPx(deepest.0.max(geometry.below_baseline().0))
            });

        let text_advance: f32 = clusters[start..end].iter().map(|held| held.advance.0).sum();
        let box_advance: f32 = on_line.iter().map(|(geometry, _)| geometry.width.0).sum();
        let width = text_advance + box_advance;
        let indent = if index == 0 { request.indent().0 } else { 0.0 };
        // The spaces that end the line hang (CSS Text 3 §4.1.2): they count in its width, as the
        // engine counts them, and alignment places the line without them, as the engine does.
        let spaces: f32 = clusters[start..end]
            .iter()
            .rev()
            .take_while(|cluster| cluster.breakable)
            .map(|cluster| cluster.advance.0)
            .sum();
        let after_spaces = width - spaces;
        let hanging = if on_line.iter().any(|(geometry, before)| {
            !geometry.glued && *before + geometry.width.0 > after_spaces
        }) {
            0.0
        } else {
            spaces
        };
        let offset = align_offset(request, CssPx(width - hanging + indent)).0 + indent;
        let baseline = CssPx(top.0 + ascent.0);

        for (geometry, before) in on_line {
            boxes.push(InlineBoxPlacement {
                id: geometry.id,
                origin: Point::new(
                    CssPx(offset + before),
                    CssPx(baseline.0 - geometry.shaper_height().0),
                ),
                line: index,
            });
        }

        widest = widest.max(width + indent);
        lines.push(LineGeometry {
            text: clusters.get(start).map_or(0, |held| held.offset)
                ..clusters
                    .get(end)
                    .map_or(shaped.text().len(), |held| held.offset),
            top,
            baseline,
            height: CssPx(ascent.0 + descent.0),
            width: CssPx(width),
            offset: CssPx(offset),
        });
        top = CssPx(top.0 + ascent.0 + descent.0);
    }

    BrokenParagraph {
        geometry: std::sync::Arc::new(TextGeometry {
            size: Size::<CssPx, Css>::new(CssPx(widest), top),
            lines,
            is_rtl: request.paragraph.direction == Direction::RightToLeft,
        }),
        boxes,
    }
}

/// How far one line of `width` is pushed in from the start edge by alignment.
///
/// `start` and `end` are resolved against the paragraph's base direction, which is the one place
/// direction changes anything here: no glyph is reordered, because nothing is shaped.
fn align_offset(request: &BreakRequest<'_>, width: CssPx) -> CssPx {
    let Some(limit) = request.max_advance else {
        return CssPx::ZERO;
    };
    let free = (limit.0 - width.0).max(0.0);
    let rtl = request.paragraph.direction == Direction::RightToLeft;
    match request.paragraph.align {
        TextAlign::Center => CssPx(free / 2.0),
        TextAlign::Right => CssPx(free),
        TextAlign::Left => CssPx::ZERO,
        TextAlign::Start if rtl => CssPx(free),
        TextAlign::End if !rtl => CssPx(free),
        _ => CssPx::ZERO,
    }
}
