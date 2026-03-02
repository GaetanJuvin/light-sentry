use axum::{Router, routing::{get, post}};
use tower_http::trace::TraceLayer;
use tower_sessions::{MemoryStore, SessionManagerLayer, Expiry};
use time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
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

    let state = AppState { db: pool };

    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::hours(2)));

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
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
        .with_state(state)
        .layer(session_layer)
        .layer(TraceLayer::new_for_http());

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
