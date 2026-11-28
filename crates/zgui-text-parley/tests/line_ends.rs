//! Which line the content beside a break goes to: the edges of inline boxes, and the space after
//! an inline block that overflows its line.
//!
//! CSS Text 3 §5.1 puts a line break opportunity between two pieces of content, and an inline
//! box's start edge belongs to what follows it, its end edge to what precedes it. So a break at a
//! space before a `<span>` takes the span's start edge, and its padding, to the next line; a break
//! after an inline block leaves the end edge of the span around it behind; and a break before an
//! inline block takes the start edge that opens into it. CSS Text 3 §4.1.2 hangs a space at the
//! end of a line, so the line after it starts at its content.
//!
//! Chromium 141 (headless, 16px sans-serif, 24px inline blocks), relative to the block:
//!
//! | markup, block width | Chromium 141 |
//! | --- | --- |
//! | `ab <span style="padding-left: 6px">wordwordword</span>`, 60 | the span starts the second line at (0, 27) |
//! | `abcd <span style="padding-left: 6px">ef</span>`, 60 | one line, the span at x 39.14 (80 here: the test faces are wider) |
//! | `<span style="padding-right: 6px">ab </span>wordwordword`, 60 | the span ends on the first line |
//! | `<span style="padding-right: 6px">[24][24]</span>`, 40 | the second box and the 6px edge on line two, 30 wide |
//! | `[24]<span style="padding-left: 6px">[24]</span>`, 40 | the edge starts line two; the box at (6, 24) |
//! | `[24] [100] [24]`, 60 | the last box at (0, 48) |
//! | `[100] [24]`, 60 | the second box at (0, 24) |
//!
//! No mocks: the real shaper over the faces this crate's tests ship.

mod support;

use std::sync::Arc;

use zgui_geom::CssPx;
use zgui_scene::PaintSlot;
use zgui_text::{
    BreakRequest, InlineBoxGeometry, ParagraphContent, ParagraphShaper, StyledRun, TextMap,
};
use zgui_text_parley::Controls;
use zgui_text_style::ParagraphStyle;

/// One box of a case.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Spec {
    offset: usize,
    width: f32,
    height: f32,
    glued: bool,
    opens: bool,
}

/// An atomic inline, 24px tall.
fn atomic(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        height: 24.0,
        glued: false,
        opens: false,
    }
}

/// The start edge of a nested inline box.
fn opening(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        height: 0.0,
        glued: true,
        opens: true,
    }
}

/// The end edge of a nested inline box.
fn closing(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        height: 0.0,
        glued: true,
        opens: false,
    }
}

/// The line count, and each box's `(line, x)` by id; ids count from 1 in `boxes` order.
fn lay_out(text: &str, boxes: &[Spec], width: f32) -> (usize, Vec<(usize, f32)>) {
    let (_fonts, mut shaper) = support::shaper(Controls::Mark);
    let mut map = TextMap::new();
    map.push(0..text.len(), 0, 0);
    let paragraph = ParagraphStyle::initial();
    let runs = [StyledRun {
        text: 0..text.len(),
        style: Arc::new(support::style()),
        brush: PaintSlot(0),
    }];
    let geometry: Vec<InlineBoxGeometry> = boxes
        .iter()
        .enumerate()
        .map(|(index, spec)| InlineBoxGeometry {
            id: index as u64 + 1,
            offset: spec.offset,
            width: CssPx(spec.width),
            height: CssPx(spec.height),
            ascent: CssPx(spec.height),
            shift: CssPx(0.0),
            glued: spec.glued,
            opens: spec.opens,
        })
        .collect();
    let content = ParagraphContent {
        text,
        map: &map,
        runs: &runs,
        boxes: &geometry,
        paragraph: &paragraph,
        scale: 1.0,
    };
    let mut shaped = shaper.shape(&content);
    let broken = shaper.break_lines(
        &mut shaped,
        &BreakRequest::new(&content, Some(CssPx(width))),
    );
    let mut placed = vec![(usize::MAX, f32::NAN); boxes.len()];
    for placement in &broken.boxes {
        placed[placement.id as usize - 1] = (placement.line, placement.origin.x.0);
    }
    (broken.geometry.lines.len(), placed)
}

#[test]
fn a_break_at_a_space_takes_the_start_edge_after_it_to_the_next_line() {
    let (lines, placed) = lay_out(
        "ab wordwordword",
        &[opening(3, 6.0), closing(15, 0.0)],
        60.0,
    );
    assert_eq!(lines, 2);
    assert_eq!(placed[0], (1, 0.0), "the padded start edge begins line two");
}

#[test]
fn a_start_edge_after_a_space_that_fits_stays_on_its_line() {
    let (lines, placed) = lay_out("abcd ef", &[opening(5, 6.0), closing(7, 0.0)], 80.0);
    assert_eq!(lines, 1);
    assert_eq!(placed[0].0, 0);
}

#[test]
fn an_end_edge_after_a_space_stays_on_the_line_it_ends() {
    let (lines, placed) = lay_out("ab wordwordword", &[opening(0, 0.0), closing(3, 6.0)], 60.0);
    assert_eq!(lines, 2);
    assert_eq!(placed[1].0, 0, "the padded end edge closes line one");
}

#[test]
fn an_end_edge_after_an_inline_block_goes_with_the_block() {
    let (lines, placed) = lay_out(
        "",
        &[
            opening(0, 0.0),
            atomic(0, 24.0),
            atomic(0, 24.0),
            closing(0, 6.0),
        ],
        40.0,
    );
    assert_eq!(lines, 2);
    assert_eq!(placed[1], (0, 0.0));
    assert_eq!(placed[2], (1, 0.0));
    assert_eq!(
        placed[3],
        (1, 24.0),
        "the end edge follows the box onto line two"
    );
}

#[test]
fn a_start_edge_before_an_inline_block_goes_with_the_block() {
    let (lines, placed) = lay_out(
        "",
        &[
            atomic(0, 24.0),
            opening(0, 6.0),
            atomic(0, 24.0),
            closing(0, 0.0),
        ],
        40.0,
    );
    assert_eq!(lines, 2);
    assert_eq!(placed[1], (1, 0.0), "the start edge begins line two");
    assert_eq!(placed[2], (1, 6.0), "the box sits after its padding");
}

#[test]
fn the_space_after_an_overflowing_inline_block_hangs_on_its_line() {
    let (lines, placed) = lay_out(
        "  ",
        &[atomic(0, 24.0), atomic(1, 100.0), atomic(2, 24.0)],
        60.0,
    );
    assert_eq!(lines, 3);
    assert_eq!(placed[1], (1, 0.0));
    assert_eq!(
        placed[2],
        (2, 0.0),
        "the line after the wide box starts at its box"
    );

    let (lines, placed) = lay_out(" ", &[atomic(0, 100.0), atomic(1, 24.0)], 60.0);
    assert_eq!(lines, 2);
    assert_eq!(placed[1], (1, 0.0));
}
