//! Class definitions — the blueprint layer of HubFlow.
//!
//! A [`ClassDefinition`] describes the property schema for Object Knots.
//! It acts as a template defining which properties an Object has, their
//! data types, default values, and optional physical units.
//!
//! # Example
//!
//! ```rust
//! use hubflow::core::class::{ClassDefinition, PropertySchema, DataType};
//! use serde_json::json;
//!
//! let class = ClassDefinition::new("TemperatureSensor")
//!     .with_description("A temperature measurement device")
//!     .with_property(
//!         PropertySchema::new("temperature", DataType::Number)
//!             .with_unit("°C")
//!             .with_default(json!(20.0))
//!     )
//!     .with_property(
//!         PropertySchema::new("active", DataType::Boolean)
//!             .with_default(json!(true))
//!     );
//!
//! let defaults = class.default_state();
//! assert_eq!(defaults["temperature"], json!(20.0));
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// ── DataType ──────────────────────────────────────────────────────────────────

/// The primitive data type of a class property.
///
/// Used during schema validation to check that values stored on Object Knots
/// conform to the property's declared type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    /// A UTF-8 text string.
    String,
    /// A 64-bit floating-point number.
    Number,
    /// A boolean flag (`true` / `false`).
    Boolean,
    /// An arbitrary JSON value (any shape; no structural constraints).
    Json,
}

impl DataType {
    /// Returns `true` if `value` is structurally compatible with this type.
    ///
    /// `Json` is compatible with any value. All other types require an exact
    /// JSON primitive match.
    pub fn is_compatible(&self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Json => true,
        }
    }

    /// Returns the canonical zero / empty default for this type.
    pub fn zero_value(&self) -> Value {
        match self {
            Self::String => Value::String(String::new()),
            Self::Number => serde_json::json!(0.0),
            Self::Boolean => Value::Bool(false),
            Self::Json => Value::Null,
        }
    }
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String => write!(f, "string"),
            Self::Number => write!(f, "number"),
            Self::Boolean => write!(f, "boolean"),
            Self::Json => write!(f, "json"),
        }
    }
}

// ── PropertySchema ────────────────────────────────────────────────────────────

/// Schema definition for a single named property within a [`ClassDefinition`].
///
/// Use the builder methods to attach optional metadata:
///
/// ```rust
/// # use hubflow::core::class::{PropertySchema, DataType};
/// # use serde_json::json;
/// let schema = PropertySchema::new("pressure", DataType::Number)
///     .with_default(json!(1013.25))
///     .with_unit("hPa")
///     .with_description("Atmospheric pressure reading");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertySchema {
    /// Unique name of the property within the class (e.g. `"temperature"`).
    pub name: String,

    /// Expected data type for values stored under this property.
    pub data_type: DataType,

    /// Explicit default value. Falls back to [`DataType::zero_value`] when absent.
    pub default_value: Option<Value>,

    /// Optional physical unit label (e.g. `"°C"`, `"m/s"`, `"bar"`).
    pub unit: Option<String>,

    /// Optional human-readable description of what this property represents.
    pub description: Option<String>,
}

impl PropertySchema {
    /// Creates a new [`PropertySchema`] with the given name and data type.
    ///
    /// All optional fields default to `None`; use the builder methods to
    /// attach them.
    pub fn new(name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            data_type,
            default_value: None,
            unit: None,
            description: None,
        }
    }

    /// Sets an explicit default value for this property.
    pub fn with_default(mut self, value: Value) -> Self {
        self.default_value = Some(value);
        self
    }

    /// Attaches a physical unit label (e.g. `"°C"`) to this property.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Attaches a human-readable description to this property.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Returns the effective default value for this property.
    ///
    /// If an explicit default was set via [`with_default`](Self::with_default),
    /// that value is returned. Otherwise the type's zero value is used.
    pub fn effective_default(&self) -> Value {
        self.default_value
            .clone()
            .unwrap_or_else(|| self.data_type.zero_value())
    }
}

// ── Validation error ──────────────────────────────────────────────────────────

/// Errors produced when validating a property value against a class schema.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SchemaError {
    /// The named property does not exist in this class.
    #[error("property '{0}' is not defined in the class schema")]
    UnknownProperty(String),

    /// The value's JSON type does not match the property's declared type.
    #[error("type mismatch for '{property}': expected {expected}, got {got}")]
    TypeMismatch {
        property: String,
        expected: DataType,
        got: Value,
    },
}

// ── ClassDefinition ───────────────────────────────────────────────────────────

/// A blueprint that defines the property structure for Object Knots.
///
/// `ClassDefinition` is the HubFlow equivalent of a type or schema. It records
/// the ordered list of properties that every Object of this class must maintain.
///
/// # Builder pattern
///
/// ```rust
/// use hubflow::core::class::{ClassDefinition, PropertySchema, DataType};
/// use serde_json::json;
///
/// let class = ClassDefinition::new("Sensor")
///     .with_description("Generic IoT sensor")
///     .with_property(PropertySchema::new("value", DataType::Number).with_default(json!(0.0)))
///     .with_property(PropertySchema::new("unit",  DataType::String));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassDefinition {
    /// System-wide unique identifier for this class.
    pub id: Uuid,

    /// Human-readable class name (e.g. `"TemperatureSensor"`).
    pub name: String,

    /// Optional free-form description.
    pub description: Option<String>,

    /// Ordered list of property schemas that constitute this class.
    pub properties: Vec<PropertySchema>,
}

