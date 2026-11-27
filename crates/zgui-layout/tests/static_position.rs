//! An absolutely positioned box with `auto` insets sits where it was written, and the line it is
//! written beside is as tall as Chromium makes it.
//!
//! CSS places a box whose insets in an axis are both `auto` at its static position: where its
//! margin box would have started had it been in flow (CSS 2.1 §10.3.7 and §10.6.4; css-position-3
//! §4.6). The box tree holds such a box as a child of its containing block, after that block's
//! in-flow children, so the block's own layout put every one of them at its bottom.
//!
//! The fixtures are CodeTracer's file tree, which is jsTree markup: a `position: relative` list
//! holds the rows, and each row starts with a full-width `top: auto` highlight before two
//! `vertical-align: top` inline blocks. Every number asserted is what Chromium 141 (headless, the
//! same markup and rules) reports through `getBoundingClientRect`, relative to the list:
//!
//! | case | Chromium 141 |
//! | --- | --- |
//! | highlight first in the row | row 0..24, highlight y 0 |
//! | highlight after the inline blocks | highlight y 24 |
//! | nested rows | rows at 24, 48, 72, each highlight on its own row; list 96 tall |
//! | 24-tall `top` inline blocks under a 16-tall strut | row 24 tall |
//! | highlight first in a row with 5px top padding, `left: auto` | highlight at (8, 5) |
//! | after a 10-tall block with a 6px bottom margin | highlight y 16 |
//! | after text wrapped onto a second line | row 48 tall, highlight y 48 |
//!
//! No mocks: a real document, cascade, box tree and layout. The text shaper is the deterministic
//! one every layout test uses; no assertion depends on a glyph's size.

mod support;

use support::{Element, Fixture, lay_out, measurer};
use zgui_layout::tree::store::LayoutStore;

/// The border box of the first box `class` generated, relative to the root's, summed over its
/// layout parents.
fn rect(store: &LayoutStore, fixture: &Fixture, class: &str) -> (f32, f32, f32, f32) {
    let key = fixture
        .document
        .store()
        .key_of(support::find::with_class(fixture, class));
    let mut at = *store.boxes_of(key).first().expect("the element generated a box");
    let border = store.layout_of(at).expect("laid out").border_box();
    let (mut x, mut y) = (border.origin.x.0, border.origin.y.0);
    while let Some(parent) = store.node(at).parent {
        let origin = store.layout_of(parent).expect("laid out").border_box().origin;
        x += origin.x.0;
        y += origin.y.0;
        at = parent;
    }
    (x, y, border.size.width.0, border.size.height.0)
}

/// jsTree's rules, as `jstree_default.css` and CodeTracer's overrides resolve them at 16px.
const CSS: &str = "root { display: block; width: 400px }
     list { position: relative; display: block }
     children { display: block }
     node { display: block; white-space: nowrap; line-height: 24px; min-height: 24px;
            margin-left: 8px }
     row { display: block; position: absolute; left: 0; width: 100%; height: 24px }
     icon { display: inline-block; width: 24px; height: 24px; vertical-align: top }
     anchor { display: inline-block; height: 24px; line-height: 24px; vertical-align: top }";

/// One jsTree row: an expander icon and an anchor holding an icon and a label.
fn row_content(label: &'static str) -> Vec<Element> {
    vec![
        Element::new("icon"),
        Element::new("anchor").children(vec![
            Element::new("icon"),
            Element::new("label").text(label),
        ]),
    ]
}

/// A list holding one row, laid out at 400 by 600.
fn one_row(node: Element, css: &str) -> (Fixture, LayoutStore) {
    let fixture = Fixture::new(
        Element::new("root").children(vec![Element::new("list").classes(&["list"]).children(vec![node])]),
        css,
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 600.0);
    (fixture, store)
}

#[test]
fn a_highlight_written_first_in_a_row_sits_on_that_row() {
    let mut children = vec![Element::new("row").classes(&["highlight"])];
    children.extend(row_content("name"));
    let (fixture, store) = one_row(Element::new("node").classes(&["node"]).children(children), CSS);
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 392.0, 24.0));
    assert_eq!(
        rect(&store, &fixture, "highlight"),
        (0.0, 0.0, 400.0, 24.0),
        "the static position is the top of the row's content box, not the bottom of the list"
    );
}

#[test]
fn a_highlight_written_after_the_inline_content_sits_below_its_line() {
    // An absolutely positioned box is block-level, so had it been in flow it would have started a
    // line of its own below the one the inline blocks are on.
    let mut children = row_content("name");
    children.push(Element::new("row").classes(&["highlight"]));
    let (fixture, store) = one_row(Element::new("node").classes(&["node"]).children(children), CSS);
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 392.0, 24.0));
    assert_eq!(rect(&store, &fixture, "highlight"), (0.0, 24.0, 400.0, 24.0));
}

