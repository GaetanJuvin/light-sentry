# Light Sentry

A lightweight, self-hosted error tracking and performance monitoring server compatible with the Sentry SDK protocol.

## Prerequisites

- Rust 1.85+ (2024 edition)
- PostgreSQL 16+
- Docker (optional, for running Postgres)

## Quick Start

### 1. Start PostgreSQL

```bash
docker compose up -d
```

### 2. Configure environment

```bash
cp .env.example .env
# Edit .env if needed
```

### 3. Run the server

```bash
cargo run
```

The server runs database migrations automatically on startup.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `LISTEN_ADDR` | `0.0.0.0:3000` | Address and port to bind |
| `RUST_LOG` | `info` | Log level filter (e.g. `debug`, `info,sqlx=warn`) |
| `RETENTION_DAYS` | `30` | Days to keep events before automatic cleanup |
| `PUBLIC_HOST` | `localhost:3000` | Public hostname shown in DSN URLs |

## Usage

### 1. Register and create a project

1. Open `http://localhost:3000/register` and create an account
2. Log in at `http://localhost:3000/login`
3. Create a project at `http://localhost:3000/projects/new`
4. Copy the DSN shown on the projects page

The DSN format is: `http://<public_key>@<PUBLIC_HOST>/<project_id>`

### 2. Configure a Sentry SDK

#### Python

```python
import sentry_sdk

sentry_sdk.init(
    dsn="http://<public_key>@localhost:3000/<project_id>",
    traces_sample_rate=1.0,
)
```

#### JavaScript (Node.js)

```javascript
const Sentry = require("@sentry/node");

Sentry.init({
  dsn: "http://<public_key>@localhost:3000/<project_id>",
  tracesSampleRate: 1.0,
});
```

#### JavaScript (Browser)

```javascript
import * as Sentry from "@sentry/browser";

Sentry.init({
  dsn: "http://<public_key>@localhost:3000/<project_id>",
  tracesSampleRate: 1.0,
});
```

## Dashboards

- **Issues** -- `/{project_id}/issues` -- Error events grouped by fingerprint
- **Performance** -- `/{project_id}/performance` -- Transaction traces and durations
- **Logs** -- `/{project_id}/logs` -- Structured log entries with live streaming

## License

MIT
