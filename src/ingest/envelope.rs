use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use super::auth::{authenticate_project, extract_auth};
use super::fingerprint::compute_fingerprint;
use super::store::{build_context, extract_title};
use crate::state::AppState;

// --- Envelope parsing ---

pub struct Envelope {
    pub header: Value,
    pub items: Vec<EnvelopeItem>,
}

pub struct EnvelopeItem {
    pub item_type: String,
    pub payload: Value,
}

const SUPPORTED_TYPES: &[&str] = &["event", "transaction", "log"];

pub fn parse_envelope(raw: &str) -> Option<Envelope> {
    let mut lines = raw.split('\n');

    let header_line = lines.next()?;
    let header: Value = serde_json::from_str(header_line).ok()?;

    let mut items = Vec::new();

    while let Some(item_header_line) = lines.next() {
        if item_header_line.trim().is_empty() {
            continue;
        }

        let item_header: Value = match serde_json::from_str(item_header_line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let item_type = match item_header.get("type").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => continue,
        };

        // Collect payload lines. If length is specified, read that many bytes.
        // Otherwise, read the next line.
        let length = item_header.get("length").and_then(|v| v.as_u64());

        let payload_str = if let Some(len) = length {
            let len = len as usize;
            let mut collected = String::new();
            while collected.len() < len {
                match lines.next() {
                    Some(line) => {
                        if !collected.is_empty() {
                            collected.push('\n');
                        }
                        collected.push_str(line);
                    }
                    None => break,
                }
            }
            collected
        } else {
            // No length -- payload is the next line
            lines.next().unwrap_or("{}").to_string()
        };

        if SUPPORTED_TYPES.contains(&item_type.as_str()) {
            if let Ok(payload) = serde_json::from_str(&payload_str) {
                items.push(EnvelopeItem {
                    item_type,
                    payload,
                });
            }
        }
    }

    Some(Envelope { header, items })
}

// --- HTTP handler ---

pub async fn envelope_handler(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: String,
) -> Result<Json<Value>, StatusCode> {
    let _project_id_from_path = project_id;

    let auth = extract_auth(&headers, &query);

    let envelope = parse_envelope(&body).ok_or(StatusCode::BAD_REQUEST)?;

    let public_key = if let Some(a) = &auth {
        a.public_key.clone()
    } else if let Some(dsn) = envelope.header.get("dsn").and_then(|v| v.as_str()) {
        dsn.split("://")
            .nth(1)
            .and_then(|s| s.split('@').next())
            .map(|s| s.to_string())
            .ok_or(StatusCode::UNAUTHORIZED)?
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let db_project_id = authenticate_project(&state.db, &public_key)
        .await
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let event_id = envelope
        .header
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    for item in envelope.items {
        match item.item_type.as_str() {
            "event" => {
                ingest_error_event(&state.db, db_project_id, &item.payload)
                    .await
                    .ok();
            }
            "transaction" => {
                ingest_transaction(&state.db, db_project_id, &item.payload)
                    .await
                    .ok();
            }
            "log" => {
                ingest_logs(&state.db, db_project_id, &item.payload)
                    .await
                    .ok();
            }
            _ => {}
        }
    }

    Ok(Json(json!({"id": event_id})))
}

async fn ingest_error_event(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    event: &Value,
) -> Result<(), sqlx::Error> {
    let event_id = event
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fingerprint = compute_fingerprint(event);
    let level = event
        .get("level")
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();
    let title = extract_title(event);
    let message = event
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let stack_trace = event.pointer("/exception/values").cloned();
    let context = build_context(event);

    sqlx::query(
        "INSERT INTO error_events (project_id, event_id, fingerprint, level, title, message, stack_trace, context) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(project_id)
    .bind(&event_id)
    .bind(&fingerprint)
    .bind(&level)
    .bind(&title)
    .bind(&message)
    .bind(&stack_trace)
    .bind(&context)
    .execute(pool)
    .await?;
    Ok(())
}

async fn ingest_transaction(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    event: &Value,
) -> Result<(), sqlx::Error> {
    let event_id = event
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let trace_id = event
        .pointer("/contexts/trace/trace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = event
        .get("transaction")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let duration_ms = match (
        event.get("timestamp").and_then(parse_timestamp),
        event.get("start_timestamp").and_then(parse_timestamp),
    ) {
        (Some(end), Some(start)) => (end - start) * 1000.0,
        _ => 0.0,
    };

    let status = event
        .pointer("/contexts/trace/status")
        .and_then(|v| v.as_str())
        .unwrap_or("ok")
        .to_string();
    let spans = event.get("spans").cloned();
    let context = build_context(event);

    sqlx::query(
        "INSERT INTO transactions (project_id, event_id, trace_id, name, duration_ms, status, spans, context) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(project_id)
    .bind(&event_id)
    .bind(&trace_id)
    .bind(&name)
    .bind(duration_ms)
    .bind(&status)
    .bind(&spans)
    .bind(&context)
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_timestamp(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| {
                dt.timestamp() as f64 + dt.timestamp_subsec_nanos() as f64 / 1_000_000_000.0
            })
            .ok(),
        _ => None,
    }
}

async fn ingest_logs(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    payload: &Value,
) -> Result<(), sqlx::Error> {
    let items = payload.get("items").and_then(|v| v.as_array());
    let Some(items) = items else {
        return Ok(());
    };

    for log_entry in items {
        let level = log_entry
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info")
            .to_string();
        let message = log_entry
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let context = log_entry
            .get("attributes")
            .cloned()
            .unwrap_or(json!({}));

        sqlx::query(
            "INSERT INTO logs (project_id, level, message, context) VALUES ($1, $2, $3, $4)",
        )
        .bind(project_id)
        .bind(&level)
        .bind(&message)
        .bind(&context)
        .execute(pool)
        .await?;
    }
    Ok(())
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_envelope() {
        let raw = "{\"event_id\":\"abc123\",\"dsn\":\"https://pubkey@host/1\"}\n{\"type\":\"event\"}\n{\"exception\":{\"values\":[]}}";
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items.len(), 1);
        assert_eq!(envelope.items[0].item_type, "event");
    }

    #[test]
    fn parse_transaction_envelope() {
        let raw = "{\"event_id\":\"abc123\"}\n{\"type\":\"transaction\"}\n{\"type\":\"transaction\",\"transaction\":\"GET /api\"}";
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items[0].item_type, "transaction");
    }

    #[test]
    fn parse_log_envelope() {
        let raw = "{}\n{\"type\":\"log\"}\n{\"items\":[{\"timestamp\":1234,\"level\":\"info\",\"body\":\"hello\",\"trace_id\":\"abc\"}]}";
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items[0].item_type, "log");
    }

    #[test]
    fn skip_unknown_item_types() {
        let raw = "{}\n{\"type\":\"session\"}\n{}\n{\"type\":\"event\"}\n{\"exception\":{\"values\":[]}}";
        let envelope = parse_envelope(raw).unwrap();
        assert_eq!(envelope.items.len(), 1);
        assert_eq!(envelope.items[0].item_type, "event");
    }
}