#[test]
fn every_nested_row_carries_its_own_highlight() {
    fn node(
        class: &'static [&'static str],
        label: &'static str,
        nested: Vec<Element>,
    ) -> Element {
        let mut children = vec![Element::new("row").classes(&class[1..])];
        children.extend(row_content(label));
        if !nested.is_empty() {
            children.push(Element::new("children").children(nested));
        }
        Element::new("node").classes(&class[..1]).children(children)
    }
    let tree = node(
        &["n1", "h1"],
        "one",
        vec![
            node(&["n2", "h2"], "two", vec![node(&["n3", "h3"], "three", vec![])]),
            node(&["n4", "h4"], "four", vec![]),
        ],
    );
    let (fixture, store) = one_row(tree, CSS);
    assert_eq!(rect(&store, &fixture, "list"), (0.0, 0.0, 400.0, 96.0));
    assert_eq!(rect(&store, &fixture, "n1"), (8.0, 0.0, 392.0, 96.0));
    assert_eq!(rect(&store, &fixture, "n2"), (16.0, 24.0, 384.0, 48.0));
    assert_eq!(rect(&store, &fixture, "n3"), (24.0, 48.0, 376.0, 24.0));
    assert_eq!(rect(&store, &fixture, "n4"), (16.0, 72.0, 384.0, 24.0));
    for (class, y) in [("h1", 0.0), ("h2", 24.0), ("h3", 48.0), ("h4", 72.0)] {
        assert_eq!(rect(&store, &fixture, class), (0.0, y, 400.0, 24.0), "{class}");
    }
}

#[test]
fn top_aligned_inline_blocks_do_not_stretch_a_line_with_a_shorter_strut() {
    // The strut is 16 tall; the two inline blocks are 24 tall and aligned with the line's top, so
    // the line is 24. Counting the anchor where it would sit on the baseline — its own baseline
    // 16.8 down, 7.2 of it below — made the line 31.2 tall, and so every jsTree row 31.
    let css = format!("{CSS}\n node {{ line-height: 16px; min-height: 0 }}");
    let (fixture, store) = one_row(
        Element::new("node").classes(&["node"]).children(row_content("name")),
        &css,
    );
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 392.0, 24.0));
}

#[test]
fn top_aligned_inline_blocks_under_an_equal_strut_make_one_strut_tall_line() {
    // jsTree's own case: the strut is 24, the inline blocks are 24, the row is 24.
    let (fixture, store) = one_row(
        Element::new("node").classes(&["node"]).children(row_content("name")),
        &format!("{CSS}\n node {{ min-height: 0 }}"),
    );
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 392.0, 24.0));
}

#[test]
fn an_auto_inline_inset_starts_the_box_at_the_content_edge_of_its_row() {
    let css = format!("{CSS}\n node {{ padding-top: 5px }}\n row {{ left: auto }}");
    let (fixture, store) = one_row(
        Element::new("node").classes(&["node"]).children(vec![
            Element::new("row").classes(&["highlight"]),
            Element::new("icon"),
        ]),
        &css,
    );
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 392.0, 29.0));
    assert_eq!(rect(&store, &fixture, "highlight"), (8.0, 5.0, 400.0, 24.0));
}

#[test]
fn after_a_block_the_box_starts_below_that_block_s_margin() {
    let css = format!(
        "{CSS}\n block {{ display: block; height: 10px; margin-bottom: 6px }}
         next {{ display: block; height: 10px; margin-top: 4px }}"
    );
    let (fixture, store) = one_row(
        Element::new("node").classes(&["node"]).children(vec![
            Element::new("block"),
            Element::new("row").classes(&["highlight"]),
            Element::new("next"),
        ]),
        &css,
    );
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 392.0, 26.0));
    assert_eq!(rect(&store, &fixture, "highlight"), (0.0, 16.0, 400.0, 24.0));
}

#[test]
fn after_text_wrapped_onto_a_second_line_the_box_starts_below_it() {
    // A 60-wide row holds two words wider than itself, so each takes a line of its own. The box
    // written after them starts below the second line.
    let css = format!("{CSS}\n node {{ width: 60px; white-space: normal }}");
    let (fixture, store) = one_row(
        Element::new("node").classes(&["node"]).children(vec![
            Element::new("word").text("wordwordword wordwordword"),
            Element::new("row").classes(&["highlight"]),
        ]),
        &css,
    );
    assert_eq!(rect(&store, &fixture, "node"), (8.0, 0.0, 60.0, 48.0), "two lines");
    assert_eq!(rect(&store, &fixture, "highlight"), (0.0, 48.0, 400.0, 24.0));
}
