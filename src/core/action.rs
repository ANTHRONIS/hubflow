//! Action Knot logic — the execution engine layer of HubFlow.
//!
//! An Action Knot processes incoming [`Packet`]s by running a [`LogicEngine`]
//! that can read Object Knot state, write property updates, and emit new
//! packets into the system.
//!
//! # Built-in engines
//!
//! | Engine             | Behaviour                                             |
//! |--------------------|-------------------------------------------------------|
//! | [`EchoEngine`]     | Reflects the incoming packet back to its sender       |
//! | [`TransformEngine`]| Applies a user-supplied closure to the payload        |
//! | [`ScriptEngine`]   | Placeholder for future embedded Python execution      |
//!
//! # Object safety & async
//!
//! `LogicEngine` is object-safe. Its `execute` method returns a
//! [`BoxFuture`] so it can be used as `Arc<dyn LogicEngine>` without
//! requiring any unstable features.

use crate::core::packet::Packet;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// ── BoxFuture ─────────────────────────────────────────────────────────────────

/// Heap-allocated, pinned, [`Send`]-able async future used as the return type
/// of [`LogicEngine::execute`] to achieve object safety.
pub type BoxFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

// ── Snapshots & context ───────────────────────────────────────────────────────

/// A point-in-time snapshot of an Object Knot's property state.
#[derive(Debug, Clone)]
pub struct ObjectSnapshot {
    /// UUID of the Object Knot.
    pub id: Uuid,

    /// UUID of the Class this Object was instantiated from, if any.
    pub class_id: Option<Uuid>,

    /// Current property values, keyed by property name.
    pub properties: HashMap<String, Value>,
}

/// Execution context passed to a [`LogicEngine`] when a packet arrives.
///
/// The context provides the engine with the triggering packet and read-only
/// snapshots of every Object Knot that this Action is linked to.
#[derive(Debug)]
pub struct ActionContext {
    /// UUID of the Action Knot executing this context.
    pub self_id: Uuid,

    /// The packet that triggered this execution.
    pub incoming: Packet,

    /// Snapshots of linked Object Knots, keyed by their UUIDs.
    pub objects: HashMap<Uuid, ObjectSnapshot>,
}

// ── Output ────────────────────────────────────────────────────────────────────

/// A property write-back produced by a [`LogicEngine`].
#[derive(Debug, Clone)]
pub struct PropertyUpdate {
    /// UUID of the target Object Knot.
    pub object_id: Uuid,
    /// Name of the property to overwrite.
    pub property_name: String,
    /// The new value.
    pub value: Value,
}

/// The result of a single [`LogicEngine::execute`] invocation.
#[derive(Debug, Default)]
pub struct ActionOutput {
    /// Property updates to persist onto linked Object Knots.
    pub updates: Vec<PropertyUpdate>,

    /// Packets to route back into the HubFlow network.
    pub packets: Vec<Packet>,

    /// Human-readable log messages emitted by the engine (surfaced in the
    /// event stream for the frontend).
    pub logs: Vec<String>,
}

// ── LogicEngine ───────────────────────────────────────────────────────────────

/// An async, object-safe computation hook for Action Knots.
///
/// Implement this trait to define custom logic. Two built-in implementations
/// are provided: [`EchoEngine`] and [`TransformEngine`].
///
/// # Object safety
///
/// Use `Arc<dyn LogicEngine>` for dynamic dispatch. The async `execute` method
/// returns a [`BoxFuture`] to satisfy the object-safety requirement.
///
/// # Python hook
///
/// [`ScriptEngine`] is a forward-looking placeholder that will delegate to an
/// embedded Python interpreter (e.g. `pyo3`) in a future iteration.
pub trait LogicEngine: Send + Sync {
    /// Returns the human-readable name of this engine instance.
    fn name(&self) -> &str;

    /// Executes the engine's logic for the given context and returns its output.
    ///
    /// The returned future **must** be `Send` so it can be polled across
    /// Tokio's thread pool boundaries.
    fn execute<'a>(&'a self, ctx: &'a ActionContext) -> BoxFuture<'a, ActionOutput>;
}

// ── EchoEngine ────────────────────────────────────────────────────────────────

/// A built-in engine that reflects the incoming packet back to its sender.
///
/// Useful as a default value and for smoke-testing Action Knot routing without
/// writing any custom logic.
#[derive(Debug, Clone, Default)]
pub struct EchoEngine;

impl LogicEngine for EchoEngine {
    fn name(&self) -> &str { "EchoEngine" }

    fn execute<'a>(&'a self, ctx: &'a ActionContext) -> BoxFuture<'a, ActionOutput> {
        Box::pin(async move {
            let response = Packet::new(
                ctx.self_id,
                ctx.incoming.sender_id,
                ctx.incoming.priority,
                Duration::from_secs(5),
                ctx.incoming.payload.clone(),
            );
            ActionOutput {
                packets: vec![response],
                logs: vec![format!(
                    "echo: reflected packet from {}",
                    ctx.incoming.sender_id
                )],
                ..Default::default()
            }
        })
    }
}

