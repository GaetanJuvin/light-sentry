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
pub struct LogRow {
    pub level: String,
    pub message: String,
    pub received_at: chrono::DateTime<chrono::Utc>,
    pub context: serde_json::Value,
}

#[derive(Deserialize, Default)]
pub struct LogFilter {
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Template)]
#[template(path = "logs.html")]
struct LogsTemplate {
    project_id: uuid::Uuid,
    logs: Vec<LogRow>,
    current_level: Option<String>,
    current_search: String,
}

#[derive(Template)]
#[template(path = "partials/log_stream.html")]
struct LogStreamTemplate {
    logs: Vec<LogRow>,
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

    let logs = fetch_logs(&state.db, project_id, level, search).await;

    render(LogsTemplate {
        project_id,
        logs,
        current_level: level.map(String::from),
        current_search: search.unwrap_or_default().to_string(),
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

    let logs = fetch_logs(&state.db, project_id, level, search).await;

    render(LogStreamTemplate { logs })
}

async fn fetch_logs(
    db: &sqlx::PgPool,
    project_id: uuid::Uuid,
    level: Option<&str>,
    search: Option<&str>,
) -> Vec<LogRow> {
    sqlx::query_as(
        "SELECT level, message, received_at, context \
         FROM logs \
         WHERE project_id = $1 \
           AND ($2::text IS NULL OR level = $2) \
           AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%') \
         ORDER BY received_at DESC \
         LIMIT 200",
    )
    .bind(project_id)
    .bind(level)
    .bind(search)
    .fetch_all(db)
    .await
    .unwrap_or_default()
}
