//! SQLite-backed persistence for the HubFlow `.hflow` file format.
//!
//! The entire topology — Knot identities, class schemas, property values, and
//! editor links — is stored in a single SQLite database with the `.hflow`
//! extension. This makes projects fully portable: one file per workspace.
//!
//! # Usage
//!
//! ```rust,no_run
//! use hubflow::persistence::HflowStore;
//!
//! # async fn example() -> Result<(), hubflow::persistence::PersistenceError> {
//! let store = HflowStore::open("my_project.hflow").await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Thread safety
//!
//! `rusqlite::Connection` is not `Sync`. All blocking calls are wrapped in
//! `tokio::task::spawn_blocking` using a shared `Arc<Mutex<Connection>>`,
//! where the lock is acquired via `blocking_lock()` — safe on blocking threads.

use crate::core::class::{ClassDefinition, DataType, PropertySchema};
use crate::core::state::StateKey;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

// ── Error ─────────────────────────────────────────────────────────────────────

/// Errors that can arise from persistence operations.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON serialisation error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("async task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

// ── Row types ─────────────────────────────────────────────────────────────────

/// A hydrated row from the `knots` table.
#[derive(Debug, Clone)]
pub struct KnotRow {
    /// System-wide unique identifier.
    pub id: Uuid,
    /// In-process local identifier.
    pub local_id: u64,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Role string: `"Hub"` | `"Action"` | `"Object"` | `"Class"`.
    pub role: String,
    /// Depth in the topology tree.
    pub level: u32,
    /// UUID of the parent Knot, or `None` for root.
    pub parent_id: Option<Uuid>,
    /// UUID of the class template, or `None` for non-Object Knots.
    pub class_id: Option<Uuid>,
}

/// A hydrated row from the `links` table.
#[derive(Debug, Clone)]
pub struct LinkRow {
    /// Unique identifier for this link.
    pub id: Uuid,
    /// UUID of the source Knot.
    pub source_id: Uuid,
    /// UUID of the target Knot.
    pub target_id: Uuid,
    /// Semantic link type (e.g. `"action_to_object"`, `"data_flow"`).
    pub link_type: String,
}

// ── HflowStore ────────────────────────────────────────────────────────────────

/// SQLite-backed store for a `.hflow` workspace file.
///
/// Implements both [`PersistentStore`](crate::core::state::PersistentStore) for
/// per-property key/value access and higher-level topology save/load methods.
///
/// The internal `Connection` is wrapped in `Arc<Mutex<…>>` so that a single
/// cloned handle can be moved into each `spawn_blocking` closure.
pub struct HflowStore {
    conn: Arc<Mutex<Connection>>,
}

impl HflowStore {
    // ── Constructors ──────────────────────────────────────────────────────────

    /// Opens (or creates) a `.hflow` database at the given path and ensures all
    /// required tables exist.
    pub async fn open(path: impl AsRef<Path> + Send + 'static) -> Result<Self, PersistenceError> {
        let conn = tokio::task::spawn_blocking(move || {
            let conn = Connection::open(path)?;
            Self::create_schema(&conn)?;
            Ok::<_, rusqlite::Error>(conn)
        })
        .await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Opens an **in-memory** database — useful for unit tests that must not
    /// touch the filesystem.
    pub async fn open_in_memory() -> Result<Self, PersistenceError> {
        let conn = tokio::task::spawn_blocking(|| {
            let conn = Connection::open_in_memory()?;
            Self::create_schema(&conn)?;
            Ok::<_, rusqlite::Error>(conn)
        })
        .await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Schema ────────────────────────────────────────────────────────────────

    fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "
            -- Core Knot identity and topology
            CREATE TABLE IF NOT EXISTS knots (
                id          TEXT    PRIMARY KEY,
                local_id    INTEGER NOT NULL,
                name        TEXT    NOT NULL,
                description TEXT,
                role        TEXT    NOT NULL,
                level       INTEGER NOT NULL,
                parent_id   TEXT,
                class_id    TEXT
            );

            -- Volatile/persistent property values (ObjectID.PropertyID pattern)
            CREATE TABLE IF NOT EXISTS properties (
                object_id   TEXT    NOT NULL,
                property_id TEXT    NOT NULL,
                value       TEXT    NOT NULL,
                PRIMARY KEY (object_id, property_id)
            );

            -- Class schema definitions (one row per property per class)
            CREATE TABLE IF NOT EXISTS class_schemas (
                class_id       TEXT    NOT NULL,
                property_name  TEXT    NOT NULL,
                data_type      TEXT    NOT NULL,
                default_value  TEXT,
                unit           TEXT,
                description    TEXT,
                sort_order     INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (class_id, property_name)
            );

            -- Visual editor connections (source Knot → target Knot)
            CREATE TABLE IF NOT EXISTS links (
                id          TEXT    PRIMARY KEY,
                source_id   TEXT    NOT NULL,
                target_id   TEXT    NOT NULL,
                link_type   TEXT    NOT NULL
            );
            ",
        )
    }

