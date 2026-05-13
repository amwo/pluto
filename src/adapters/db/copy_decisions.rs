use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::CopyDecision;

pub struct CopyDecisions<'a> {
    pool: &'a PgPool,
}

impl<'a> CopyDecisions<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(
        &self,
        session_id: Uuid,
        observed_trade_id: i64,
        decision: &CopyDecision,
    ) -> Result<i64> {
        let (size, reason) = match decision {
            CopyDecision::Copy { size_lamports } => (Some(*size_lamports as i64), None),
            CopyDecision::Skip(r) => (None, Some(r.as_str())),
        };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO copy_decisions (session_id, observed_trade_id, action, size_lamports, skip_reason)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id",
        )
        .bind(session_id)
        .bind(observed_trade_id)
        .bind(decision.action())
        .bind(size)
        .bind(reason)
        .fetch_one(self.pool)
        .await?;
        Ok(id)
    }
}
