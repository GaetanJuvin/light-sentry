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

const PAGE_SIZE: i64 = 50;

#[derive(sqlx::FromRow)]
pub struct LogRow {
    pub level: String,
    pub message: String,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub context: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct HistogramRow {
    bucket: chrono::DateTime<chrono::Utc>,
    count: i64,
}

pub struct HistogramBar {
    pub label: String,
    pub count: i64,
    pub bar_height: i64,
    pub x: i64,
}

#[derive(Deserialize, Default)]
pub struct LogFilter {
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default = "default_page")]
    pub page: i64,
}

fn default_page() -> i64 {
    1
}

#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    project_id: uuid::Uuid,
    logs: Vec<LogRow>,
    current_level: Option<String>,
    current_search: String,
    page: i64,
    total_pages: i64,
    total_count: i64,
    histogram: Vec<HistogramBar>,
    chart_width: i64,
}

#[derive(Template)]
#[template(path = "partials/log_stream.html")]
struct LogStreamTemplate {
    project_id: uuid::Uuid,
    logs: Vec<LogRow>,
    current_level: Option<String>,
    current_search: String,
    page: i64,
    total_pages: i64,
    total_count: i64,
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Path(project_id): Path<uuid::Uuid>,
    Query(filter): Query<LogFilter>,
) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let level = filter.level.as_deref().filter(|l| !l.is_empty());
    let search = filter.search.as_deref().filter(|s| !s.is_empty());
    let page = filter.page.max(1);

    let (logs, total_count) = fetch_logs(&state.db, project_id, level, search, page).await;
    let total_pages = ((total_count as f64) / (PAGE_SIZE as f64)).ceil() as i64;
    let total_pages = total_pages.max(1);

    let raw_histogram = fetch_histogram(&state.db, project_id, level, search).await;
    let max_count = raw_histogram.iter().map(|b| b.count).max().unwrap_or(0);
    let histogram: Vec<HistogramBar> = raw_histogram
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let bar_height = if max_count > 0 {
                (b.count as f64 / max_count as f64 * 70.0) as i64
            } else {
                0
            };
            HistogramBar {
                label: b.bucket.format("%H:%M").to_string(),
                count: b.count,
                bar_height: bar_height.max(1),
                x: (i as i64) * 14,
            }
        })
        .collect();
    let chart_width = (histogram.len() as i64) * 14;

    render(LogsTemplate {
        project_id,
        logs,
        current_level: level.map(String::from),
        current_search: search.unwrap_or_default().to_string(),
        page,
        total_pages,
        total_count,
        histogram,
        chart_width,
    })
}

pub async fn stream(
    State(state): State<AppState>,
    session: Session,
    Path(project_id): Path<uuid::Uuid>,
    Query(filter): Query<LogFilter>,
) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return (axum::http::StatusCode::UNAUTHORIZED, "").into_response();
    };

    let level = filter.level.as_deref().filter(|l| !l.is_empty());
    let search = filter.search.as_deref().filter(|s| !s.is_empty());
    let page = filter.page.max(1);

    let (logs, total_count) = fetch_logs(&state.db, project_id, level, search, page).await;
    let total_pages = ((total_count as f64) / (PAGE_SIZE as f64)).ceil() as i64;
    let total_pages = total_pages.max(1);

    render(LogStreamTemplate {
        project_id,
        logs,
        current_level: level.map(String::from),
        current_search: search.unwrap_or_default().to_string(),
        page,
        total_pages,
        total_count,
    })
}

async fn fetch_logs(
    db: &sqlx::PgPool,
    project_id: uuid::Uuid,
    level: Option<&str>,
    search: Option<&str>,
    page: i64,
) -> (Vec<LogRow>, i64) {
    let offset = (page - 1) * PAGE_SIZE;

    let logs: Vec<LogRow> = sqlx::query_as(
        "SELECT level, message, received_at, context \
         FROM logs \
         WHERE project_id = $1 \
           AND ($2::text IS NULL OR level = $2) \
           AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%') \
         ORDER BY received_at DESC \
         LIMIT $4 OFFSET $5",
    )
    .bind(project_id)
    .bind(level)
    .bind(search)
    .bind(PAGE_SIZE)
    .bind(offset)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM logs \
         WHERE project_id = $1 \
           AND ($2::text IS NULL OR level = $2) \
           AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%')",
    )
    .bind(project_id)
    .bind(level)
    .bind(search)
    .fetch_one(db)
    .await
    .unwrap_or(0);

    (logs, total_count)
}

async fn fetch_histogram(
    db: &sqlx::PgPool,
    project_id: uuid::Uuid,
    level: Option<&str>,
    search: Option<&str>,
) -> Vec<HistogramRow> {
    sqlx::query_as(
        "SELECT date_trunc('minute', received_at) AS bucket, \
                COUNT(*) AS count \
         FROM logs \
         WHERE project_id = $1 \
           AND received_at > NOW() - INTERVAL '1 hour' \
           AND ($2::text IS NULL OR level = $2) \
           AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%') \
         GROUP BY bucket \
         ORDER BY bucket",
    )
    .bind(project_id)
    .bind(level)
    .bind(search)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}
