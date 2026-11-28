//! Where the engine breaks a line around atomic inlines and around the edges of inline boxes.
//!
//! CSS Text 3 §5.1 treats an atomic inline as an ideographic character for line breaking, so a
//! line may break before and after one whether or not white space touches it; CSS 2.1 §16.6 and
//! CSS Text 3 §3 leave whether it may wrap at all to `white-space`. An element boundary inside a
//! word is no break opportunity: the margin, border and padding at the edges of a `<span>` reach
//! the breaker as a box of their width, and that box is glued to the text either side.
//!
//! Every case is one Chromium 141 measures (headless, 16px sans-serif, a 60px-wide block, 24px
//! inline blocks with `vertical-align: top`), relative to the block:
//!
//! | markup | Chromium 141 |
//! | --- | --- |
//! | `[24] [24] [24]` | two lines: boxes at x 0 and 28.45, the third at (0, 24) |
//! | `[24][24][24]` | two lines: boxes at x 0 and 24, the third at (0, 24) |
//! | `[24] wordword` | two lines: the word wraps below the box |
//! | `wordword [24]` | two lines: the box at (0, 24) |
//! | `[24] [24] [24]`, `nowrap` | one line: boxes at x 0, 28.45 and 56.91 |
//! | `[24] [100] [24]` | three lines: the wide box alone on the second, overflowing |
//! | 50px buttons in a 120px bar | three lines of 2, 2 and 1 |
//! | `word<span>word</span>word` | one line, overflowing |
//! | `ab <span>wordwordword</span>` | two lines: the break is at the space |
//! | `ab<span style="padding-left: 4px">wordwordword</span>` | one line, overflowing |
//!
//! Line counts and line assignments are asserted, and the x of a box the font's space advance
//! cannot move. The faces are the two this crate's tests ship, so a space is 4.16px here against
//! Chromium's 4.45px.
//!
//! No mocks: the real shaper over real faces.

mod support;

use std::sync::Arc;

use zgui_geom::CssPx;
use zgui_scene::PaintSlot;
use zgui_text::{
    BreakRequest, InlineBoxGeometry, ParagraphContent, ParagraphShaper, StyledRun, TextMap,
};
use zgui_text_parley::Controls;
use zgui_text_style::{ParagraphStyle, WrapMode};

/// One box of a case: its offset in the text, its width, and whether it is an edge of an inline
/// box rather than an atomic inline.
#[derive(Clone, Copy)]
struct Spec {
    offset: usize,
    width: f32,
    height: f32,
    glued: bool,
}

/// An atomic inline, 24px tall.
fn atomic(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        height: 24.0,
        glued: false,
    }
}

/// An edge of a nested inline box: zero height, glued.
fn edge(offset: usize, width: f32) -> Spec {
    Spec {
        offset,
        width,
        height: 0.0,
        glued: true,
    }
}

/// What a break came out at: the number of lines, each box's line and x by id (ids count from 1
/// in `boxes` order), and the paragraph's min-content width.
struct Broken {
    lines: usize,
    boxes: Vec<(u64, usize, f32)>,
    min: f32,
    max: f32,
}

impl Broken {
    /// The line and x of the box with `id`.
    fn of(&self, id: u64) -> (usize, f32) {
        let (_, line, x) = *self
            .boxes
            .iter()
            .find(|(held, ..)| *held == id)
            .unwrap_or_else(|| panic!("box {id} was not placed"));
        (line, x)
    }
}

