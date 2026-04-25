//! HubFlow — bootstrap demo
//!
//! Topology assembled here:
//!
//! ```text
//!  ┌──────────────────────────────┐
//!  │   Root Hub  (level 0)        │
//!  │   role: Hub                  │
//!  └──────┬──────────────┬────────┘
//!         │              │
//!  ┌──────▼──────┐ ┌─────▼───────┐
//!  │ ActionKnot  │ │ ObjectKnot  │
//!  │ (level 1)   │ │ (level 1)   │
//!  └─────────────┘ └─────────────┘
//! ```
//!
//! Scenarios exercised:
//!
//! 1. Direct delivery   — packet addressed to Root Hub itself.
//! 2. Forward down      — packet addressed to a child Satellite.
//! 3. Forward up        — child sends a packet for an unknown target;
//!                        the Hub receives it and (no further parent)
//!                        emits a DeadLetter(NoRoute).
//! 4. TTL expiry        — a packet with zero TTL is dead-lettered immediately.
//! 5. Graceful shutdown — all three Knots stop cleanly.

mod core;

use std::time::Duration;

use serde_json::json;
use tokio::sync::watch;
use uuid::Uuid;

use core::cell::CellMeta;
use core::event::KnotEvent;
use core::knot::{Knot, KnotRole};
use core::packet::Packet;
use core::state::StateKey;

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Structured logging: RUST_LOG=debug cargo run
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    println!();
    println!("╔══════════════════════════════════════════════╗");
    println!("║          HubFlow — bootstrap demo            ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();

    // ── 1. Build topology ─────────────────────────────────────────────────────

    // Root Hub (level 0, no parent)
    let (mut hub, mut hub_events) = Knot::new(
        0,
        CellMeta::with_description("RootHub", "Central hub — the star of the system"),
        KnotRole::Hub,
        0,
        None,
        None,
    );

    // Action Satellite (level 1, parent = hub)
    let (mut action_knot, mut action_events) = Knot::new(
        1,
        CellMeta::with_description("ActionKnot", "Handles executable logic"),
        KnotRole::Action,
        1,
        Some(hub.id()),
        Some(hub.inbox()), // upward routing: unknown targets go to Hub
    );

    // Object Satellite (level 1, parent = hub)
    let (mut object_knot, mut object_events) = Knot::new(
        2,
        CellMeta::with_description("ObjectKnot", "Holds structured data"),
        KnotRole::Object,
        1,
        Some(hub.id()),
        Some(hub.inbox()),
    );

    // Register both satellites in the Hub's routing table
    hub.attach_child(action_knot.as_child_handle());
    hub.attach_child(object_knot.as_child_handle());

    // Capture IDs before moving the Knots into their tasks
    let hub_id = hub.id();
    let action_id = action_knot.id();
    let object_id = object_knot.id();
    let action_inbox = action_knot.inbox();
    let hub_inbox = hub.inbox();

    println!("Topology");
    println!("  RootHub     id = {hub_id}");
    println!("  ActionKnot  id = {action_id}");
    println!("  ObjectKnot  id = {object_id}");
    println!();

    // ── 2. Volatile state example ─────────────────────────────────────────────
    //
    // Write a property to the Hub's in-memory store before the run loop starts.
    // ObjectID = hub_id, PropertyID = "status"
    let status_key = StateKey::new(hub_id, "status");
    hub.set(status_key.clone(), json!("initialising"));
    println!("State  [hub.status] = {:?}", hub.get(&status_key).unwrap());
    println!();

    // ── 3. Create shutdown channels ───────────────────────────────────────────

    let (hub_shutdown_tx, hub_shutdown_rx) = watch::channel(false);
    let (action_shutdown_tx, action_shutdown_rx) = watch::channel(false);
    let (object_shutdown_tx, object_shutdown_rx) = watch::channel(false);

    // ── 4. Spawn Knot run-loops ───────────────────────────────────────────────

    let hub_task = tokio::spawn(async move {
        hub.run(hub_shutdown_rx).await;
    });

    let action_task = tokio::spawn(async move {
        action_knot.run(action_shutdown_rx).await;
    });

    let object_task = tokio::spawn(async move {
        object_knot.run(object_shutdown_rx).await;
    });

    // Give the event loops a moment to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ── 5. Scenario 1 — direct delivery to Hub ────────────────────────────────

    section("Scenario 1 — direct delivery to Hub");
    let p = Packet::new(
        action_id,
        hub_id,
        200,
        Duration::from_secs(5),
        json!({ "op": "ping" }),
    );
    hub_inbox.push(p).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    drain_events("Hub", &mut hub_events);

    // ── 6. Scenario 2 — forward down (Hub → ActionKnot) ──────────────────────

    section("Scenario 2 — Hub forwards packet ↓ to ActionKnot");
    let p = Packet::new(
        hub_id,
        action_id,
        128,
        Duration::from_secs(5),
        json!({ "op": "compute", "x": 42 }),
    );
    hub_inbox.push(p).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    drain_events("Hub", &mut hub_events);
    drain_events("ActionKnot", &mut action_events);

    // ── 7. Scenario 3 — forward up (ActionKnot → Hub, unknown target) ─────────

    section("Scenario 3 — ActionKnot forwards packet ↑ (unknown target → Hub dead-letters)");
    let unknown = Uuid::new_v4();
    let p = Packet::new(
        action_id,
        unknown, // target unknown to both Action and Hub
        64,
        Duration::from_secs(5),
        json!({ "op": "broadcast" }),
    );
    action_inbox.push(p).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drain_events("ActionKnot", &mut action_events);
    drain_events("Hub", &mut hub_events);

    // ── 8. Scenario 4 — TTL expiry → dead-letter ─────────────────────────────

    section("Scenario 4 — expired packet → DeadLetter(TtlExpired)");
    let p = Packet::new(
        Uuid::new_v4(),
        hub_id,
        255,
        Duration::ZERO, // already expired
        json!({ "op": "stale" }),
    );
    hub_inbox.push(p).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    drain_events("Hub", &mut hub_events);

    // ── 9. Graceful shutdown ──────────────────────────────────────────────────

    section("Shutting down all Knots");
    let _ = action_shutdown_tx.send(true);
    let _ = object_shutdown_tx.send(true);
    let _ = hub_shutdown_tx.send(true);

    let _ = tokio::join!(hub_task, action_task, object_task);

    tokio::time::sleep(Duration::from_millis(50)).await;
    drain_events("Hub", &mut hub_events);
    drain_events("ActionKnot", &mut action_events);
    drain_events("ObjectKnot", &mut object_events);

    println!();
    println!("All Knots stopped. Goodbye.");
    println!();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Prints a section header.
fn section(title: &str) {
    println!("── {title}");
}

/// Drains all available events from a broadcast receiver and pretty-prints them.
fn drain_events(label: &str, rx: &mut tokio::sync::broadcast::Receiver<KnotEvent>) {
    loop {
        match rx.try_recv() {
            Ok(event) => println!("  [{label}] {event}"),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                println!("  [{label}] ⚠ lagged — missed {n} events");
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
        }
    }
}
