use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::{routes::auth::require_user, state::AppState};

fn render<T: Template>(tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("template render error: {e}");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
        }
    }
}

#[derive(sqlx::FromRow)]
struct PerfQueryRow {
    name: String,
    count: Option<i64>,
    p50: Option<f64>,
    p95: Option<f64>,
    last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

/// Template-friendly version
pub struct PerfDisplay {
    pub name: String,
    pub count: i64,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<PerfQueryRow> for PerfDisplay {
    fn from(row: PerfQueryRow) -> Self {
        Self {
            name: row.name,
            count: row.count.unwrap_or(0),
            p50: row.p50,
            p95: row.p95,
            last_seen: row.last_seen,
        }
    }
}

#[derive(sqlx::FromRow)]
pub struct TransactionRow {
    pub event_id: String,
    pub duration_ms: f64,
    pub status: String,
    pub spans: Option<serde_json::Value>,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

pub struct SpanRow {
    pub op: String,
    pub description: String,
    pub duration_ms: Option<f64>,
}

#[derive(Deserialize, Default)]
pub struct PerfSort {
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_dir")]
    pub dir: String,
}

fn default_sort() -> String { "p95".into() }
fn default_dir() -> String { "desc".into() }

#[derive(Template)]
#[template(path = "performance.html")]
struct PerformanceTemplate {
    project_id: uuid::Uuid,
    transactions: Vec<PerfDisplay>,
    sort: String,
    dir: String,
}

#[derive(Template)]
#[template(path = "performance_detail.html")]
struct PerformanceDetailTemplate {
    project_id: uuid::Uuid,
    txn_name: String,
    transactions: Vec<TransactionRow>,
    spans: Vec<SpanRow>,
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Path(project_id): Path<uuid::Uuid>,
    Query(params): Query<PerfSort>,
) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let order_col = match params.sort.as_str() {
        "name" => "name",
        "count" => "count",
        "p50" => "p50",
        "last_seen" => "last_seen",
        _ => "p95",
    };
    let order_dir = if params.dir == "asc" { "ASC" } else { "DESC" };
    let nulls = if params.dir == "asc" { "NULLS FIRST" } else { "NULLS LAST" };

    let query = format!(
        "SELECT name, \
               COUNT(*) as count, \
               PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY duration_ms) as p50, \
               PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) as p95, \
               MAX(received_at) as last_seen \
         FROM transactions \
         WHERE project_id = $1 \
         GROUP BY name \
         ORDER BY {order_col} {order_dir} {nulls} \
         LIMIT 100"
    );

    let rows: Vec<PerfQueryRow> = sqlx::query_as(&query)
        .bind(project_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();

    let transactions: Vec<PerfDisplay> = rows.into_iter().map(PerfDisplay::from).collect();

    render(PerformanceTemplate {
        project_id,
        transactions,
        sort: params.sort,
        dir: params.dir,
    })
}

pub async fn detail(
    State(state): State<AppState>,
    session: Session,
    Path((project_id, name)): Path<(uuid::Uuid, String)>,
) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let transactions: Vec<TransactionRow> = sqlx::query_as(
        "SELECT event_id, duration_ms, status, spans, received_at \
         FROM transactions \
         WHERE project_id = $1 AND name = $2 \
         ORDER BY received_at DESC \
         LIMIT 50",
    )
    .bind(project_id)
    .bind(&name)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Extract spans from the latest transaction
    let spans = extract_spans(&transactions);

    render(PerformanceDetailTemplate {
        project_id,
        txn_name: name,
        transactions,
        spans,
    })
}

fn extract_spans(transactions: &[TransactionRow]) -> Vec<SpanRow> {
    let Some(txn) = transactions.first() else {
        return vec![];
    };
    let Some(spans_val) = &txn.spans else {
        return vec![];
    };
    let Some(arr) = spans_val.as_array() else {
        return vec![];
    };

    arr.iter()
        .map(|s| SpanRow {
            op: s
                .get("op")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: s
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            duration_ms: s.get("duration_ms").and_then(|v| v.as_f64()).or_else(|| {
                // Try computing from start_timestamp and timestamp
                let start = s.get("start_timestamp")?.as_f64()?;
                let end = s.get("timestamp")?.as_f64()?;
                Some((end - start) * 1000.0)
            }),
        })
        .collect()
}
