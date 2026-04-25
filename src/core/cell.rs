//! Core cell abstraction — the fundamental building block of the system.
//!
//! Every entity in HubFlow is, at its root, a [`Cell`]. A cell carries a
//! globally unique identifier (UUID v4) for cross-system addressing, a local
//! identifier for efficient in-process lookup, and a [`CellMeta`] record that
//! provides human-readable context.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Local identity ────────────────────────────────────────────────────────────

/// A numeric identifier that is unique within a single running process / node.
///
/// Local IDs are cheap to compare and index but carry no meaning across process
/// boundaries. Use the cell's [`Uuid`] whenever you need global addressability.
pub type LocalId = u64;

// ── Metadata ──────────────────────────────────────────────────────────────────

/// Human-readable descriptors attached to every [`Cell`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellMeta {
    /// A short, human-readable name for the cell (e.g. `"UserService"`).
    pub name: String,

    /// An optional, free-form description explaining the cell's purpose.
    pub description: Option<String>,
}

impl CellMeta {
    /// Creates a new [`CellMeta`] with a name and no description.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }

    /// Creates a new [`CellMeta`] with both a name and a description.
    pub fn with_description(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: Some(description.into()),
        }
    }
}

// ── Cell ──────────────────────────────────────────────────────────────────────

/// The primordial unit of the HubFlow system.
///
/// A `Cell` is the smallest addressable entity. Every higher-level construct
/// (nodes, components, services, …) is built on top of cells.
///
/// # Identity model
///
/// | Field      | Scope   | Type       | Purpose                              |
/// |------------|---------|------------|--------------------------------------|
/// | `id`       | Global  | [`Uuid`]   | Stable, cross-system unique identity |
/// | `local_id` | Local   | [`LocalId`]| Fast in-process lookup / indexing    |
///
/// # Example
///
/// ```rust
/// use hubflow::core::cell::{Cell, CellMeta};
///
/// let cell = Cell::new(42, CellMeta::with_description("Alpha", "First cell"));
/// assert_eq!(cell.meta().name, "Alpha");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// System-wide unique identifier (UUID v4).
    id: Uuid,

    /// Node-local identifier for efficient in-process lookup.
    local_id: LocalId,

    /// Human-readable metadata attached to this cell.
    meta: CellMeta,
}

impl Cell {
    /// Creates a new [`Cell`], generating a fresh UUID v4 automatically.
    ///
    /// # Arguments
    ///
    /// * `local_id` — The caller-assigned local identifier for this cell.
    /// * `meta`     — Human-readable metadata (name + optional description).
    pub fn new(local_id: LocalId, meta: CellMeta) -> Self {
        Self {
            id: Uuid::new_v4(),
            local_id,
            meta,
        }
    }

    /// Reconstructs a [`Cell`] from its persisted components (e.g. from a database).
    ///
    /// Prefer [`Cell::new`] for fresh cells; use this only when restoring state.
    pub fn from_parts(id: Uuid, local_id: LocalId, meta: CellMeta) -> Self {
        Self { id, local_id, meta }
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Returns the system-wide unique identifier of this cell.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the node-local identifier of this cell.
    pub fn local_id(&self) -> LocalId {
        self.local_id
    }

    /// Returns a reference to the human-readable metadata of this cell.
    pub fn meta(&self) -> &CellMeta {
        &self.meta
    }

    /// Returns a mutable reference to the metadata, allowing in-place updates.
    pub fn meta_mut(&mut self) -> &mut CellMeta {
        &mut self.meta
    }
}

// ── Display ───────────────────────────────────────────────────────────────────

impl std::fmt::Display for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cell {{ id: {}, local_id: {}, name: {:?} }}",
            self.id, self.local_id, self.meta.name
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cell_has_unique_uuids() {
        let a = Cell::new(1, CellMeta::new("A"));
        let b = Cell::new(2, CellMeta::new("B"));
        assert_ne!(a.id(), b.id(), "every new cell must receive a unique UUID");
    }

    #[test]
    fn local_id_is_preserved() {
        let cell = Cell::new(99, CellMeta::new("Test"));
        assert_eq!(cell.local_id(), 99);
    }

    #[test]
    fn meta_with_description() {
        let meta = CellMeta::with_description("Node", "A processing node");
        assert_eq!(meta.name, "Node");
        assert_eq!(meta.description.as_deref(), Some("A processing node"));
    }

    #[test]
    fn meta_without_description() {
        let meta = CellMeta::new("Bare");
        assert!(meta.description.is_none());
    }

    #[test]
    fn from_parts_preserves_uuid() {
        let id = Uuid::new_v4();
        let cell = Cell::from_parts(id, 7, CellMeta::new("Restored"));
        assert_eq!(cell.id(), id);
    }

    #[test]
    fn cell_is_serializable() {
        let cell = Cell::new(1, CellMeta::with_description("Ser", "serialisation test"));
        let json = serde_json::to_string(&cell).expect("serialisation must succeed");
        let restored: Cell = serde_json::from_str(&json).expect("deserialisation must succeed");
        assert_eq!(cell, restored);
    }

    #[test]
    fn display_contains_name_and_ids() {
        let cell = Cell::new(3, CellMeta::new("DisplayCell"));
        let s = cell.to_string();
        assert!(s.contains("DisplayCell"));
        assert!(s.contains('3'));
    }
}
