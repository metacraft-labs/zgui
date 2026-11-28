//! Lines break around inline blocks as browsers break them, and never at the edge of an inline box
//! inside a word.
//!
//! CSS Text 3 §5.1 treats an atomic inline as an ideographic character for line breaking: a line
//! may break before and after one whether or not white space touches it. CSS 2.1 §16.6 and CSS
//! Text 3 §3 leave whether a line wraps at all to `white-space`. An element boundary inside a word
//! is not an opportunity, which is why the edges of an inline box are glued to their text.
//!
//! Chromium 141 (headless, 16px sans-serif) measures these, relative to a 60px-wide block of 24px
//! inline blocks with `vertical-align: top` and 24px lines:
//!
//! | markup | Chromium 141 |
//! | --- | --- |
//! | `[24] [24] [24]` | block 48 tall; boxes at (0, 0), (28.45, 0), (0, 24) |
//! | `[24][24][24]` | block 48 tall; boxes at (0, 0), (24, 0), (0, 24) |
//! | `[24] wordword` | block 48 tall; the box at (0, 0) |
//! | `wordword [24]` | block 48 tall; the box at (0, 24) |
//! | `[24] [24] [24]` under `white-space: nowrap` | block 24 tall; boxes at x 0, 28.45, 56.91 |
//! | `[24] [100] [24]` | block 72 tall; the wide box at (0, 24), the last at (0, 48) |
//! | five 50px buttons in a 120px bar | bar 72 tall; buttons at (0, 0), (54.45, 0), (0, 24), (54.45, 24), (0, 48) |
//! | `word<span>word</span>word` | block 24 tall |
//! | `ab<span style="padding-left: 4px">wordwordword</span>` | block 24 tall |
//!
//! The shaper behind these tests advances 8px a character, so a space is 8px rather than
//! Chromium's 4.45px, and the x of a box after a space on its line is the shaper's. A space at the
//! end of a line hangs there (CSS Text 3 §4.1.2), so a wrapped line starts at x 0 as in Chromium;
//! every line, every height and every x the space does not move is Chromium's. No mocks: a real document, cascade,
//! box tree and layout over the deterministic shaper.

mod support;

use support::{Element, Fixture, lay_out, measurer};
use zgui_layout::tree::store::LayoutStore;

const CSS: &str = "root { display: block; width: 400px }
     block { display: block; width: 60px; line-height: 24px }
     bar { display: block; width: 120px; line-height: 24px }
     icon { display: inline-block; width: 24px; height: 24px; vertical-align: top }
     wide { display: inline-block; width: 100px; height: 24px; vertical-align: top }
     button { display: inline-block; width: 50px; height: 24px; vertical-align: top }
     span { display: inline }
     .padded { padding-left: 4px }";

/// A space between two inline-level boxes.
fn space() -> Element {
    Element::new("span").text(" ")
}

