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
//! a real backend (e.g. [`crate::persistence::HflowStore`]) when persistence
//! is required.

use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use uuid::Uuid;

// ── PinBoxFuture ──────────────────────────────────────────────────────────────

/// Convenience alias for a heap-allocated, pinned, `Send`-able async future.
///
/// Used as the return type of [`PersistentStore`] methods to make the trait
/// object-safe (compatible with `dyn PersistentStore`).
///
/// Without this alias the trait would rely on `impl Future` return types, which
/// are not object-safe because their concrete size is unknown at compile time.
pub type PinBoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

// ── StateKey ──────────────────────────────────────────────────────────────────

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

// ── VolatileStore ─────────────────────────────────────────────────────────────

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

// ── PersistentStore ───────────────────────────────────────────────────────────

/// Object-safe async persistent key-value store.
///
/// Implementations are expected to be backed by an external system such as
/// SQLite (see [`crate::persistence::HflowStore`]), sled, Redis, or a cloud
/// database.
///
/// The trait uses [`PinBoxFuture`] instead of `impl Future` return types so
/// that it remains **object-safe** — i.e. usable as `dyn PersistentStore`.
///
/// # Example
///
/// ```rust,ignore
/// use hubflow::core::state::{PersistentStore, StateKey};
/// use uuid::Uuid;
///
/// async fn example(store: &dyn PersistentStore) {
///     let key = StateKey::new(Uuid::new_v4(), "temperature");
///     store.save(key.clone(), serde_json::json!(22.5)).await;
///     let val = store.load(&key).await;
/// }
/// ```
pub trait PersistentStore: Send + Sync {
    /// Loads the value at `key` from the persistent backend.
    ///
    /// Returns `None` if the key does not exist.
    fn load<'a>(&'a self, key: &'a StateKey) -> PinBoxFuture<'a, Option<Value>>;

    /// Saves `value` at `key` in the persistent backend.
    ///
    /// Overwrites any existing value for the same key.
    fn save<'a>(&'a self, key: StateKey, value: Value) -> PinBoxFuture<'a, ()>;

    /// Removes the value at `key` from the persistent backend.
    ///
    /// This is a no-op if the key does not exist.
    fn delete<'a>(&'a self, key: &'a StateKey) -> PinBoxFuture<'a, ()>;
}

// ── InMemoryStore ─────────────────────────────────────────────────────────────

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

// ── NoopPersistentStore ───────────────────────────────────────────────────────

/// Placeholder [`PersistentStore`] that discards all operations.
///
/// Replace with a real backend implementation (e.g.
/// [`crate::persistence::HflowStore`]) when durable state is required.
#[derive(Debug, Default)]
pub struct NoopPersistentStore;

impl PersistentStore for NoopPersistentStore {
    fn load<'a>(&'a self, _key: &'a StateKey) -> PinBoxFuture<'a, Option<Value>> {
        Box::pin(async { None })
    }

    fn save<'a>(&'a self, _key: StateKey, _value: Value) -> PinBoxFuture<'a, ()> {
        Box::pin(async {})
    }

    fn delete<'a>(&'a self, _key: &'a StateKey) -> PinBoxFuture<'a, ()> {
        Box::pin(async {})
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

    #[test]
    fn state_key_clone_and_eq() {
        let k1 = key("alpha");
        let k2 = k1.clone();
        assert_eq!(k1, k2);
    }

    #[test]
    fn state_key_hash_consistency() {
        use std::collections::HashSet;
        let k = key("beta");
        let mut set = HashSet::new();
        set.insert(k.clone());
        // Inserting the same key again must not grow the set.
        set.insert(k.clone());
        assert_eq!(set.len(), 1);
    }

    #[tokio::test]
    async fn noop_persistent_store_load_returns_none() {
        let store = NoopPersistentStore;
        let k = key("data");
        assert!(store.load(&k).await.is_none());
    }

    #[tokio::test]
    async fn noop_persistent_store_save_is_silent() {
        let store = NoopPersistentStore;
        let k = key("data");
        // save must not panic or error.
        store.save(k.clone(), json!({"x": 1})).await;
        // value must still be absent afterwards.
        assert!(store.load(&k).await.is_none());
    }

    #[tokio::test]
    async fn noop_persistent_store_delete_is_silent() {
        let store = NoopPersistentStore;
        let k = key("data");
        // delete must not panic or error, even if key never existed.
        store.delete(&k).await;
    }

    #[tokio::test]
    async fn persistent_store_is_object_safe() {
        // This test verifies at compile time that PersistentStore can be used
        // as a trait object (dyn PersistentStore).
        let store: Box<dyn PersistentStore> = Box::new(NoopPersistentStore);
        let k = key("dyn_test");
        assert!(store.load(&k).await.is_none());
        store.save(k.clone(), json!(1)).await;
        store.delete(&k).await;
    }
}
