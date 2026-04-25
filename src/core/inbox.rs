//! Async priority inbox for incoming [`Packet`]s.
//!
//! The inbox is backed by a [`BinaryHeap`] protected by a [`tokio::sync::Mutex`].
//! A [`tokio::sync::Notify`] wakes the consumer task without busy-waiting.
//!
//! # Priority ordering
//! Higher [`Packet::priority`] values are dequeued first.
//! Ties are broken by arrival time: the packet that arrived earlier is
//! served first (FIFO within the same priority band).
//!
//! # Cancellation safety
//! [`Inbox::pop`] is cancellation-safe: the [`Notify`] permit mechanism
//! ensures that a wakeup is never lost when the future is dropped between
//! polling cycles.

use crate::core::packet::Packet;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

// ── Priority wrapper ──────────────────────────────────────────────────────────

/// Wraps a [`Packet`] and implements [`Ord`] so the heap returns the
/// highest-priority, earliest-arriving packet first.
struct PrioritizedPacket(Packet);

impl PartialEq for PrioritizedPacket {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority == other.0.priority && self.0.created_at == other.0.created_at
    }
}

impl Eq for PrioritizedPacket {}

impl Ord for PrioritizedPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher priority value → first out of the heap.
        // Equal priority → earlier created_at → first out (FIFO within band).
        self.0
            .priority
            .cmp(&other.0.priority)
            .then_with(|| other.0.created_at.cmp(&self.0.created_at))
    }
}

impl PartialOrd for PrioritizedPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ── Inbox ─────────────────────────────────────────────────────────────────────

/// Thread-safe, async-aware priority inbox for a [`Knot`](crate::core::knot::Knot).
///
/// Cloning an [`Arc<Inbox>`] gives a second handle to the **same** inbox,
/// which is how parent/sibling Knots push packets into each other.
pub struct Inbox {
    queue: Mutex<BinaryHeap<PrioritizedPacket>>,
    notify: Notify,
}

impl Inbox {
    /// Creates a new, empty inbox wrapped in an [`Arc`].
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(BinaryHeap::new()),
            notify: Notify::new(),
        })
    }

    /// Pushes a packet into the inbox.
    ///
    /// This is a cheap, non-blocking operation (aside from the async mutex
    /// acquisition) and can be called from any task.
    pub async fn push(&self, packet: Packet) {
        self.queue.lock().await.push(PrioritizedPacket(packet));
        self.notify.notify_one();
    }

    /// Waits for and returns the highest-priority packet.
    ///
    /// If the inbox is empty this method suspends the calling task until a
    /// packet is pushed.
    ///
    /// # Cancellation safety
    /// The [`Notify`] stores at most one permit. If a `notify_one()` fires
    /// between the empty-queue check and the `notified().await`, the stored
    /// permit is consumed on the next poll — no wakeup is ever lost.
    pub async fn pop(&self) -> Packet {
        loop {
            // Register interest BEFORE inspecting the queue so that any
            // notify_one() that fires during the queue check is captured.
            let notified = self.notify.notified();
            tokio::pin!(notified);

            {
                let mut q = self.queue.lock().await;
                if let Some(p) = q.pop() {
                    return p.0;
                }
                // Queue is empty; release the lock before waiting.
            }

            notified.await;
        }
    }

    /// Returns the highest-priority packet without waiting.
    ///
    /// Returns [`None`] immediately if the inbox is currently empty.
    pub async fn try_pop(&self) -> Option<Packet> {
        self.queue.lock().await.pop().map(|p| p.0)
    }

    /// Returns the number of packets currently in the inbox.
    pub async fn len(&self) -> usize {
        self.queue.lock().await.len()
    }

    /// Returns `true` if the inbox contains no packets.
    pub async fn is_empty(&self) -> bool {
        self.queue.lock().await.is_empty()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use uuid::Uuid;

    fn make_packet(priority: u8) -> Packet {
        Packet::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            priority,
            Duration::from_secs(60),
            json!({"priority": priority}),
        )
    }

    #[tokio::test]
    async fn push_and_pop_single_packet() {
        let inbox = Inbox::new();
        inbox.push(make_packet(10)).await;
        let p = inbox.pop().await;
        assert_eq!(p.priority, 10);
    }

    #[tokio::test]
    async fn priority_ordering_high_first() {
        let inbox = Inbox::new();
        inbox.push(make_packet(5)).await;
        inbox.push(make_packet(200)).await;
        inbox.push(make_packet(50)).await;

        let first = inbox.pop().await;
        let second = inbox.pop().await;
        let third = inbox.pop().await;

        assert_eq!(first.priority, 200);
        assert_eq!(second.priority, 50);
        assert_eq!(third.priority, 5);
    }

    #[tokio::test]
    async fn try_pop_returns_none_when_empty() {
        let inbox = Inbox::new();
        assert!(inbox.try_pop().await.is_none());
    }

    #[tokio::test]
    async fn len_reflects_inbox_size() {
        let inbox = Inbox::new();
        assert_eq!(inbox.len().await, 0);
        inbox.push(make_packet(1)).await;
        inbox.push(make_packet(2)).await;
        assert_eq!(inbox.len().await, 2);
        inbox.pop().await;
        assert_eq!(inbox.len().await, 1);
    }
}
