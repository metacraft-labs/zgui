//! A change that reaches one row of a long list visits a number of nodes bounded by the change,
//! not by the length of the list.
//!
//! The emit walk asks a box's children whether the damage reaches them. For a list it asks only
//! the rows a damaged band can reach, found in an index of where the rows paint, and the index is
//! rebuilt when the layout says a row's ink moved. The walk checks the index against the question
//! it narrows in every debug build (`zgui_paint`'s child index), so each change below is also a
//! test that no row the damage reaches was dropped.

mod support;

use zgui_interned::{ClassName, ElementName};
use zgui_profile::{Counter, counter};
use zgui_view::{BuildCx, Dom, IntoView, ScrollBehavior, ScrollTarget, View, ViewHost};

/// Held by each test for its whole length: the work counters are process-wide, and a test run
/// beside another would count the other's work as its own.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Takes [`SERIAL`], also after a test that held it failed.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|held| held.into_inner())
}

const CSS: &str = "root { display: block; width: 400px; height: 300px }
                   scroll { display: block; height: 300px; overflow: auto }
                   row { display: flex; height: 18px }";

/// A window whose scroll container holds `rows` rows, each one text node, and the handles of the
/// container and of each row's text.
fn list(
    rows: usize,
) -> (
    zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    zgui_view::NodeId,
    Vec<zgui_view::NodeId>,
) {
    let mut harness = support::app_with_text(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(zgui_elements::column().class("root").into_view().build(cx))
    });
    harness.settle(8);
    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let pane = dom.create_element(ElementName::new("scroll"));
    dom.insert(dom.root_node(), pane, None);
    let mut texts = Vec::with_capacity(rows);
    for index in 0..rows {
        let row = dom.create_element(ElementName::new("row"));
        dom.set_classes(row, &[ClassName::new("r")]);
        let text = dom.create_text(&format!("row {index}"));
        dom.insert(row, text, None);
        dom.insert(pane, row, None);
        texts.push(text);
    }
    harness.settle(16);
    (harness, pane, texts)
}

/// The nodes the next settle visits, after `change` runs.
fn visits(
    harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    change: impl FnOnce(&zgui_view_dom::DocumentDom),
) -> u64 {
    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let before = counter::get(Counter::NodesVisited);
    change(&dom);
    assert_eq!(harness.settle(16), 1, "the change ran no frame");
    counter::get(Counter::NodesVisited) - before
}

#[test]
fn a_label_change_in_a_long_list_visits_nodes_bounded_by_the_change() {
    let _serial = serial();
    let (mut short, _, short_texts) = list(300);
    let (mut long, pane, long_texts) = list(3_000);

    // The first change after building can still find indexes to build; the second is the one
    // measured.
    visits(&mut short, |dom| dom.set_text(short_texts[5], "warm"));
    visits(&mut long, |dom| dom.set_text(long_texts[5], "warm"));
    let at_300 = visits(&mut short, |dom| dom.set_text(short_texts[5], "changed"));
    let at_3000 = visits(&mut long, |dom| dom.set_text(long_texts[5], "changed"));
    assert!(
        at_3000 < 200,
        "one label in 3,000 rows visited {at_3000} nodes, so the walk still asks every row"
    );
    assert!(
        at_3000 <= at_300 + 8,
        "the visits grew with the list: {at_300} at 300 rows, {at_3000} at 3,000"
    );

    // Scrolled: the rows' ink is measured in the content space, which moved, and the index is
    // asked through the new offset.
    let host = std::rc::Rc::clone(long.app().windows()[0].host());
    host.scroll_to(
        pane,
        ScrollTarget::Offset(zgui_geom::Point::new(
            zgui_geom::DevicePx(0.0),
            zgui_geom::DevicePx(18.0 * 1_500.0),
        )),
        ScrollBehavior::Instant,
    );
    long.settle(16);
    let scrolled = visits(&mut long, |dom| dom.set_text(long_texts[1_505], "far down"));
    assert!(
        scrolled < 200,
        "a change after scrolling visited {scrolled} nodes"
    );

    // A row inserted in the middle moves every row below it; the index is rebuilt for the frame
    // after, and the walk checks it against every row.
    visits(&mut long, |dom| {
        let row = dom.create_element(ElementName::new("row"));
        let text = dom.create_text("inserted");
        dom.insert(row, text, None);
        let before = dom.parent(long_texts[1_500]).expect("a row holds the text");
        dom.insert(pane, row, Some(before));
    });
    let after_insert = visits(&mut long, |dom| dom.set_text(long_texts[1_506], "again"));
    assert!(
        after_insert < 200,
        "a change after an insertion visited {after_insert} nodes"
    );

    // A row removed, and a row that grew, likewise.
    visits(&mut long, |dom| {
        dom.detach(dom.parent(long_texts[1_502]).expect("a row"))
    });
    visits(&mut long, |dom| {
        let row = dom.parent(long_texts[1_503]).expect("a row");
        dom.set_style_property(row, "height", Some("60px"));
    });
    let after_resize = visits(&mut long, |dom| dom.set_text(long_texts[1_504], "below"));
    assert!(
        after_resize < 200,
        "a change after a resize visited {after_resize} nodes"
    );
}

