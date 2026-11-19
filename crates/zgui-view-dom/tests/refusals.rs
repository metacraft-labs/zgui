//! A call the document cannot carry out is refused before it changes anything.
//!
//! The `try_` calls return the reason. The infallible calls panic with it, and the panic happens
//! before the document's change batch opens, so the document is still usable afterwards: a panic
//! inside a batch poisons it for good.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use zgui_dom::Document;
use zgui_interned::ElementName;
use zgui_view::{Dom, DomError, ListenerOptions, NodeId};
use zgui_view_dom::DocumentDom;
use zgui_vocab::EventKind;

fn tree() -> (DocumentDom, NodeId, NodeId, NodeId) {
    let document = Rc::new(RefCell::new(Document::new()));
    let dom = DocumentDom::new(Rc::clone(&document));
    let list = dom.create_element(ElementName::new("column"));
    dom.insert(dom.root(list), list, None);
    let row = dom.create_element(ElementName::new("row"));
    let text = dom.create_text("a");
    dom.insert(list, row, None);
    dom.insert(row, text, None);
    (dom, list, row, text)
}

#[test]
fn every_invalid_call_is_refused_with_its_reason_and_changes_nothing() {
    let (dom, list, row, text) = tree();
    let other = dom.create_element(ElementName::new("row"));

    assert_eq!(
        dom.try_insert(text, other, None),
        Err(DomError::CannotHoldChildren(text))
    );
    assert_eq!(
        dom.try_insert(list, other, Some(text)),
        Err(DomError::NotAChild {
            parent: list,
            before: text
        })
    );
    assert_eq!(
        dom.try_insert(row, list, None),
        Err(DomError::Cycle {
            parent: row,
            child: list
        })
    );
    assert_eq!(
        dom.try_insert(row, row, None),
        Err(DomError::Cycle {
            parent: row,
            child: row
        })
    );
    assert_eq!(dom.try_set_text(row, "x"), Err(DomError::NotText(row)));
    assert_eq!(dom.children(list), vec![row]);
    assert_eq!(dom.text_content(list), "a");

    // A node that was linked in and is still out when the frame ends is dropped.
    dom.insert(list, other, None);
    dom.detach(other);
    dom.end_frame();
    assert_eq!(
        dom.try_insert(list, other, None),
        Err(DomError::Gone(other))
    );
    assert_eq!(dom.try_set_text(other, "x"), Err(DomError::Gone(other)));
    let refused = dom.try_add_listener(
        other,
        EventKind::Click,
        ListenerOptions::default(),
        Rc::new(|_| {}),
    );
    assert_eq!(refused.err(), Some(DomError::Gone(other)));
    assert_eq!(dom.text_content(other), "", "a gone node reads as empty");

    // Valid calls still succeed.
    assert_eq!(dom.try_set_text(text, "b"), Ok(()));
    assert_eq!(dom.text_content(list), "b");
}

#[test]
fn a_refused_infallible_call_panics_without_poisoning_the_document() {
    let (dom, list, row, text) = tree();
    let refused = catch_unwind(AssertUnwindSafe(|| dom.set_text(row, "x")));
    assert!(refused.is_err(), "setting text on an element was accepted");
    let refused = catch_unwind(AssertUnwindSafe(|| dom.insert(row, list, None)));
    assert!(refused.is_err(), "a cycle was accepted");
    assert!(
        !dom.document().is_poisoned(),
        "a refusal poisoned the document"
    );

    let added = dom.create_element(ElementName::new("row"));
    dom.insert(list, added, None);
    dom.set_text(text, "still usable");
    assert_eq!(dom.children(list), vec![row, added]);
    assert_eq!(dom.text_content(list), "still usable");
}
