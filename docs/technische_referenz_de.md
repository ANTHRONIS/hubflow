# HubFlow Technische Referenz

> Version 0.2.0 — Rust-Kern

## Inhaltsverzeichnis
1. Überblick
2. Architektur — Sonnensystem-Topologie
3. Kerntypen
4. Weiterleitungsprotokoll
5. Das Triumvirat — Class, Object, Action
6. Paketschema
7. Zustandsverwaltung
8. Das .hflow-Datenbankschema
9. Frontend-API (WebSocket + REST)
10. Glossar

---

## 1. Überblick

HubFlow ist ein dezentralisiertes, ereignisgesteuertes System, das auf einer Hierarchie autonomer **Knoten** aufbaut. Jeder Knoten ist eine unabhängig adressierbare Einheit mit eigenem asynchronen Ereignis-Loop, priorisiertem Posteingang und austauschbarer Logik-Engine. Die Kommunikation erfolgt ausschließlich paketbasiert: kein gemeinsamer Speicher, keine direkten Funktionsaufrufe über Knotengrenzen hinweg.

Zentrale Designziele:
- **Dezentralisierung** — kein einzelner Fehlerpunkt; jeder Knoten kann eigenständig weiterleiten.
- **Prioritätsbasierte Verarbeitung** — Pakete mit hoher Priorität überspringen die Warteschlange.
- **Beobachtbarkeit** — jede Weiterleitungsentscheidung erzeugt ein strukturiertes Ereignis.
- **Portabilität** — der gesamte Arbeitsbereich ist eine einzige `.hflow`-SQLite-Datei.

---

## 2. Architektur — Sonnensystem-Topologie

HubFlow verwendet eine Baumtopologie, die metaphorisch als „Sonnensystem" beschrieben wird:

```
Ebene 0   ┌────────────────────┐
          │      Root Hub       │   ← Der „Stern"
          └─────────┬──────────┘
                    │
Ebene 1   ┌─────────┼──────────┐
          ▼         ▼          ▼
        Hub A     Hub B      Hub C   ← „Planeten"
        /  |  \
Ebene 2  Act  Obj  Cls              ← „Satelliten"
```

| Ebene | Rolle | Metapher | Verantwortlichkeit |
|-------|-------|----------|--------------------|
| 0 | Hub | Stern | Systemwurzel, globale Weiterleitung |
| 1+ | Hub | Planet | Teilsystem-Koordinator |
| Beliebig | Action | Satellit | Ausführbare Logik |
| Beliebig | Object | Satellit | Strukturierte Daten |
| Beliebig | Class | Satellit | Schema / Vorlage |

### KnotRole

```rust
pub enum KnotRole { Hub, Action, Object, Class }
```

Die Rolle dient ausschließlich der **semantischen Klassifizierung**. Alle Rollen nehmen am identischen Weiterleitungsprotokoll teil.

---

## 3. Kerntypen

### Cell (Identitätsprimitiv)

```
LocalId   = u64            // schnelle prozessinterne Suche
CellMeta  = { name, description? }
Uuid      = UUID v4        // global eindeutig, systemübergreifende Identität
```

### Knoten

Die zentrale Abstraktion. Jeder Knoten besitzt:

| Feld | Typ | Zweck |
|------|-----|-------|
| `id` | `Uuid` | Globale Identität |
| `local_id` | `u64` | Prozessinterner Index |
| `meta` | `CellMeta` | Name + Beschreibung |
| `role` | `KnotRole` | Semantische Klassifizierung |
| `level` | `u32` | Baumtiefe |
| `parent_id` | `Option<Uuid>` | UUID des übergeordneten Knotens |
| `inbox` | `Arc<Inbox>` | Asynchrone Prioritätswarteschlange |
| `variant` | `KnotVariant` | Rollenspezifischer Zustand |
| `volatile` | `InMemoryStore` | RAM-Zustandsspeicher |
| `persistent` | `Arc<dyn PersistentStore>` | Dauerhaftes Backend |
| `registry` | `Option<Arc<KnotRegistry>>` | Knotenübergreifendes Verzeichnis |

### Posteingang (Inbox)

Der `Inbox` ist ein `BinaryHeap<PrioritizedPacket>`, der durch einen `tokio::sync::Mutex` geschützt und über einen `tokio::sync::Notify` benachrichtigt wird. Er garantiert:
- **Prioritätsreihenfolge**: Pakete mit `priority = 255` werden vor `priority = 0` verarbeitet.
- **FIFO bei gleicher Priorität**: Früher eingetroffene Pakete werden zuerst bedient.
- **Abbruchsicherheit**: `pop()` nutzt den `Notify`-Permit-Mechanismus — kein Aufweckimpuls geht verloren.

---

## 4. Weiterleitungsprotokoll

### Regeln (werden der Reihe nach ausgewertet)