// ── TransformEngine ───────────────────────────────────────────────────────────

/// A built-in engine that applies a user-supplied closure to the incoming payload
/// and sends the transformed result back to the packet's sender.
///
/// # Example
///
/// ```rust
/// # use hubflow::core::action::TransformEngine;
/// # use serde_json::json;
/// let engine = TransformEngine::new("negate", |v| {
///     let n = v.get("n").and_then(|x| x.as_f64()).unwrap_or(0.0);
///     json!({ "n": -n })
/// });
/// ```
pub struct TransformEngine {
    name: String,
    transform: Arc<dyn Fn(&Value) -> Value + Send + Sync>,
}

impl TransformEngine {
    /// Creates a new [`TransformEngine`].
    ///
    /// `transform` receives the raw incoming payload and must return the
    /// new payload that will be placed in the response packet.
    pub fn new(
        name: impl Into<String>,
        transform: impl Fn(&Value) -> Value + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            transform: Arc::new(transform),
        }
    }
}

impl LogicEngine for TransformEngine {
    fn name(&self) -> &str { &self.name }

    fn execute<'a>(&'a self, ctx: &'a ActionContext) -> BoxFuture<'a, ActionOutput> {
        Box::pin(async move {
            let new_payload = (self.transform)(&ctx.incoming.payload);
            let response = Packet::new(
                ctx.self_id,
                ctx.incoming.sender_id,
                ctx.incoming.priority,
                Duration::from_secs(5),
                new_payload,
            );
            ActionOutput {
                packets: vec![response],
                logs: vec![format!("transform '{}': payload processed", self.name)],
                ..Default::default()
            }
        })
    }
}

// ── ScriptEngine ──────────────────────────────────────────────────────────────

/// A forward-looking placeholder for embedded Python script execution.
///
/// Currently a no-op. The interface is intentionally stable: when a real Python
/// interpreter (e.g. `pyo3`) is integrated, consumer code that holds an
/// `Arc<dyn LogicEngine>` pointing to a `ScriptEngine` will not need to change.
#[derive(Debug, Clone)]
pub struct ScriptEngine {
    /// Display name of this engine.
    pub name: String,
    /// Script source code (Python syntax anticipated).
    pub script: String,
}

impl ScriptEngine {
    /// Creates a new [`ScriptEngine`] with the given name and script body.
    pub fn new(name: impl Into<String>, script: impl Into<String>) -> Self {
        Self { name: name.into(), script: script.into() }
    }
}

impl LogicEngine for ScriptEngine {
    fn name(&self) -> &str { &self.name }

    fn execute<'a>(&'a self, _ctx: &'a ActionContext) -> BoxFuture<'a, ActionOutput> {
        Box::pin(async move {
            // TODO: integrate pyo3 or Python RPC bridge.
            ActionOutput {
                logs: vec![format!(
                    "ScriptEngine '{}': Python execution not yet implemented",
                    self.name
                )],
                ..Default::default()
            }
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(payload: Value) -> ActionContext {
        let self_id = Uuid::new_v4();
        let sender  = Uuid::new_v4();
        ActionContext {
            self_id,
            incoming: Packet::new(sender, self_id, 128, Duration::from_secs(5), payload),
            objects:  HashMap::new(),
        }
    }

    #[tokio::test]
    async fn echo_reflects_payload_to_sender() {
        let engine = EchoEngine;
        let ctx = ctx(json!({ "msg": "hello" }));
        let sender = ctx.incoming.sender_id;
        let out = engine.execute(&ctx).await;

        assert_eq!(out.packets.len(), 1);
        assert_eq!(out.packets[0].target_id,  sender);
        assert_eq!(out.packets[0].payload,    json!({ "msg": "hello" }));
        assert!(!out.logs.is_empty());
    }

    #[tokio::test]
    async fn transform_engine_applies_closure() {
        let engine = TransformEngine::new("double", |v| {
            let n = v.get("n").and_then(|x| x.as_f64()).unwrap_or(0.0);
            json!({ "n": n * 2.0 })
        });
        let out = engine.execute(&ctx(json!({ "n": 5.0 }))).await;
        assert_eq!(out.packets[0].payload, json!({ "n": 10.0 }));
    }

    #[tokio::test]
    async fn script_engine_is_placeholder() {
        let engine = ScriptEngine::new("test", "print('hi')");
        let out = engine.execute(&ctx(json!(null))).await;
        assert!(out.packets.is_empty());
        assert!(out.logs[0].contains("not yet implemented"));
    }
}
