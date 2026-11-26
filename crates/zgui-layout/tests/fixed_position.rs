//! A fixed box is positioned against the initial containing block, not the nearest positioned
//! ancestor.
//!
//! An absolutely positioned box resolves its insets against the nearest positioned ancestor; a
//! fixed one against the viewport, whose box is the initial containing block, whatever is
//! positioned in between. CodeTracer's status bar is `position: fixed; bottom: 0` inside a
//! `position: relative` row of no height: placed against that row it sat above the window's top.

mod support;

use support::{Element, Fixture, find, lay_out, measurer};

fn status_bar(position: &str) -> Fixture {
    Fixture::new(
        Element::new("root").children(vec![
            Element::new("status")
                .classes(&["status"])
                .children(vec![Element::new("bar").classes(&["bar"])]),
        ]),
        &format!(
            "root {{ display: block; height: 300px }}
             status {{ display: block; position: relative; height: 0 }}
             bar {{ display: block; position: {position}; bottom: 0; left: 0; width: 100%;
                    height: 32px }}"
        ),
    )
}

#[test]
fn a_fixed_box_sits_at_the_bottom_of_the_viewport() {
    let fixture = status_bar("fixed");
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);
    let (x, y, width, height) = find::rect(&store, &fixture, "bar");
    assert_eq!((x, y, width, height), (0.0, 268.0, 400.0, 32.0));
}

#[test]
fn an_absolute_box_sits_against_its_positioned_ancestor() {
    let fixture = status_bar("absolute");
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);
    let (_, y, _, height) = find::rect(&store, &fixture, "bar");
    assert_eq!(
        (y, height),
        (-32.0, 32.0),
        "the row has no height, so the bar ends at its top"
    );
}
