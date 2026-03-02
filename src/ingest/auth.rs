use axum::http::HeaderMap;
use std::collections::HashMap;

pub struct SentryAuth {
    pub public_key: String,
    pub version: String,
    pub client: Option<String>,
}

impl SentryAuth {
    pub fn from_header(header: &str) -> Option<Self> {
        let header = header.strip_prefix("Sentry ")?.trim();
        let mut key = None;
        let mut version = None;
        let mut client = None;

        for part in header.split(',') {
            let part = part.trim();
            if let Some((k, v)) = part.split_once('=') {
                match k.trim() {
                    "sentry_key" => key = Some(v.trim().to_string()),
                    "sentry_version" => version = Some(v.trim().to_string()),
                    "sentry_client" => client = Some(v.trim().to_string()),
                    _ => {}
                }
            }
        }

        Some(SentryAuth {
            public_key: key?,
            version: version.unwrap_or_else(|| "7".to_string()),
            client,
        })
    }

    pub fn from_query(query: &str) -> Option<Self> {
        let mut key = None;
        let mut version = None;
        let mut client = None;

        for part in query.split('&') {
            if let Some((k, v)) = part.split_once('=') {
                match k {
                    "sentry_key" => key = Some(v.to_string()),
                    "sentry_version" => version = Some(v.to_string()),
                    "sentry_client" => client = Some(v.to_string()),
                    _ => {}
                }
            }
        }

        Some(SentryAuth {
            public_key: key?,
            version: version.unwrap_or_else(|| "7".to_string()),
            client,
        })
    }
}

pub async fn authenticate_project(
    pool: &sqlx::PgPool,
    public_key: &str,
) -> Option<uuid::Uuid> {
    let row: Option<(uuid::Uuid,)> =
        sqlx::query_as("SELECT id FROM projects WHERE dsn_public = $1")
            .bind(public_key)
            .fetch_optional(pool)
            .await
            .ok()?;
    row.map(|r| r.0)
}

pub fn extract_auth(
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Option<SentryAuth> {
    if let Some(h) = headers.get("X-Sentry-Auth").and_then(|v| v.to_str().ok()) {
        return SentryAuth::from_header(h);
    }
    if let Some(h) = headers.get("Authorization").and_then(|v| v.to_str().ok()) {
        return SentryAuth::from_header(h);
    }
    if let Some(key) = query.get("sentry_key") {
        return Some(SentryAuth {
            public_key: key.clone(),
            version: query
                .get("sentry_version")
                .cloned()
                .unwrap_or_else(|| "7".to_string()),
            client: query.get("sentry_client").cloned(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_x_sentry_auth_header() {
        let header =
            "Sentry sentry_version=7, sentry_client=sentry.python/1.0, sentry_key=abc123";
        let auth = SentryAuth::from_header(header).unwrap();
        assert_eq!(auth.public_key, "abc123");
        assert_eq!(auth.version, "7");
        assert_eq!(auth.client, Some("sentry.python/1.0".to_string()));
    }

    #[test]
    fn parse_query_string() {
        let query = "sentry_key=abc123&sentry_version=7";
        let auth = SentryAuth::from_query(query).unwrap();
        assert_eq!(auth.public_key, "abc123");
    }

    #[test]
    fn missing_key_fails() {
        let header = "Sentry sentry_version=7, sentry_client=sentry.python/1.0";
        assert!(SentryAuth::from_header(header).is_none());
    }
}