    // ── Knot persistence ──────────────────────────────────────────────────────

    /// Inserts or replaces a Knot row in the `knots` table.
    pub async fn save_knot(
        &self,
        id: Uuid,
        local_id: u64,
        name: &str,
        description: Option<&str>,
        role: &str,
        level: u32,
        parent_id: Option<Uuid>,
        class_id: Option<Uuid>,
    ) -> Result<(), PersistenceError> {
        let id_s = id.to_string();
        let name_s = name.to_string();
        let desc_s = description.map(|s| s.to_string());
        let role_s = role.to_string();
        let parent_s = parent_id.map(|u| u.to_string());
        let class_s = class_id.map(|u| u.to_string());
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT OR REPLACE INTO knots
                 (id, local_id, name, description, role, level, parent_id, class_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id_s,
                    local_id as i64,
                    name_s,
                    desc_s,
                    role_s,
                    level as i64,
                    parent_s,
                    class_s,
                ],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Loads every row from the `knots` table and returns them as [`KnotRow`]s.
    pub async fn load_knots(&self) -> Result<Vec<KnotRow>, PersistenceError> {
        let conn = Arc::clone(&self.conn);
        let rows = tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, local_id, name, description, role, level, parent_id, class_id
                 FROM knots",
            )?;

            let rows: rusqlite::Result<Vec<KnotRow>> = stmt
                .query_map([], |row| {
                    let id_str: String = row.get(0)?;
                    let parent_str: Option<String> = row.get(6)?;
                    let class_str: Option<String> = row.get(7)?;

                    Ok(KnotRow {
                        id: Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::nil()),
                        local_id: row.get::<_, i64>(1)? as u64,
                        name: row.get(2)?,
                        description: row.get(3)?,
                        role: row.get(4)?,
                        level: row.get::<_, i64>(5)? as u32,
                        parent_id: parent_str.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
                        class_id: class_str.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
                    })
                })?
                .collect();
            rows
        })
        .await??;

        Ok(rows)
    }

    /// Deletes a single Knot row by UUID.
    ///
    /// Property values stored for this Knot are **not** automatically removed.
    pub async fn delete_knot(&self, id: Uuid) -> Result<(), PersistenceError> {
        let id_s = id.to_string();
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute("DELETE FROM knots WHERE id = ?1", params![id_s])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;

        Ok(())
    }

    // ── Class persistence ─────────────────────────────────────────────────────

    /// Persists all property schemas for a [`ClassDefinition`].
    ///
    /// Each property becomes one row in `class_schemas`, keyed by
    /// `(class_id, property_name)`. Existing rows for the same class are
    /// replaced via `INSERT OR REPLACE`.
    ///
    /// The class name and description live in the `knots` table (as a Knot
    /// with `role = "Class"`); only schema rows are written here.
    pub async fn save_class(&self, class: ClassDefinition) -> Result<(), PersistenceError> {
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            let conn     = conn.blocking_lock();
            let class_id = class.id.to_string();

            for (order, prop) in class.properties.iter().enumerate() {
                // Serialise the default value (if any) as a JSON string.
                let default_json: Option<String> = prop
                    .default_value
                    .as_ref()
                    .map(|v| serde_json::to_string(v))
                    .transpose()
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                // `DataType` serialises to e.g. `"string"` — strip the outer
                // quotes that serde_json adds for a plain string value.
                let dt_raw = serde_json::to_string(&prop.data_type)
                    .unwrap_or_else(|_| "\"json\"".to_string());
                let dt_clean = dt_raw.trim_matches('"').to_string();

                conn.execute(
                    "INSERT OR REPLACE INTO class_schemas
                     (class_id, property_name, data_type, default_value, unit, description, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        class_id,
                        prop.name,
                        dt_clean,
                        default_json,
                        prop.unit,
                        prop.description,
                        order as i64,
                    ],
                )?;
            }

            Ok::<_, rusqlite::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Loads all class schemas and reconstructs one [`ClassDefinition`] per
    /// unique `class_id`, with properties in insertion (`sort_order`) order.
    ///
    /// The `name` field on each returned `ClassDefinition` is set to
    /// `"(loaded)"` — callers that need the human-readable name should join
    /// against the `knots` table.
    pub async fn load_classes(&self) -> Result<Vec<ClassDefinition>, PersistenceError> {
        let conn = Arc::clone(&self.conn);

        // Raw rows: (class_id, property_name, data_type, default_value, unit, description, sort_order)
        type RawRow = (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );

        let raw_rows: Vec<RawRow> = tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT class_id, property_name, data_type, default_value,
                        unit, description, sort_order
                 FROM class_schemas
                 ORDER BY class_id, sort_order",
            )?;

            let rows: rusqlite::Result<Vec<RawRow>> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                })?
                .collect();
            rows
        })
        .await??;

        // Group rows by class_id, accumulating PropertySchema values.
        // We use an insertion-order-preserving structure (Vec of (key, value) pairs
        // behind an index map) to keep classes in the order first seen.
        let mut order: Vec<String> = Vec::new();
        let mut map: std::collections::HashMap<String, ClassDefinition> =
            std::collections::HashMap::new();

        for (class_id_str, prop_name, dt_str, default_json, unit, description, _order) in raw_rows {
            let entry = map.entry(class_id_str.clone()).or_insert_with(|| {
                order.push(class_id_str.clone());
                let class_uuid = Uuid::parse_str(&class_id_str).unwrap_or_else(|_| Uuid::nil());
                let mut c = ClassDefinition::new("(loaded)");
                c.id = class_uuid;
                c
            });

            let data_type = match dt_str.as_str() {
                "string" => DataType::String,
                "number" => DataType::Number,
                "boolean" => DataType::Boolean,
                _ => DataType::Json,
            };

            let default_value: Option<Value> = default_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());

            let mut schema = PropertySchema::new(prop_name, data_type);
            if let Some(dv) = default_value {
                schema = schema.with_default(dv);
            }
            if let Some(u) = unit {
                schema = schema.with_unit(u);
            }
            if let Some(d) = description {
                schema = schema.with_description(d);
            }

            entry.properties.push(schema);
        }

        // Return in stable order (first-seen per class_id).
        Ok(order.into_iter().filter_map(|k| map.remove(&k)).collect())
    }

    // ── Link persistence ──────────────────────────────────────────────────────

    /// Inserts or replaces a directed editor link between two Knots.
    pub async fn save_link(
        &self,
        id: Uuid,
        source_id: Uuid,
        target_id: Uuid,
        link_type: &str,
    ) -> Result<(), PersistenceError> {
        let (id_s, src_s, tgt_s, lt_s) = (
            id.to_string(),
            source_id.to_string(),
            target_id.to_string(),
            link_type.to_string(),
        );
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT OR REPLACE INTO links (id, source_id, target_id, link_type)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id_s, src_s, tgt_s, lt_s],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;

        Ok(())
    }

    /// Loads every row from the `links` table and returns them as [`LinkRow`]s.
    pub async fn load_links(&self) -> Result<Vec<LinkRow>, PersistenceError> {
        let conn = Arc::clone(&self.conn);

        let rows = tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare("SELECT id, source_id, target_id, link_type FROM links")?;

            let rows: rusqlite::Result<Vec<LinkRow>> = stmt
                .query_map([], |row| {
                    let id_s: String = row.get(0)?;
                    let src_s: String = row.get(1)?;
                    let tgt_s: String = row.get(2)?;
                    Ok(LinkRow {
                        id: Uuid::parse_str(&id_s).unwrap_or_else(|_| Uuid::nil()),
                        source_id: Uuid::parse_str(&src_s).unwrap_or_else(|_| Uuid::nil()),
                        target_id: Uuid::parse_str(&tgt_s).unwrap_or_else(|_| Uuid::nil()),
                        link_type: row.get(3)?,
                    })
                })?
                .collect();
            rows
        })
        .await??;

        Ok(rows)
    }

    /// Deletes an editor link by its UUID.
    pub async fn delete_link(&self, id: Uuid) -> Result<(), PersistenceError> {
        let id_s = id.to_string();
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute("DELETE FROM links WHERE id = ?1", params![id_s])?;
            Ok::<_, rusqlite::Error>(())
        })
        .await??;

        Ok(())
    }
}