1. **TTL-Prüfung** — wenn `now - created_at > ttl`, wird `DeadLetter(TtlExpired)` ausgelöst und das Paket verworfen.
2. **Lokale Zustellung** — wenn `target_id == self.id`, wird `execute()` aufgerufen.
3. **Weiterleitung abwärts** — wenn `target_id ∈ children`, wird das Paket in den Posteingang des Kindknotens gelegt.
4. **Weiterleitung aufwärts** — wenn ein übergeordneter Knoten existiert, wird das Paket in dessen Posteingang gelegt.
5. **Keine Route** — `DeadLetter(NoRoute)` wird ausgelöst und das Paket verworfen.

### Herunterfahren

Knoten verwenden einen `tokio::sync::watch::Receiver<bool>`. Das Senden von `true` oder das Schließen des Senders löst einen sauberen Stopp aus. Der Ausführungs-Loop verarbeitet das aktuelle Paket noch vollständig, bevor er anhält.

### Ereignisbus (Event Bus)

Jeder Knoten stellt einen `broadcast::Sender<KnotEvent>` bereit. Abonnenten empfangen:

| Ereignis | Auslöser |
|----------|----------|
| `Started` | Ausführungs-Loop beginnt |
| `Stopped` | Ausführungs-Loop endet |
| `PacketReceived` | Lokale Zustellung |
| `PacketForwardedDown` | Weiterleitung an Kindknoten |
| `PacketForwardedUp` | Eskalation an übergeordneten Knoten |
| `DeadLetter` | Nicht zustellbares Paket |
| `ObjectStateChanged` | Eigenschaft eines Object-Knotens wurde geschrieben |
| `ActionExecuted` | Logik-Engine eines Action-Knotens abgeschlossen |

---

## 5. Das Triumvirat — Class, Object, Action

### Class-Knoten

Speichert eine `ClassDefinition` — eine geordnete Liste von `PropertySchema`-Einträgen:

```rust
PropertySchema {
    name: String,
    data_type: DataType,   // String | Number | Boolean | Json
    default_value: Option<Value>,
    unit: Option<String>,
    description: Option<String>,
}
```

Wenn ein Class-Knoten ein Paket empfängt, antwortet er mit seinem Schema (`op: "class_schema"`).

### Object-Knoten

Hält eine `HashMap<String, Value>` (den **gemeinsamen Zustand**), der aus dem `default_state()` einer Class initialisiert wird. Vier Operationen werden über den Paket-Payload verarbeitet:

| `op` | Beschreibung | Antwort `op` |
|------|--------------|--------------|
| `get` | Eine Eigenschaft lesen | `get_response` |
| `set` | Eine Eigenschaft schreiben | `set_ack` |
| `set_many` | Mehrere Eigenschaften schreiben | `set_many_ack` |
| `get_all` | Alle Eigenschaften lesen | `get_all_response` |

Zustandsänderungen lösen `ObjectStateChanged`-Ereignisse aus und werden mit der `KnotRegistry` synchronisiert, damit Action-Knoten darauf zugreifen können.

### Action-Knoten

Führt bei jeder lokal zugestellten Nachricht eine `LogicEngine` aus:

```rust
pub trait LogicEngine: Send + Sync {
    fn name(&self) -> &str;
    fn execute<'a>(&'a self, ctx: &'a ActionContext) -> BoxFuture<'a, ActionOutput>;
}
```

**Eingebaute Engines:**

| Engine | Verhalten |
|--------|-----------|
| `EchoEngine` | Spiegelt das Paket an den Absender zurück |
| `TransformEngine` | Wendet einen benutzerdefinierten Closure auf den Payload an |
| `ScriptEngine` | Platzhalter für Python (pyo3) — derzeit keine Funktion |

Die Engine empfängt `ActionContext { self_id, incoming: Packet, objects: HashMap<Uuid, ObjectSnapshot> }` und gibt `ActionOutput { updates, packets, logs }` zurück.

---

## 6. Paketschema

```
Packet {
    target_id : Uuid          // Zielknoten
    sender_id : Uuid          // Ursprungsknoten
    priority  : u8            // 0 (niedrig) … 255 (hoch)
    ttl       : Duration      // serialisiert als Millisekunden (u64)
    payload   : JSON Value    // beliebig strukturierte Daten
}
```

### Object-Operationsnutzlasten

```json
// Lesen
{ "op": "get",      "property": "temperature" }
// Schreiben
{ "op": "set",      "property": "temperature", "value": 42.5 }
// Stapelschreiben
{ "op": "set_many", "properties": { "temperature": 42.5, "active": true } }
// Alle lesen
{ "op": "get_all" }
```

---

## 7. Zustandsverwaltung

### Zweischichtiges Modell

| Schicht | Trait | Standardimplementierung | Lebensdauer |
|---------|-------|-------------------------|-------------|
| Flüchtig | `VolatileStore` | `InMemoryStore` | Prozesslebensdauer |
| Persistent | `PersistentStore` | `NoopPersistentStore` | Über Neustarts hinweg |

### ObjectID.PropertyID-Adressierung

