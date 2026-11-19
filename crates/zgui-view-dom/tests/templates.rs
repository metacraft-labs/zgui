//! Creating a subtree from a template in one batch, sharing one parsed declaration block between
//! elements, and setting a paint-only value without a restyle.

use std::cell::RefCell;
use std::rc::Rc;

use zgui_bits::Dirty;
use zgui_dom::Document;
use zgui_interned::{AttrName, ClassName, ElementName};
use zgui_view::stub::StubDom;
use zgui_view::{DocumentId, Dom, NodeId, PaintProperty, StyleId, TemplateNode, TreeTemplate};
use zgui_view_dom::DocumentDom;

/// A row: an element with a class, an attribute and a shared style, holding a label element that
/// holds a text node, and a second text node after the label.
fn row(style: &StyleId) -> TreeTemplate {
    let mut root = TemplateNode::element(None, ElementName::new("row"));
    root.classes = vec![ClassName::new("row")];
    root.attributes = vec![(AttrName::new("data-kind"), "entry".to_owned())];
    root.style = Some(style.clone());
    let mut label = TemplateNode::element(Some(0), ElementName::new("label"));
    label.classes = vec![ClassName::new("idx"), ClassName::new("muted")];
    TreeTemplate {
        nodes: vec![
            root,
            label,
            TemplateNode::text(Some(1), "7"),
            TemplateNode::text(Some(0), "step into fn_7"),
        ],
    }
}

/// The subtree under `node`, as text: each element's name, classes, attributes and children.
fn describe(dom: &dyn Dom, node: NodeId, names: &dyn Fn(NodeId) -> String) -> String {
    let mut out = names(node);
    if let Some(kind) = dom.attribute(node, AttrName::new("data-kind")) {
        out.push_str(&format!("[data-kind={kind}]"));
    }
    let mut child = dom.first_child(node);
    let mut inner = Vec::new();
    while let Some(c) = child {
        inner.push(describe(dom, c, names));
        child = dom.next_sibling(c);
    }
    if !inner.is_empty() {
        out.push_str(&format!("({})", inner.join(",")));
    }
    out
}

#[test]
fn a_template_builds_the_tree_the_calls_build_in_one_batch() {
    let style = StyleId::new("padding-left: 6px; color: rgb(1, 2, 3)");
    let template = row(&style);

    let document = Rc::new(RefCell::new(Document::new()));
    let dom = DocumentDom::new(Rc::clone(&document));
    // The style is parsed when it is first used; parse it first so only the tree is counted.
    let warm = dom.create_element(ElementName::new("box"));
    dom.set_style(warm, Some(&style));
    let before = dom.revision();
    let nodes = dom.create_tree(&template);
    assert_eq!(
        dom.revision() - before,
        1,
        "the template was not applied as one batch"
    );
    assert_eq!(nodes.len(), 4);
    assert_eq!(dom.parent(nodes[0]), None, "the root is left detached");

    // The same template through the default, one call at a time, on the stub backend.
    let stub = StubDom::new(DocumentId::FIRST);
    let reference = stub.create_tree(&template);

    let names_doc = |node: NodeId| {
        let classes = dom.classes(node);
        let text = dom.text_content(node);
        if dom.first_child(node).is_none() && !text.is_empty() {
            format!("'{text}'")
        } else {
            let classes: Vec<_> = classes.iter().map(|c| c.as_str().to_owned()).collect();
            format!("e.{}", classes.join("."))
        }
    };
    let names_stub = |node: NodeId| match stub.text(node) {
        Some(text) => format!("'{text}'"),
        None => {
            let classes: Vec<_> = stub
                .classes(node)
                .iter()
                .map(|c| c.as_str().to_owned())
                .collect();
            format!("e.{}", classes.join("."))
        }
    };
    assert_eq!(
        describe(&dom, nodes[0], &names_doc),
        describe(&stub, reference[0], &names_stub)
    );
    assert_eq!(
        describe(&dom, nodes[0], &names_doc),
        "e.row[data-kind=entry](e.idx.muted('7'),'step into fn_7')"
    );
}

#[test]
fn elements_given_one_style_share_its_block_until_one_of_them_writes_its_own() {
    let style = StyleId::new("padding-left: 6px");
    let document = Rc::new(RefCell::new(Document::new()));
    let dom = DocumentDom::new(Rc::clone(&document));
    let a = dom.create_element(ElementName::new("box"));
    let b = dom.create_element(ElementName::new("box"));
    dom.set_style(a, Some(&style));
    dom.set_style(b, Some(&style));
    let block = |node: NodeId| {
        let document = document.borrow();
        let key = zgui_view_dom::id::to_document(node).expect("a document node");
        document
            .store()
            .columns()
            .inline_style
            .get(key)
            .and_then(Clone::clone)
            .expect("a block")
    };
    assert!(
        servo_arc::Arc::ptr_eq(&block(a), &block(b)),
        "the block was parsed twice"
    );

    // A write to one element's own declarations copies the block first.
    dom.set_style_property(a, "padding-right", Some("2px"));
    assert!(!servo_arc::Arc::ptr_eq(&block(a), &block(b)));
    let text = |node: NodeId| {
        let document = document.borrow();
        let lock = document.store().lock().clone();
        let guard = lock.read();
        let held = block(node);
        held.read_with(&guard).declarations().len()
    };
    assert_eq!(text(a), 2);
    assert_eq!(
        text(b),
        1,
        "the write reached the element that shared the block"
    );
}

#[test]
fn a_paint_only_value_marks_a_repaint_and_no_restyle() {
    let document = Rc::new(RefCell::new(Document::new()));
    let dom = DocumentDom::new(Rc::clone(&document));
    let el = dom.create_element(ElementName::new("box"));
    dom.insert(dom.root(el), el, None);
    let index = dom.index_of(el);
    let key = zgui_view_dom::id::to_document(el).expect("a document node");
    document
        .borrow()
        .store()
        .core(index)
        .dirty()
        .clear_own(Dirty::all());

    dom.set_paint(el, PaintProperty::BackgroundColor, Some("rgb(10, 20, 30)"));
    {
        let document = document.borrow();
        let own = document.store().core(index).dirty().own();
        assert!(own.contains(Dirty::REPAINT));
        assert!(!own.intersects(Dirty::RESTYLE | Dirty::RELAYOUT), "{own:?}");
        let held = document
            .store()
            .columns()
            .paint
            .get(key)
            .and_then(Option::as_ref);
        assert!(held.and_then(|over| over.background_color).is_some());
        assert!(
            document
                .store()
                .columns()
                .inline_style
                .get(key)
                .and_then(Option::as_ref)
                .is_none(),
            "a paint-only value went through the cascade"
        );
    }

    // A value the painter cannot take on its own goes through the cascade instead.
    dom.set_paint(el, PaintProperty::OutlineColor, Some("currentColor"));
    assert!(
        document
            .borrow()
            .store()
            .columns()
            .inline_style
            .get(key)
            .and_then(Option::as_ref)
            .is_some()
    );

    // Removing it leaves nothing behind.
    dom.set_paint(el, PaintProperty::BackgroundColor, None);
    assert!(
        document
            .borrow()
            .store()
            .columns()
            .paint
            .get(key)
            .and_then(Option::as_ref)
            .is_none()
    );
}
