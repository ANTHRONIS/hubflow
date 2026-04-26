//! Persistence layer for HubFlow.
//!
//! Implements save/load of the full topology (Knots, Classes, properties, links)
//! to a single SQLite file with the `.hflow` extension.

pub mod sqlite;
pub use sqlite::{HflowStore, PersistenceError};
