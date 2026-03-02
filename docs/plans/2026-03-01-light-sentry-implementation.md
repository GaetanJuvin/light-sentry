# light-sentry Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a lightweight, self-hosted Sentry replacement — single Rust binary with error tracking, basic performance monitoring, and log ingestion, compatible with existing Sentry SDKs.

**Architecture:** Single Axum server serving both the Sentry-compatible ingestion API and an HTMX dashboard. PostgreSQL for storage. Background retention cleanup via Tokio tasks. All templates compiled into the binary.

**Tech Stack:** Rust 2024 edition, Axum 0.8, SQLx 0.8 (Postgres), Askama 0.12, HTMX, Pico CSS, Argon2, tower-sessions

---

### Task 1: Project Scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/state.rs`
- Create: `src/error.rs`
- Create: `src/routes/mod.rs`
- Create: `.env.example`
- Create: `.gitignore`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "light-sentry"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.8", features = ["macros"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["fs", "trace"] }
askama = { version = "0.12", features = ["with-axum"] }
sqlx = { version = "0.8", features = [
    "runtime-tokio",
    "tls-rustls",
    "postgres",
    "uuid",
    "chrono",
    "migrate",
] }
tower-sessions = { version = "0.15" }
argon2 = "0.5"
password-hash = { version = "0.5", features = ["rand_core"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
dotenvy = "0.15"
time = { version = "0.3", features = ["serde"] }
sha2 = "0.10"
hex = "0.4"
rand = "0.8"
```

**Step 2: Create src/state.rs**

```rust
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}
```

**Step 3: Create src/error.rs**

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, self.to_string()).into_response()
    }
}
```

**Step 4: Create src/main.rs**

Minimal main that builds the pool, runs migrations, and starts an Axum server with a health check route at `GET /health`.

```rust
use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod error;
mod routes;
mod state;

use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,sqlx=warn".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = AppState { db: pool };

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
```

**Step 5: Create .env.example, .gitignore, empty migrations dir, src/routes/mod.rs**

```
# .env.example
DATABASE_URL=postgres://light_sentry:light_sentry@localhost:5432/light_sentry
LISTEN_ADDR=0.0.0.0:3000
RUST_LOG=info
```

**Step 6: Verify it compiles**

Run: `cargo check`
Expected: Compiles with no errors (migrations dir can be empty — create a `.keep` file)

**Step 7: Commit**

```bash
git add -A
git commit -m "feat: project scaffolding with Axum, SQLx, health check"
```

---

### Task 2: Database Migrations

**Files:**
- Create: `migrations/001_create_users.sql`
- Create: `migrations/002_create_projects.sql`
- Create: `migrations/003_create_error_events.sql`
- Create: `migrations/004_create_transactions.sql`
- Create: `migrations/005_create_logs.sql`

**Step 1: Write the users migration**

```sql
-- migrations/001_create_users.sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

**Step 2: Write the projects migration**

```sql
-- migrations/002_create_projects.sql
CREATE TABLE projects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    dsn_public TEXT NOT NULL UNIQUE,
    dsn_secret TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

**Step 3: Write the error_events migration**

```sql
-- migrations/003_create_error_events.sql
CREATE TABLE error_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    level TEXT NOT NULL DEFAULT 'error',
    title TEXT NOT NULL,
    message TEXT NOT NULL DEFAULT '',
    stack_trace JSONB,
    context JSONB NOT NULL DEFAULT '{}',
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_error_events_project_received ON error_events(project_id, received_at DESC);
CREATE INDEX idx_error_events_project_fingerprint ON error_events(project_id, fingerprint);
```

**Step 4: Write the transactions migration**

```sql
-- migrations/004_create_transactions.sql
CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    trace_id TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    duration_ms DOUBLE PRECISION NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'ok',
    spans JSONB,
    context JSONB NOT NULL DEFAULT '{}',
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_transactions_project_received ON transactions(project_id, received_at DESC);
```

**Step 5: Write the logs migration**

```sql
-- migrations/005_create_logs.sql
CREATE TABLE logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    level TEXT NOT NULL DEFAULT 'info',
    message TEXT NOT NULL,
    context JSONB NOT NULL DEFAULT '{}',
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_logs_project_received ON logs(project_id, received_at DESC);
```

**Step 6: Verify migrations run**

Run: `cargo check` (compile-time migration embedding check)
Then manually: create a local Postgres DB and run the binary to verify migrations apply.

**Step 7: Commit**

```bash
git add migrations/
git commit -m "feat: database migrations for users, projects, events, transactions, logs"
```

---

### Task 3: Sentry Auth Parsing

**Files:**
- Create: `src/ingest/mod.rs`
- Create: `src/ingest/auth.rs`
- Modify: `src/main.rs` (add `mod ingest`)

**Step 1: Write tests for auth parsing**

The `X-Sentry-Auth` header format is:
```
Sentry sentry_version=7, sentry_client=sentry.python/1.0, sentry_key=<public_key>
```

Or query string: `?sentry_key=<public_key>&sentry_version=7`

Write tests in `src/ingest/auth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_x_sentry_auth_header() {
        let header = "Sentry sentry_version=7, sentry_client=sentry.python/1.0, sentry_key=abc123";
        let auth = SentryAuth::from_header(header).unwrap();
        assert_eq!(auth.public_key, "abc123");
        assert_eq!(auth.version, "7");
        assert_eq!(auth.client, Some("sentry.python/1.0".to_string()));
    }

    #[test]
    fn parse_query_string() {
        let query = "sentry_key=abc123&sentry_version=7";
        let auth = SentryAuth::from_query(query).unwrap();
        assert_eq!(auth.public_key, "abc123");
    }

    #[test]
    fn missing_key_fails() {
        let header = "Sentry sentry_version=7, sentry_client=sentry.python/1.0";
        assert!(SentryAuth::from_header(header).is_none());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL — `SentryAuth` not defined

**Step 3: Implement SentryAuth**

```rust
// src/ingest/auth.rs

pub struct SentryAuth {
    pub public_key: String,
    pub version: String,
    pub client: Option<String>,
}

impl SentryAuth {
    /// Parse from X-Sentry-Auth header value.
    /// Format: "Sentry sentry_version=7, sentry_client=..., sentry_key=..."
    pub fn from_header(header: &str) -> Option<Self> {
        let header = header.strip_prefix("Sentry ")?.trim();
        let mut key = None;
        let mut version = None;
        let mut client = None;

        for part in header.split(',') {
            let part = part.trim();
            if let Some((k, v)) = part.split_once('=') {
                match k.trim() {
                    "sentry_key" => key = Some(v.trim().to_string()),
                    "sentry_version" => version = Some(v.trim().to_string()),
                    "sentry_client" => client = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }

        Some(SentryAuth {
            public_key: key?,
            version: version.unwrap_or_else(|| "7".to_string()),
            client,
        })
    }

    /// Parse from query string: sentry_key=...&sentry_version=...
    pub fn from_query(query: &str) -> Option<Self> {
        let mut key = None;
        let mut version = None;
        let mut client = None;

        for part in query.split('&') {
            if let Some((k, v)) = part.split_once('=') {
                match k {
                    "sentry_key" => key = Some(v.to_string()),
                    "sentry_version" => version = Some(v.to_string()),
                    "sentry_client" => client = Some(v.to_string()),
                    _ => {}
                }
            }
        }

        Some(SentryAuth {
            public_key: key?,
            version: version.unwrap_or_else(|| "7".to_string()),
            client,
        })
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test`
Expected: PASS

**Step 5: Add project lookup helper**

In `src/ingest/auth.rs`, add a function to validate the public key against the database:

```rust
use sqlx::PgPool;
use uuid::Uuid;

pub async fn authenticate_project(pool: &PgPool, public_key: &str) -> Option<Uuid> {
    let row = sqlx::query_scalar!(
        "SELECT id FROM projects WHERE dsn_public = $1",
        public_key
    )
    .fetch_optional(pool)
    .await
    .ok()?;
    row
}
```

**Step 6: Commit**

```bash
git add src/ingest/
git commit -m "feat: Sentry auth header and query string parsing"
```

---

### Task 4: Store Endpoint (Legacy Single Event)

**Files:**
- Create: `src/ingest/store.rs`
- Create: `src/ingest/fingerprint.rs`
- Modify: `src/ingest/mod.rs`
- Modify: `src/main.rs` (add route)

**Step 1: Write fingerprint tests**

Fingerprint = SHA256 hash of `{exception_type}:{exception_value}:{top_in_app_filename}:{top_in_app_function}`.

```rust
// src/ingest/fingerprint.rs
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fingerprint_from_exception() {
        let event = json!({
            "exception": {
                "values": [{
                    "type": "ValueError",
                    "value": "invalid literal",
                    "stacktrace": {
                        "frames": [
                            {"filename": "lib.py", "function": "inner", "in_app": false},
                            {"filename": "app.py", "function": "handler", "in_app": true}
                        ]
                    }
                }]
            }
        });
        let fp = compute_fingerprint(&event);
        assert!(!fp.is_empty());

        // Same input → same fingerprint
        let fp2 = compute_fingerprint(&event);
        assert_eq!(fp, fp2);
    }

    #[test]
    fn fingerprint_without_exception_uses_message() {
        let event = json!({"message": "something broke", "level": "error"});
        let fp = compute_fingerprint(&event);
        assert!(!fp.is_empty());
    }
}
```

**Step 2: Run tests — should fail**

**Step 3: Implement fingerprinting**

```rust
// src/ingest/fingerprint.rs
use serde_json::Value;
use sha2::{Sha256, Digest};

pub fn compute_fingerprint(event: &Value) -> String {
    let mut hasher = Sha256::new();

    // Try exception-based fingerprint first
    if let Some(values) = event.pointer("/exception/values").and_then(|v| v.as_array()) {
        if let Some(exc) = values.last() {
            let exc_type = exc.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let exc_value = exc.get("value").and_then(|v| v.as_str()).unwrap_or("");
            hasher.update(exc_type.as_bytes());
            hasher.update(b":");
            hasher.update(exc_value.as_bytes());

            // Find top in_app frame
            if let Some(frames) = exc.pointer("/stacktrace/frames").and_then(|v| v.as_array()) {
                for frame in frames.iter().rev() {
                    let in_app = frame.get("in_app").and_then(|v| v.as_bool()).unwrap_or(false);
                    if in_app {
                        let filename = frame.get("filename").and_then(|v| v.as_str()).unwrap_or("");
                        let function = frame.get("function").and_then(|v| v.as_str()).unwrap_or("");
                        hasher.update(b":");
                        hasher.update(filename.as_bytes());
                        hasher.update(b":");
                        hasher.update(function.as_bytes());
                        break;
                    }
                }
            }

            return hex::encode(hasher.finalize());
        }
    }

    // Fallback: hash the message
    let message = event.get("message").and_then(|v| v.as_str())
        .or_else(|| event.get("logentry").and_then(|v| v.get("message")).and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    hasher.update(message.as_bytes());
    hex::encode(hasher.finalize())
}
```

**Step 4: Run tests — should pass**

**Step 5: Implement the store endpoint**

```rust
// src/ingest/store.rs
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::state::AppState;
use super::auth::{SentryAuth, authenticate_project};
use super::fingerprint::compute_fingerprint;

pub async fn store_event(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    // Authenticate
    let auth = extract_auth(&headers, &query).ok_or(StatusCode::UNAUTHORIZED)?;
    let db_project_id = authenticate_project(&state.db, &auth.public_key)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Parse event
    let event: Value = serde_json::from_str(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    let event_id = event.get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fingerprint = compute_fingerprint(&event);
    let level = event.get("level").and_then(|v| v.as_str()).unwrap_or("error");
    let title = extract_title(&event);
    let message = event.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let stack_trace = event.pointer("/exception/values").cloned();
    let context = build_context(&event);

    sqlx::query!(
        r#"INSERT INTO error_events (project_id, event_id, fingerprint, level, title, message, stack_trace, context)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        db_project_id,
        event_id,
        fingerprint,
        level,
        title,
        message,
        stack_trace,
        context,
    )
    .execute(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({"id": event_id})))
}

fn extract_auth(headers: &HeaderMap, query: &std::collections::HashMap<String, String>) -> Option<SentryAuth> {
    // Try header first
    if let Some(h) = headers.get("X-Sentry-Auth").and_then(|v| v.to_str().ok()) {
        return SentryAuth::from_header(h);
    }
    // Try Authorization header
    if let Some(h) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
        return SentryAuth::from_header(h);
    }
    // Try query string
    if let Some(key) = query.get("sentry_key") {
        return Some(SentryAuth {
            public_key: key.clone(),
            version: query.get("sentry_version").cloned().unwrap_or_else(|| "7".to_string()),
            client: query.get("sentry_client").cloned(),
        });
    }
    None
}

fn extract_title(event: &Value) -> String {
    // Try exception type: value
    if let Some(values) = event.pointer("/exception/values").and_then(|v| v.as_array()) {
        if let Some(exc) = values.last() {
            let t = exc.get("type").and_then(|v| v.as_str()).unwrap_or("Error");
            let v = exc.get("value").and_then(|v| v.as_str()).unwrap_or("");
            return format!("{}: {}", t, v);
        }
    }
    // Fallback to message
    event.get("message").and_then(|v| v.as_str())
        .unwrap_or("Unknown error")
        .chars().take(200).collect()
}

fn build_context(event: &Value) -> Value {
    let mut ctx = serde_json::Map::new();
    for key in &["tags", "user", "request", "contexts", "breadcrumbs", "extra", "sdk",
                  "server_name", "environment", "release", "platform"] {
        if let Some(v) = event.get(*key) {
            ctx.insert(key.to_string(), v.clone());
        }
    }
    Value::Object(ctx)
}
```

**Step 6: Wire the route in main.rs**

Add to the router:
```rust
.route("/api/{project_id}/store/", axum::routing::post(ingest::store::store_event))
```

**Step 7: Verify it compiles**

Run: `cargo check`

**Step 8: Commit**

```bash
git add src/ingest/
git commit -m "feat: /store/ endpoint with fingerprinting and event ingestion"
```

---

### Task 5: Envelope Endpoint

**Files:**
- Create: `src/ingest/envelope.rs`
- Modify: `src/ingest/mod.rs`
- Modify: `src/main.rs` (add route)

**Step 1: Write envelope parsing tests**

Sentry envelope format is newline-separated:
- Line 1: envelope header JSON (contains `event_id`, `dsn`, `sent_at`, `sdk`)
- For each item: item header JSON (contains `type`, `length`) + newline + payload

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_envelope() {
        let raw = r#"{"event_id":"abc123","dsn":"https://pubkey@host/1"}
{"type":"event","length":27}
{"exception":{"values":[]}}"#;
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items.len(), 1);
        assert_eq!(envelope.items[0].item_type, "event");
    }

    #[test]
    fn parse_transaction_envelope() {
        let raw = r#"{"event_id":"abc123"}
{"type":"transaction","length":46}
{"type":"transaction","transaction":"GET /api"}"#;
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items[0].item_type, "transaction");
    }

    #[test]
    fn parse_log_envelope() {
        let raw = r#"{}
{"type":"log","item_count":1,"content_type":"application/vnd.sentry.items.log+json"}
{"items":[{"timestamp":1234,"level":"info","body":"hello","trace_id":"abc"}]}"#;
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items[0].item_type, "log");
    }

    #[test]
    fn skip_unknown_item_types() {
        let raw = r#"{}
{"type":"session","length":2}
{}
{"type":"event","length":27}
{"exception":{"values":[]}}"#;
        let envelope = parse_envelope(raw).unwrap();
        // session is skipped, only event is kept
        assert_eq!(envelope.items.len(), 1);
    }
}
```

**Step 2: Run tests — should fail**

**Step 3: Implement envelope parser**

```rust
// src/ingest/envelope.rs
use serde_json::Value;

pub struct Envelope {
    pub header: Value,
    pub items: Vec<EnvelopeItem>,
}

pub struct EnvelopeItem {
    pub item_type: String,
    pub payload: Value,
}

const SUPPORTED_TYPES: &[&str] = &["event", "transaction", "log"];

pub fn parse_envelope(raw: &str) -> Option<Envelope> {
    let mut lines = raw.split('\n');

    // First line: envelope header
    let header_line = lines.next()?;
    let header: Value = serde_json::from_str(header_line).ok()?;

    let mut items = Vec::new();

    while let Some(item_header_line) = lines.next() {
        if item_header_line.is_empty() {
            continue;
        }

        let item_header: Value = serde_json::from_str(item_header_line).ok()?;
        let item_type = item_header.get("type")?.as_str()?.to_string();

        // Read the payload
        let length = item_header.get("length").and_then(|v| v.as_u64());

        let payload_str = if let Some(len) = length {
            // Read exactly `len` bytes from remaining lines
            let remaining: String = lines.clone().collect::<Vec<&str>>().join("\n");
            let payload = &remaining[..std::cmp::min(len as usize, remaining.len())];
            let payload_owned = payload.to_string();
            // Advance the iterator past the payload
            let mut consumed = 0;
            while consumed < len as usize {
                if let Some(line) = lines.next() {
                    consumed += line.len() + 1; // +1 for newline
                } else {
                    break;
                }
            }
            payload_owned
        } else {
            // No length — payload is the next line
            lines.next().unwrap_or("{}").to_string()
        };

        if SUPPORTED_TYPES.contains(&item_type.as_str()) {
            if let Ok(payload) = serde_json::from_str(&payload_str) {
                items.push(EnvelopeItem { item_type, payload });
            }
        }
    }

    Some(Envelope { header, items })
}
```

**Step 4: Run tests — should pass**

**Step 5: Implement the envelope HTTP handler**

```rust
// In src/ingest/envelope.rs, add the handler

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use uuid::Uuid;
use serde_json::json;

use crate::state::AppState;
use super::auth::{SentryAuth, authenticate_project};
use super::fingerprint::compute_fingerprint;
use super::store::extract_auth;

pub async fn envelope_handler(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    // Auth from header/query or from envelope header's dsn field
    let auth = extract_auth(&headers, &query);

    let envelope = parse_envelope(&body).ok_or(StatusCode::BAD_REQUEST)?;

    // If no auth from headers, try DSN in envelope header
    let public_key = if let Some(a) = &auth {
        a.public_key.clone()
    } else if let Some(dsn) = envelope.header.get("dsn").and_then(|v| v.as_str()) {
        // Parse public key from DSN: https://{public_key}@{host}/{project_id}
        dsn.split("://").nth(1)
            .and_then(|s| s.split('@').next())
            .map(|s| s.to_string())
            .ok_or(StatusCode::UNAUTHORIZED)?
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let db_project_id = authenticate_project(&state.db, &public_key)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let event_id = envelope.header.get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    for item in envelope.items {
        match item.item_type.as_str() {
            "event" => {
                ingest_error_event(&state.db, db_project_id, &item.payload).await.ok();
            }
            "transaction" => {
                ingest_transaction(&state.db, db_project_id, &item.payload).await.ok();
            }
            "log" => {
                ingest_logs(&state.db, db_project_id, &item.payload).await.ok();
            }
            _ => {} // skip unsupported types
        }
    }

    Ok(Json(json!({"id": event_id})))
}

async fn ingest_error_event(pool: &sqlx::PgPool, project_id: Uuid, event: &Value) -> Result<(), sqlx::Error> {
    let event_id = event.get("event_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let fingerprint = compute_fingerprint(event);
    let level = event.get("level").and_then(|v| v.as_str()).unwrap_or("error");
    let title = super::store::extract_title(event);
    let message = event.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let stack_trace = event.pointer("/exception/values").cloned();
    let context = super::store::build_context(event);

    sqlx::query!(
        r#"INSERT INTO error_events (project_id, event_id, fingerprint, level, title, message, stack_trace, context)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        project_id, event_id, fingerprint, level, title, message, stack_trace, context,
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn ingest_transaction(pool: &sqlx::PgPool, project_id: Uuid, event: &Value) -> Result<(), sqlx::Error> {
    let event_id = event.get("event_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let trace_id = event.pointer("/contexts/trace/trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("").to_string();
    let name = event.get("transaction").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();

    // Duration = timestamp - start_timestamp (both in seconds)
    let duration_ms = match (
        event.get("timestamp").and_then(parse_timestamp),
        event.get("start_timestamp").and_then(parse_timestamp),
    ) {
        (Some(end), Some(start)) => (end - start) * 1000.0,
        _ => 0.0,
    };

    let status = event.pointer("/contexts/trace/status")
        .and_then(|v| v.as_str())
        .unwrap_or("ok");
    let spans = event.get("spans").cloned();
    let context = super::store::build_context(event);

    sqlx::query!(
        r#"INSERT INTO transactions (project_id, event_id, trace_id, name, duration_ms, status, spans, context)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        project_id, event_id, trace_id, name, duration_ms, status, spans, context,
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_timestamp(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            // Try parsing as RFC3339
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 / 1_000_000_000.0)
                .ok()
        }
        _ => None,
    }
}

async fn ingest_logs(pool: &sqlx::PgPool, project_id: Uuid, payload: &Value) -> Result<(), sqlx::Error> {
    let items = payload.get("items").and_then(|v| v.as_array());
    let Some(items) = items else { return Ok(()) };

    for log_entry in items {
        let level = log_entry.get("level").and_then(|v| v.as_str()).unwrap_or("info");
        let message = log_entry.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let context = log_entry.get("attributes").cloned().unwrap_or(json!({}));

        sqlx::query!(
            r#"INSERT INTO logs (project_id, level, message, context)
               VALUES ($1, $2, $3, $4)"#,
            project_id, level, message, context,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}
```

**Step 6: Wire the route in main.rs**

```rust
.route("/api/{project_id}/envelope/", axum::routing::post(ingest::envelope::envelope_handler))
```

**Step 7: Run tests and compile check**

Run: `cargo test && cargo check`

**Step 8: Commit**

```bash
git add src/ingest/
git commit -m "feat: /envelope/ endpoint with event, transaction, and log ingestion"
```

---

### Task 6: User Authentication (Register/Login)

**Files:**
- Create: `src/auth.rs`
- Create: `src/routes/auth.rs`
- Create: `templates/login.html`
- Create: `templates/register.html`
- Create: `templates/base.html`
- Modify: `src/main.rs` (add session layer, auth routes)
- Modify: `src/routes/mod.rs`

**Step 1: Create base template**

```html
<!-- templates/base.html -->
<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{% block title %}light-sentry{% endblock %}</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/@picocss/pico@2/css/pico.min.css" />
  <script src="https://unpkg.com/htmx.org@2.0.4" defer></script>
  <style>
    :root { --pico-font-size: 14px; }
    nav { padding: 0.5rem 1rem; }
    .container { max-width: 1200px; margin: 0 auto; padding: 1rem; }
    table { font-size: 0.9rem; }
    .badge { display: inline-block; padding: 0.15rem 0.5rem; border-radius: 4px; font-size: 0.75rem; font-weight: 600; }
    .badge-error { background: #fee; color: #c00; }
    .badge-warning { background: #fff3cd; color: #856404; }
    .badge-fatal { background: #c00; color: #fff; }
    .badge-info { background: #d1ecf1; color: #0c5460; }
    .badge-debug { background: #e2e3e5; color: #383d41; }
  </style>
  {% block head %}{% endblock %}
</head>
<body>
  {% block nav %}
  <nav class="container">
    <ul>
      <li><strong><a href="/projects">light-sentry</a></strong></li>
    </ul>
    <ul>
      <li><a href="/projects">Projects</a></li>
      <li><form method="post" action="/logout" style="margin:0"><button type="submit" class="outline secondary" style="margin:0;padding:0.25rem 0.75rem">Logout</button></form></li>
    </ul>
  </nav>
  {% endblock %}
  <main class="container">
    {% block content %}{% endblock %}
  </main>
</body>
</html>
```

**Step 2: Create login and register templates**

```html
<!-- templates/login.html -->
{% extends "base.html" %}
{% block nav %}{% endblock %}
{% block title %}Login — light-sentry{% endblock %}
{% block content %}
<article style="max-width:400px;margin:4rem auto">
  <h2>Login</h2>
  {% if error %}
  <p style="color:var(--pico-del-color)">{{ error }}</p>
  {% endif %}
  <form method="post" action="/login">
    <label>Email <input type="email" name="email" required /></label>
    <label>Password <input type="password" name="password" required /></label>
    <button type="submit">Login</button>
  </form>
  <small>No account? <a href="/register">Register</a></small>
</article>
{% endblock %}
```

```html
<!-- templates/register.html -->
{% extends "base.html" %}
{% block nav %}{% endblock %}
{% block title %}Register — light-sentry{% endblock %}
{% block content %}
<article style="max-width:400px;margin:4rem auto">
  <h2>Register</h2>
  {% if error %}
  <p style="color:var(--pico-del-color)">{{ error }}</p>
  {% endif %}
  <form method="post" action="/register">
    <label>Email <input type="email" name="email" required /></label>
    <label>Password <input type="password" name="password" required minlength="8" /></label>
    <button type="submit">Register</button>
  </form>
  <small>Already have an account? <a href="/login">Login</a></small>
</article>
{% endblock %}
```

**Step 3: Implement auth module**

```rust
// src/auth.rs
use argon2::{Argon2, password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng}};

pub fn hash_password(plaintext: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(plaintext.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash error: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(plaintext: &str, stored_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else { return false };
    Argon2::default().verify_password(plaintext.as_bytes(), &parsed).is_ok()
}
```

**Step 4: Implement auth routes**

```rust
// src/routes/auth.rs
use askama::Template;
use axum::{extract::State, response::{IntoResponse, Redirect}, Form};
use serde::Deserialize;
use tower_sessions::Session;
use crate::{auth, state::AppState};

const USER_ID_KEY: &str = "user_id";

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate { error: Option<String> }

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate { error: Option<String> }

#[derive(Deserialize)]
pub struct AuthForm { email: String, password: String }

pub async fn login_page() -> impl IntoResponse {
    LoginTemplate { error: None }
}

pub async fn login_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AuthForm>,
) -> impl IntoResponse {
    let row = sqlx::query!("SELECT id, password_hash FROM users WHERE email = $1", form.email)
        .fetch_optional(&state.db).await.ok().flatten();

    let Some(user) = row else {
        return LoginTemplate { error: Some("Invalid credentials".into()) }.into_response();
    };

    let hash = user.password_hash.clone();
    let pw = form.password.clone();
    let valid = tokio::task::spawn_blocking(move || auth::verify_password(&pw, &hash))
        .await.unwrap_or(false);

    if !valid {
        return LoginTemplate { error: Some("Invalid credentials".into()) }.into_response();
    }

    session.insert(USER_ID_KEY, user.id.to_string()).await.ok();
    Redirect::to("/projects").into_response()
}

pub async fn register_page() -> impl IntoResponse {
    RegisterTemplate { error: None }
}

pub async fn register_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AuthForm>,
) -> impl IntoResponse {
    if form.password.len() < 8 {
        return RegisterTemplate { error: Some("Password must be at least 8 characters".into()) }.into_response();
    }

    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || auth::hash_password(&pw)).await {
        Ok(Ok(h)) => h,
        _ => return RegisterTemplate { error: Some("Registration failed".into()) }.into_response(),
    };

    let result = sqlx::query!("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id",
        form.email, hash)
        .fetch_one(&state.db).await;

    match result {
        Ok(user) => {
            session.insert(USER_ID_KEY, user.id.to_string()).await.ok();
            Redirect::to("/projects").into_response()
        }
        Err(_) => RegisterTemplate { error: Some("Email already registered".into()) }.into_response(),
    }
}

pub async fn logout(session: Session) -> impl IntoResponse {
    session.flush().await.ok();
    Redirect::to("/login")
}

pub async fn require_user(session: &Session) -> Option<uuid::Uuid> {
    let id_str: String = session.get(USER_ID_KEY).await.ok()??;
    id_str.parse().ok()
}
```

**Step 5: Update main.rs with session layer and auth routes**

Add `tower_sessions::MemoryStore` and `SessionManagerLayer`. Add routes:
```rust
.route("/login", get(routes::auth::login_page).post(routes::auth::login_submit))
.route("/register", get(routes::auth::register_page).post(routes::auth::register_submit))
.route("/logout", post(routes::auth::logout))
```

**Step 6: Verify it compiles**

Run: `cargo check`

**Step 7: Commit**

```bash
git add src/ templates/
git commit -m "feat: user authentication with register, login, logout"
```

---

### Task 7: Projects Dashboard

**Files:**
- Create: `src/routes/projects.rs`
- Create: `templates/projects.html`
- Create: `templates/project_new.html`
- Modify: `src/routes/mod.rs`
- Modify: `src/main.rs` (add routes)

**Step 1: Create projects templates**

```html
<!-- templates/projects.html -->
{% extends "base.html" %}
{% block title %}Projects — light-sentry{% endblock %}
{% block content %}
<header>
  <h2>Projects</h2>
  <a href="/projects/new" role="button">New Project</a>
</header>
<table>
  <thead><tr><th>Name</th><th>DSN</th><th>Created</th></tr></thead>
  <tbody>
    {% for project in projects %}
    <tr>
      <td><a href="/{{ project.id }}/issues">{{ project.name }}</a></td>
      <td><code style="font-size:0.75rem">{{ host }}://{{ project.dsn_public }}@{{ host }}/{{ project.id }}</code></td>
      <td>{{ project.created_at.format("%Y-%m-%d") }}</td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% if projects.is_empty() %}
<p>No projects yet. <a href="/projects/new">Create one</a>.</p>
{% endif %}
{% endblock %}
```

```html
<!-- templates/project_new.html -->
{% extends "base.html" %}
{% block title %}New Project — light-sentry{% endblock %}
{% block content %}
<article style="max-width:500px">
  <h2>New Project</h2>
  <form method="post" action="/projects">
    <label>Project Name <input type="text" name="name" required /></label>
    <button type="submit">Create</button>
  </form>
</article>
{% endblock %}
```

**Step 2: Implement projects routes**

```rust
// src/routes/projects.rs
use askama::Template;
use axum::{extract::State, response::{IntoResponse, Redirect}, Form};
use serde::Deserialize;
use tower_sessions::Session;
use uuid::Uuid;

use crate::state::AppState;
use super::auth::require_user;

struct ProjectRow {
    id: Uuid,
    name: String,
    dsn_public: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Template)]
#[template(path = "projects.html")]
struct ProjectsTemplate {
    projects: Vec<ProjectRow>,
    host: String,
}

#[derive(Template)]
#[template(path = "project_new.html")]
struct NewProjectTemplate {}

#[derive(Deserialize)]
pub struct NewProjectForm { name: String }

pub async fn list(State(state): State<AppState>, session: Session) -> impl IntoResponse {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let projects = sqlx::query_as!(ProjectRow,
        "SELECT id, name, dsn_public, created_at FROM projects ORDER BY created_at DESC")
        .fetch_all(&state.db).await.unwrap_or_default();

    let host = std::env::var("PUBLIC_HOST").unwrap_or_else(|_| "http://localhost:3000".into());

    ProjectsTemplate { projects, host }.into_response()
}

pub async fn new_form(session: Session) -> impl IntoResponse {
    let Some(_) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };
    NewProjectTemplate {}.into_response()
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<NewProjectForm>,
) -> impl IntoResponse {
    let Some(_) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let dsn_public = hex::encode(&rand::random::<[u8; 16]>());
    let dsn_secret = hex::encode(&rand::random::<[u8; 16]>());

    sqlx::query!("INSERT INTO projects (name, dsn_public, dsn_secret) VALUES ($1, $2, $3)",
        form.name, dsn_public, dsn_secret)
        .execute(&state.db).await.ok();

    Redirect::to("/projects").into_response()
}
```

**Step 3: Wire routes**

```rust
.route("/projects", get(routes::projects::list).post(routes::projects::create))
.route("/projects/new", get(routes::projects::new_form))
```

Add redirect from `/` to `/projects`.

**Step 4: Commit**

```bash
git add src/routes/ templates/
git commit -m "feat: projects dashboard with create and list"
```

---

### Task 8: Issues Dashboard (Error Events)

**Files:**
- Create: `src/routes/issues.rs`
- Create: `templates/issues.html`
- Create: `templates/issue_detail.html`
- Modify: `src/routes/mod.rs`
- Modify: `src/main.rs`

**Step 1: Create issues list template**

```html
<!-- templates/issues.html -->
{% extends "base.html" %}
{% block title %}Issues — {{ project_name }}{% endblock %}
{% block content %}
{% include "partials/project_nav.html" %}
<h2>Issues</h2>
<table>
  <thead><tr><th>Title</th><th>Level</th><th>Events</th><th>Last Seen</th></tr></thead>
  <tbody>
    {% for issue in issues %}
    <tr>
      <td><a href="/{{ project_id }}/issues/{{ issue.fingerprint }}">{{ issue.title }}</a></td>
      <td><span class="badge badge-{{ issue.level }}">{{ issue.level }}</span></td>
      <td>{{ issue.count }}</td>
      <td>{{ issue.last_seen.format("%Y-%m-%d %H:%M") }}</td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% if issues.is_empty() %}
<p>No errors captured yet.</p>
{% endif %}
{% endblock %}
```

**Step 2: Create issue detail template**

```html
<!-- templates/issue_detail.html -->
{% extends "base.html" %}
{% block title %}{{ title }} — light-sentry{% endblock %}
{% block head %}<style>
  pre { background: #1e1e1e; color: #d4d4d4; padding: 1rem; overflow-x: auto; border-radius: 4px; font-size: 0.8rem; }
  .frame { margin-bottom: 0.5rem; }
  .frame-file { color: #569cd6; }
  .frame-func { color: #dcdcaa; }
  .frame-line { color: #b5cea8; }
</style>{% endblock %}
{% block content %}
{% include "partials/project_nav.html" %}
<h2>{{ title }}</h2>
<p><span class="badge badge-{{ level }}">{{ level }}</span> — {{ count }} events — Last seen {{ last_seen.format("%Y-%m-%d %H:%M") }}</p>

{% if stack_trace.is_some() %}
<h3>Stack Trace</h3>
<pre>{% for frame in frames %}
<span class="frame"><span class="frame-file">{{ frame.filename }}</span>:<span class="frame-line">{{ frame.lineno }}</span> in <span class="frame-func">{{ frame.function }}</span>
{% if frame.context_line.is_some() %}  {{ frame.context_line.as_ref().unwrap() }}{% endif %}
</span>{% endfor %}</pre>
{% endif %}

<h3>Recent Events</h3>
<table>
  <thead><tr><th>Event ID</th><th>Message</th><th>Time</th></tr></thead>
  <tbody>
    {% for event in events %}
    <tr>
      <td><code>{{ event.event_id|truncate(12) }}</code></td>
      <td>{{ event.message }}</td>
      <td>{{ event.received_at.format("%Y-%m-%d %H:%M:%S") }}</td>
    </tr>
    {% endfor %}
  </tbody>
</table>
{% endblock %}
```

**Step 3: Create project nav partial**

```html
<!-- templates/partials/project_nav.html -->
<nav>
  <ul>
    <li><a href="/{{ project_id }}/issues">Issues</a></li>
    <li><a href="/{{ project_id }}/performance">Performance</a></li>
    <li><a href="/{{ project_id }}/logs">Logs</a></li>
  </ul>
</nav>
```

**Step 4: Implement issues routes**

```rust
// src/routes/issues.rs
// Two handlers:
// GET /{project_id}/issues — grouped issues list
//   Query: SELECT fingerprint, MAX(title) as title, MAX(level) as level,
//          COUNT(*) as count, MAX(received_at) as last_seen
//          FROM error_events WHERE project_id = $1
//          GROUP BY fingerprint ORDER BY last_seen DESC LIMIT 100
//
// GET /{project_id}/issues/{fingerprint} — issue detail
//   Query the group info + recent events for that fingerprint
//   Parse stack_trace JSON into displayable frames
```

Implement with proper structs and templates.

**Step 5: Wire routes and commit**

```bash
git commit -m "feat: issues dashboard with grouped errors and detail view"
```

---

### Task 9: Performance Dashboard

**Files:**
- Create: `src/routes/performance.rs`
- Create: `templates/performance.html`
- Create: `templates/performance_detail.html`
- Modify: `src/routes/mod.rs`
- Modify: `src/main.rs`

**Step 1: Create performance list template**

Shows transactions grouped by name with P50, P95, count.

```sql
-- Query for performance list
SELECT name,
       COUNT(*) as count,
       PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY duration_ms) as p50,
       PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) as p95,
       MAX(received_at) as last_seen
FROM transactions
WHERE project_id = $1
GROUP BY name
ORDER BY p95 DESC NULLS LAST
LIMIT 100
```

**Step 2: Create performance detail template**

Shows recent transactions for a given name with duration, status, time. Plus the span waterfall from the JSONB spans column.

**Step 3: Implement routes, wire them, commit**

```bash
git commit -m "feat: performance dashboard with P50/P95 and span waterfall"
```

---

### Task 10: Logs Dashboard

**Files:**
- Create: `src/routes/logs.rs`
- Create: `templates/logs.html`
- Modify: `src/routes/mod.rs`
- Modify: `src/main.rs`

**Step 1: Create logs template**

Filterable log list with level badges, message, time. HTMX polling for live tail.

```html
<!-- templates/logs.html -->
{% extends "base.html" %}
{% block title %}Logs — {{ project_name }}{% endblock %}
{% block content %}
{% include "partials/project_nav.html" %}
<h2>Logs</h2>
<form hx-get="/{{ project_id }}/logs/stream" hx-target="#log-table-body" hx-swap="innerHTML"
      hx-trigger="change" style="display:flex;gap:1rem;margin-bottom:1rem">
  <select name="level">
    <option value="">All Levels</option>
    <option value="debug">Debug</option>
    <option value="info">Info</option>
    <option value="warn">Warn</option>
    <option value="error">Error</option>
    <option value="fatal">Fatal</option>
  </select>
  <input type="search" name="search" placeholder="Search messages..." />
</form>

<table>
  <thead><tr><th>Level</th><th>Message</th><th>Time</th></tr></thead>
  <tbody id="log-table-body"
         hx-get="/{{ project_id }}/logs/stream"
         hx-trigger="every 5s"
         hx-swap="innerHTML">
    {% for log in logs %}
    {% include "partials/log_row.html" %}
    {% endfor %}
  </tbody>
</table>
{% endblock %}
```

```html
<!-- templates/partials/log_row.html -->
<tr>
  <td><span class="badge badge-{{ log.level }}">{{ log.level }}</span></td>
  <td>{{ log.message }}</td>
  <td>{{ log.received_at.format("%H:%M:%S") }}</td>
</tr>
```

**Step 2: Implement logs route with HTMX partial for live tail**

The `/stream` endpoint returns just the `<tr>` rows for HTMX to swap in.

**Step 3: Wire routes and commit**

```bash
git commit -m "feat: logs dashboard with filtering and HTMX live tail"
```

---

### Task 11: Background Retention Cleanup

**Files:**
- Modify: `src/main.rs` (spawn background task)
- Create: `src/background.rs`

**Step 1: Implement retention cleanup**

```rust
// src/background.rs
use sqlx::PgPool;
use std::time::Duration;

pub fn spawn_retention_cleanup(pool: PgPool) {
    tokio::spawn(async move {
        let retention_days: i64 = std::env::var("RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let cutoff = format!("{} days", retention_days);
            for table in &["error_events", "transactions", "logs"] {
                let query = format!(
                    "DELETE FROM {} WHERE received_at < NOW() - INTERVAL '{}'",
                    table, cutoff
                );
                match sqlx::query(&query).execute(&pool).await {
                    Ok(result) => {
                        if result.rows_affected() > 0 {
                            tracing::info!(
                                "Retention cleanup: deleted {} rows from {}",
                                result.rows_affected(), table
                            );
                        }
                    }
                    Err(e) => tracing::error!("Retention cleanup error on {}: {}", table, e),
                }
            }
        }
    });
}
```

**Step 2: Spawn in main.rs before server start**

```rust
background::spawn_retention_cleanup(state.db.clone());
```

**Step 3: Commit**

```bash
git commit -m "feat: background retention cleanup (configurable, default 30 days)"
```

---

### Task 12: Polish & Integration Test

**Files:**
- Modify: `src/main.rs` (final route wiring, root redirect)
- Create: `tests/integration.rs` (optional)
- Create: `README.md`

**Step 1: Add root redirect**

```rust
.route("/", get(|| async { Redirect::to("/projects") }))
```

**Step 2: Manual integration test**

1. Start Postgres: `docker run -d --name ls-pg -e POSTGRES_PASSWORD=light_sentry -e POSTGRES_USER=light_sentry -e POSTGRES_DB=light_sentry -p 5432:5432 postgres:16`
2. Start server: `cargo run`
3. Register at `http://localhost:3000/register`
4. Create a project, copy the DSN
5. Test with curl:

```bash
# Test store endpoint
curl -X POST http://localhost:3000/api/{project_id}/store/ \
  -H "X-Sentry-Auth: Sentry sentry_key={dsn_public},sentry_version=7" \
  -H "Content-Type: application/json" \
  -d '{"event_id":"test123","platform":"python","level":"error","exception":{"values":[{"type":"ValueError","value":"test error","stacktrace":{"frames":[{"filename":"app.py","function":"main","lineno":42,"in_app":true}]}}]}}'

# Test envelope endpoint
curl -X POST http://localhost:3000/api/{project_id}/envelope/ \
  -H "X-Sentry-Auth: Sentry sentry_key={dsn_public},sentry_version=7" \
  -H "Content-Type: application/x-sentry-envelope" \
  -d '{"event_id":"test456"}
{"type":"event"}
{"event_id":"test456","platform":"python","level":"error","message":"envelope test"}'
```

6. Check issues appear in dashboard

**Step 3: Test with a real Sentry SDK**

```python
import sentry_sdk
sentry_sdk.init(dsn="http://{dsn_public}@localhost:3000/{project_id}")
raise ValueError("test from real SDK")
```

**Step 4: Write README.md**

Quick start, configuration env vars, DSN format.

**Step 5: Final commit**

```bash
git commit -m "feat: polish, integration testing, README"
git push -u origin main
```
