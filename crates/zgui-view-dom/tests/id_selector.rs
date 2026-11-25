//! An element's `id`, written through the view seam, is what `#id` selectors match.
//!
//! The document keeps `id` in the node record rather than among the attributes, because that is
//! where the style engine's matching reads it. A view that wrote `id` as an ordinary attribute
//! (IsoNim's `setAttribute("id", …)`, any foreign caller) stored it where nothing matched it: the
//! attribute read back, and every `#id` rule of a sheet written for a browser was silently lost.

mod support;

use std::sync::Arc;

use zgui_css::values::color;
use zgui_elements::control;
use zgui_geom::CssPx;
use zgui_interned::AttrName;
use zgui_style::{SheetOrigin, SheetSource, StyleEngine, Viewport};
use zgui_text::FixedMetrics;
use zgui_view::{Anchor, Dom, IntoView, View};

use crate::support::Window;

fn colour(window: &Window, node: zgui_view::NodeId) -> [u8; 3] {
    let index = window.backend.index_of(node);
    let document = window.document.borrow();
    let style = document
        .node(index)
        .primary_style()
        .expect("the element was styled");
    let [r, g, b, _] = color::to_color(color::current(&style)).to_premultiplied_srgb();
    [
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    ]
}

#[test]
fn an_id_written_as_an_attribute_is_matched_by_an_id_selector_and_reads_back() {
    let window = Window::open();
    let view = control().class("c");
    let mut built = window
        .window
        .with(|| view.into_view().build(&mut window.cx.cx()));
    built.mount(&window.dom, window.root, None);
    let node = built.node();

    let mut engine = {
        let document = window.document.borrow();
        StyleEngine::new(
            &document,
            Arc::new(FixedMetrics::new()),
            Viewport::new(CssPx(800.0), CssPx(600.0)),
        )
    };
    let _sheet = {
        let document = window.document.borrow();
        let (handle, diagnostics) = engine.add_sheet(
            &document,
            SheetOrigin::Author,
            SheetSource::Text(".c { color: rgb(10, 10, 10) } #main { color: rgb(40, 40, 40) }"),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        handle
    };
    let mut restyle = |engine: &mut StyleEngine| {
        let mut document = window.document.borrow_mut();
        engine.restyle(&mut document, None);
    };

    restyle(&mut engine);
    assert_eq!(
        colour(&window, node),
        [10, 10, 10],
        "no id yet: only the class rule"
    );

    window
        .backend
        .set_attribute(node, AttrName::new("id"), Some("main"));
    restyle(&mut engine);
    assert_eq!(
        colour(&window, node),
        [40, 40, 40],
        "the id rule matches the written id"
    );
    assert_eq!(
        Dom::attribute(&*window.backend, node, AttrName::new("id")).as_deref(),
        Some("main"),
        "and the id reads back as the attribute it was written as"
    );

    window
        .backend
        .set_attribute(node, AttrName::new("id"), None);
    restyle(&mut engine);
    assert_eq!(
        colour(&window, node),
        [10, 10, 10],
        "clearing it stops the id rule matching"
    );
    assert_eq!(
        Dom::attribute(&*window.backend, node, AttrName::new("id")),
        None
    );

    window.window.unmount();
}
