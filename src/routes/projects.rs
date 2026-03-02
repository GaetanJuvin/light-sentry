use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
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
struct ProjectRow {
    id: uuid::Uuid,
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
struct ProjectNewTemplate;

#[derive(Deserialize)]
pub struct NewProjectForm {
    name: String,
}

pub async fn list(State(state): State<AppState>, session: Session) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let host =
        std::env::var("PUBLIC_HOST").unwrap_or_else(|_| "localhost:3000".into());

    let projects: Vec<ProjectRow> =
        sqlx::query_as("SELECT id, name, dsn_public, created_at FROM projects ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    render(ProjectsTemplate { projects, host })
}

pub async fn new_form(session: Session) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };
    render(ProjectNewTemplate)
}

pub async fn create(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<NewProjectForm>,
) -> Response {
    let Some(_user_id) = require_user(&session).await else {
        return Redirect::to("/login").into_response();
    };

    let dsn_public = hex::encode(rand::random::<[u8; 16]>());
    let dsn_secret = hex::encode(rand::random::<[u8; 16]>());

    let _result: Result<(uuid::Uuid,), _> = sqlx::query_as(
        "INSERT INTO projects (name, dsn_public, dsn_secret) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(&form.name)
    .bind(&dsn_public)
    .bind(&dsn_secret)
    .fetch_one(&state.db)
    .await;

    Redirect::to("/projects").into_response()
}
