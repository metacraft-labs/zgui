//! A `vector` carrying an SVG document is sized the way a browser sizes an inline `<svg>`.
//!
//! The document is the element's content, and its root `<svg>` element gives that content natural
//! dimensions: a ratio from `viewBox`, a size from absolute `width` and `height` attributes, and
//! the CSS default object size, 300 x 150, on an axis nothing else resolves (SVG 2 §8.2; CSS
//! Images 3 §4 and §4.3). Layout then sizes the box as replaced content.
//!
//! Every expected number was measured in Chromium 147.0.7727.137, headless, with
//! `getBoundingClientRect()` on the equivalent inline `<svg>` at `font-size: 16px`, inside a
//! `display: flex; align-items: center` row and inside a block. Chromium lays out in 1/64 px
//! units, so it reports 18 * 16 / 17 = 16.94 as 16.9375; the comparisons allow that.
//!
//! The measurer is zgui's production one for replaced content (`NaturalSize`, behind the
//! deterministic test shaper); no stand-in decides any size here.

mod support;

use support::find::{rect, with_class};
use support::mono::MonoShaper;
use support::{Element, Fixture};
use zgui_dom::NodeIndex;
use zgui_layout::FormattingContext;
use zgui_layout::style::DeviceStyle;
use zgui_layout::text::Paragraphs;
use zgui_layout::tree::LayoutTree;
use zgui_layout::tree::store::LayoutStore;
use zgui_vocab::prop::drawing;
use zgui_vocab::{PropKey, PropValue};

/// How far a size may be from Chromium's: one of its layout units.
const CHROMIUM_UNIT: f32 = 1.0 / 64.0;

/// A row and a block, each holding one `vector` with class `mark`, and the given style.
fn holder(container: &'static str, css: &str) -> Fixture {
    Fixture::new(
        Element::new("root").children(vec![
            Element::new(container)
                .classes(&["holder"])
                .children(vec![Element::new("vector").classes(&["mark"])]),
        ]),
        &format!(
            "root {{ display: block; width: 1000px; font-size: 16px }}
             row {{ display: flex; align-items: center }}
             block {{ display: block }}
             {css}"
        ),
    )
}

/// Sets `source` as the document of the element with class `mark`.
fn draw(fixture: &mut Fixture, source: &str) -> NodeIndex {
    let mark = with_class(fixture, "mark");
    let source = source.to_owned();
    fixture.edit_and_restyle(move |edit| {
        edit.set_property(
            mark,
            PropKey::new(drawing::DOCUMENT),
            Some(PropValue::from(source.as_str())),
        );
    });
    mark
}

/// Lays `store` out with the production measurer for replaced content.
fn lay_out(store: &mut LayoutStore) {
    let mut content = Paragraphs::new(MonoShaper::default());
    let mut tree = LayoutTree::new(store, &mut content, DeviceStyle::default());
    assert!(tree.layout_root(cephal::Size {
        width: 1000.0,
        height: 800.0
    }));
}

/// The mark's border-box size in a fresh layout of `fixture`.
fn mark_size(fixture: &Fixture) -> (f32, f32) {
    let mut store = fixture.box_tree();
    lay_out(&mut store);
    let (_, _, width, height) = rect(&store, fixture, "mark");
    (width, height)
}

/// Asserts `actual` is Chromium's `expected` to within one of its layout units.
fn assert_near(actual: (f32, f32), expected: (f32, f32), what: &str) {
    assert!(
        (actual.0 - expected.0).abs() <= CHROMIUM_UNIT
            && (actual.1 - expected.1).abs() <= CHROMIUM_UNIT,
        "{what}: zgui {actual:?}, Chromium {expected:?}"
    );
}

/// The toolbar mark: `<svg viewBox="0 0 18 17">` styled `height: 1em`, width left `auto`.
const TOOLBAR_MARK: &str =
    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 17"><path d="M0 0h18v17z"/></svg>"#;

#[test]
fn a_height_and_a_view_box_give_the_width_the_ratio_implies() {
    // Chromium: 16.9375 x 16 in a flex row and in a block. Before this, zgui gave 0 x 16 in the
    // row: the vector had no content size, and the toolbar buttons around it collapsed.
    for container in ["row", "block"] {
        let mut fixture = holder(container, ".mark { height: 1em }");
        draw(&mut fixture, TOOLBAR_MARK);
        assert_near(
            mark_size(&fixture),
            (18.0 * 16.0 / 17.0, 16.0),
            &format!("viewBox 18x17, height 1em, in a {container}"),
        );
    }
}

#[test]
fn a_square_view_box_gives_a_square_mark() {
    // Chromium: `<svg viewBox="0 0 16 16" style="height: 1em">` is 16 x 16.
    let mut fixture = holder("row", ".mark { height: 1em }");
    draw(
        &mut fixture,
        r#"<svg viewBox="0 0 16 16"><path d="M0 0h16v16z"/></svg>"#,
    );
    assert_near(
        mark_size(&fixture),
        (16.0, 16.0),
        "viewBox 16x16, height 1em",
    );
}