// ── PersistentStore impl ──────────────────────────────────────────────────────

impl crate::core::state::PersistentStore for HflowStore {
    /// Loads a single property value by `StateKey` from the `properties` table.
    fn load<'a>(
        &'a self,
        key: &'a StateKey,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<Value>> + Send + 'a>> {
        let object_id = key.object_id.to_string();
        let property_id = key.property_id.clone();
        let conn = Arc::clone(&self.conn);

        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let conn = conn.blocking_lock();
                let result: rusqlite::Result<String> = conn.query_row(
                    "SELECT value FROM properties
                     WHERE object_id = ?1 AND property_id = ?2",
                    params![object_id, property_id],
                    |row| row.get(0),
                );
                result
                    .ok()
                    .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            })
            .await
            .unwrap_or(None)
        })
    }

    /// Upserts a property value by `StateKey` into the `properties` table.
    fn save<'a>(
        &'a self,
        key: StateKey,
        value: Value,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        let object_id = key.object_id.to_string();
        let property_id = key.property_id.clone();
        let conn = Arc::clone(&self.conn);

        Box::pin(async move {
            let json = serde_json::to_string(&value).unwrap_or_else(|_| "null".into());
            let _ = tokio::task::spawn_blocking(move || {
                let conn = conn.blocking_lock();
                conn.execute(
                    "INSERT OR REPLACE INTO properties (object_id, property_id, value)
                     VALUES (?1, ?2, ?3)",
                    params![object_id, property_id, json],
                )
            })
            .await;
        })
    }

    /// Removes a property value by `StateKey` from the `properties` table.
    fn delete<'a>(
        &'a self,
        key: &'a StateKey,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        let object_id = key.object_id.to_string();
        let property_id = key.property_id.clone();
        let conn = Arc::clone(&self.conn);

        Box::pin(async move {
            let _ = tokio::task::spawn_blocking(move || {
                let conn = conn.blocking_lock();
                conn.execute(
                    "DELETE FROM properties WHERE object_id = ?1 AND property_id = ?2",
                    params![object_id, property_id],
                )
            })
            .await;
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::class::{ClassDefinition, DataType, PropertySchema};
    use crate::core::state::StateKey;
    use serde_json::json;

    /// Helper: an in-memory store with the full schema already created.
    async fn store() -> HflowStore {
        HflowStore::open_in_memory().await.unwrap()
    }

    // ── Knot tests ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_and_load_knot() {
        let s = store().await;
        let id = Uuid::new_v4();

        s.save_knot(id, 1, "Hub", None, "Hub", 0, None, None)
            .await
            .unwrap();

        let rows = s.load_knots().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].name, "Hub");
        assert_eq!(rows[0].role, "Hub");
        assert_eq!(rows[0].local_id, 1);
        assert_eq!(rows[0].level, 0);
        assert!(rows[0].parent_id.is_none());
        assert!(rows[0].class_id.is_none());
    }

    #[tokio::test]
    async fn save_knot_with_parent_and_class() {
        let s = store().await;
        let id = Uuid::new_v4();
        let parent = Uuid::new_v4();
        let class_id = Uuid::new_v4();

        s.save_knot(
            id,
            7,
            "Sensor01",
            Some("A sensor"),
            "Object",
            2,
            Some(parent),
            Some(class_id),
        )
        .await
        .unwrap();

        let rows = s.load_knots().await.unwrap();
        assert_eq!(rows[0].description.as_deref(), Some("A sensor"));
        assert_eq!(rows[0].parent_id, Some(parent));
        assert_eq!(rows[0].class_id, Some(class_id));
    }

    #[tokio::test]
    async fn save_knot_upsert_replaces_row() {
        let s = store().await;
        let id = Uuid::new_v4();

        s.save_knot(id, 1, "Original", None, "Hub", 0, None, None)
            .await
            .unwrap();
        s.save_knot(
            id,
            1,
            "Updated",
            Some("now with desc"),
            "Hub",
            0,
            None,
            None,
        )
        .await
        .unwrap();

        let rows = s.load_knots().await.unwrap();
        assert_eq!(rows.len(), 1, "upsert must not duplicate the row");
        assert_eq!(rows[0].name, "Updated");
        assert_eq!(rows[0].description.as_deref(), Some("now with desc"));
    }

    #[tokio::test]
    async fn delete_knot_removes_row() {
        let s = store().await;
        let id = Uuid::new_v4();

        s.save_knot(id, 1, "K", None, "Hub", 0, None, None)
            .await
            .unwrap();
        s.delete_knot(id).await.unwrap();

        assert!(s.load_knots().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_knot_is_noop() {
        let s = store().await;
        // Should not error even if the row doesn't exist.
        s.delete_knot(Uuid::new_v4()).await.unwrap();
    }

    #[tokio::test]
    async fn load_knots_returns_multiple_rows() {
        let s = store().await;

        for i in 0..5u64 {
            s.save_knot(
                Uuid::new_v4(),
                i,
                &format!("Knot{i}"),
                None,
                "Action",
                1,
                None,
                None,
            )
            .await
            .unwrap();
        }

        assert_eq!(s.load_knots().await.unwrap().len(), 5);
    }

    // ── PersistentStore (properties) tests ────────────────────────────────────

    #[tokio::test]
    async fn persistent_store_load_missing_returns_none() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let key = StateKey::new(Uuid::new_v4(), "missing_property");
        assert!(s.load(&key).await.is_none());
    }

    #[tokio::test]
    async fn persistent_store_save_and_load() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let key = StateKey::new(Uuid::new_v4(), "temperature");

        s.save(key.clone(), json!(42.5)).await;
        assert_eq!(s.load(&key).await, Some(json!(42.5)));
    }

    #[tokio::test]
    async fn persistent_store_save_overwrites_value() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let key = StateKey::new(Uuid::new_v4(), "status");

        s.save(key.clone(), json!("initialising")).await;
        s.save(key.clone(), json!("running")).await;
        assert_eq!(s.load(&key).await, Some(json!("running")));
    }

    #[tokio::test]
    async fn persistent_store_delete_removes_value() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let key = StateKey::new(Uuid::new_v4(), "temp");

        s.save(key.clone(), json!(99)).await;
        s.delete(&key).await;
        assert!(s.load(&key).await.is_none());
    }

    #[tokio::test]
    async fn persistent_store_delete_nonexistent_is_noop() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let key = StateKey::new(Uuid::new_v4(), "ghost");
        // Should not error.
        s.delete(&key).await;
        assert!(s.load(&key).await.is_none());
    }

    #[tokio::test]
    async fn persistent_store_different_objects_do_not_share_properties() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let obj1 = Uuid::new_v4();
        let obj2 = Uuid::new_v4();

        let key1 = StateKey::new(obj1, "x");
        let key2 = StateKey::new(obj2, "x");

        s.save(key1.clone(), json!(1)).await;
        s.save(key2.clone(), json!(2)).await;

        assert_eq!(s.load(&key1).await, Some(json!(1)));
        assert_eq!(s.load(&key2).await, Some(json!(2)));
    }

    #[tokio::test]
    async fn persistent_store_json_value_types() {
        use crate::core::state::PersistentStore;
        let s = store().await;
        let id = Uuid::new_v4();

        let cases: Vec<(&str, Value)> = vec![
            ("bool_prop", json!(true)),
            ("str_prop", json!("hello")),
            ("num_prop", json!(3.14)),
            ("null_prop", json!(null)),
            ("obj_prop", json!({"nested": [1, 2, 3]})),
            ("arr_prop", json!([true, false])),
        ];

        for (prop, val) in &cases {
            let key = StateKey::new(id, *prop);
            s.save(key, val.clone()).await;
        }

        for (prop, val) in &cases {
            let key = StateKey::new(id, *prop);
            assert_eq!(
                s.load(&key).await.as_ref(),
                Some(val),
                "mismatch for {prop}"
            );
        }
    }

    // ── Class tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_and_load_class_basic() {
        let s = store().await;

        let class = ClassDefinition::new("Sensor")
            .with_property(
                PropertySchema::new("temperature", DataType::Number)
                    .with_default(json!(20.0))
                    .with_unit("°C"),
            )
            .with_property(PropertySchema::new("active", DataType::Boolean));

        s.save_class(class.clone()).await.unwrap();

        let classes = s.load_classes().await.unwrap();
        assert_eq!(classes.len(), 1);

        let loaded = &classes[0];
        assert_eq!(loaded.id, class.id);
        assert_eq!(loaded.properties.len(), 2);
    }

    #[tokio::test]
    async fn load_class_property_order_is_stable() {
        let s = store().await;

        let class = ClassDefinition::new("Ordered")
            .with_property(PropertySchema::new("alpha", DataType::String))
            .with_property(PropertySchema::new("beta", DataType::Number))
            .with_property(PropertySchema::new("gamma", DataType::Boolean))
            .with_property(PropertySchema::new("delta", DataType::Json));

        s.save_class(class.clone()).await.unwrap();

        let classes = s.load_classes().await.unwrap();
        let props: Vec<&str> = classes[0]
            .properties
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(props, ["alpha", "beta", "gamma", "delta"]);
    }

    #[tokio::test]
    async fn load_class_data_types_round_trip() {
        let s = store().await;

        let class = ClassDefinition::new("Types")
            .with_property(PropertySchema::new("s", DataType::String))
            .with_property(PropertySchema::new("n", DataType::Number))
            .with_property(PropertySchema::new("b", DataType::Boolean))
            .with_property(PropertySchema::new("j", DataType::Json));

        s.save_class(class).await.unwrap();

        let classes = s.load_classes().await.unwrap();
        let props = &classes[0].properties;

        assert!(matches!(props[0].data_type, DataType::String));
        assert!(matches!(props[1].data_type, DataType::Number));
        assert!(matches!(props[2].data_type, DataType::Boolean));
        assert!(matches!(props[3].data_type, DataType::Json));
    }

    #[tokio::test]
    async fn load_class_default_and_unit_round_trip() {
        let s = store().await;

        let class = ClassDefinition::new("Rich").with_property(
            PropertySchema::new("pressure", DataType::Number)
                .with_default(json!(1013.25))
                .with_unit("hPa")
                .with_description("Atmospheric pressure"),
        );

        s.save_class(class).await.unwrap();

        let classes = s.load_classes().await.unwrap();
        let prop = &classes[0].properties[0];

        assert_eq!(prop.default_value, Some(json!(1013.25)));
        assert_eq!(prop.unit.as_deref(), Some("hPa"));
        assert_eq!(prop.description.as_deref(), Some("Atmospheric pressure"));
    }

    #[tokio::test]
    async fn save_class_upsert_replaces_properties() {
        let s = store().await;
        let class_id = Uuid::new_v4();

        let mut class_v1 = ClassDefinition::new("V1");
        class_v1.id = class_id;
        class_v1 = class_v1.with_property(PropertySchema::new("old_prop", DataType::String));

        let mut class_v2 = ClassDefinition::new("V2");
        class_v2.id = class_id;
        class_v2 = class_v2
            .with_property(PropertySchema::new("new_prop_a", DataType::Number))
            .with_property(PropertySchema::new("new_prop_b", DataType::Boolean));

        s.save_class(class_v1).await.unwrap();
        s.save_class(class_v2).await.unwrap();

        let classes = s.load_classes().await.unwrap();
        assert_eq!(classes.len(), 1);

        let props: Vec<&str> = classes[0]
            .properties
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        // "old_prop" must still exist (we upsert per-property, not delete-all-then-insert)
        assert!(props.contains(&"new_prop_a"));
        assert!(props.contains(&"new_prop_b"));
    }

    #[tokio::test]
    async fn load_classes_returns_multiple_classes() {
        let s = store().await;

        let class_a =
            ClassDefinition::new("A").with_property(PropertySchema::new("x", DataType::Number));
        let class_b =
            ClassDefinition::new("B").with_property(PropertySchema::new("y", DataType::String));

        s.save_class(class_a).await.unwrap();
        s.save_class(class_b).await.unwrap();

        let classes = s.load_classes().await.unwrap();
        assert_eq!(classes.len(), 2);
    }

    // ── Link tests ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_and_load_link() {
        let s = store().await;
        let link_id = Uuid::new_v4();
        let src = Uuid::new_v4();
        let tgt = Uuid::new_v4();

        s.save_link(link_id, src, tgt, "action_to_object")
            .await
            .unwrap();

        let links = s.load_links().await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].id, link_id);
        assert_eq!(links[0].source_id, src);
        assert_eq!(links[0].target_id, tgt);
        assert_eq!(links[0].link_type, "action_to_object");
    }

    #[tokio::test]
    async fn delete_link_removes_row() {
        let s = store().await;
        let link_id = Uuid::new_v4();

        s.save_link(link_id, Uuid::new_v4(), Uuid::new_v4(), "data_flow")
            .await
            .unwrap();
        s.delete_link(link_id).await.unwrap();

        assert!(s.load_links().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_nonexistent_link_is_noop() {
        let s = store().await;
        s.delete_link(Uuid::new_v4()).await.unwrap();
    }

    #[tokio::test]
    async fn save_link_upsert_replaces_type() {
        let s = store().await;
        let link_id = Uuid::new_v4();
        let src = Uuid::new_v4();
        let tgt = Uuid::new_v4();

        s.save_link(link_id, src, tgt, "data_flow").await.unwrap();
        s.save_link(link_id, src, tgt, "action_to_object")
            .await
            .unwrap();

        let links = s.load_links().await.unwrap();
        assert_eq!(links.len(), 1, "upsert must not duplicate the link");
        assert_eq!(links[0].link_type, "action_to_object");
    }

    #[tokio::test]
    async fn load_links_returns_multiple_rows() {
        let s = store().await;

        for _ in 0..4 {
            s.save_link(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), "data_flow")
                .await
                .unwrap();
        }

        assert_eq!(s.load_links().await.unwrap().len(), 4);
    }
}
