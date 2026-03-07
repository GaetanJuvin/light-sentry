use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::{auth, state::AppState};

const USER_ID_KEY: &str = "user_id";

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: Option<String>,
    registration_enabled: bool,
}

#[derive(Template)]
#[template(path = "register.html")]
struct RegisterTemplate {
    error: Option<String>,
}

#[derive(Deserialize)]
pub struct AuthForm {
    email: String,
    password: String,
}

fn render<T: Template>(tmpl: T) -> Response {
    match tmpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!("template render error: {e}");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
        }
    }
}

pub async fn login_page(State(state): State<AppState>) -> Response {
    render(LoginTemplate { error: None, registration_enabled: state.registration_enabled })
}

pub async fn login_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AuthForm>,
) -> Response {
    let registration_enabled = state.registration_enabled;

    let row: Option<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE email = $1")
            .bind(&form.email)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

    let Some((user_id, password_hash)) = row else {
        return render(LoginTemplate {
            error: Some("Invalid credentials".into()),
            registration_enabled,
        });
    };

    let pw = form.password.clone();
    let valid = tokio::task::spawn_blocking(move || auth::verify_password(&pw, &password_hash))
        .await
        .unwrap_or(false);

    if !valid {
        return render(LoginTemplate {
            error: Some("Invalid credentials".into()),
            registration_enabled,
        });
    }

    session
        .insert(USER_ID_KEY, user_id.to_string())
        .await
        .ok();
    Redirect::to("/projects").into_response()
}

pub async fn register_page(State(state): State<AppState>) -> Response {
    if !state.registration_enabled {
        return Redirect::to("/login").into_response();
    }
    render(RegisterTemplate { error: None })
}

pub async fn register_submit(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AuthForm>,
) -> Response {
    if !state.registration_enabled {
        return Redirect::to("/login").into_response();
    }
    if form.password.len() < 8 {
        return render(RegisterTemplate {
            error: Some("Password must be at least 8 characters".into()),
        });
    }

    let pw = form.password.clone();
    let hash = match tokio::task::spawn_blocking(move || auth::hash_password(&pw)).await {
        Ok(Ok(h)) => h,
        _ => {
            return render(RegisterTemplate {
                error: Some("Registration failed".into()),
            })
        }
    };

    let result: Result<(uuid::Uuid,), _> =
        sqlx::query_as("INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id")
            .bind(&form.email)
            .bind(&hash)
            .fetch_one(&state.db)
            .await;

    match result {
        Ok((user_id,)) => {
            session
                .insert(USER_ID_KEY, user_id.to_string())
                .await
                .ok();
            Redirect::to("/projects").into_response()
        }
        Err(_) => render(RegisterTemplate {
            error: Some("Email already registered".into()),
        }),
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
