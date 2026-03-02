# light-sentry Design

## Problem

Sentry is powerful but heavy — complex to self-host, expensive at scale, and packed with features most teams don't need. light-sentry is a drop-in replacement that covers the core: error tracking, basic performance monitoring, and lightweight logging.

## Architecture

Single Rust binary serving everything: API ingestion, HTMX dashboard, and background tasks. PostgreSQL is the only external dependency.

```
┌─────────────┐       ┌────────────┐
│ light-sentry│──────▶│ PostgreSQL │
│   (1 pod)   │       └────────────┘
└─────────────┘
```

### Tech Stack

| Component | Choice | Why |
|-----------|--------|-----|
| Language | Rust | Single binary, minimal memory, max performance |
| HTTP | Axum | Async, tower ecosystem, best Rust web framework |
| Database | SQLx + PostgreSQL | Compile-time query checking, async, production-grade |
| Templates | Askama | Compiled into binary, type-safe, zero runtime overhead |
| Interactivity | HTMX | Server-driven UI, minimal JS |
| CSS | Pico CSS | Clean defaults, zero build step |
| Runtime | Tokio | Async processing and background tasks |

### Deployment

- One binary, one pod, one process
- Background tasks as Tokio spawned tasks (no separate workers)
- No message queues, no sidecars
- PostgreSQL is the only external dependency

## Data Model

### projects

| Column | Type | Notes |
|--------|------|-------|
| id | UUID | PK |
| name | TEXT | |
| dsn_public | TEXT | Unique, used in Sentry DSN |
| dsn_secret | TEXT | API key for auth |
| created_at | TIMESTAMPTZ | |

### error_events

| Column | Type | Notes |
|--------|------|-------|
| id | UUID | PK |
| project_id | UUID | FK → projects |
| fingerprint | TEXT | Grouping key: hash of type+value+top frame |
| level | TEXT | error, warning, fatal |
| title | TEXT | e.g. "TypeError: x is not a function" |
| message | TEXT | |
| stack_trace | JSONB | |
| context | JSONB | Tags, user, browser, OS, etc. |
| received_at | TIMESTAMPTZ | |

### transactions

| Column | Type | Notes |
|--------|------|-------|
| id | UUID | PK |
| project_id | UUID | FK → projects |
| trace_id | TEXT | |
| name | TEXT | e.g. "GET /api/users" |
| duration_ms | FLOAT | |
| status | TEXT | ok, error, timeout |
| spans | JSONB | Nested span tree |
| context | JSONB | |
| received_at | TIMESTAMPTZ | |

### logs

| Column | Type | Notes |
|--------|------|-------|
| id | UUID | PK |
| project_id | UUID | FK → projects |
| level | TEXT | debug, info, warn, error |
| message | TEXT | |
| context | JSONB | Structured fields |
| received_at | TIMESTAMPTZ | |

### Indexes

- `(project_id, received_at)` on all three event tables
- `(project_id, fingerprint)` on error_events

### Grouping

Errors grouped by `fingerprint` — a hash of exception type + value + top stack frame. Dashboard shows grouped issues with count and last seen via `GROUP BY fingerprint` queries. No separate issues table.

## API

### Sentry-Compatible Ingestion

Two endpoints matching Sentry's protocol:

- `POST /api/{project_id}/store/` — Legacy single event endpoint. JSON body. Auth via `X-Sentry-Auth` header or DSN query params.
- `POST /api/{project_id}/envelope/` — Modern envelope endpoint. Newline-separated JSON headers + payloads. Parses `event`, `transaction`, `log` item types. Ignores `session`, `attachment`, etc.

**DSN format:** `https://{public_key}@{host}/{project_id}`

Public key validated against project's `dsn_public`. Secret key optional (Sentry SDKs stopped requiring it).

### Dashboard (internal, cookie-session auth)

Endpoints return HTML partials for HTMX. No separate JSON API.

## Dashboard Pages

### Auth
- `/login` — email/password
- `/register` — first-user setup, then invite-only

### Main Views
- `/projects` — list, create, get DSN
- `/{project}/issues` — errors grouped by fingerprint, sorted by last seen
- `/{project}/issues/{fingerprint}` — stack trace, context, occurrence timeline
- `/{project}/performance` — transactions sorted by P50/P95 duration
- `/{project}/performance/{name}` — duration histogram, span waterfall
- `/{project}/logs` — filterable log stream, live tail via HTMX polling

### UI Approach
- Pico CSS for clean defaults
- Tables for dense data display
- HTMX for pagination, filtering, expanding details
- No charts library in v1

## Authentication

- Dashboard: email/password with cookie sessions
- Ingestion: DSN public key validation
- First registered user is admin, subsequent users invited

## Background Tasks

Tokio spawned tasks inside the same process:

- **Retention cleanup** — runs hourly, deletes events older than configurable threshold (default 30 days)
- Fingerprint grouping done synchronously at ingestion (just a hash)
- Aggregations done as queries at read time

## Out of Scope (v1)

- Alerting / notifications
- Rate limiting
- Event sampling
- Source map processing
- Release tracking
- Session replays
- Cron monitoring
- Charts / visualizations
