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
    project_name: String,
    logs: Vec<LogRow>,
    current_level: Option<String>,
    current_search: String,
    page: i64,
    total_pages: i64,
    total_count: i64,
    histogram: Vec<HistogramBar>,
    chart_width: i64,
    max_count: i64,
    first_label: String,
    last_label: String,
    active_tab: &'static str,
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

    let project_name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Project".into());

    let level = filter.level.as_deref().filter(|l| !l.is_empty());
    let search = filter.search.as_deref().filter(|s| !s.is_empty());
    let page = filter.page.max(1);

    let (logs_result, raw_histogram) = tokio::join!(
        fetch_logs(&state.db, project_id, level, search, page),
        fetch_histogram(&state.db, project_id, level, search),
    );
    let (logs, total_count) = logs_result;
    let total_pages = ((total_count as f64) / (PAGE_SIZE as f64)).ceil() as i64;
    let total_pages = total_pages.max(1);
    let max_count = raw_histogram.iter().map(|b| b.count).max().unwrap_or(0);
    let first_label = raw_histogram.first().map(|b| b.bucket.format("%H:%M").to_string()).unwrap_or_default();
    let last_label = raw_histogram.last().map(|b| b.bucket.format("%H:%M").to_string()).unwrap_or_default();
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
        project_name,
        logs,
        current_level: level.map(String::from),
        current_search: search.unwrap_or_default().to_string(),
        page,
        total_pages,
        total_count,
        histogram,
        chart_width,
        max_count,
        first_label,
        last_label,
        active_tab: "logs",
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

    // Build WHERE clause dynamically to avoid ($x IS NULL OR col = $x) anti-pattern
    // which prevents Postgres from using indexes effectively.
    let mut where_clauses = vec!["project_id = $1".to_string()];
    let mut param_idx = 2u32;

    let level_param_idx = if level.is_some() {
        let idx = param_idx;
        where_clauses.push(format!("level = ${idx}"));
        param_idx += 1;
        Some(idx)
    } else {
        None
    };

    let search_param_idx = if search.is_some() {
        let idx = param_idx;
        where_clauses.push(format!("message ILIKE '%' || ${idx} || '%'"));
        param_idx += 1;
        Some(idx)
    } else {
        None
    };

    let where_sql = where_clauses.join(" AND ");

    let data_sql = format!(
        "SELECT level, message, received_at, context \
         FROM logs WHERE {where_sql} \
         ORDER BY received_at DESC \
         LIMIT ${param_idx} OFFSET ${}",
        param_idx + 1
    );

    let count_sql = format!("SELECT COUNT(*) FROM logs WHERE {where_sql}");

    // Build and execute both queries concurrently
    let mut data_q = sqlx::query_as::<_, LogRow>(&data_sql).bind(project_id);
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(project_id);

    if let Some(_) = level_param_idx {
        data_q = data_q.bind(level);
        count_q = count_q.bind(level);
    }
    if let Some(_) = search_param_idx {
        data_q = data_q.bind(search);
        count_q = count_q.bind(search);
    }

    let data_q = data_q.bind(PAGE_SIZE).bind(offset);

    let (logs_result, count_result) = tokio::join!(
        data_q.fetch_all(db),
        count_q.fetch_one(db),
    );

    (logs_result.unwrap_or_default(), count_result.unwrap_or(0))
}

async fn fetch_histogram(
    db: &sqlx::PgPool,
    project_id: uuid::Uuid,
    level: Option<&str>,
    search: Option<&str>,
) -> Vec<HistogramRow> {
    let mut where_clauses = vec![
        "project_id = $1".to_string(),
        "received_at > NOW() - INTERVAL '1 hour'".to_string(),
    ];
    let mut param_idx = 2u32;

    if level.is_some() {
        where_clauses.push(format!("level = ${param_idx}"));
        param_idx += 1;
    }
    if search.is_some() {
        where_clauses.push(format!("message ILIKE '%' || ${param_idx} || '%'"));
    }

    let where_sql = where_clauses.join(" AND ");
    let sql = format!(
        "SELECT date_trunc('minute', received_at) AS bucket, \
                COUNT(*) AS count \
         FROM logs WHERE {where_sql} \
         GROUP BY bucket ORDER BY bucket"
    );

    let mut q = sqlx::query_as::<_, HistogramRow>(&sql).bind(project_id);
    if level.is_some() {
        q = q.bind(level);
    }
    if search.is_some() {
        q = q.bind(search);
    }

    q.fetch_all(db).await.unwrap_or_default()
}
