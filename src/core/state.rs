//! State management for Knots.
//!
//! HubFlow uses a two-tier state model addressed by the
//! **ObjectID.PropertyID** pattern:
//!
//! | Tier       | Backing store        | Lifetime              | Trait               |
//! |------------|----------------------|-----------------------|---------------------|
//! | Volatile   | In-process RAM       | Knot lifetime         | [`VolatileStore`]   |
//! | Persistent | External DB / file   | Cross-restart durable | [`PersistentStore`] |
//!
//! The concrete types [`InMemoryStore`] and [`NoopPersistentStore`] are
//! provided as default implementations. Replace [`NoopPersistentStore`] with
//! a real backend (e.g. `sled`, SQLite, Redis) when persistence is required.

use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// ── State key ─────────────────────────────────────────────────────────────────

/// Compound address key following the **ObjectID.PropertyID** pattern.
///
/// ```text
/// <object_id>.<property_id>
/// e.g. "3fa8…c1d2.velocity"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StateKey {
    /// UUID of the object that owns this property.
    pub object_id: Uuid,

    /// Name of the property within the object.
    pub property_id: String,
}

impl StateKey {
    /// Constructs a new [`StateKey`].
    pub fn new(object_id: Uuid, property_id: impl Into<String>) -> Self {
        Self {
            object_id,
            property_id: property_id.into(),
        }
    }
}

impl std::fmt::Display for StateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.object_id, self.property_id)
    }
}

// ── Volatile store ────────────────────────────────────────────────────────────

/// Synchronous, in-memory key-value store for volatile Knot state.
///
/// Data lives only as long as the owning [`Knot`](crate::core::knot::Knot).
/// Thread-safety is provided by the Knot's ownership model (single async task).
pub trait VolatileStore: Send + Sync {
    /// Returns the value at `key`, or `None` if absent.
    fn get(&self, key: &StateKey) -> Option<&Value>;

    /// Inserts or replaces the value at `key`.
    fn set(&mut self, key: StateKey, value: Value);

    /// Removes and returns the value at `key`, or `None` if absent.
    fn remove(&mut self, key: &StateKey) -> Option<Value>;
}

// ── Persistent store ──────────────────────────────────────────────────────────

/// Asynchronous persistent key-value store for durable Knot state.
///
/// Implementations are expected to be backed by an external system such as
/// SQLite, sled, Redis, or a cloud database. The trait uses `async fn`
/// (stable since Rust 1.75).
pub trait PersistentStore: Send + Sync {
    /// Loads the value at `key` from the persistent backend.
    fn load(&self, key: &StateKey) -> impl std::future::Future<Output = Option<Value>> + Send;

    /// Saves `value` at `key` in the persistent backend.
    fn save(&self, key: StateKey, value: Value) -> impl std::future::Future<Output = ()> + Send;

    /// Removes the value at `key` from the persistent backend.
    fn delete(&self, key: &StateKey) -> impl std::future::Future<Output = ()> + Send;
}

// ── In-memory volatile store ──────────────────────────────────────────────────

/// Default [`VolatileStore`] backed by a [`HashMap`].
#[derive(Debug, Default)]
pub struct InMemoryStore {
    data: HashMap<StateKey, Value>,
}

impl VolatileStore for InMemoryStore {
    fn get(&self, key: &StateKey) -> Option<&Value> {
        self.data.get(key)
    }

    fn set(&mut self, key: StateKey, value: Value) {
        self.data.insert(key, value);
    }

    fn remove(&mut self, key: &StateKey) -> Option<Value> {
        self.data.remove(key)
    }
}

// ── No-op persistent store ────────────────────────────────────────────────────

/// Placeholder [`PersistentStore`] that discards all operations.
///
/// Replace with a real backend implementation (e.g. sled, SQLite, Redis)
/// when durable state is required.
#[derive(Debug, Default)]
pub struct NoopPersistentStore;

impl PersistentStore for NoopPersistentStore {
    async fn load(&self, _key: &StateKey) -> Option<Value> {
        // TODO: implement with a real persistent backend
        None
    }

    async fn save(&self, _key: StateKey, _value: Value) {
        // TODO: implement with a real persistent backend
    }

    async fn delete(&self, _key: &StateKey) {
        // TODO: implement with a real persistent backend
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(label: &str) -> StateKey {
        StateKey::new(Uuid::new_v4(), label)
    }

    #[test]
    fn volatile_set_and_get() {
        let mut store = InMemoryStore::default();
        let k = key("speed");
        store.set(k.clone(), json!(42));
        assert_eq!(store.get(&k), Some(&json!(42)));
    }

    #[test]
    fn volatile_remove() {
        let mut store = InMemoryStore::default();
        let k = key("temp");
        store.set(k.clone(), json!("hot"));
        let removed = store.remove(&k);
        assert_eq!(removed, Some(json!("hot")));
        assert!(store.get(&k).is_none());
    }

    #[test]
    fn volatile_get_missing_returns_none() {
        let store = InMemoryStore::default();
        assert!(store.get(&key("missing")).is_none());
    }

    #[test]
    fn state_key_display() {
        let id = Uuid::new_v4();
        let k = StateKey::new(id, "velocity");
        // Display must follow the "<object_id>.<property_id>" pattern.
        let s = k.to_string();
        assert!(
            s.ends_with(".velocity"),
            "expected '.<property_id>' suffix, got: {s}"
        );
        assert!(
            s.starts_with(&id.to_string()),
            "expected '<object_id>.' prefix, got: {s}"
        );
    }

    #[tokio::test]
    async fn noop_persistent_store_returns_none() {
        let store = NoopPersistentStore;
        let k = key("data");
        assert!(store.load(&k).await.is_none());
    }
}