impl ClassDefinition {
    /// Creates a new [`ClassDefinition`] with a randomly generated UUID.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            properties: Vec::new(),
        }
    }

    /// Attaches a description to this class definition.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Appends a property schema (builder / fluent API).
    pub fn with_property(mut self, schema: PropertySchema) -> Self {
        self.properties.push(schema);
        self
    }

    // ── Lookups ───────────────────────────────────────────────────────────────

    /// Returns the schema for the named property, or `None` if absent.
    pub fn property(&self, name: &str) -> Option<&PropertySchema> {
        self.properties.iter().find(|p| p.name == name)
    }

    // ── Instantiation ─────────────────────────────────────────────────────────

    /// Returns a map of **all** property names to their effective default values.
    ///
    /// Used by Object Knots to initialise their volatile state store when the
    /// class is first attached.
    pub fn default_state(&self) -> HashMap<String, Value> {
        self.properties
            .iter()
            .map(|p| (p.name.clone(), p.effective_default()))
            .collect()
    }

    // ── Validation ────────────────────────────────────────────────────────────

    /// Validates that `value` is compatible with the named property's data type.
    pub fn validate_property(&self, name: &str, value: &Value) -> Result<(), SchemaError> {
        let schema = self
            .property(name)
            .ok_or_else(|| SchemaError::UnknownProperty(name.to_string()))?;

        if !schema.data_type.is_compatible(value) {
            return Err(SchemaError::TypeMismatch {
                property: name.to_string(),
                expected: schema.data_type.clone(),
                got: value.clone(),
            });
        }
        Ok(())
    }

    /// Validates every entry in `state` against this class schema.
    ///
    /// Returns the first [`SchemaError`] encountered, or `Ok(())` if all
    /// properties pass validation.
    pub fn validate_state(&self, state: &HashMap<String, Value>) -> Result<(), SchemaError> {
        for (name, value) in state {
            self.validate_property(name, value)?;
        }
        Ok(())
    }
}

impl std::fmt::Display for ClassDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Class({}, {} properties)",
            self.name,
            self.properties.len()
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sensor_class() -> ClassDefinition {
        ClassDefinition::new("Sensor")
            .with_description("A generic sensor")
            .with_property(
                PropertySchema::new("temperature", DataType::Number)
                    .with_default(json!(20.0))
                    .with_unit("°C"),
            )
            .with_property(
                PropertySchema::new("active", DataType::Boolean).with_default(json!(true)),
            )
            .with_property(PropertySchema::new("label", DataType::String))
            .with_property(PropertySchema::new("meta", DataType::Json))
    }

    #[test]
    fn default_state_uses_explicit_defaults() {
        let state = sensor_class().default_state();
        assert_eq!(state["temperature"], json!(20.0));
        assert_eq!(state["active"], json!(true));
    }

    #[test]
    fn default_state_falls_back_to_zero_values() {
        let state = sensor_class().default_state();
        assert_eq!(state["label"], json!(""));
        assert_eq!(state["meta"], json!(null));
    }

    #[test]
    fn property_lookup() {
        let class = sensor_class();
        assert!(class.property("temperature").is_some());
        assert!(class.property("nonexistent").is_none());
    }

    #[test]
    fn validate_property_ok() {
        let class = sensor_class();
        assert!(class.validate_property("temperature", &json!(42.5)).is_ok());
        assert!(class.validate_property("active", &json!(false)).is_ok());
        assert!(class.validate_property("label", &json!("tag")).is_ok());
        assert!(class.validate_property("meta", &json!({"x": 1})).is_ok());
    }

    #[test]
    fn validate_property_unknown_returns_error() {
        assert!(matches!(
            sensor_class().validate_property("xyz", &json!(1)),
            Err(SchemaError::UnknownProperty(_))
        ));
    }

    #[test]
    fn validate_property_type_mismatch() {
        assert!(matches!(
            sensor_class().validate_property("temperature", &json!("hot")),
            Err(SchemaError::TypeMismatch { .. })
        ));
        assert!(matches!(
            sensor_class().validate_property("active", &json!(42)),
            Err(SchemaError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn class_serialisation_round_trip() {
        let original = sensor_class();
        let json = serde_json::to_string(&original).unwrap();
        let restored: ClassDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(original.id, restored.id);
        assert_eq!(original.name, restored.name);
        assert_eq!(original.properties.len(), restored.properties.len());
    }

    #[test]
    fn data_type_zero_values() {
        assert_eq!(DataType::String.zero_value(), json!(""));
        assert_eq!(DataType::Number.zero_value(), json!(0.0));
        assert_eq!(DataType::Boolean.zero_value(), json!(false));
        assert_eq!(DataType::Json.zero_value(), json!(null));
    }
}
