use anyhow::Result;
use sqlx::PgPool;

use crate::domain::Pubkey;

pub struct MintBlocklist<'a> {
    pool: &'a PgPool,
}

impl<'a> MintBlocklist<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn is_blocked(&self, mint: Pubkey, threshold: u32, ttl_secs: i64) -> Result<bool> {
        let blocked: Option<bool> = sqlx::query_scalar(
            "SELECT loss_count >= $2 AND last_loss_at > NOW() - make_interval(secs => $3)
             FROM mint_blocklist WHERE mint = $1",
        )
        .bind(mint.as_bytes().to_vec())
        .bind(i32::try_from(threshold).unwrap_or(i32::MAX))
        .bind(ttl_secs as f64)
        .fetch_optional(self.pool)
        .await?;
        Ok(blocked.unwrap_or(false))
    }

    pub async fn record_loss(&self, mint: Pubkey) -> Result<i32> {
        let count: i32 = sqlx::query_scalar(
            "INSERT INTO mint_blocklist (mint, loss_count, first_loss_at, last_loss_at)
             VALUES ($1, 1, NOW(), NOW())
             ON CONFLICT (mint) DO UPDATE
                 SET loss_count = mint_blocklist.loss_count + 1,
                     last_loss_at = NOW(),
                     first_loss_at = COALESCE(mint_blocklist.first_loss_at, NOW())
             RETURNING loss_count",
        )
        .bind(mint.as_bytes().to_vec())
        .fetch_one(self.pool)
        .await?;
        Ok(count)
    }

    pub async fn clear(&self, mint: Pubkey) -> Result<()> {
        sqlx::query("DELETE FROM mint_blocklist WHERE mint = $1")
            .bind(mint.as_bytes().to_vec())
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
