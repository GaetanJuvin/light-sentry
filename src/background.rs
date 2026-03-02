use sqlx::PgPool;
use std::time::Duration;

pub fn spawn_retention_cleanup(pool: PgPool) {
    tokio::spawn(async move {
        let retention_days: i64 = std::env::var("RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            for table in &["error_events", "transactions", "logs"] {
                let query = format!(
                    "DELETE FROM {} WHERE received_at < NOW() - INTERVAL '{} days'",
                    table, retention_days
                );
                match sqlx::query(&query).execute(&pool).await {
                    Ok(result) => {
                        if result.rows_affected() > 0 {
                            tracing::info!(
                                "Retention cleanup: deleted {} rows from {}",
                                result.rows_affected(),
                                table
                            );
                        }
                    }
                    Err(e) => tracing::error!("Retention cleanup error on {}: {}", table, e),
                }
            }
        }
    });
}
