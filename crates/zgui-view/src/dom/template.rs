//! What a caller that knows a tree's shape ahead of time hands the backend in one call.
//!
//! A view built node by node issues one call per node, and a backend that applies each call as a
//! batch of its own pays the batch's bookkeeping once per node. A caller that knows the shape of a
//! subtree before it builds it — a compiler that saw the markup, a list that stamps out the same
//! row a thousand times — describes the subtree once as a [`TreeTemplate`] and creates it with
//! [`Dom::create_tree`](crate::Dom::create_tree), which a backend may apply as one batch.
//!
//! [`StyleId`] is the same idea for inline declarations: a block of declarations named once, and
//! applied to any number of elements with [`Dom::set_style`](crate::Dom::set_style), which a
//! backend may parse once and share between them.

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use zgui_interned::{AttrName, ClassName, ElementName};

/// A block of inline declarations, named once and applied to any number of elements.
///
/// Two elements given the same `StyleId` carry the same declarations. A backend that shares one
/// parsed block between them parses the text once, and lets its style engine see that the two
/// elements' own declarations are the same.
#[derive(Clone, Debug)]
pub struct StyleId {
    id: u64,
    css: Rc<str>,
}

impl StyleId {
    /// Names the declarations `css` spells out, in the syntax of a `style` attribute.
    pub fn new(css: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            css: Rc::from(css),
        }
    }

    /// A number no other `StyleId` in the process has.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The declarations, as text.
    pub fn css(&self) -> &str {
        &self.css
    }
}

/// What one node of a [`TreeTemplate`] is.
#[derive(Clone, Debug)]
pub enum TemplateKind {
    /// An element called this.
    Element(ElementName),
    /// A text node holding this.
    Text(String),
}

/// One node of a [`TreeTemplate`].
#[derive(Clone, Debug)]
pub struct TemplateNode {
    /// The position in the template of the node this one is the last child of so far, or `None`
    /// for the root, which is the first node.
    pub parent: Option<usize>,
    /// What the node is.
    pub kind: TemplateKind,
    /// The element's classes.
    pub classes: Vec<ClassName>,
    /// The element's attributes.
    pub attributes: Vec<(AttrName, String)>,
    /// The element's inline declarations.
    pub style: Option<StyleId>,
}

impl TemplateNode {
    /// An element called `name` under the node at `parent`.
    pub fn element(parent: Option<usize>, name: ElementName) -> Self {
        Self {
            parent,
            kind: TemplateKind::Element(name),
            classes: Vec::new(),
            attributes: Vec::new(),
            style: None,
        }
    }

    /// A text node holding `text` under the node at `parent`.
    pub fn text(parent: Option<usize>, text: impl Into<String>) -> Self {
        Self {
            parent,
            kind: TemplateKind::Text(text.into()),
            classes: Vec::new(),
            attributes: Vec::new(),
            style: None,
        }
    }
}

/// A subtree described ahead of time: its nodes in document order, each after its parent and
/// after its earlier siblings.
#[derive(Clone, Debug, Default)]
pub struct TreeTemplate {
    /// The nodes; the first is the root.
    pub nodes: Vec<TemplateNode>,
}

impl TreeTemplate {
    /// Whether every node's parent comes before it, and only the first node has none.
    pub fn is_well_formed(&self) -> bool {
        self.nodes
            .iter()
            .enumerate()
            .all(|(at, node)| match node.parent {
                None => at == 0,
                Some(parent) => {
                    parent < at && matches!(self.nodes[parent].kind, TemplateKind::Element(_))
                }
            })
            && !self.nodes.is_empty()
    }
}

/// A property that changes only what an element is painted in, which a backend can set without
/// computing the element's style again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PaintProperty {
    /// `background-color`.
    BackgroundColor,
    /// `opacity`.
    Opacity,
    /// `border-color`, on all four sides.
    BorderColor,
    /// `outline-color`.
    OutlineColor,
}

impl PaintProperty {
    /// Every paint property.
    pub const ALL: [Self; 4] = [
        Self::BackgroundColor,
        Self::Opacity,
        Self::BorderColor,
        Self::OutlineColor,
    ];

    /// The property's name in a style sheet.
    pub fn css_name(self) -> &'static str {
        match self {
            Self::BackgroundColor => "background-color",
            Self::Opacity => "opacity",
            Self::BorderColor => "border-color",
            Self::OutlineColor => "outline-color",
        }
    }

    /// The paint property named `name` in a style sheet.
    pub fn from_css_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.css_name() == name)
    }
}
