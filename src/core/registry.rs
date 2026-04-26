//! Knot topology registry — stub implementation.
//!
//! The authoritative implementation of this module is being written by a
//! separate process. This file provides just enough structure for the rest of
//! the codebase (particularly the API layer) to compile while that work is in
//! progress.
//!
//! **Do not add business logic here.** Replace this file wholesale once the
//! real implementation lands.

use crate::core::cell::CellMeta;
use crate::core::knot::KnotRole;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ── RegistryEntry ─────────────────────────────────────────────────────────────

/// A serialisable snapshot of a single Knot's identity and topology position.
///
/// Returned by [`KnotRegistry::all`] and [`KnotRegistry::get`].
/// The `inbox` and `shared_state` fields present on the live `Knot` struct are
/// intentionally omitted here — they cannot be serialised and are not needed by
/// API consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// System-wide unique identifier.
    pub id: Uuid,

    /// Node-local numeric identifier (fast in-process lookup).
    pub local_id: u64,

    /// Human-readable name and optional description.
    pub meta: CellMeta,

    /// Functional role: Hub | Action | Object | Class.
    pub role: KnotRole,

    /// Depth in the topology tree (0 = root).
    pub level: u32,

    /// UUID of the parent Knot, or `None` for the root.
    pub parent_id: Option<Uuid>,

    /// UUID of the Class Knot this entry was instantiated from, if any.
    pub class_id: Option<Uuid>,

    /// UUIDs of all directly registered child Knots.
    pub child_ids: Vec<Uuid>,

    /// Canvas X coordinate (pixels) for the visual editor layout.
    pub x: f32,
    /// Canvas Y coordinate (pixels) for the visual editor layout.
    pub y: f32,
}

// ── KnotRegistry ──────────────────────────────────────────────────────────────

/// Thread-safe, in-memory directory of all live Knots.
///
/// Wrapped in an [`Arc`] and shared between the run-loops and the API server.
/// All mutation and read paths go through an async [`RwLock`] to prevent data
/// races without blocking the Tokio thread pool.
///
/// # Stub note
/// The current implementation is intentionally minimal — every mutating
/// operation is a no-op and every query returns `None` / an empty collection.
/// The real implementation will populate and maintain this registry as Knots
/// start, stop, and reconfigure.
pub struct KnotRegistry {
    entries: RwLock<HashMap<Uuid, RegistryEntry>>,
    /// Per-knot property state snapshots (Object Knots only).
    states: RwLock<HashMap<Uuid, HashMap<String, Value>>>,
}

impl KnotRegistry {
    /// Creates a new, empty registry wrapped in an [`Arc`].
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: RwLock::new(HashMap::new()),
            states: RwLock::new(HashMap::new()),
        })
    }

    // ── Read API (used by the HTTP layer) ─────────────────────────────────────

    /// Returns a snapshot of every registered Knot.
    pub async fn all(&self) -> Vec<RegistryEntry> {
        self.entries.read().await.values().cloned().collect()
    }

    /// Looks up a single Knot by its UUID.
    ///
    /// Returns `None` if the UUID is unknown.
    pub async fn get(&self, id: &Uuid) -> Option<RegistryEntry> {
        self.entries.read().await.get(id).cloned()
    }

    /// Returns the current property-state map for an Object Knot.
    ///
    /// Returns `None` if the Knot does not exist or has no tracked state
    /// (e.g. it is a Hub or Action Knot).
    pub async fn get_state_snapshot(&self, id: &Uuid) -> Option<HashMap<String, Value>> {
        self.states.read().await.get(id).cloned()
    }

    // ── Write API (used by Knot run-loops) ────────────────────────────────────

    /// Inserts or replaces a registry entry for the given Knot.
    pub async fn upsert(&self, entry: RegistryEntry) {
        self.entries.write().await.insert(entry.id, entry);
    }

    /// Removes a Knot from the registry (called on graceful stop).
    pub async fn remove(&self, id: &Uuid) {
        self.entries.write().await.remove(id);
        self.states.write().await.remove(id);
    }

    /// Updates (or inserts) a single property in an Object Knot's state map.
    pub async fn set_state(&self, knot_id: Uuid, property: String, value: Value) {
        self.states
            .write()
            .await
            .entry(knot_id)
            .or_default()
            .insert(property, value);
    }

    /// Updates the name and/or description of a registered Knot in-place.
    ///
    /// Returns `true` if the entry was found, `false` otherwise.
    pub async fn update_meta(
        &self,
        id: &Uuid,
        name: Option<String>,
        description: Option<String>,
    ) -> bool {
        let mut guard = self.entries.write().await;
        if let Some(entry) = guard.get_mut(id) {
            if let Some(n) = name {
                entry.meta.name = n;
            }
            if let Some(d) = description {
                entry.meta.description = Some(d);
            }
            true
        } else {
            false
        }
    }

    /// Updates the canvas position of a registered Knot in-place.
    ///
    /// Returns `true` if the entry was found, `false` otherwise.
    pub async fn update_position(&self, id: &Uuid, x: f32, y: f32) -> bool {
        let mut guard = self.entries.write().await;
        if let Some(entry) = guard.get_mut(id) {
            entry.x = x;
            entry.y = y;
            true
        } else {
            false
        }
    }

    /// Appends `child_id` to the `child_ids` list of `parent_id`, if present.
    pub async fn add_child_id(&self, parent_id: &Uuid, child_id: Uuid) {
        let mut guard = self.entries.write().await;
        if let Some(parent) = guard.get_mut(parent_id) {
            if !parent.child_ids.contains(&child_id) {
                parent.child_ids.push(child_id);
            }
        }
    }

    /// Removes `child_id` from the `child_ids` list of every registered Knot.
    pub async fn remove_child_id_everywhere(&self, child_id: &Uuid) {
        let mut guard = self.entries.write().await;
        for entry in guard.values_mut() {
            entry.child_ids.retain(|id| id != child_id);
        }
    }

    /// Sets the `parent_id` and `level` of a registered Knot in-place.
    ///
    /// Used when a new parent-child link is drawn in the editor.
    /// Returns `true` if the entry was found, `false` otherwise.
    pub async fn set_parent(&self, id: &Uuid, parent_id: Uuid, level: u32) -> bool {
        let mut guard = self.entries.write().await;
        if let Some(entry) = guard.get_mut(id) {
            entry.parent_id = Some(parent_id);
            entry.level = level;
            true
        } else {
            false
        }
    }
}

impl Default for KnotRegistry {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            states: RwLock::new(HashMap::new()),
        }
    }
}
