//! What building a long list costs the first frame that lays it out, in work that grows with the
//! list and nothing faster.
//!
//! The fragment pass places each newly indexed box between its neighbours in painting order, which
//! needs the next sibling that already indexes something. On the first frame no sibling does, and
//! a pass that looked for one per child read every later sibling for every child: a list of `n`
//! rows read `n²/2` siblings. It reads each one once now.

mod support;

use zgui_interned::ElementName;
use zgui_profile::{Counter, counter};
use zgui_view::{BuildCx, Dom, IntoView, View};

const CSS: &str = "root { display: block; width: 400px; height: 300px }
                   column { display: block }
                   row { display: flex; height: 18px }";

/// The sibling orders the first frame of a list of `rows` rows read.
fn orders_read(rows: usize) -> u64 {
    let mut harness = support::app(CSS, |cx: &mut BuildCx<'_>| {
        Box::new(zgui_elements::column().class("root").into_view().build(cx))
    });
    harness.settle(8);
    let dom = std::rc::Rc::clone(harness.app().windows()[0].dom());
    let list = dom.create_element(ElementName::new("column"));
    for _ in 0..rows {
        let row = dom.create_element(ElementName::new("row"));
        dom.insert(row, dom.create_text("row"), None);
        dom.insert(list, row, None);
    }
    let before = counter::get(Counter::SiblingOrdersRead);
    dom.insert(dom.root_node(), list, None);
    harness.settle(8);
    counter::get(Counter::SiblingOrdersRead) - before
}

#[test]
fn the_first_frame_of_a_list_reads_each_sibling_once() {
    let small = orders_read(500);
    let large = orders_read(4_000);
    assert!(
        large <= 3 * 4_000,
        "the first frame of 4,000 rows read {large} sibling orders, which grows faster than the list"
    );
    assert!(
        large < 10 * small,
        "eight times the rows read {large} sibling orders against {small}"
    );
}