/// Breaks `text` with `boxes` at `width`.
fn lay_out(text: &str, boxes: &[Spec], width: f32, wrap: WrapMode) -> Broken {
    let (_fonts, mut shaper) = support::shaper(Controls::Mark);
    let style = Arc::new(zgui_text_style::TextStyle {
        wrap_mode: wrap,
        ..support::style()
    });
    let mut map = TextMap::new();
    map.push(0..text.len(), 0, 0);
    let paragraph = ParagraphStyle::initial();
    let runs = [StyledRun {
        text: 0..text.len(),
        style,
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
    let widths = shaped.content_widths();
    let broken = shaper.break_lines(
        &mut shaped,
        &BreakRequest::new(&content, Some(CssPx(width))),
    );
    Broken {
        lines: broken.geometry.lines.len(),
        boxes: broken
            .boxes
            .iter()
            .map(|placed| (placed.id, placed.line, placed.origin.x.0))
            .collect(),
        min: widths.min.0,
        max: widths.max.0,
    }
}

#[test]
fn inline_blocks_separated_by_spaces_wrap_at_a_space() {
    let broken = lay_out(
        "  ",
        &[atomic(0, 24.0), atomic(1, 24.0), atomic(2, 24.0)],
        60.0,
        WrapMode::Wrap,
    );
    assert_eq!(broken.lines, 2);
    assert_eq!(broken.of(1), (0, 0.0));
    assert_eq!(broken.of(2).0, 0);
    assert_eq!(broken.of(3), (1, 0.0));
}

#[test]
fn inline_blocks_with_nothing_between_them_wrap_between_two_of_them() {
    let broken = lay_out(
        "",
        &[atomic(0, 24.0), atomic(0, 24.0), atomic(0, 24.0)],
        60.0,
        WrapMode::Wrap,
    );
    assert_eq!(broken.lines, 2);
    assert_eq!(broken.of(2), (0, 24.0));
    assert_eq!(broken.of(3), (1, 0.0));
}

#[test]
fn a_word_after_an_inline_block_wraps_below_it() {
    let broken = lay_out(" wordword", &[atomic(0, 24.0)], 60.0, WrapMode::Wrap);
    assert_eq!(broken.lines, 2);
    assert_eq!(broken.of(1), (0, 0.0));
}

#[test]
fn an_inline_block_after_a_word_wraps_below_it() {
    let broken = lay_out("wordword ", &[atomic(9, 24.0)], 60.0, WrapMode::Wrap);
    assert_eq!(broken.lines, 2);
    assert_eq!(broken.of(1), (1, 0.0));
}

#[test]
fn nowrap_keeps_every_inline_block_on_one_line() {
    let broken = lay_out(
        "  ",
        &[atomic(0, 24.0), atomic(1, 24.0), atomic(2, 24.0)],
        60.0,
        WrapMode::NoWrap,
    );
    assert_eq!(broken.lines, 1);
    for id in 1..=3 {
        assert_eq!(broken.of(id).0, 0, "box {id}");
    }
}

#[test]
fn an_inline_block_wider_than_the_line_overflows_on_a_line_of_its_own() {
    let broken = lay_out(
        "  ",
        &[atomic(0, 24.0), atomic(1, 100.0), atomic(2, 24.0)],
        60.0,
        WrapMode::Wrap,
    );
    assert_eq!(broken.lines, 3);
    assert_eq!(broken.of(1), (0, 0.0));
    assert_eq!(broken.of(2), (1, 0.0));
    assert_eq!(broken.of(3).0, 2);
}

#[test]
fn a_bar_of_buttons_wraps_two_to_a_line() {
    let buttons: Vec<Spec> = (0..5).map(|at| atomic(at, 50.0)).collect();
    let broken = lay_out("    ", &buttons, 120.0, WrapMode::Wrap);
    assert_eq!(broken.lines, 3);
    let lines: Vec<usize> = (1..=5).map(|id| broken.of(id).0).collect();
    assert_eq!(lines, vec![0, 0, 1, 1, 2]);
    assert_eq!(broken.of(3).1, 0.0);
    assert_eq!(broken.of(5).1, 0.0);
}

#[test]
fn a_word_split_across_an_inline_box_does_not_break_at_its_edges() {
    // `word<span>word</span>word`: the span's edges are zero-width boxes at 4 and 8. The engine
    // broke at both, giving three lines.
    let broken = lay_out(
        "wordwordword",
        &[edge(4, 0.0), edge(8, 0.0)],
        60.0,
        WrapMode::Wrap,
    );
    assert_eq!(broken.lines, 1, "no break inside the word");
    assert_eq!(
        broken.min, broken.max,
        "the whole word is the narrowest the paragraph gets"
    );
}

#[test]
fn a_padded_edge_widens_its_word_and_still_does_not_break() {
    // `ab<span style="padding-left: 4px">wordwordword</span>`.
    let broken = lay_out(
        "abwordwordword",
        &[edge(2, 4.0), edge(14, 0.0)],
        60.0,
        WrapMode::Wrap,
    );
    assert_eq!(broken.lines, 1);
    assert_eq!(broken.min, broken.max, "one word, edge included");
}

#[test]
fn a_space_before_an_inline_box_is_where_the_line_breaks() {
    // `ab <span>wordwordword</span>`: two lines, broken at the space and not inside the span.
    let broken = lay_out(
        "ab wordwordword",
        &[edge(3, 0.0), edge(15, 0.0)],
        60.0,
        WrapMode::Wrap,
    );
    assert_eq!(broken.lines, 2);
}
