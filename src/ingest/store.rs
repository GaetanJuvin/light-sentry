use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use super::auth::{authenticate_project, extract_auth};
use super::decompress::decompress_body;
use super::fingerprint::compute_fingerprint;
use crate::state::AppState;

pub async fn store_event(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    let _project_id_from_path = project_id;

    let auth = extract_auth(&headers, &query).ok_or(StatusCode::UNAUTHORIZED)?;
    let db_project_id = authenticate_project(&state.db, &auth.public_key)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let body_str = decompress_body(&headers, &body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let event: Value = serde_json::from_str(&body_str).map_err(|_| StatusCode::BAD_REQUEST)?;

    let event_id = event
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fingerprint = compute_fingerprint(&event);
    let level = event
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();
    let title = extract_title(&event);
    let message = event
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stack_trace = event.pointer("/exception/values").cloned();
    let context = build_context(&event);

    sqlx::query(
        "INSERT INTO error_events (project_id, event_id, fingerprint, level, title, message, stack_trace, context) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(db_project_id)
    .bind(&event_id)
    .bind(&fingerprint)
    .bind(&level)
    .bind(&title)
    .bind(&message)
    .bind(&stack_trace)
    .bind(&context)
    .execute(&state.db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({"id": event_id})))
}

pub fn extract_title(event: &Value) -> String {
    if let Some(values) = event.pointer("/exception/values").and_then(|v| v.as_array()) {
        if let Some(exc) = values.last() {
            let t = exc.get("type").and_then(|v| v.as_str()).unwrap_or("Error");
            let v = exc.get("value").and_then(|v| v.as_str()).unwrap_or("");
            return format!("{}: {}", t, v);
        }
    }
    event
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error")
        .chars()
        .take(200)
        .collect()
}

pub fn build_context(event: &Value) -> Value {
    let mut ctx = serde_json::Map::new();
    for key in &[
        "tags",
        "user",
        "request",
        "contexts",
        "breadcrumbs",
        "extra",
        "sdk",
        "server_name",
        "environment",
        "release",
        "platform",
    ] {
        if let Some(v) = event.get(*key) {
            ctx.insert(key.to_string(), v.clone());
        }
    }
    Value::Object(ctx)
}
