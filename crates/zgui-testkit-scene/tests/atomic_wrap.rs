//! The deterministic shaper breaks lines where browsers do: after a space, which hangs at the end
//! of its line, and before and after an atomic inline, never at the edge of an inline box.
//!
//! The runtime and conformance suites lay out on this shaper, so its breaks are the ones they see.
//! CSS Text 3 §5.1 treats an atomic inline as an ideographic character for line breaking, an inline
//! box's start edge belongs to what follows it and its end edge to what precedes it, and CSS Text 3
//! §4.1.2 hangs a space at the end of a line. Chromium 141 measures these (headless, 16px
//! sans-serif, 24px inline blocks), relative to the block:
//!
//! | markup, block width | Chromium 141 |
//! | --- | --- |
//! | `[24] [24] [24]`, 60 | two lines; the third box at (0, 24) |
//! | `[24][24][24]`, 60 | two lines; boxes at x 0 and 24, the third at (0, 24) |
//! | `aaaa bbbb`, 60 | two lines; `b` starts the second at x 0 |
//! | `word<span>word</span>word`, 60 | one line |
//! | `[24]<span style="padding-left: 6px">[24]</span>`, 40 | the second box at (6, 24) |
//!
//! Every cluster is 8px here, a space too, so x positions after a space on its line are this
//! shaper's. No mocks: the shaper itself, through the text protocol.

use std::sync::Arc;

use zgui_geom::CssPx;
use zgui_scene::PaintSlot;
use zgui_testkit_scene::{MonoLayout, MonoShaper};
use zgui_text::{
    BreakRequest, BrokenParagraph, InlineBoxGeometry, ParagraphCache, ParagraphContent,
    ParagraphShaper, StyledRun, TextMap, lay_out,
};
use zgui_text_style::{ParagraphStyle, TextStyle};

/// One box: offset, width, whether it is an edge, and for an edge whether it is the start edge.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Spec {
    offset: usize,
    width: f32,
    glued: bool,
    opens: bool,
}

fn atomic(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        glued: false,
        opens: false,
    }
}

fn opening(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        glued: true,
        opens: true,
    }
}

fn closing(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        glued: true,
        opens: false,
    }
}

/// Breaks `text` with `boxes` at `width`; box ids count from 1 in `boxes` order.
fn broken(text: &str, boxes: &[Spec], width: f32) -> BrokenParagraph {
    let mut map = TextMap::new();
    map.push(0..text.len(), 0, 0);
    let runs = [StyledRun {
        text: 0..text.len(),
        style: Arc::new(TextStyle::initial()),
        brush: PaintSlot(0),
    }];
    let geometry: Vec<InlineBoxGeometry> = boxes
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let height = if spec.glued { 0.0 } else { 24.0 };
            InlineBoxGeometry {
                id: index as u64 + 1,
                offset: spec.offset,
                width: CssPx(spec.width),
                height: CssPx(height),
                ascent: CssPx(height),
                shift: CssPx::ZERO,
                glued: spec.glued,
                opens: spec.opens,
            }
        })
        .collect();
    let paragraph = ParagraphStyle::initial();
    let content = ParagraphContent {
        text,
        map: &map,
        runs: &runs,
        boxes: &geometry,
        paragraph: &paragraph,
        scale: 1.0,
    };
    let mut shaper = MonoShaper::new();
    let mut cache: ParagraphCache<MonoLayout> = ParagraphCache::new();
    let request = BreakRequest::new(&content, Some(CssPx(width)));
    lay_out(&mut shaper, &mut cache, &content, &request).1
}

/// The `(line, x)` of the box with `id`.
fn placed(broken: &BrokenParagraph, id: u64) -> (usize, f32) {
    let placement = broken
        .boxes
        .iter()
        .find(|placement| placement.id == id)
        .unwrap_or_else(|| panic!("box {id} was not placed"));
    (placement.line, placement.origin.x.0)
}

#[test]
fn inline_blocks_separated_by_spaces_wrap_at_a_space() {
    let broken = broken(
        "  ",
        &[atomic(0, 24.0), atomic(1, 24.0), atomic(2, 24.0)],
        60.0,
    );
    assert_eq!(broken.geometry.lines.len(), 2);
    assert_eq!(placed(&broken, 2), (0, 32.0), "after one 8px space");
    assert_eq!(placed(&broken, 3), (1, 0.0));
}

#[test]
fn inline_blocks_with_nothing_between_them_wrap_between_two_of_them() {
    let broken = broken(
        "",
        &[atomic(0, 24.0), atomic(0, 24.0), atomic(0, 24.0)],
        60.0,
    );
    assert_eq!(broken.geometry.lines.len(), 2);
    assert_eq!(placed(&broken, 2), (0, 24.0));
    assert_eq!(placed(&broken, 3), (1, 0.0));
}

#[test]
fn a_line_broken_at_a_space_starts_at_the_next_word() {
    let broken = broken("aaaa bbbb", &[], 60.0);
    let lines = &broken.geometry.lines;
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[1].text.start, 5,
        "the space hangs at the end of line one"
    );
    assert_eq!(lines[1].width, CssPx(32.0));
}

#[test]
fn a_word_split_across_an_inline_box_does_not_break_at_its_edges() {
    let broken = broken("wordwordword", &[opening(4, 0.0), closing(8, 0.0)], 60.0);
    assert_eq!(broken.geometry.lines.len(), 1);
}

#[test]
fn a_start_edge_before_an_inline_block_goes_to_the_next_line_with_it() {
    let broken = broken(
        "",
        &[
            atomic(0, 24.0),
            opening(0, 6.0),
            atomic(0, 24.0),
            closing(0, 0.0),
        ],
        40.0,
    );
    assert_eq!(broken.geometry.lines.len(), 2);
    assert_eq!(placed(&broken, 2), (1, 0.0));
    assert_eq!(placed(&broken, 3), (1, 6.0));
}

#[test]
fn a_box_widens_the_narrowest_the_paragraph_gets_and_the_widest() {
    // An atomic inline is as wide as itself at the narrowest and takes its width on the one line
    // of the widest; an edge widens the word it sits in.
    let mut map = TextMap::new();
    let text = "ab cd";
    map.push(0..text.len(), 0, 0);
    let runs = [StyledRun {
        text: 0..text.len(),
        style: Arc::new(TextStyle::initial()),
        brush: PaintSlot(0),
    }];
    let paragraph = ParagraphStyle::initial();
    let boxes: Vec<InlineBoxGeometry> = [atomic(0, 30.0), opening(3, 6.0)]
        .iter()
        .enumerate()
        .map(|(index, spec)| InlineBoxGeometry {
            id: index as u64 + 1,
            offset: spec.offset,
            width: CssPx(spec.width),
            height: CssPx::ZERO,
            ascent: CssPx::ZERO,
            shift: CssPx::ZERO,
            glued: spec.glued,
            opens: spec.opens,
        })
        .collect();
    let content = ParagraphContent {
        text,
        map: &map,
        runs: &runs,
        boxes: &boxes,
        paragraph: &paragraph,
        scale: 1.0,
    };
    let shaped = MonoShaper::new().shape(&content);
    let widths = shaped.content_widths();
    assert_eq!(
        widths.min,
        CssPx(30.0),
        "the 30px box is the widest unbreakable piece"
    );
    assert_eq!(
        widths.max,
        CssPx(76.0),
        "five clusters, the box and the edge"
    );
}
