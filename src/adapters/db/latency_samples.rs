use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::LatencyKind;

pub struct LatencySamples<'a> {
    pool: &'a PgPool,
}

impl<'a> LatencySamples<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        session_id: Uuid,
        kind: LatencyKind,
        elapsed_ms: i32,
        success: bool,
        detail: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO latency_samples (session_id, kind, elapsed_ms, success, detail)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(session_id)
        .bind(kind.as_str())
        .bind(elapsed_ms)
        .bind(success)
        .bind(detail)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
