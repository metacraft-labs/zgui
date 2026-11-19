//! A paint-only value set through the node-tree seam is repainted in the next frame without a
//! restyle or a layout.

mod support;

use zgui_interned::ElementName;
use zgui_profile::{Counter, counter};
use zgui_view::{BuildCx, Dom, IntoView, PaintProperty, View};

const CSS: &str = "root { display: block; width: 400px; height: 300px }
                   box { display: block; width: 100px; height: 40px; background-color: red }";

#[test]
fn a_paint_only_value_repaints_without_restyling_or_laying_out() {
    let mut harness = support::app(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(zgui_elements::column().class("root").into_view().build(cx))
    });
    harness.settle(8);
    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let el = dom.create_element(ElementName::new("box"));
    dom.insert(dom.root_node(), el, None);
    harness.settle(8);

    // The first value is a declaration; the ones after it are paint-only.
    dom.set_paint(el, PaintProperty::BackgroundColor, Some("rgb(0, 64, 0)"));
    harness.settle(8);
    let relaid = counter::get(Counter::NodesRelaidOut);
    dom.set_paint(el, PaintProperty::BackgroundColor, Some("rgb(0, 128, 0)"));
    assert_eq!(harness.settle(8), 1, "the value asked for no frame");
    let report = harness.app().windows()[0]
        .last_frame_report()
        .expect("a frame ran");
    assert_eq!(
        report.restyled, 0,
        "a paint-only value restyled the element"
    );
    assert_eq!(
        counter::get(Counter::NodesRelaidOut),
        relaid,
        "it laid something out"
    );

    // The same property through the cascade restyles, which is what the setter saves.
    dom.set_style_property(el, "background-color", Some("rgb(0, 0, 128)"));
    harness.settle(8);
    let report = harness.app().windows()[0]
        .last_frame_report()
        .expect("a frame ran");
    assert_eq!(report.restyled, 1);
}
