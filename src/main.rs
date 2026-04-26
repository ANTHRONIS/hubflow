//! HubFlow v0.2 — comprehensive bootstrap demo
//!
//! Demonstrates the full stack in one executable:
//!   ① Class / Object / Action Triumvirat
//!   ② Priority routing (Hub → EnvHub → Satellites)
//!   ③ SQLite persistence (.hflow workspace file)
//!   ④ axum API server with WebSocket event stream
//!
//! Topology built here:
//!
//! ```text
//! Level 0   [RootHub]
//!                │
//! Level 1   [EnvHub]
//!            /  |  \  \
//! Level 2  [Cls][S1][S2][Monitor]
//!          Class Obj Obj  Action
//! ```
//!
//! Routing note: packets are injected into EnvHub's inbox so the hub can
//! route them down to the correct Satellite. Only packets addressed directly
//! to a Knot's *immediate* children are forwarded down; all others escalate up.

mod api;
mod core;
mod persistence;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use api::ApiServer;
use core::action::TransformEngine;
use core::cell::CellMeta;
use core::class::{ClassDefinition, DataType, PropertySchema};
use core::event::KnotEvent;
use core::knot::Knot;
use core::packet::Packet;
use core::registry::KnotRegistry;
use core::state::StateKey;
use persistence::HflowStore;

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Set RUST_LOG=debug to see detailed tracing output.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    banner();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 1 — Class definition
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 1 — Class Definition: TemperatureSensor");

    let sensor_class = ClassDefinition::new("TemperatureSensor")
        .with_description("Generic temperature + humidity sensor")
        .with_property(
            PropertySchema::new("temperature", DataType::Number)
                .with_default(json!(20.0))
                .with_unit("°C")
                .with_description("Current temperature reading"),
        )
        .with_property(
            PropertySchema::new("humidity", DataType::Number)
                .with_default(json!(50.0))
                .with_unit("%rH"),
        )
        .with_property(PropertySchema::new("active", DataType::Boolean).with_default(json!(true)))
        .with_property(PropertySchema::new("label", DataType::String));

    println!("  Class     : {sensor_class}");
    let defaults = sensor_class.default_state();
    println!(
        "  Defaults  : temperature={}, humidity={}, active={}",
        defaults["temperature"], defaults["humidity"], defaults["active"]
    );
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 2 — Topology assembly
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 2 — Topology Assembly");

    // Root Hub — level 0, no parent
    let (mut root, root_evt_rx) = Knot::new_hub(
        0,
        CellMeta::with_description("RootHub", "Level-0 central star"),
        0,
        None,
        None,
    );

    // Environment Hub — level 1, child of Root
    let (mut env_hub, env_evt_rx) = Knot::new_hub(
        1,
        CellMeta::with_description("EnvHub", "Manages environmental sensors"),
        1,
        Some(root.id()),
        Some(root.inbox()),
    );

    // Class Knot — stores the TemperatureSensor schema
    let (mut class_knot, _class_evt_rx) = Knot::new_class(
        2,
        CellMeta::new("SensorClass"),
        sensor_class.clone(),
        2,
        Some(env_hub.id()),
        Some(env_hub.inbox()),
    );

    // Object Knots — initialised with class defaults
    let (mut sensor1, sensor1_evt_rx) = Knot::new_object(
        3,
        CellMeta::with_description("Sensor1", "Indoor sensor"),
        Some(&sensor_class),
        2,
        Some(env_hub.id()),
        Some(env_hub.inbox()),
    );

    let (mut sensor2, _sensor2_evt_rx) = Knot::new_object(
        4,
        CellMeta::with_description("Sensor2", "Outdoor sensor"),
        Some(&sensor_class),
        2,
        Some(env_hub.id()),
        Some(env_hub.inbox()),
    );

    // Action Knot — doubles the "n" field in the incoming packet payload
    let double_engine = TransformEngine::new("double", |v| {
        let n = v.get("n").and_then(|x| x.as_f64()).unwrap_or(0.0);
        json!({ "n": n * 2.0, "note": "doubled by Monitor" })
    });
    let (mut monitor, monitor_evt_rx) = Knot::new_action(
        5,
        CellMeta::with_description("Monitor", "Doubles incoming numeric values"),
        Arc::new(double_engine),
        2,
        Some(env_hub.id()),
        Some(env_hub.inbox()),
    );

    // Capture IDs and inbox handles before moving Knots into async tasks
    let root_id = root.id();
    let env_hub_id = env_hub.id();
    let sensor1_id = sensor1.id();
    let sensor2_id = sensor2.id();
    let monitor_id = monitor.id();
    let class_id = class_knot.id();

    let root_inbox = root.inbox();
    let env_inbox = env_hub.inbox(); // inject here to reach Satellites
    let sensor1_inbox = sensor1.inbox();

    // Wire up routing tables
    root.attach_child(env_hub.as_child_handle());
    env_hub.attach_child(class_knot.as_child_handle());
    env_hub.attach_child(sensor1.as_child_handle());
    env_hub.attach_child(sensor2.as_child_handle());
    env_hub.attach_child(monitor.as_child_handle());

    // Link Sensor1 state to Monitor so ActionContext carries its snapshot
    monitor.link_object(sensor1_id);

    println!("  RootHub   id = {root_id}");
    println!("  EnvHub    id = {env_hub_id}");
    println!("  SensorCls id = {class_id}");
    println!("  Sensor1   id = {sensor1_id}");
    println!("  Sensor2   id = {sensor2_id}");
    println!("  Monitor   id = {monitor_id}");
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 3 — Registry
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 3 — KnotRegistry");

    let registry = KnotRegistry::new();
    root.set_registry(Arc::clone(&registry));
    env_hub.set_registry(Arc::clone(&registry));
    class_knot.set_registry(Arc::clone(&registry));
    sensor1.set_registry(Arc::clone(&registry));
    sensor2.set_registry(Arc::clone(&registry));
    monitor.set_registry(Arc::clone(&registry));

    println!("  Registry attached to all 6 Knots.");
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 4 — System event bus (fan-in from all Knot event streams)
    // ─────────────────────────────────────────────────────────────────────────

    let (sys_tx, mut sys_rx) = broadcast::channel::<KnotEvent>(1024);

    // Forward each Knot's event bus into the shared system channel.
    // The API server subscribes to sys_tx so it sees all events.
    for (label, mut rx) in [
        ("root", root_evt_rx),
        ("env_hub", env_evt_rx),
        ("sensor1", sensor1_evt_rx),
        ("monitor", monitor_evt_rx),
    ] {
        let tx = sys_tx.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        let _ = tx.send(evt);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("event forwarder '{label}' lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 5 — Shutdown channels + run loops
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 4 — Starting Knot Run Loops");

    let (root_sd_tx, root_sd_rx) = watch::channel(false);
    let (env_sd_tx, env_sd_rx) = watch::channel(false);
    let (class_sd_tx, class_sd_rx) = watch::channel(false);
    let (s1_sd_tx, s1_sd_rx) = watch::channel(false);
    let (s2_sd_tx, s2_sd_rx) = watch::channel(false);
    let (monitor_sd_tx, monitor_sd_rx) = watch::channel(false);

    let root_task = tokio::spawn(async move { root.run(root_sd_rx).await });
    let env_task = tokio::spawn(async move { env_hub.run(env_sd_rx).await });
    let class_task = tokio::spawn(async move { class_knot.run(class_sd_rx).await });
    let s1_task = tokio::spawn(async move { sensor1.run(s1_sd_rx).await });
    let s2_task = tokio::spawn(async move { sensor2.run(s2_sd_rx).await });
    let monitor_task = tokio::spawn(async move { monitor.run(monitor_sd_rx).await });

    // Allow run loops to start and emit Started events
    tokio::time::sleep(Duration::from_millis(100)).await;
    drain_events("startup", &mut sys_rx);
    println!("  All 6 Knots running.\n");

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO A — Object property write
    //   Inject into EnvHub (parent of Sensor1) → routed DOWN to Sensor1.
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario A — Set temperature on Sensor1 via EnvHub");

    env_inbox
        .push(Packet::new(
            root_id,
            sensor1_id,
            200,
            Duration::from_secs(5),
            json!({ "op": "set", "property": "temperature", "value": 42.5 }),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify via the registry snapshot (updated by Object execute handler)
    if let Some(state) = registry.get_state_snapshot(&sensor1_id).await {
        println!(
            "  Sensor1 temperature = {} °C  (registry snapshot)",
            state
                .get("temperature")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        );
    } else {
        println!("  (no registry snapshot yet — ensure run loop processed the packet)");
    }
    drain_events("Scenario A", &mut sys_rx);
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO B — Object property read
    //   Inject get directly into Sensor1's inbox.
    //   Sensor1 queues a get_response packet back in its inbox, which the run
    //   loop routes UP → EnvHub → DOWN → Monitor (if Monitor is registered).
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario B — Read humidity from Sensor1");

    sensor1_inbox
        .push(Packet::new(
            monitor_id, // sender = Monitor
            sensor1_id, // addressed to Sensor1
            150,
            Duration::from_secs(5),
            json!({ "op": "get", "property": "humidity" }),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(120)).await;

    // The response packet (get_response) was addressed to monitor_id.
    // Sensor1 → EnvHub → Monitor (Monitor's run loop logged it).
    drain_events("Scenario B", &mut sys_rx);
    println!("  get_humidity dispatched (response routed Sensor1 → EnvHub → Monitor).");
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO C — Batch write + get_all
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario C — Batch write (set_many) then get_all on Sensor2");

    env_inbox
        .push(Packet::new(
            root_id,
            sensor2_id,
            180,
            Duration::from_secs(5),
            json!({
                "op": "set_many",
                "properties": {
                    "temperature": -5.0,
                    "humidity": 80.0,
                    "label": "rooftop"
                }
            }),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;

    if let Some(state) = registry.get_state_snapshot(&sensor2_id).await {
        println!(
            "  Sensor2 state: temperature={} °C  humidity={}%%  label={:?}",
            state
                .get("temperature")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            state
                .get("humidity")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            state.get("label").and_then(|v| v.as_str()).unwrap_or(""),
        );
    }
    drain_events("Scenario C", &mut sys_rx);
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO D — Action Knot (TransformEngine doubles n)
    //   Inject via EnvHub so routing EnvHub → Monitor is exercised.
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario D — Action Knot: double n=21 via EnvHub → Monitor");

    env_inbox
        .push(Packet::new(
            root_id,
            monitor_id,
            255, // max priority
            Duration::from_secs(5),
            json!({ "n": 21.0 }),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(120)).await;

    let mut saw_action = false;
    while let Ok(evt) = sys_rx.try_recv() {
        match &evt {
            KnotEvent::ActionExecuted {
                engine_name, logs, ..
            } => {
                println!("  ActionExecuted — engine: {engine_name}");
                for log in logs {
                    println!("    log: {log}");
                }
                saw_action = true;
            }
            KnotEvent::PacketReceived { packet } => {
                if packet.target_id == monitor_id {
                    println!("  Monitor received packet — payload: {}", packet.payload);
                }
            }
            _ => {}
        }
    }
    // The echo/transform response packet is addressed back to root_id;
    // Monitor → EnvHub → Root Hub would process it next iteration.
    if !saw_action {
        println!("  (ActionExecuted event may still be in flight)");
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO E — TTL expiry → DeadLetter
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario E — Expired packet → DeadLetter(TtlExpired)");

    env_inbox
        .push(Packet::new(
            root_id,
            sensor1_id,
            100,
            Duration::ZERO, // already expired at construction time
            json!({ "op": "set", "property": "active", "value": false }),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(80)).await;

    let mut saw_dead_letter = false;
    while let Ok(evt) = sys_rx.try_recv() {
        if let KnotEvent::DeadLetter { reason, .. } = &evt {
            println!("  DeadLetter — reason: {reason}");
            saw_dead_letter = true;
        }
    }
    if !saw_dead_letter {
        println!("  (DeadLetter event may still be in flight)");
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO F — NoRoute
    //   Root Hub has no parent. Target unknown to Root → DeadLetter(NoRoute).
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario F — Unknown target on root → DeadLetter(NoRoute)");

    root_inbox
        .push(Packet::new(
            root_id,
            Uuid::new_v4(), // completely unknown UUID
            50,
            Duration::from_secs(5),
            json!({ "op": "ping" }),
        ))
        .await;

    tokio::time::sleep(Duration::from_millis(80)).await;

    while let Ok(evt) = sys_rx.try_recv() {
        if let KnotEvent::DeadLetter { reason, .. } = &evt {
            println!("  DeadLetter — reason: {reason}");
        }
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // SCENARIO G — Volatile generic state (ObjectID.PropertyID)
    // ─────────────────────────────────────────────────────────────────────────

    section("Scenario G — Volatile generic state (InMemoryStore)");

    {
        // Demonstrate the generic key/value store that exists on every Knot.
        // We create a fresh temporary Knot for this illustration.
        let (mut demo, _) = Knot::new_hub(99, CellMeta::new("Demo"), 0, None, None);
        let counter_key = StateKey::new(demo.id(), "counter");
        demo.set(counter_key.clone(), json!(0));
        for i in 1..=3 {
            demo.set(counter_key.clone(), json!(i));
        }
        println!("  volatile counter = {:?}", demo.get(&counter_key).unwrap());
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 6 — SQLite persistence (.hflow)
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 5 — SQLite Persistence (workspace.hflow)");

    let db_path = "/tmp/hubflow_demo.hflow";

    match HflowStore::open(db_path).await {
        Ok(store) => {
            // ── Save topology ──────────────────────────────────────────────
            store
                .save_knot(root_id, 0, "RootHub", None, "Hub", 0, None, None)
                .await
                .unwrap();
            store
                .save_knot(
                    env_hub_id,
                    1,
                    "EnvHub",
                    Some("Environment sub-system"),
                    "Hub",
                    1,
                    Some(root_id),
                    None,
                )
                .await
                .unwrap();
            store
                .save_knot(
                    class_id,
                    2,
                    "SensorClass",
                    None,
                    "Class",
                    2,
                    Some(env_hub_id),
                    None,
                )
                .await
                .unwrap();
            store
                .save_knot(
                    sensor1_id,
                    3,
                    "Sensor1",
                    Some("Indoor sensor"),
                    "Object",
                    2,
                    Some(env_hub_id),
                    Some(sensor_class.id),
                )
                .await
                .unwrap();
            store
                .save_knot(
                    sensor2_id,
                    4,
                    "Sensor2",
                    Some("Outdoor sensor"),
                    "Object",
                    2,
                    Some(env_hub_id),
                    Some(sensor_class.id),
                )
                .await
                .unwrap();
            store
                .save_knot(
                    monitor_id,
                    5,
                    "Monitor",
                    Some("Doubles numeric values"),
                    "Action",
                    2,
                    Some(env_hub_id),
                    None,
                )
                .await
                .unwrap();

            // ── Save class schema ──────────────────────────────────────────
            store.save_class(sensor_class.clone()).await.unwrap();

            // ── Save editor links ──────────────────────────────────────────
            store
                .save_link(Uuid::new_v4(), monitor_id, sensor1_id, "action_to_object")
                .await
                .unwrap();

            println!("  Saved to {db_path}");

            // ── Verify load ────────────────────────────────────────────────
            let knots = store.load_knots().await.unwrap();
            let classes = store.load_classes().await.unwrap();
            let links = store.load_links().await.unwrap();

            println!(
                "  Loaded : {} knots, {} classes, {} links",
                knots.len(),
                classes.len(),
                links.len()
            );

            if let Some(c) = classes.first() {
                println!(
                    "  Class  : '{}' — {} properties",
                    c.name,
                    c.properties.len()
                );
                for p in &c.properties {
                    let unit = p
                        .unit
                        .as_deref()
                        .map(|u| format!(" [{u}]"))
                        .unwrap_or_default();
                    println!("    • {} : {}{}", p.name, p.data_type, unit);
                }
            }

            // ── Demonstrate PersistentStore key/value ──────────────────────
            use core::state::PersistentStore as _;
            let prop_key = StateKey::new(sensor1_id, "temperature");
            store.save(prop_key.clone(), json!(42.5)).await;
            let loaded = store.load(&prop_key).await;
            println!("  Persistent sensor1.temperature = {:?}", loaded);
        }
        Err(e) => eprintln!("  SQLite error: {e}"),
    }
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 7 — API server
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 6 — API Server (axum on :8080)");

    let api_server = ApiServer::new(Arc::clone(&registry), root_inbox.clone(), sys_tx.clone());

    let api_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let api_task = tokio::spawn(async move {
        if let Err(e) = api_server.serve(api_addr).await {
            // EADDRINUSE is common in repeated runs; suppress for demo.
            if !e.to_string().contains("Address already in use") {
                eprintln!("  API server error: {e}");
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    println!("  Listening on http://{api_addr}");
    println!();
    println!("  Endpoints:");
    println!("    GET  http://{api_addr}/api/graph            → Knot topology JSON");
    println!("    GET  http://{api_addr}/api/knot/<uuid>      → Single Knot details");
    println!("    GET  http://{api_addr}/api/knot/<uuid>/state→ Object property map");
    println!("    POST http://{api_addr}/api/packet           → Inject a packet");
    println!("    WS   ws://{api_addr}/api/ws                 → Real-time event stream");
    println!();
    println!("  Example curl commands:");
    println!("    curl http://{api_addr}/api/graph");
    println!("    curl -X POST http://{api_addr}/api/packet \\",);
    println!("         -H 'Content-Type: application/json' \\",);
    println!(
        "         -d '{{\"target_id\":\"{sensor1_id}\",\"priority\":200,\"payload\":{{\"op\":\"get_all\"}}}}'",
    );
    println!();

    // Keep the server alive briefly so a human can interact with it.
    println!("  (Server live for 5 seconds — send requests now)");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 8 — Graceful shutdown
    // ─────────────────────────────────────────────────────────────────────────

    section("Step 7 — Graceful Shutdown");

    let _ = monitor_sd_tx.send(true);
    let _ = s1_sd_tx.send(true);
    let _ = s2_sd_tx.send(true);
    let _ = class_sd_tx.send(true);
    let _ = env_sd_tx.send(true);
    let _ = root_sd_tx.send(true);
    api_task.abort();

    let _ = tokio::join!(
        root_task,
        env_task,
        class_task,
        s1_task,
        s2_task,
        monitor_task
    );

    tokio::time::sleep(Duration::from_millis(80)).await;

    while let Ok(evt) = sys_rx.try_recv() {
        if matches!(evt, KnotEvent::Stopped { .. }) {
            println!("  {evt}");
        }
    }

    println!();
    println!("All Knots stopped. Workspace saved to {db_path}.");
    println!();
}

// ── UI helpers ────────────────────────────────────────────────────────────────

fn banner() {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  HubFlow v0.2 — Triumvirat  ·  SQLite  ·  axum API             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
}

fn section(title: &str) {
    println!("── {title}");
}

/// Drains and discards buffered events, printing a compact summary.
fn drain_events(phase: &str, rx: &mut broadcast::Receiver<KnotEvent>) {
    let mut routing = 0u32;
    let mut state = 0u32;
    let mut dead = 0u32;
    while let Ok(evt) = rx.try_recv() {
        match evt {
            KnotEvent::PacketForwardedDown { .. } | KnotEvent::PacketForwardedUp { .. } => {
                routing += 1;
            }
            KnotEvent::ObjectStateChanged { .. } | KnotEvent::ActionExecuted { .. } => {
                state += 1;
            }
            KnotEvent::DeadLetter { .. } => dead += 1,
            _ => {}
        }
    }
    if routing + state + dead > 0 {
        println!("  [{phase}] events: {routing} routing, {state} state-change, {dead} dead-letter");
    }
}
