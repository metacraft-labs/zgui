//! Elements whose selector matching the match cache answers from an earlier element's match.
//!
//! Each case builds elements that agree on most of what selectors read and differ in one thing a
//! rule reads, in the order that gives the later element the earlier one's rules if the difference
//! is lost. They hold in every mode of the cache, so a shadow run (`ZGUI_MATCH_CACHE=shadow`)
//! proves them against matching as well.

use zgui_dom::NodeIndex;
use zgui_interned::AttrName;
use zgui_profile::{COUNTERS_ENABLED, Counter, counter};

use crate::support::{Harness, background, color};

/// An element called `name`, in `classes`, as the last child of `parent`.
fn child(harness: &mut Harness, parent: NodeIndex, name: &str, classes: &[&str]) -> NodeIndex {
    let node = harness.append(parent, name);
    harness.set_classes(node, classes);
    node
}

/// Sets the attribute `name` of `node` to `value`.
fn set_attribute(harness: &mut Harness, node: NodeIndex, name: &str, value: &str) {
    harness.edit(|edit| {
        edit.set_attribute(
            node,
            AttrName::new(name),
            Some(zgui_vocab::SharedString::from(value)),
        );
    });
}

/// Sets the inline style of `node`.
fn set_inline(harness: &mut Harness, node: NodeIndex, css: &str) {
    harness.edit(|edit| edit.set_inline_style(node, Some(css)));
}

/// Whether this run gives elements kept matches.
fn cached() -> bool {
    std::env::var("ZGUI_MATCH_CACHE").map_or(true, |mode| mode == "on")
}

#[test]
fn rows_with_their_own_inline_styles_skip_matching_and_keep_their_position() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(
        ".row { color: rgb(1, 1, 1) }
         .row:nth-child(odd) { background-color: rgb(0, 0, 9) }
         .idx { color: rgb(2, 2, 2) }",
    );
    let list = child(&mut harness, root, "column", &["list"]);
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    for index in 0..200 {
        let row = child(&mut harness, list, "row", &["row"]);
        // A style of its own on every row, as a list built one call at a time writes them: the
        // engine's sharing cache compares style attributes, so it shares none of these rows.
        set_inline(&mut harness, row, &format!("height: {}px", 10 + index));
        cells.push(child(&mut harness, row, "text", &["idx"]));
        rows.push(row);
    }
    let hits = counter::get(Counter::MatchCacheHits);
    harness.frame();
    for (index, row) in rows.iter().enumerate() {
        let expected = if index % 2 == 0 { (0, 0, 9) } else { (0, 0, 0) };
        assert_eq!(background(&harness, *row), expected, "row {index}");
    }
    for cell in &cells {
        assert_eq!(color(&harness, *cell), (2, 2, 2));
    }
    let hits = counter::get(Counter::MatchCacheHits) - hits;
    if COUNTERS_ENABLED && cached() {
        assert!(
            hits >= 150,
            "200 rows of two shapes, each with its own style, took {hits} matches from the cache"
        );
    }
}

#[test]
fn an_ancestor_class_reaches_the_rows_under_it() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(
        ".x { color: rgb(1, 1, 1) }
         .hot .x { color: rgb(9, 0, 0) }",
    );
    let cold = child(&mut harness, root, "column", &["pane"]);
    let hot = child(&mut harness, root, "column", &["pane", "hot"]);
    let mut cold_cells = Vec::new();
    let mut hot_cells = Vec::new();
    for _ in 0..20 {
        let row = child(&mut harness, cold, "row", &[]);
        cold_cells.push(child(&mut harness, row, "text", &["x"]));
        let row = child(&mut harness, hot, "row", &[]);
        hot_cells.push(child(&mut harness, row, "text", &["x"]));
    }
    harness.frame();
    for cell in cold_cells {
        assert_eq!(color(&harness, cell), (1, 1, 1));
    }
    for cell in hot_cells {
        assert_eq!(color(&harness, cell), (9, 0, 0));
    }
}

