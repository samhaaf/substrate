//! Slot tracking for concurrent completion execution.
//!
//! llama-server has a fixed number of "slots" (controlled by `--parallel N`).
//! Each slot can process one completion at a time. `SlotTracker` tracks which
//! completions are currently using slots and enforces the concurrency ceiling.
//!
//! ## Design
//!
//! `SlotTracker` is a lightweight in-memory counter backed by an `Arc<AtomicU32>`.
//! `SlotGuard` is a RAII guard that decrements the counter on drop, ensuring
//! inflight counts are always accurate even when completions fail or are cancelled.
//!
//! The tracker does NOT know about slot indices — llama-server assigns slots
//! internally. The tracker only enforces admission: at most `max` guards live at once.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use substrate_types::CompletionId;

/// Tracks in-flight completion slots.
///
/// Cheap to clone — wraps an `Arc`.
#[derive(Clone)]
pub struct SlotTracker {
    max: u32,
    active: Arc<AtomicU32>,
}

impl SlotTracker {
    /// Create a new tracker with a ceiling of `max` concurrent completions.
    pub fn new(max: u32) -> Self {
        Self {
            max,
            active: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Try to acquire a slot. Returns `Some(SlotGuard)` if a slot was available,
    /// or `None` if all `max` slots are already in use.
    ///
    /// The returned `SlotGuard` releases the slot when dropped.
    pub fn try_acquire(&self, completion_id: CompletionId) -> Option<SlotGuard> {
        // Atomically increment, but back off if we would exceed max.
        let prev = self.active.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            if current < self.max {
                Some(current + 1)
            } else {
                None
            }
        });

        match prev {
            Ok(_) => Some(SlotGuard {
                active: Arc::clone(&self.active),
                completion_id,
            }),
            Err(_) => None,
        }
    }

    /// Number of slots currently in use.
    pub fn active_count(&self) -> u32 {
        self.active.load(Ordering::Relaxed)
    }

    /// Maximum allowed concurrent completions.
    pub fn max(&self) -> u32 {
        self.max
    }

    /// Returns `true` if at least one slot is available.
    pub fn has_capacity(&self) -> bool {
        self.active_count() < self.max
    }
}

/// RAII guard for a single acquired slot.
///
/// Dropping this guard releases the slot back to the tracker.
pub struct SlotGuard {
    active: Arc<AtomicU32>,
    /// The completion currently occupying this slot (for logging / diagnostics).
    pub completion_id: CompletionId,
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        // Decrement the counter. The saturating_sub guards against underflow
        // on programming errors (double-drop, etc.).
        self.active.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |c| {
            Some(c.saturating_sub(1))
        }).ok();
    }
}

impl std::fmt::Debug for SlotGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SlotGuard({})", self.completion_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn acquire_up_to_max() {
        let tracker = SlotTracker::new(2);
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        let g1 = tracker.try_acquire(id1).expect("first slot available");
        let g2 = tracker.try_acquire(id2).expect("second slot available");
        assert_eq!(tracker.active_count(), 2);

        let g3 = tracker.try_acquire(id3);
        assert!(g3.is_none(), "third slot should be unavailable");

        drop(g1);
        assert_eq!(tracker.active_count(), 1);

        let g3 = tracker.try_acquire(id3).expect("slot available after drop");
        assert_eq!(tracker.active_count(), 2);

        drop(g2);
        drop(g3);
        assert_eq!(tracker.active_count(), 0);
    }
}