```
StateKey { object_id: Uuid, property_id: String }
Darstellung: "<uuid>.<eigenschaftsname>"
```

### PersistentStore (objektsicher)

```rust
pub trait PersistentStore: Send + Sync {
    fn load<'a>(&'a self, key: &'a StateKey) -> PinBoxFuture<'a, Option<Value>>;
    fn save<'a>(&'a self, key: StateKey, value: Value)   -> PinBoxFuture<'a, ()>;
    fn delete<'a>(&'a self, key: &'a StateKey)           -> PinBoxFuture<'a, ()>;
}
```

---

## 8. Das .hflow-Datenbankschema

Eine `.hflow`-Datei ist eine SQLite-Datenbank. Alle `spawn_blocking`-Aufrufe kapseln jede Datenbankoperation, um eine Blockierung des Tokio-Executors zu vermeiden.

### Tabelle: `knots`

```sql
CREATE TABLE knots (
    id          TEXT    PRIMARY KEY,   -- UUID
    local_id    INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    description TEXT,
    role        TEXT    NOT NULL,      -- "Hub" | "Action" | "Object" | "Class"
    level       INTEGER NOT NULL,
    parent_id   TEXT,                  -- NULL für die Wurzel
    class_id    TEXT                   -- NULL, außer bei Object
);
```

### Tabelle: `properties`

```sql
CREATE TABLE properties (
    object_id   TEXT NOT NULL,
    property_id TEXT NOT NULL,
    value       TEXT NOT NULL,         -- JSON-kodiert
    PRIMARY KEY (object_id, property_id)
);
```

### Tabelle: `class_schemas`

```sql
CREATE TABLE class_schemas (
    class_id       TEXT    NOT NULL,
    property_name  TEXT    NOT NULL,
    data_type      TEXT    NOT NULL,   -- "string"|"number"|"boolean"|"json"
    default_value  TEXT,               -- JSON oder NULL
    unit           TEXT,
    description    TEXT,
    sort_order     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (class_id, property_name)
);
```

### Tabelle: `links`

```sql
CREATE TABLE links (
    id          TEXT PRIMARY KEY,      -- UUID
    source_id   TEXT NOT NULL,         -- Knoten-UUID
    target_id   TEXT NOT NULL,         -- Knoten-UUID
    link_type   TEXT NOT NULL          -- z. B. "action_to_object", "data_flow"
);
```

---

## 9. Frontend-API (WebSocket + REST)

Der `ApiServer` (betrieben von axum 0.7) stellt folgende Schnittstellen bereit:

### REST-Endpunkte

| Methode | Pfad | Antwort |
|---------|------|---------|
| `GET` | `/api/graph` | `JSON-Array` aller `RegistryEntry`-Einträge |
| `GET` | `/api/knot/:id` | Einzelner `RegistryEntry` oder 404 |
| `GET` | `/api/knot/:id/state` | Eigenschafts-Map des Object-Knotens oder 404 |
| `POST` | `/api/packet` | `{"status":"queued"}` |

### POST /api/packet — Anfragekörper

```json
{
  "target_id": "<uuid>",
  "sender_id": "<uuid>",    // optional, Standard: nil-UUID
  "priority":  200,          // optional, Standard: 128
  "ttl_ms":    5000,         // optional, Standard: 5000
  "payload":   { ... }
}
```

### WebSocket — GET /api/ws

**Server → Client:** `EventEnvelope { event_type: string, payload: object }`

Beispiel:
```json
{ "event_type": "ObjectStateChanged", "payload": { "knot_id": "...", "property": "temperature", "value": 42.5 } }
```

**Client → Server:** JSON-RPC 2.0

```json
{ "jsonrpc": "2.0", "method": "send_packet", "params": { "target_id": "...", "payload": {} }, "id": 1 }
{ "jsonrpc": "2.0", "method": "get_graph",   "id": 2 }
```

---

## 10. Glossar

| Begriff | Definition |
|---------|-----------|
| Knoten (Knot) | Die grundlegende adressierbare Einheit in HubFlow |
| Hub | Ein Knoten, der untergeordnete Knoten koordiniert (der „Planet") |
| Satellit | Ein Action-, Object- oder Class-Knoten, der einem Hub zugeordnet ist |
| Posteingang (Inbox) | Die asynchrone Prioritätswarteschlange eines Knotens |
| Paket (Packet) | Die Kommunikationseinheit zwischen Knoten |
| TTL | Time-To-Live: maximale Paketlebensdauer vor dem Dead-Lettering |
| Dead Letter | Ein Paket, das nicht zugestellt werden konnte |
| LogicEngine | Ein austauschbarer asynchroner Berechnungs-Hook für Action-Knoten |
| ClassDefinition | Ein Schema, das die Eigenschaften eines Object-Knotens beschreibt |
| .hflow | Eine SQLite-Arbeitsdatei, die die vollständige Topologie enthält |
| KnotRegistry | Das speicherinterne Verzeichnis aller aktiven Knoten |