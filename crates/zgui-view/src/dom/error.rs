//! Why a fallible node-tree call refused.

use core::fmt;

use crate::id::NodeId;

/// Why a `try_` call on a [`Dom`](crate::Dom) refused, leaving the tree unchanged.
///
/// The infallible calls treat each of these as a programming error. A caller that hands the tree
/// handles it cannot vouch for — a foreign function interface, a script — calls the `try_` form and
/// gets one of these back instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DomError {
    /// The node no longer exists in the tree.
    Gone(NodeId),
    /// The node belongs to another window's tree.
    ForeignDocument(NodeId),
    /// The operation needs a text node and this node is an element or a marker.
    NotText(NodeId),
    /// The node cannot hold children: it is a text node or a marker.
    CannotHoldChildren(NodeId),
    /// The reference node of an insertion is not a child of the parent.
    NotAChild {
        /// The parent of the insertion.
        parent: NodeId,
        /// The node the child was to go before.
        before: NodeId,
    },
    /// The insertion would put a node inside itself.
    Cycle {
        /// The parent of the insertion.
        parent: NodeId,
        /// The node that is the parent or one of its ancestors.
        child: NodeId,
    },
    /// An earlier change unwound and the tree accepts no more changes.
    Poisoned,
}

impl fmt::Display for DomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone(node) => write!(formatter, "{node:?} no longer exists"),
            Self::ForeignDocument(node) => {
                write!(formatter, "{node:?} belongs to another window's tree")
            }
            Self::NotText(node) => write!(formatter, "{node:?} is not a text node"),
            Self::CannotHoldChildren(node) => {
                write!(
                    formatter,
                    "{node:?} is a text node or a marker and holds no children"
                )
            }
            Self::NotAChild { parent, before } => {
                write!(formatter, "{before:?} is not a child of {parent:?}")
            }
            Self::Cycle { parent, child } => {
                write!(
                    formatter,
                    "inserting {child:?} under {parent:?} puts it inside itself"
                )
            }
            Self::Poisoned => formatter.write_str("the tree was poisoned by a change that unwound"),
        }
    }
}

impl std::error::Error for DomError {}
