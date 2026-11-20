//! Recording what one element's selector matching wrote and consulted.
//!
//! A match cache gives a later element the rules an earlier one matched. That is correct only if
//! the earlier match read nothing outside the cache key, and only if the later element receives
//! the selector flags the match would have written. While a recording runs on a thread, this module
//! notes each selector flag the matcher writes, by its distance up the ancestor chain from the
//! element being matched, and taints the recording when the matcher writes a flag anywhere else,
//! asks a container query, or receives presentational hints.

use std::cell::RefCell;

use selectors::matching::ElementSelectorFlags;
use smallvec::SmallVec;

use crate::NodeIndex;
use crate::node::handle::Node;

/// The ancestor depth a recording follows. A flag written above it taints the recording.
const DEPTH: usize = 32;

/// What one recorded match wrote and whether it consulted anything a key cannot hold.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    /// The bits of each selector flag the matcher wrote, with the receiving node's distance above
    /// the matched element: 0 for the element, 1 for its parent.
    pub flags: SmallVec<[(u8, usize); 2]>,
    /// Whether the matcher wrote a flag outside the ancestor chain, asked a container query or
    /// received presentational hints.
    pub tainted: bool,
}

/// The recording in progress on this thread.
struct Recording {
    /// The matched element, then its ancestors, nearest first.
    chain: SmallVec<[NodeIndex; 16]>,
    /// What was recorded so far.
    recorded: Recorded,
}

thread_local! {
    static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) };
    static PHASE: core::cell::Cell<Phase> = const { core::cell::Cell::new(Phase::Other) };
}

/// What the matcher's questions on this thread are being asked for, for the work counters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Keying an element for a match cache.
    Keying,
    /// Matching an element's rules.
    Matching,
    /// Anything else: invalidation, the engine's sharing cache.
    Other,
}

/// What the questions asked on this thread now are for.
pub fn phase() -> Phase {
    PHASE.with(core::cell::Cell::get)
}

/// Runs `f` with the questions it asks counted as being for `phase`.
pub fn in_phase<R>(phase: Phase, f: impl FnOnce() -> R) -> R {
    let outer = PHASE.with(|held| held.replace(phase));
    /// Puts the outer phase back when `f` returns or unwinds.
    struct Restore(Phase);
    impl Drop for Restore {
        fn drop(&mut self) {
            PHASE.with(|held| held.set(self.0));
        }
    }
    let _restore = Restore(outer);
    f()
}

/// Runs `match_element`, which matches `target`, and returns its result with what it recorded.
///
/// # Panics
///
/// If a recording is already in progress on this thread.
pub fn record<R>(target: Node<'_>, match_element: impl FnOnce() -> R) -> (R, Recorded) {
    let mut chain = SmallVec::new();
    let mut at = Some(target);
    while let Some(node) = at {
        if chain.len() == DEPTH {
            break;
        }
        chain.push(node.index());
        at = node.parent_node_handle();
    }
    RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        assert!(recording.is_none(), "one match is recorded at a time");
        *recording = Some(Recording {
            chain,
            recorded: Recorded::default(),
        });
    });
    /// Ends the recording when the match returns or unwinds.
    struct End;
    impl Drop for End {
        fn drop(&mut self) {
            RECORDING.with(|recording| recording.borrow_mut().take());
        }
    }
    let end = End;
    let result = in_phase(Phase::Matching, match_element);
    let recorded = RECORDING
        .with(|recording| recording.borrow_mut().take())
        .map(|recording| recording.recorded)
        .unwrap_or_default();
    drop(end);
    (result, recorded)
}

/// Notes that the matcher wrote `flags` on `node`.
pub(crate) fn note_flags(node: Node<'_>, flags: ElementSelectorFlags) {
    RECORDING.with(|recording| {
        if let Some(recording) = recording.borrow_mut().as_mut() {
            let index = node.index();
            match recording.chain.iter().position(|at| *at == index) {
                Some(depth) => {
                    let depth = depth as u8;
                    match recording
                        .recorded
                        .flags
                        .iter_mut()
                        .find(|(at, _)| *at == depth)
                    {
                        Some((_, held)) => *held |= flags.bits(),
                        None => recording.recorded.flags.push((depth, flags.bits())),
                    }
                }
                None => recording.recorded.tainted = true,
            }
        }
    });
}

/// Notes that the matcher consulted something no key holds.
pub(crate) fn taint() {
    RECORDING.with(|recording| {
        if let Some(recording) = recording.borrow_mut().as_mut() {
            recording.recorded.tainted = true;
        }
    });
}

/// Writes `flags`, recorded for another element, on `target` and its ancestors.
pub fn replay(target: Node<'_>, flags: &[(u8, usize)]) {
    for (depth, flags) in flags {
        let mut at = Some(target);
        for _ in 0..*depth {
            at = at.and_then(Node::parent_node_handle);
        }
        if let Some(node) = at {
            node.record_selector_flags(ElementSelectorFlags::from_bits_retain(*flags));
        }
    }
}
