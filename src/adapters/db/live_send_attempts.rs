use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub struct LiveSendAttempts<'a> {
    pool: &'a PgPool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimResult {
    Fresh,
    Duplicate,
}

impl<'a> LiveSendAttempts<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn try_claim(&self, signature: &str, session_id: Uuid) -> Result<ClaimResult> {
        let res = sqlx::query(
            "INSERT INTO live_send_attempts (signature, session_id) VALUES ($1, $2)
             ON CONFLICT (signature) DO NOTHING",
        )
        .bind(signature)
        .bind(session_id)
        .execute(self.pool)
        .await?;
        Ok(if res.rows_affected() == 1 {
            ClaimResult::Fresh
        } else {
            ClaimResult::Duplicate
        })
    }

    pub async fn complete(
        &self,
        signature: &str,
        bundle_id: Option<&str>,
        endpoint: Option<&str>,
        landed: bool,
        confirm_error: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE live_send_attempts
             SET completed_at = NOW(),
                 bundle_id = $2,
                 endpoint = $3,
                 landed = $4,
                 confirm_error = $5
             WHERE signature = $1",
        )
        .bind(signature)
        .bind(bundle_id)
        .bind(endpoint)
        .bind(landed)
        .bind(confirm_error)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
