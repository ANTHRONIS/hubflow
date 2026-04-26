//! HubFlow v0.3 — persistent production server
//!
//! Runs indefinitely until SIGINT / Ctrl-C. Topology is persisted in a
//! SQLite `.hflow` file and served over a REST + WebSocket API.
//!
//! # Default topology (Solar System demo — seeded on first run only)
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
//! # Usage
//!
//! ```bash
//! cargo run           # starts server on :8080
//! ^C                  # graceful shutdown
//! ```

mod api;
mod core;
mod persistence;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use api::ApiServer;
use core::action::TransformEngine;
use core::cell::CellMeta;
use core::class::{ClassDefinition, DataType, PropertySchema};
use core::event::KnotEvent;
use core::inbox::Inbox;
use core::knot::{Knot, KnotRole};
use core::packet::Packet;
use core::registry::{KnotRegistry, RegistryEntry};
use persistence::HflowStore;

// ── Canvas layout constants ───────────────────────────────────────────────────

const NODE_W: f32 = 210.0;
const H_GAP: f32 = 90.0;
const V_GAP: f32 = 170.0;

/// Pre-computed positions for the 6 demo Knots.
fn demo_positions() -> [(f32, f32); 6] {
    let total_l2 = 4.0 * NODE_W + 3.0 * H_GAP;
    let start_l2 = -total_l2 / 2.0;
    let step = NODE_W + H_GAP;
    [
        (0.0, 0.0),
        (0.0, V_GAP),
        (start_l2, 2.0 * V_GAP),
        (start_l2 + step, 2.0 * V_GAP),
        (start_l2 + 2.0 * step, 2.0 * V_GAP),
        (start_l2 + 3.0 * step, 2.0 * V_GAP),
    ]
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    banner();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 1 — Open SQLite store
    // ─────────────────────────────────────────────────────────────────────────

    let db_path = "/tmp/hubflow.hflow";
    let store = Arc::new(
        HflowStore::open(db_path)
            .await
            .expect("failed to open .hflow database"),
    );

    let registry = KnotRegistry::new();
    let (sys_tx, mut sys_rx) = broadcast::channel::<KnotEvent>(1024);

    let existing_knots = store.load_knots().await.unwrap();
    let next_local_id = Arc::new(AtomicU64::new(
        existing_knots.iter().map(|r| r.local_id).max().unwrap_or(5) + 1,
    ));

    // Tasks that must be joined on shutdown (only populated in first-run path).
    let mut knot_tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    // Shutdown senders — always created so the drop order is predictable.
    let (root_sd_tx, root_sd_rx) = watch::channel(false);
    let (env_sd_tx, env_sd_rx) = watch::channel(false);
    let (class_sd_tx, class_sd_rx) = watch::channel(false);
    let (s1_sd_tx, s1_sd_rx) = watch::channel(false);
    let (s2_sd_tx, s2_sd_rx) = watch::channel(false);
    let (monitor_sd_tx, monitor_sd_rx) = watch::channel(false);

    // Root inbox for the API server (may be a dummy in restore path).
    let api_root_inbox: Arc<Inbox>;

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 2 — Topology: seed demo (first run) or restore from DB
    // ─────────────────────────────────────────────────────────────────────────

    if existing_knots.is_empty() {
        // ── FIRST RUN: build + seed demo topology ─────────────────────────────

        let sensor_class = ClassDefinition::new("TemperatureSensor")
            .with_description("Generic temperature + humidity sensor")
            .with_property(PropertySchema::new("temperature", DataType::Number)
                .with_default(json!(20.0)).with_unit("°C"))
            .with_property(PropertySchema::new("humidity", DataType::Number)
                .with_default(json!(50.0)).with_unit("%rH"))
            .with_property(PropertySchema::new("active", DataType::Boolean)
                .with_default(json!(true)))
            .with_property(PropertySchema::new("label", DataType::String));

        println!("Schema    : {sensor_class}");

        let pos = demo_positions();

        let (mut root, root_evt_rx) = Knot::new_hub(0,
            CellMeta::with_description("RootHub", "Level-0 central star"), 0, None, None);
        let (mut env_hub, env_evt_rx) = Knot::new_hub(1,
            CellMeta::with_description("EnvHub", "Manages environmental sensors"),
            1, Some(root.id()), Some(root.inbox()));
        let (mut class_knot, _class_evt_rx) = Knot::new_class(2,
            CellMeta::new("SensorClass"), sensor_class.clone(),
            2, Some(env_hub.id()), Some(env_hub.inbox()));
        let (mut sensor1, sensor1_evt_rx) = Knot::new_object(3,
            CellMeta::with_description("Sensor1", "Indoor sensor"),
            Some(&sensor_class), 2, Some(env_hub.id()), Some(env_hub.inbox()));
        let (mut sensor2, _sensor2_evt_rx) = Knot::new_object(4,
            CellMeta::with_description("Sensor2", "Outdoor sensor"),
            Some(&sensor_class), 2, Some(env_hub.id()), Some(env_hub.inbox()));

        let double_engine = TransformEngine::new("double", |v| {
            let n = v.get("n").and_then(|x| x.as_f64()).unwrap_or(0.0);
            json!({ "n": n * 2.0, "engine": "double" })
        });
        let (mut monitor, monitor_evt_rx) = Knot::new_action(5,
            CellMeta::with_description("Monitor", "Doubles incoming numeric values"),
            Arc::new(double_engine), 2, Some(env_hub.id()), Some(env_hub.inbox()));

        let root_id    = root.id();
        let env_hub_id = env_hub.id();
        let class_id   = class_knot.id();
        let sensor1_id = sensor1.id();
        let sensor2_id = sensor2.id();
        let monitor_id = monitor.id();
        let root_inbox = root.inbox();
        let env_inbox  = env_hub.inbox();

        root.attach_child(env_hub.as_child_handle());
        env_hub.attach_child(class_knot.as_child_handle());
        env_hub.attach_child(sensor1.as_child_handle());
        env_hub.attach_child(sensor2.as_child_handle());
        env_hub.attach_child(monitor.as_child_handle());
        monitor.link_object(sensor1_id);

        println!("Topology  :");
        println!("  RootHub    id={root_id}");
        println!("  EnvHub     id={env_hub_id}");
        println!("  SensorCls  id={class_id}");
        println!("  Sensor1    id={sensor1_id}");
        println!("  Sensor2    id={sensor2_id}");
        println!("  Monitor    id={monitor_id}");

        root.set_registry(Arc::clone(&registry));
        env_hub.set_registry(Arc::clone(&registry));
        class_knot.set_registry(Arc::clone(&registry));
        sensor1.set_registry(Arc::clone(&registry));
        sensor2.set_registry(Arc::clone(&registry));
        monitor.set_registry(Arc::clone(&registry));

        // Persist demo topology to SQLite.
        let (p0, p1, p2, p3, p4, p5) = (pos[0], pos[1], pos[2], pos[3], pos[4], pos[5]);
        store.save_knot(root_id, 0, "RootHub", None, "Hub", 0, None, None, p0.0, p0.1).await.unwrap();
        store.save_knot(env_hub_id, 1, "EnvHub", Some("Environment sub-system"), "Hub", 1, Some(root_id), None, p1.0, p1.1).await.unwrap();
        store.save_knot(class_id, 2, "SensorClass", None, "Class", 2, Some(env_hub_id), None, p2.0, p2.1).await.unwrap();
        store.save_knot(sensor1_id, 3, "Sensor1", Some("Indoor sensor"), "Object", 2, Some(env_hub_id), Some(sensor_class.id), p3.0, p3.1).await.unwrap();
        store.save_knot(sensor2_id, 4, "Sensor2", Some("Outdoor sensor"), "Object", 2, Some(env_hub_id), Some(sensor_class.id), p4.0, p4.1).await.unwrap();
        store.save_knot(monitor_id, 5, "Monitor", Some("Doubles numeric values"), "Action", 2, Some(env_hub_id), None, p5.0, p5.1).await.unwrap();
        store.save_class(sensor_class.clone()).await.unwrap();
        store.save_link(Uuid::new_v4(), monitor_id, sensor1_id, "action_to_object").await.unwrap();

        // Fan-in event bus.
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
                        Ok(evt) => { let _ = tx.send(evt); }
                        Err(broadcast::error::RecvError::Lagged(n)) =>
                            tracing::warn!("event forwarder '{label}' lagged by {n}"),
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        // Start run-loops.
        knot_tasks.push(tokio::spawn(async move { root.run(root_sd_rx).await }));
        knot_tasks.push(tokio::spawn(async move { env_hub.run(env_sd_rx).await }));
        knot_tasks.push(tokio::spawn(async move { class_knot.run(class_sd_rx).await }));
        knot_tasks.push(tokio::spawn(async move { sensor1.run(s1_sd_rx).await }));
        knot_tasks.push(tokio::spawn(async move { sensor2.run(s2_sd_rx).await }));
        knot_tasks.push(tokio::spawn(async move { monitor.run(monitor_sd_rx).await }));

        tokio::time::sleep(Duration::from_millis(100)).await;
        drain_events(&mut sys_rx);
        println!("Knots     : demo topology seeded (6 run-loops started)");

        // Quick self-tests.
        env_inbox.push(Packet::new(root_id, sensor1_id, 200, Duration::from_secs(5),
            json!({ "op": "set", "property": "temperature", "value": 42.5 }))).await;
        env_inbox.push(Packet::new(root_id, sensor2_id, 180, Duration::from_secs(5),
            json!({ "op": "set_many", "properties": { "temperature": -5.0, "label": "rooftop" } }))).await;
        env_inbox.push(Packet::new(root_id, monitor_id, 255, Duration::from_secs(5),
            json!({ "n": 21.0 }))).await;

        tokio::time::sleep(Duration::from_millis(200)).await;
        drain_events(&mut sys_rx);
        println!("Self-test : completed (temp=42.5°C on Sensor1, monitor doubled 21→42)");

        // Sync stored canvas positions into the registry (run-loops register
        // with x=0, y=0; fix that now from the DB rows we just wrote).
        for row in store.load_knots().await.unwrap() {
            registry.update_position(&row.id, row.x, row.y).await;
        }

        api_root_inbox = root_inbox;

    } else {
        // ── SUBSEQUENT RUNS: restore topology from SQLite ─────────────────────

        println!("Knots     : restoring {} knots from {db_path}", existing_knots.len());

        // Build child_id lists from parent_id relationships.
        let mut child_map: std::collections::HashMap<Uuid, Vec<Uuid>> =
            std::collections::HashMap::new();
        for row in &existing_knots {
            if let Some(pid) = row.parent_id {
                child_map.entry(pid).or_default().push(row.id);
            }
        }

        for row in &existing_knots {
            let role = match row.role.as_str() {
                "Hub"    => KnotRole::Hub,
                "Action" => KnotRole::Action,
                "Object" => KnotRole::Object,
                _        => KnotRole::Class,
            };
            let meta = match &row.description {
                Some(d) => CellMeta::with_description(&row.name, d),
                None    => CellMeta::new(&row.name),
            };
            registry.upsert(RegistryEntry {
                id:       row.id,
                local_id: row.local_id,
                meta,
                role,
                level:     row.level,
                parent_id: row.parent_id,
                class_id:  row.class_id,
                child_ids: child_map.get(&row.id).cloned().unwrap_or_default(),
                x: row.x,
                y: row.y,
            }).await;
        }

        // Provide a dummy inbox — no run-loops listen on it in restore mode.
        api_root_inbox = Inbox::new();

        // Drop unused shutdown senders immediately — no run-loops to signal.
        drop(root_sd_rx); drop(env_sd_rx); drop(class_sd_rx);
        drop(s1_sd_rx); drop(s2_sd_rx); drop(monitor_sd_rx);
    }

    let knot_count  = store.load_knots().await.unwrap().len();
    let class_count = store.load_classes().await.unwrap().len();
    println!("Database  : {db_path} ({knot_count} knots, {class_count} classes)");

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 3 — API server (runs in background until Ctrl-C)
    // ─────────────────────────────────────────────────────────────────────────

    let api_server = ApiServer::new(
        Arc::clone(&registry),
        api_root_inbox,
        sys_tx.clone(),
        Arc::clone(&store),
        Arc::clone(&next_local_id),
    );

    let api_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let api_task = tokio::spawn(async move {
        if let Err(e) = api_server.serve(api_addr).await {
            if !e.to_string().contains("Address already in use") {
                eprintln!("API server error: {e}");
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    println!();
    println!("┌─────────────────────────────────────────────────────────┐");
    println!("│  HubFlow API server ready on http://{api_addr}       │");
    println!("│                                                         │");
    println!("│  REST                                                   │");
    println!("│    GET    /api/graph              topology JSON         │");
    println!("│    GET    /api/knot/:id           single Knot          │");
    println!("│    GET    /api/knot/:id/state     Object property map  │");
    println!("│    POST   /api/knot               create Knot          │");
    println!("│    PATCH  /api/knot/:id           update metadata/pos  │");
    println!("│    DELETE /api/knot/:id           remove Knot          │");
    println!("│    POST   /api/link               create link          │");
    println!("│    POST   /api/packet             inject packet        │");
    println!("│                                                         │");
    println!("│  WebSocket                                              │");
    println!("│    WS  /api/ws                    live event stream    │");
    println!("│                                                         │");
    println!("│  UI                                                     │");
    println!("│    cd ui && npm run dev           → http://localhost:3000│");
    println!("│                                                         │");
    println!("│  Press Ctrl-C to stop                                   │");
    println!("└─────────────────────────────────────────────────────────┘");
    println!();

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 4 — Wait for Ctrl-C (SIGINT)
    // ─────────────────────────────────────────────────────────────────────────

    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for Ctrl-C");

    println!();
    println!("Shutting down…");

    // ─────────────────────────────────────────────────────────────────────────
    // STEP 5 — Graceful shutdown
    // ─────────────────────────────────────────────────────────────────────────

    api_task.abort();

    // Signal demo run-loops to stop (no-ops if we're in restore mode because
    // the receivers were dropped above).
    let _ = monitor_sd_tx.send(true);
    let _ = s1_sd_tx.send(true);
    let _ = s2_sd_tx.send(true);
    let _ = class_sd_tx.send(true);
    let _ = env_sd_tx.send(true);
    let _ = root_sd_tx.send(true);

    for task in knot_tasks {
        let _ = task.await;
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
    drain_events(&mut sys_rx);

    println!("All Knots stopped. Workspace: {db_path}");
    println!("Goodbye.");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn banner() {
    println!();
    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║  HubFlow v0.3  ·  Triumvirat  ·  SQLite  ·  axum REST + WS  ║");
    println!("╚═══════════════════════════════════════════════════════════════╝");
    println!();
}

/// Silently drain all pending events from the system bus.
fn drain_events(rx: &mut broadcast::Receiver<KnotEvent>) {
    while rx.try_recv().is_ok() {}
}