/// What the next settle costs the layout and fragment passes, after `change` runs: the boxes
/// laid out again and the children the fragment pass asked.
fn passes_work(
    harness: &mut zgui_platform_headless::Harness<zgui_runtime::Runtime>,
    change: impl FnOnce(&zgui_view_dom::DocumentDom),
) -> (u64, u64) {
    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let relaid = counter::get(Counter::NodesRelaidOut);
    let asked = counter::get(Counter::FragmentChildrenAsked);
    change(&dom);
    assert_eq!(harness.settle(16), 1, "the change ran no frame");
    (
        counter::get(Counter::NodesRelaidOut) - relaid,
        counter::get(Counter::FragmentChildrenAsked) - asked,
    )
}

#[test]
fn a_label_change_in_a_long_list_lays_out_and_composes_work_bounded_by_the_change() {
    let _serial = serial();
    let (mut short, _, short_texts) = list(300);
    let (mut long, pane, long_texts) = list(3_000);
    // The first change after building records what the next one reads; the later ones are
    // measured, a label growing and then shrinking inside its row.
    passes_work(&mut short, |dom| dom.set_text(short_texts[150], "warm"));
    passes_work(&mut long, |dom| dom.set_text(long_texts[1_500], "warm"));
    for text in ["a longer label than before", "short"] {
        let (relaid_300, asked_300) =
            passes_work(&mut short, |dom| dom.set_text(short_texts[150], text));
        let (relaid_3000, asked_3000) =
            passes_work(&mut long, |dom| dom.set_text(long_texts[1_500], text));
        assert!(
            relaid_3000 <= relaid_300 && relaid_3000 <= 8,
            "laying out one label relaid {relaid_3000} boxes in 3,000 rows and {relaid_300} in 300"
        );
        assert!(
            asked_3000 < 64 && asked_3000 <= asked_300,
            "composing one label asked {asked_3000} children in 3,000 rows and {asked_300} in 300"
        );
    }

    // After an insertion and a removal the next change is again bounded: the record of the rows is
    // made again by the frame that folds every row.
    passes_work(&mut long, |dom| {
        let row = dom.create_element(ElementName::new("row"));
        dom.insert(row, dom.create_text("inserted"), None);
        let before = dom.parent(long_texts[1_000]).expect("a row holds the text");
        dom.insert(pane, row, Some(before));
    });
    passes_work(&mut long, |dom| {
        dom.detach(dom.parent(long_texts[2_000]).expect("a row"))
    });
    passes_work(&mut long, |dom| dom.set_text(long_texts[1_500], "rebuilt"));
    let (_, asked) = passes_work(&mut long, |dom| dom.set_text(long_texts[1_500], "after"));
    assert!(
        asked < 64,
        "a change after an insertion asked {asked} children"
    );
}
