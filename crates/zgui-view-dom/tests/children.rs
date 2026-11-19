//! Reading a node's children back through the node-tree seam.
//!
//! A caller that drives the document from outside the view layer keeps no tree of its own. It asks
//! the document for a node's first child and for the sibling after a node, and the answers follow
//! every insert, move and detach.

use std::cell::RefCell;
use std::rc::Rc;

use zgui_dom::Document;
use zgui_interned::ElementName;
use zgui_view::{Dom, NodeId};
use zgui_view_dom::DocumentDom;

/// The children of `node`, read one step at a time through the seam.
fn walk(dom: &dyn Dom, node: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut next = dom.first_child(node);
    while let Some(child) = next {
        out.push(child);
        next = dom.next_sibling(child);
    }
    out
}

#[test]
fn the_seam_lists_children_in_order_through_inserts_moves_and_detaches() {
    let document = Rc::new(RefCell::new(Document::new()));
    let dom = DocumentDom::new(Rc::clone(&document));
    let list = dom.create_element(ElementName::new("column"));
    dom.insert(dom.root(list), list, None);
    assert_eq!(dom.first_child(list), None);

    let a = dom.create_element(ElementName::new("row"));
    let b = dom.create_text("b");
    let c = dom.create_element(ElementName::new("row"));
    dom.insert(list, a, None);
    dom.insert(list, c, None);
    dom.insert(list, b, Some(c));
    assert_eq!(walk(&dom, list), vec![a, b, c]);
    assert_eq!(walk(&dom, list), dom.children(list));

    // A move is an insert of a node already in the tree.
    dom.insert(list, c, Some(a));
    assert_eq!(walk(&dom, list), vec![c, a, b]);

    dom.detach(a);
    assert_eq!(walk(&dom, list), vec![c, b]);
    assert_eq!(dom.next_sibling(a), None, "a detached node has no siblings");

    // A node that has gone answers nothing, rather than panicking.
    dom.end_frame();
    assert!(!dom.is_live(a));
    assert_eq!(dom.first_child(a), None);
    assert_eq!(dom.next_sibling(a), None);
}

#[test]
fn a_view_root_has_no_sibling_among_the_window_roots() {
    let document = Rc::new(RefCell::new(Document::new()));
    let dom = DocumentDom::new(Rc::clone(&document));
    let probe = dom.create_element(ElementName::new("box"));
    let root = dom.root(probe);
    assert_eq!(dom.next_sibling(root), None);
}