#[test]
fn width_and_height_attributes_are_the_size_when_css_gives_none() {
    // Chromium: `<svg width="24" height="10">` is 24 x 10 in a row and in a block, with or
    // without a viewBox of another ratio; `<svg width="12pt" height="1in">` is 16 x 96.
    for container in ["row", "block"] {
        for source in [
            r#"<svg width="24" height="10"></svg>"#,
            r#"<svg width="24" height="10" viewBox="0 0 18 17"></svg>"#,
        ] {
            let mut fixture = holder(container, "");
            draw(&mut fixture, source);
            assert_near(mark_size(&fixture), (24.0, 10.0), source);
        }
    }
    let mut fixture = holder("block", "");
    draw(&mut fixture, r#"<svg width="12pt" height="1in"></svg>"#);
    assert_near(mark_size(&fixture), (16.0, 96.0), "absolute units");
}

#[test]
fn one_attribute_and_a_view_box_give_the_other_through_the_ratio() {
    // Chromium: `<svg width="36" viewBox="0 0 18 17">` is 36 x 34.
    let mut fixture = holder("block", "");
    draw(
        &mut fixture,
        r#"<svg width="36" viewBox="0 0 18 17"></svg>"#,
    );
    assert_near(mark_size(&fixture), (36.0, 34.0), "width 36, viewBox 18x17");
}

#[test]
fn a_document_with_no_size_and_no_ratio_takes_the_default_object_size() {
    // Chromium: a bare `<svg>` is 300 x 150 in a row and in a block; one styled `height: 16px`
    // with no usable viewBox is 300 x 16; `<svg width="24">` is 24 x 150 and `<svg height="24">`
    // is 300 x 24.
    for container in ["row", "block"] {
        let mut fixture = holder(container, "");
        draw(&mut fixture, "<svg></svg>");
        assert_near(mark_size(&fixture), (300.0, 150.0), container);
    }
    let mut fixture = holder("row", ".mark { height: 16px }");
    draw(&mut fixture, r#"<svg viewBox="0 0 0 17"></svg>"#);
    assert_near(mark_size(&fixture), (300.0, 16.0), "degenerate viewBox");

    let mut fixture = holder("block", "");
    draw(&mut fixture, r#"<svg width="24"></svg>"#);
    assert_near(mark_size(&fixture), (24.0, 150.0), "width only");

    let mut fixture = holder("block", "");
    draw(&mut fixture, r#"<svg height="24"></svg>"#);
    assert_near(mark_size(&fixture), (300.0, 24.0), "height only");
}

#[test]
fn a_vector_drawing_outlines_keeps_the_box_its_style_gives_it() {
    // Outlines in the element's own coordinates (a chart mark) are not an SVG document and have
    // no natural size: the box is an ordinary one, as before.
    let mut fixture = holder("block", ".mark { height: 10px }");
    let mark = with_class(&fixture, "mark");
    fixture.edit_and_restyle(|edit| {
        edit.set_property(
            mark,
            PropKey::new(drawing::PATHS),
            Some(PropValue::from("M0 0 L8 0 L8 8 Z")),
        );
    });
    let mut store = fixture.box_tree();
    lay_out(&mut store);
    let key = fixture.document.store().key_of(mark);
    let box_ = store.boxes_of(key)[0];
    assert_ne!(store.node(box_).fc, FormattingContext::Replaced);
    let (_, _, width, height) = rect(&store, &fixture, "mark");
    assert_eq!(
        (width, height),
        (0.0, 10.0),
        "an element with no content and no natural size: no width, its CSS height, as before"
    );
}

#[test]
fn a_new_view_box_resizes_the_mark_on_the_next_frame() {
    // The document is rewritten after every change inside an inline `<svg>`, so its root can
    // change under a box that already exists; the box follows it.
    let mut fixture = holder("row", ".mark { height: 16px }");
    let mark = draw(&mut fixture, r#"<svg viewBox="0 0 16 16"></svg>"#);
    let mut store = fixture.box_tree();
    lay_out(&mut store);
    assert_near(
        {
            let (_, _, w, h) = rect(&store, &fixture, "mark");
            (w, h)
        },
        (16.0, 16.0),
        "before",
    );

    fixture.edit_and_sync_boxes(&mut store, |edit| {
        edit.set_property(
            mark,
            PropKey::new(drawing::DOCUMENT),
            Some(PropValue::from(r#"<svg viewBox="0 0 32 16"></svg>"#)),
        );
    });
    lay_out(&mut store);
    let (_, _, width, height) = rect(&store, &fixture, "mark");
    assert_near(
        (width, height),
        (32.0, 16.0),
        "after the viewBox doubled its width",
    );
}
