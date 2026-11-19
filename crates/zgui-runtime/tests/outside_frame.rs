//! A change made to the document outside the frame loop asks for the frame that shows it.
//!
//! Listeners, timers and effects run inside a frame and are shown by it. Code that runs on its own
//! schedule — a foreign caller driving the document between frames, or work handed over from
//! another thread — runs outside one, and a change it makes is shown only when something asks for
//! a frame. The document asks for it.

mod support;

use zgui_interned::ElementName;
use zgui_view::{BuildCx, Dom, IntoView, View, ViewHost};

/// The sheet every fixture here is styled by.
const CSS: &str = "root { display: block; width: 400px; height: 300px }
                   column { display: block }";

fn window() -> zgui_platform_headless::Harness<zgui_runtime::Runtime> {
    let mut harness = support::app(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(zgui_elements::column().class("root").into_view().build(cx))
    });
    harness.settle(8);
    harness
}

#[test]
fn a_change_made_between_frames_runs_one_frame_and_an_idle_loop_runs_none() {
    let mut harness = window();
    assert_eq!(
        harness.settle(8),
        0,
        "the fixture is not idle before the change"
    );

    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let root = dom.root_node();
    let added = dom.create_element(ElementName::new("column"));
    dom.insert(root, added, None);
    dom.set_classes(added, &[zgui_interned::ClassName::new("late")]);

    assert_eq!(
        harness.settle(8),
        1,
        "a change made outside the frame loop ran no frame, so nothing shows it until something \
         else asks for one; several changes before the frame must also cost one frame, not one each"
    );
    assert!(
        harness.app().windows()[0]
            .host()
            .border_box(added)
            .is_some(),
        "the frame that ran did not lay the new element out"
    );
    assert_eq!(harness.settle(8), 0, "an idle loop ran a frame");
}

#[test]
fn every_change_between_frames_is_shown_however_many_frames_have_run() {
    let mut harness = window();
    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let root = dom.root_node();
    for round in 0..3 {
        let added = dom.create_element(ElementName::new("column"));
        dom.insert(root, added, None);
        assert_eq!(harness.settle(8), 1, "round {round} ran no frame");
    }
}
