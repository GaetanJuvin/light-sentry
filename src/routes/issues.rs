use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
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
struct IssueQueryRow {
    fingerprint: String,
    title: Option<String>,
    level: Option<String>,
    count: Option<i64>,
    last_seen: Option<chrono::DateTime<chrono::Utc>>,
    last_context: Option<serde_json::Value>,
}

/// Template-friendly version with no Option fields for direct display
pub struct IssueDisplay {
    pub fingerprint: String,
    pub title: String,
    pub level: String,
    pub count: i64,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub request_path: Option<String>,
}

impl From<IssueQueryRow> for IssueDisplay {
    fn from(row: IssueQueryRow) -> Self {
        let request_path = row
            .last_context
            .as_ref()
            .and_then(|ctx| ctx.get("request"))
            .and_then(|req| req.get("url"))
            .and_then(|v| v.as_str())
            .map(|url| {
                // Extract path from URL: skip "https://host" prefix
                if let Some(pos) = url.find("://") {
                    if let Some(path_start) = url[pos + 3..].find('/') {
                        return url[pos + 3 + path_start..].to_string();
                    }
                }
                url.to_string()
            });

        Self {
            fingerprint: row.fingerprint,
            title: row.title.unwrap_or_else(|| "(unknown)".into()),
            level: row.level.unwrap_or_else(|| "error".into()),
            count: row.count.unwrap_or(0),
            last_seen: row.last_seen,
            request_path,
        }
    }
}

#[derive(sqlx::FromRow)]
pub struct EventRow {
    pub event_id: String,
    pub message: String,
    pub stack_trace: Option<serde_json::Value>,
    pub context: Option<serde_json::Value>,
    pub received_at: chrono::DateTime<chrono::Utc>,
}

impl EventRow {
    pub fn request_url(&self) -> Option<String> {
        self.context
            .as_ref()?
            .get("request")?
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn request_method(&self) -> Option<String> {
        self.context
            .as_ref()?
            .get("request")?
            .get("method")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}

pub struct StackFrame {
    pub filename: String,
    pub lineno: String,
    pub function: String,
}

#[derive(Template)]
#[template(path = "issues.html")]
struct IssuesTemplate {
    project_id: uuid::Uuid,
    project_name: String,
    issues: Vec<IssueDisplay>,
    active_tab: &'static str,
}

#[derive(Template)]
#[template(path = "issue_detail.html")]
struct IssueDetailTemplate {
    project_id: uuid::Uuid,
    project_name: String,
    issue: IssueDisplay,
    frames: Vec<StackFrame>,
    events: Vec<EventRow>,
    active_tab: &'static str,
}

pub async fn list(
    State(state): State<AppState>,
    session: Session,
    Path(project_id): Path<uuid::Uuid>,
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

    let rows: Vec<IssueQueryRow> = sqlx::query_as(
        "SELECT e.fingerprint, \
               MAX(e.title) as title, \
               MAX(e.level) as level, \
               COUNT(*) as count, \
               MAX(e.received_at) as last_seen, \
               (SELECT context FROM error_events e2 \
                WHERE e2.fingerprint = e.fingerprint AND e2.project_id = $1 \
                ORDER BY e2.received_at DESC LIMIT 1) as last_context \
         FROM error_events e \
         WHERE e.project_id = $1 \
         GROUP BY e.fingerprint \
         ORDER BY last_seen DESC \
         LIMIT 100",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let issues: Vec<IssueDisplay> = rows.into_iter().map(IssueDisplay::from).collect();

    render(IssuesTemplate { project_id, project_name, issues, active_tab: "issues" })
}

pub async fn detail(
    State(state): State<AppState>,
    session: Session,
    Path((project_id, fingerprint)): Path<(uuid::Uuid, String)>,
) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    // Get project name
    let project_name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(project_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "Project".into());

    // Get issue summary
    let row: Option<IssueQueryRow> = sqlx::query_as(
        "SELECT e.fingerprint, \
               MAX(e.title) as title, \
               MAX(e.level) as level, \
               COUNT(*) as count, \
               MAX(e.received_at) as last_seen, \
               (SELECT context FROM error_events e2 \
                WHERE e2.fingerprint = e.fingerprint AND e2.project_id = $1 \
                ORDER BY e2.received_at DESC LIMIT 1) as last_context \
         FROM error_events e \
         WHERE e.project_id = $1 AND e.fingerprint = $2 \
         GROUP BY e.fingerprint",
    )
    .bind(project_id)
    .bind(&fingerprint)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let Some(row) = row else {
        return (axum::http::StatusCode::NOT_FOUND, "Issue not found").into_response();
    };

    let issue = IssueDisplay::from(row);

    // Get recent events
    let events: Vec<EventRow> = sqlx::query_as(
        "SELECT event_id, message, stack_trace, context, received_at \
         FROM error_events \
         WHERE project_id = $1 AND fingerprint = $2 \
         ORDER BY received_at DESC \
         LIMIT 20",
    )
    .bind(project_id)
    .bind(&fingerprint)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // Parse frames from the first event's stack_trace
    let frames = extract_frames(&events);

    render(IssueDetailTemplate {
        project_id,
        project_name,
        issue,
        frames,
        events,
        active_tab: "issues",
    })
}

fn extract_frames(events: &[EventRow]) -> Vec<StackFrame> {
    let Some(event) = events.first() else {
        return vec![];
    };
    let Some(st) = &event.stack_trace else {
        return vec![];
    };

    // stack_trace is the exception values array from Sentry
    // Try: {"values": [{"stacktrace": {"frames": [...]}}]}
    // or directly an array of exception values
    let values = st
        .get("values")
        .and_then(|v| v.as_array())
        .or_else(|| st.as_array());

    let Some(values) = values else {
        return vec![];
    };

    for val in values {
        if let Some(frames_arr) = val
            .get("stacktrace")
            .and_then(|st| st.get("frames"))
            .and_then(|f| f.as_array())
        {
            return frames_arr
                .iter()
                .rev()
                .map(|f| StackFrame {
                    filename: f
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                    lineno: f
                        .get("lineno")
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".to_string()),
                    function: f
                        .get("function")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string(),
                })
                .collect();
        }
    }

    vec![]
}
