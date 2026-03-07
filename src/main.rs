use axum::{Router, routing::{get, post}};
use tower_http::trace::TraceLayer;
use tower_sessions::{MemoryStore, SessionManagerLayer, Expiry};
use time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
mod background;
mod error;
mod ingest;
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

    let registration_enabled = std::env::var("REGISTRATION_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let state = AppState { db: pool, registration_enabled };

    background::spawn_retention_cleanup(state.db.clone());

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(365)));

    let app = Router::new()
        .route("/favicon.svg", get(|| async {
            (
                [("content-type", "image/svg+xml"), ("cache-control", "public, max-age=86400")],
                "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><circle cx='16' cy='7' r='3' fill='#fafafa'/><path d='M16 4L5 9l11-2 11 2z' fill='#fafafa' opacity='.4'/><rect x='13' y='10' width='6' height='3' rx='1' fill='#d4d4d8'/><path d='M14 13h4l1 15H13z' fill='#a1a1aa'/><rect x='11' y='28' width='10' height='2' rx='1' fill='#71717a'/></svg>"
            )
        }))
        .route("/health", get(|| async { "ok" }))
        .route("/", get(|| async { axum::response::Redirect::to("/projects") }))
        .route(
            "/api/{project_id}/store/",
            post(ingest::store::store_event),
        )
        .route(
            "/api/{project_id}/envelope/",
            post(ingest::envelope::envelope_handler),
        )
        .route("/login", get(routes::auth::login_page).post(routes::auth::login_submit))
        .route("/register", get(routes::auth::register_page).post(routes::auth::register_submit))
        .route("/logout", post(routes::auth::logout))
        .route("/projects", get(routes::projects::list).post(routes::projects::create))
        .route("/projects/new", get(routes::projects::new_form))
        .route("/{project_id}/issues", get(routes::issues::list))
        .route("/{project_id}/issues/{fingerprint}", get(routes::issues::detail))
        .route("/{project_id}/performance", get(routes::performance::list))
        .route("/{project_id}/performance/{name}", get(routes::performance::detail))
        .route("/{project_id}/logs", get(routes::logs::list))
        .route("/{project_id}/logs/stream", get(routes::logs::stream))
        .with_state(state)
        .layer(session_layer)
        .layer(TraceLayer::new_for_http());

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
