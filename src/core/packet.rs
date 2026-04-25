//! Communication packet — the atomic unit of data exchange between Knots.
//!
//! Every message travelling through the HubFlow network is wrapped in a
//! [`Packet`]. Packets carry addressing information, a priority for inbox
//! ordering, a time-to-live that limits their lifetime, and an arbitrary
//! JSON payload.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

// ── Packet ────────────────────────────────────────────────────────────────────

/// A routable message exchanged between [`Knot`](crate::core::knot::Knot)s.
///
/// # Priority
/// Values range from `0` (lowest) to `255` (highest). The inbox processes
/// higher-priority packets first; ties are broken by arrival time (FIFO).
///
/// # TTL
/// The `ttl` field defines the maximum lifetime of the packet. Once
/// [`Packet::is_expired`] returns `true`, the packet must be dropped and a
/// [`DeadLetter`](crate::core::event::KnotEvent::DeadLetter) event emitted.
///
/// > **Distributed note:** `created_at` is a monotonic [`Instant`] and is
/// > intentionally **not** serialised. When a packet crosses a process
/// > boundary (network transport), the receiving side should reconstruct
/// > `created_at` from a serialised wall-clock timestamp. This will be
/// > addressed in a future transport layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packet {
    /// UUID of the destination Knot.
    pub target_id: Uuid,

    /// UUID of the originating Knot.
    pub sender_id: Uuid,

    /// Delivery priority: `0` = lowest, `255` = highest.
    pub priority: u8,

    /// Maximum lifetime of this packet.
    ///
    /// Serialised as milliseconds (u64 → ~584 years maximum).
    #[serde(with = "duration_millis")]
    pub ttl: Duration,

    /// Monotonic creation timestamp used for local TTL expiry checks.
    ///
    /// Skipped during serialisation; reconstructed to `Instant::now()` on
    /// deserialisation. See the struct-level note on distributed usage.
    #[serde(skip, default = "Instant::now")]
    pub(crate) created_at: Instant,

    /// Arbitrary JSON payload.
    pub payload: serde_json::Value,
}

impl Packet {
    /// Constructs a new [`Packet`], stamping `created_at` to the current instant.
    pub fn new(
        sender_id: Uuid,
        target_id: Uuid,
        priority: u8,
        ttl: Duration,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            target_id,
            sender_id,
            priority,
            ttl,
            created_at: Instant::now(),
            payload,
        }
    }

    /// Returns `true` if the packet's TTL has elapsed since creation.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }

    /// Returns the remaining TTL, clamped to [`Duration::ZERO`] if already expired.
    pub fn remaining_ttl(&self) -> Duration {
        self.ttl
            .checked_sub(self.created_at.elapsed())
            .unwrap_or(Duration::ZERO)
    }
}

impl std::fmt::Display for Packet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Packet {{ from: {}, to: {}, priority: {}, ttl_remaining: {:?} }}",
            self.sender_id,
            self.target_id,
            self.priority,
            self.remaining_ttl(),
        )
    }
}

// ── Duration serde helper ─────────────────────────────────────────────────────

/// Serialises [`Duration`] as milliseconds (`u64`).
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        // u64 covers ~584 years; sufficient for any practical TTL.
        (d.as_millis() as u64).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(Duration::from_millis(millis))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fresh_packet_is_not_expired() {
        let p = Packet::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            128,
            Duration::from_secs(60),
            json!({}),
        );
        assert!(!p.is_expired());
    }

    #[test]
    fn packet_with_zero_ttl_is_expired() {
        let p = Packet::new(Uuid::new_v4(), Uuid::new_v4(), 0, Duration::ZERO, json!({}));
        assert!(p.is_expired());
    }

    #[test]
    fn packet_serialisation_round_trip() {
        let p = Packet::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            200,
            Duration::from_millis(5_000),
            json!({ "action": "ping" }),
        );
        let json = serde_json::to_string(&p).expect("serialise");
        let restored: Packet = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(p.sender_id, restored.sender_id);
        assert_eq!(p.target_id, restored.target_id);
        assert_eq!(p.priority, restored.priority);
        assert_eq!(p.ttl, restored.ttl);
        assert_eq!(p.payload, restored.payload);
    }

    #[test]
    fn remaining_ttl_clamps_to_zero_when_expired() {
        let p = Packet::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            0,
            Duration::ZERO,
            json!(null),
        );
        assert_eq!(p.remaining_ttl(), Duration::ZERO);
    }
}
