//! A scroll container's bar is as wide as the sheet's `::-webkit-scrollbar` rule makes it.
//!
//! A sheet written for a browser sizes its bars with `::-webkit-scrollbar`, which this build's
//! style engine does not parse. The rules are carried to the scroll container they style
//! (`zgui_style::sheets::webkit_scrollbar`); this is the consequence layout owes: the content of a
//! container whose bar the sheet narrows, or hides, is laid out that much wider, and one the sheet
//! says nothing about keeps the theme's bar.

mod support;

use support::{Element, Fixture, find, lay_out, measurer};
use zgui_layout::tree::store::LayoutStore;

/// A container whose content overflows it, so it scrolls and reserves a vertical bar.
fn scrolling(extra_css: &str) -> Fixture {
    let rows: Vec<Element> = (0..30)
        .map(|_| Element::new("row").text("alpha bravo delta gamma kappa sigma"))
        .collect();
    Fixture::new(
        Element::new("root").classes(&["s"]).children(rows),
        &format!(
            "root {{ display: block; height: 300px; overflow: auto }}
             row {{ display: block }}
             {extra_css}"
        ),
    )
}

/// The width the vertical bar of the element with `class` takes off its content.
fn bar_of(store: &LayoutStore, fixture: &Fixture, class: &str) -> f32 {
    let key = fixture
        .document
        .store()
        .key_of(find::with_class(fixture, class));
    let box_ = *store.boxes_of(key).first().expect("a box");
    store
        .layout_of(box_)
        .expect("laid out")
        .scrollbar_size
        .width
        .0
}

/// The width the container's vertical bar takes off its content.
fn bar_width(fixture: &Fixture) -> f32 {
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);
    bar_of(&store, fixture, "s")
}

#[test]
fn the_bar_is_as_wide_as_the_sheet_says_or_the_themes_when_it_says_nothing() {
    let theme = bar_width(&scrolling(""));
    assert!(
        theme > 0.0,
        "an overflowing container reserves the theme's bar"
    );

    assert_eq!(
        bar_width(&scrolling("::-webkit-scrollbar { width: 0.5rem }")),
        8.0,
        "a rule for every element sizes this one's bar, a font-relative width resolved"
    );
    assert_eq!(
        bar_width(&scrolling(
            "::-webkit-scrollbar { width: 0.5rem } .s::-webkit-scrollbar { width: 0 }"
        )),
        0.0,
        "a rule on the container hides its bar, and its content takes the whole width"
    );
    assert_eq!(
        bar_width(&scrolling(".s::-webkit-scrollbar:hover { width: 2px }")),
        theme,
        "a state of the bar itself has no counterpart, so the rule changes nothing"
    );
}

#[test]
fn a_containers_width_does_not_reach_the_scroll_containers_inside_it() {
    // Both containers overflow. `.s` hides its own bar; the inner container is styled by no rule
    // of its own, so it keeps the theme's bar. An inherited width would hide that one too.
    let inner_rows: Vec<Element> = (0..30)
        .map(|_| Element::new("row").text("alpha bravo delta gamma kappa sigma"))
        .collect();
    let fixture = Fixture::new(
        Element::new("root").classes(&["s"]).children(vec![
            Element::new("inner").classes(&["i"]).children(inner_rows),
        ]),
        "root { display: block; height: 100px; overflow: auto }
         inner { display: block; height: 200px; overflow: auto }
         row { display: block }
         .s::-webkit-scrollbar { width: 0 }",
    );
    let mut store = fixture.box_tree();
    let mut content = measurer();
    lay_out(&mut store, &mut content, 400.0, 300.0);
    assert_eq!(
        bar_of(&store, &fixture, "s"),
        0.0,
        "the outer container's bar is hidden"
    );
    assert!(
        bar_of(&store, &fixture, "i") > 0.0,
        "the inner container keeps the theme's bar"
    );
}