#[test]
fn an_ancestor_attribute_reaches_the_rows_under_it() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(
        ".x { color: rgb(1, 1, 1) }
         [data-mode=\"warn\"] .x { color: rgb(9, 9, 0) }",
    );
    let plain = child(&mut harness, root, "column", &["pane"]);
    let warn = child(&mut harness, root, "column", &["pane"]);
    set_attribute(&mut harness, warn, "data-mode", "warn");
    let first = child(&mut harness, plain, "text", &["x"]);
    let second = child(&mut harness, warn, "text", &["x"]);
    harness.frame();
    assert_eq!(color(&harness, first), (1, 1, 1));
    assert_eq!(color(&harness, second), (9, 9, 0));
}

#[test]
fn a_reused_match_keeps_each_element_its_own_inline_style() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(".x { color: rgb(1, 1, 1); background-color: rgb(3, 3, 3) }");
    let list = child(&mut harness, root, "column", &[]);
    let plain = child(&mut harness, list, "text", &["x"]);
    let styled = child(&mut harness, list, "text", &["x"]);
    set_inline(&mut harness, styled, "color: rgb(0, 9, 0)");
    let important = child(&mut harness, list, "text", &["x"]);
    set_inline(
        &mut harness,
        important,
        "background-color: rgb(0, 0, 7) !important",
    );
    let after = child(&mut harness, list, "text", &["x"]);
    harness.frame();
    assert_eq!(color(&harness, plain), (1, 1, 1));
    assert_eq!(color(&harness, styled), (0, 9, 0));
    assert_eq!(background(&harness, styled), (3, 3, 3));
    assert_eq!(background(&harness, important), (0, 0, 7));
    assert_eq!(color(&harness, after), (1, 1, 1));
    assert_eq!(background(&harness, after), (3, 3, 3));
}

#[test]
fn a_reused_match_writes_the_flags_a_later_insertion_reads() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(
        ".x { color: rgb(1, 1, 1) }
         .x:last-child { color: rgb(9, 0, 0) }",
    );
    let first = child(&mut harness, root, "column", &[]);
    let second = child(&mut harness, root, "column", &[]);
    let mut cells = Vec::new();
    for parent in [first, second] {
        for _ in 0..3 {
            cells.push(child(&mut harness, parent, "text", &["x"]));
        }
    }
    harness.frame();
    assert_eq!(color(&harness, cells[2]), (9, 0, 0));
    assert_eq!(color(&harness, cells[5]), (9, 0, 0));
    harness.retire_all();
    // The second list's cells may have taken the first list's match. The cell appended now is the
    // last child, so the one before it has to lose the rule.
    let appended = child(&mut harness, second, "text", &["x"]);
    harness.frame();
    assert_eq!(color(&harness, cells[5]), (1, 1, 1));
    assert_eq!(color(&harness, appended), (9, 0, 0));
}

#[test]
fn a_changed_sheet_is_matched_again() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(".x { color: rgb(1, 1, 1) }");
    let list = child(&mut harness, root, "column", &[]);
    let first = child(&mut harness, list, "text", &["x"]);
    harness.frame();
    assert_eq!(color(&harness, first), (1, 1, 1));
    harness.retire_all();
    harness.replace(0, ".x { color: rgb(0, 8, 0) }");
    let later = child(&mut harness, list, "text", &["x"]);
    harness.frame();
    assert_eq!(color(&harness, first), (0, 8, 0));
    assert_eq!(color(&harness, later), (0, 8, 0));
}

#[test]
fn a_state_on_an_ancestor_reaches_the_rows_under_it() {
    let mut harness = Harness::new();
    let root = harness.root;
    harness.add_author(
        ".x { color: rgb(1, 1, 1) }
         :hover > .x { color: rgb(0, 6, 6) }",
    );
    let calm = child(&mut harness, root, "row", &[]);
    let hovered = child(&mut harness, root, "row", &[]);
    harness.set_state(hovered, zgui_vocab::UiState::HOVER, true);
    let first = child(&mut harness, calm, "text", &["x"]);
    let second = child(&mut harness, hovered, "text", &["x"]);
    harness.frame();
    assert_eq!(color(&harness, first), (1, 1, 1));
    assert_eq!(color(&harness, second), (0, 6, 6));
}