/// An inline block carrying one class.
fn boxed(name: &'static str, class: &'static [&'static str]) -> Element {
    Element::new(name).classes(class)
}

/// Lays `children` out in a `container` element, under `CSS` and `extra`.
fn laid_out(
    container: &'static str,
    children: Vec<Element>,
    extra: &str,
) -> (Fixture, LayoutStore) {
    let fixture = Fixture::new(
        Element::new("root").children(vec![
            Element::new(container)
                .classes(&["container"])
                .children(children),
        ]),
        &format!("{CSS}\n{extra}"),
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 600.0);
    (fixture, store)
}

/// The `(x, y)` of the element with `class`, within the container's lines.
fn at(store: &LayoutStore, fixture: &Fixture, class: &str) -> (f32, f32) {
    let (x, y, _, _) = support::find::rect(store, fixture, class);
    (x, y)
}

/// The container's height.
fn height(store: &LayoutStore, fixture: &Fixture) -> f32 {
    support::find::rect(store, fixture, "container").3
}

fn three_icons(between: bool) -> Vec<Element> {
    let mut children = vec![boxed("icon", &["one"])];
    if between {
        children.push(space());
    }
    children.push(boxed("icon", &["two"]));
    if between {
        children.push(space());
    }
    children.push(boxed("icon", &["three"]));
    children
}

#[test]
fn inline_blocks_separated_by_spaces_wrap_at_a_space() {
    let (fixture, store) = laid_out("block", three_icons(true), "");
    assert_eq!(height(&store, &fixture), 48.0);
    assert_eq!(at(&store, &fixture, "one"), (0.0, 0.0));
    assert_eq!(
        at(&store, &fixture, "two"),
        (32.0, 0.0),
        "after one 8px space"
    );
    assert_eq!(at(&store, &fixture, "three"), (0.0, 24.0));
}

#[test]
fn inline_blocks_with_nothing_between_them_wrap_between_two_of_them() {
    let (fixture, store) = laid_out("block", three_icons(false), "");
    assert_eq!(height(&store, &fixture), 48.0);
    assert_eq!(at(&store, &fixture, "two"), (24.0, 0.0));
    assert_eq!(at(&store, &fixture, "three"), (0.0, 24.0));
}

#[test]
fn a_word_after_an_inline_block_wraps_below_it() {
    let (fixture, store) = laid_out(
        "block",
        vec![
            boxed("icon", &["one"]),
            Element::new("span").text(" wordword"),
        ],
        "",
    );
    assert_eq!(height(&store, &fixture), 48.0);
    assert_eq!(at(&store, &fixture, "one"), (0.0, 0.0));
}

#[test]
fn an_inline_block_after_a_word_wraps_below_it() {
    let (fixture, store) = laid_out(
        "block",
        vec![
            Element::new("span").text("wordword "),
            boxed("icon", &["one"]),
        ],
        "",
    );
    assert_eq!(height(&store, &fixture), 48.0);
    assert_eq!(at(&store, &fixture, "one"), (0.0, 24.0));
}

#[test]
fn nowrap_keeps_every_inline_block_on_one_line() {
    let (fixture, store) = laid_out("block", three_icons(true), "block { white-space: nowrap }");
    assert_eq!(height(&store, &fixture), 24.0);
    for class in ["one", "two", "three"] {
        assert_eq!(at(&store, &fixture, class).1, 0.0, "{class}");
    }
}

#[test]
fn an_inline_block_wider_than_the_line_overflows_on_a_line_of_its_own() {
    let (fixture, store) = laid_out(
        "block",
        vec![
            boxed("icon", &["one"]),
            space(),
            boxed("wide", &["two"]),
            space(),
            boxed("icon", &["three"]),
        ],
        "",
    );
    assert_eq!(height(&store, &fixture), 72.0);
    assert_eq!(at(&store, &fixture, "one"), (0.0, 0.0));
    assert_eq!(at(&store, &fixture, "two"), (0.0, 24.0));
    assert_eq!(at(&store, &fixture, "three"), (0.0, 48.0));
}

#[test]
fn a_bar_of_buttons_wraps_two_to_a_line() {
    let classes: [&'static [&'static str]; 5] = [&["b1"], &["b2"], &["b3"], &["b4"], &["b5"]];
    let mut children = Vec::new();
    for (index, class) in classes.into_iter().enumerate() {
        if index > 0 {
            children.push(space());
        }
        children.push(boxed("button", class));
    }
    let (fixture, store) = laid_out("bar", children, "");
    assert_eq!(height(&store, &fixture), 72.0);
    assert_eq!(at(&store, &fixture, "b1"), (0.0, 0.0));
    assert_eq!(
        at(&store, &fixture, "b2"),
        (58.0, 0.0),
        "after one 8px space"
    );
    assert_eq!(at(&store, &fixture, "b3"), (0.0, 24.0));
    assert_eq!(
        at(&store, &fixture, "b4"),
        (58.0, 24.0),
        "after one 8px space"
    );
    assert_eq!(at(&store, &fixture, "b5"), (0.0, 48.0));
}

#[test]
fn a_word_split_across_an_inline_box_does_not_break_at_its_edges() {
    let (fixture, store) = laid_out(
        "block",
        vec![
            Element::new("span").text("word"),
            Element::new("span").text("word"),
            Element::new("span").text("word"),
        ],
        "",
    );
    assert_eq!(height(&store, &fixture), 24.0, "one line, overflowing");
}

#[test]
fn a_padded_inline_box_inside_a_word_does_not_break_it() {
    let (fixture, store) = laid_out(
        "block",
        vec![
            Element::new("span").text("ab"),
            Element::new("span")
                .classes(&["padded"])
                .text("wordwordword"),
        ],
        "",
    );
    assert_eq!(height(&store, &fixture), 24.0, "one line, overflowing");
}
