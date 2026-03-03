use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct ListIssuesParams {
    #[schemars(description = "UUID of the project")]
    project_id: String,
    #[schemars(description = "Maximum number of issues to return (default 50)")]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetIssueDetailParams {
    #[schemars(description = "UUID of the project")]
    project_id: String,
    #[schemars(description = "Issue fingerprint (group key)")]
    fingerprint: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListTransactionsParams {
    #[schemars(description = "UUID of the project")]
    project_id: String,
    #[schemars(description = "Maximum number of transaction groups to return (default 50)")]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListLogsParams {
    #[schemars(description = "UUID of the project")]
    project_id: String,
    #[schemars(description = "Filter by log level (e.g. info, warning, error)")]
    level: Option<String>,
    #[schemars(description = "Search string to filter log messages (case-insensitive)")]
    search: Option<String>,
    #[schemars(description = "Maximum number of logs to return (default 50)")]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchErrorsParams {
    #[schemars(description = "UUID of the project")]
    project_id: String,
    #[schemars(description = "Search query to match against error messages (case-insensitive)")]
    query: String,
    #[schemars(description = "Maximum number of results to return (default 50)")]
    limit: Option<i64>,
}

// ---------------------------------------------------------------------------
// Helper to parse project_id
// ---------------------------------------------------------------------------

fn parse_project_id(s: &str) -> Result<uuid::Uuid, McpError> {
    s.parse::<uuid::Uuid>()
        .map_err(|e| McpError::invalid_params(format!("Invalid project_id: {e}"), None))
}

// ---------------------------------------------------------------------------
// JSON response helpers
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProjectJson {
    id: String,
    name: String,
    dsn_public: String,
    created_at: String,
}

#[derive(Serialize)]
struct IssueJson {
    fingerprint: String,
    title: String,
    level: String,
    count: i64,
    last_seen: Option<String>,
}

#[derive(Serialize)]
struct IssueDetailJson {
    fingerprint: String,
    title: String,
    level: String,
    count: i64,
    last_seen: Option<String>,
    recent_events: Vec<EventJson>,
}

#[derive(Serialize)]
struct EventJson {
    event_id: String,
    message: String,
    stack_trace: Option<serde_json::Value>,
    received_at: String,
}

#[derive(Serialize)]
struct TransactionGroupJson {
    name: String,
    count: i64,
    p50_ms: Option<f64>,
    p95_ms: Option<f64>,
    last_seen: Option<String>,
}

#[derive(Serialize)]
struct LogJson {
    level: String,
    message: String,
    received_at: String,
    context: serde_json::Value,
}

#[derive(Serialize)]
struct SearchResultJson {
    event_id: String,
    fingerprint: String,
    title: String,
    message: String,
    level: String,
    received_at: String,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LightSentryMcp {
    db: PgPool,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl LightSentryMcp {
    fn new(db: PgPool) -> Self {
        Self {
            db,
            tool_router: Self::tool_router(),
        }
    }

    /// List all projects with their id, name, and DSN.
    #[tool(description = "List all projects with their id, name, and DSN")]
    async fn list_projects(&self) -> Result<CallToolResult, McpError> {
        let rows: Vec<(uuid::Uuid, String, String, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as(
                "SELECT id, name, dsn_public, created_at FROM projects ORDER BY created_at DESC",
            )
            .fetch_all(&self.db)
            .await
            .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let projects: Vec<ProjectJson> = rows
            .into_iter()
            .map(|(id, name, dsn_public, created_at)| ProjectJson {
                id: id.to_string(),
                name,
                dsn_public,
                created_at: created_at.to_rfc3339(),
            })
            .collect();

        let json = serde_json::to_string_pretty(&projects)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List issues (grouped errors) for a project, ordered by most recent.
    #[tool(description = "List issues (grouped errors) for a project, ordered by most recent")]
    async fn list_issues(
        &self,
        Parameters(params): Parameters<ListIssuesParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&params.project_id)?;
        let limit = params.limit.unwrap_or(50).min(200);

        let rows: Vec<(
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<chrono::DateTime<chrono::Utc>>,
        )> = sqlx::query_as(
            "SELECT fingerprint, \
                    MAX(title) as title, \
                    MAX(level) as level, \
                    COUNT(*) as count, \
                    MAX(received_at) as last_seen \
             FROM error_events \
             WHERE project_id = $1 \
             GROUP BY fingerprint \
             ORDER BY last_seen DESC \
             LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.db)
        .await
        .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let issues: Vec<IssueJson> = rows
            .into_iter()
            .map(|(fingerprint, title, level, count, last_seen)| IssueJson {
                fingerprint,
                title: title.unwrap_or_else(|| "(unknown)".into()),
                level: level.unwrap_or_else(|| "error".into()),
                count: count.unwrap_or(0),
                last_seen: last_seen.map(|t| t.to_rfc3339()),
            })
            .collect();

        let json = serde_json::to_string_pretty(&issues)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get issue summary and recent events with stack traces.
    #[tool(description = "Get issue detail: summary + recent events with stack traces")]
    async fn get_issue_detail(
        &self,
        Parameters(params): Parameters<GetIssueDetailParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&params.project_id)?;

        // Issue summary
        let summary: Option<(
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<chrono::DateTime<chrono::Utc>>,
        )> = sqlx::query_as(
            "SELECT fingerprint, \
                    MAX(title) as title, \
                    MAX(level) as level, \
                    COUNT(*) as count, \
                    MAX(received_at) as last_seen \
             FROM error_events \
             WHERE project_id = $1 AND fingerprint = $2 \
             GROUP BY fingerprint",
        )
        .bind(project_id)
        .bind(&params.fingerprint)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let Some((fingerprint, title, level, count, last_seen)) = summary else {
            return Err(McpError::invalid_params(
                format!("Issue not found: {}", params.fingerprint),
                None,
            ));
        };

        // Recent events
        let events: Vec<(
            String,
            String,
            Option<serde_json::Value>,
            chrono::DateTime<chrono::Utc>,
        )> = sqlx::query_as(
            "SELECT event_id, message, stack_trace, received_at \
             FROM error_events \
             WHERE project_id = $1 AND fingerprint = $2 \
             ORDER BY received_at DESC \
             LIMIT 10",
        )
        .bind(project_id)
        .bind(&params.fingerprint)
        .fetch_all(&self.db)
        .await
        .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let detail = IssueDetailJson {
            fingerprint,
            title: title.unwrap_or_else(|| "(unknown)".into()),
            level: level.unwrap_or_else(|| "error".into()),
            count: count.unwrap_or(0),
            last_seen: last_seen.map(|t| t.to_rfc3339()),
            recent_events: events
                .into_iter()
                .map(|(event_id, message, stack_trace, received_at)| EventJson {
                    event_id,
                    message,
                    stack_trace,
                    received_at: received_at.to_rfc3339(),
                })
                .collect(),
        };

        let json = serde_json::to_string_pretty(&detail)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List transaction groups with p50/p95 latency stats.
    #[tool(description = "List transaction groups with p50/p95 latency percentiles")]
    async fn list_transactions(
        &self,
        Parameters(params): Parameters<ListTransactionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&params.project_id)?;
        let limit = params.limit.unwrap_or(50).min(200);

        let rows: Vec<(
            String,
            Option<i64>,
            Option<f64>,
            Option<f64>,
            Option<chrono::DateTime<chrono::Utc>>,
        )> = sqlx::query_as(
            "SELECT name, \
                    COUNT(*) as count, \
                    PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY duration_ms) as p50, \
                    PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY duration_ms) as p95, \
                    MAX(received_at) as last_seen \
             FROM transactions \
             WHERE project_id = $1 \
             GROUP BY name \
             ORDER BY p95 DESC NULLS LAST \
             LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&self.db)
        .await
        .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let txns: Vec<TransactionGroupJson> = rows
            .into_iter()
            .map(|(name, count, p50, p95, last_seen)| TransactionGroupJson {
                name,
                count: count.unwrap_or(0),
                p50_ms: p50,
                p95_ms: p95,
                last_seen: last_seen.map(|t| t.to_rfc3339()),
            })
            .collect();

        let json = serde_json::to_string_pretty(&txns)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List recent logs with optional level and search filtering.
    #[tool(description = "List recent logs with optional level and search filtering")]
    async fn list_logs(
        &self,
        Parameters(params): Parameters<ListLogsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&params.project_id)?;
        let limit = params.limit.unwrap_or(50).min(200);
        let level = params.level.as_deref().filter(|l| !l.is_empty());
        let search = params.search.as_deref().filter(|s| !s.is_empty());

        let rows: Vec<(String, String, chrono::DateTime<chrono::Utc>, serde_json::Value)> =
            sqlx::query_as(
                "SELECT level, message, received_at, context \
                 FROM logs \
                 WHERE project_id = $1 \
                   AND ($2::text IS NULL OR level = $2) \
                   AND ($3::text IS NULL OR message ILIKE '%' || $3 || '%') \
                 ORDER BY received_at DESC \
                 LIMIT $4",
            )
            .bind(project_id)
            .bind(level)
            .bind(search)
            .bind(limit)
            .fetch_all(&self.db)
            .await
            .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let logs: Vec<LogJson> = rows
            .into_iter()
            .map(|(level, message, received_at, context)| LogJson {
                level,
                message,
                received_at: received_at.to_rfc3339(),
                context,
            })
            .collect();

        let json = serde_json::to_string_pretty(&logs)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Full-text search across error event messages.
    #[tool(description = "Full-text search across error messages")]
    async fn search_errors(
        &self,
        Parameters(params): Parameters<SearchErrorsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project_id = parse_project_id(&params.project_id)?;
        let limit = params.limit.unwrap_or(50).min(200);

        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            chrono::DateTime<chrono::Utc>,
        )> = sqlx::query_as(
            "SELECT event_id, fingerprint, title, message, level, received_at \
             FROM error_events \
             WHERE project_id = $1 \
               AND message ILIKE '%' || $2 || '%' \
             ORDER BY received_at DESC \
             LIMIT $3",
        )
        .bind(project_id)
        .bind(&params.query)
        .bind(limit)
        .fetch_all(&self.db)
        .await
        .map_err(|e| McpError::internal_error(format!("DB error: {e}"), None))?;

        let results: Vec<SearchResultJson> = rows
            .into_iter()
            .map(
                |(event_id, fingerprint, title, message, level, received_at)| SearchResultJson {
                    event_id,
                    fingerprint,
                    title,
                    message,
                    level,
                    received_at: received_at.to_rfc3339(),
                },
            )
            .collect();

        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| McpError::internal_error(format!("JSON error: {e}"), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

#[tool_handler]
impl ServerHandler for LightSentryMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "light-sentry-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: None,
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Light Sentry MCP server. Browse projects, issues (grouped errors), \
                 performance transactions, and logs from your Light Sentry instance."
                    .into(),
            ),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Log to stderr so stdout stays clean for MCP JSON-RPC
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    tracing::info!("Light Sentry MCP server starting");

    let service = LightSentryMcp::new(pool).serve(stdio()).await?;

    service.waiting().await?;

    Ok(())
}
