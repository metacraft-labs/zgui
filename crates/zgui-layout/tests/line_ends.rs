//! A wrapped line starts at its content, and the edges of inline boxes go to the side of a break
//! their content is on.
//!
//! CSS Text 3 §4.1.2 hangs a space at the end of a line, so the line after a break at a space
//! starts with the next word. CSS Text 3 §5.1 puts a break between two pieces of content, and an
//! inline box's start edge belongs to what follows it, its end edge to what precedes it.
//!
//! Chromium 141 (headless, 16px sans-serif, 24px lines, 24px inline blocks), relative to the block:
//!
//! | markup, block width | Chromium 141 |
//! | --- | --- |
//! | `aaaa bbbb`, 60 | two lines; `b` starts the second at x 0 |
//! | `<span style="padding-right: 6px">[24][24]</span>`, 40 | the second box at (0, 24), the span's second line 30 wide |
//! | `[24]<span style="padding-left: 6px">[24]</span>`, 40 | the second box at (6, 24) |
//!
//! No mocks: a real document, cascade, box tree and layout over the deterministic shaper, which
//! advances 8px a character.

mod support;

use support::{Element, Fixture, lay_out, measurer};
use zgui_layout::tree::store::LayoutStore;

const CSS: &str = "root { display: block; width: 400px }
     block { display: block; line-height: 24px }
     icon { display: inline-block; width: 24px; height: 24px; vertical-align: top }
     span { display: inline }
     .narrow { width: 40px }
     .wide { width: 60px }
     .start { padding-left: 6px }
     .end { padding-right: 6px }";

fn laid_out(class: &'static [&'static str], children: Vec<Element>) -> (Fixture, LayoutStore) {
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new("block").classes(class).children(children),
        ]),
        CSS,
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 600.0);
    (fixture, store)
}

fn at(store: &LayoutStore, fixture: &Fixture, class: &str) -> (f32, f32) {
    let (x, y, _, _) = support::find::rect(store, fixture, class);
    (x, y)
}

#[test]
fn a_line_broken_at_a_space_starts_at_the_next_word() {
    let (_, store) = laid_out(&["wide"], vec![Element::new("span").text("aaaa bbbb")]);
    let lines = support::text::lines(&store);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines[1].text.start, 5,
        "the space hangs at the end of line one, and `b` starts line two"
    );
    assert_eq!(lines[1].width, 32.0, "four characters, no leading space");
}

#[test]
fn an_end_edge_after_an_inline_block_goes_to_the_next_line_with_it() {
    let (fixture, store) = laid_out(
        &["narrow"],
        vec![Element::new("span").classes(&["end"]).children(vec![
            Element::new("icon").classes(&["one"]),
            Element::new("icon").classes(&["two"]),
        ])],
    );
    assert_eq!(at(&store, &fixture, "one"), (0.0, 0.0));
    assert_eq!(at(&store, &fixture, "two"), (0.0, 24.0));
    let lines = support::text::lines(&store);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1].width, 30.0, "the box and the 6px end edge");
}

#[test]
fn a_start_edge_before_an_inline_block_goes_to_the_next_line_with_it() {
    let (fixture, store) = laid_out(
        &["narrow"],
        vec![
            Element::new("icon").classes(&["one"]),
            Element::new("span")
                .classes(&["start"])
                .children(vec![Element::new("icon").classes(&["two"])]),
        ],
    );
    assert_eq!(at(&store, &fixture, "one"), (0.0, 0.0));
    assert_eq!(
        at(&store, &fixture, "two"),
        (6.0, 24.0),
        "after its span's 6px start edge"
    );
}
