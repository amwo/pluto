use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::Session;

pub struct Sessions<'a> {
    pool: &'a PgPool,
}

impl<'a> Sessions<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn mark_running_as_crashed(&self) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = 'crashed' WHERE status = 'running'")
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert(&self, session: &Session) -> Result<()> {
        sqlx::query("INSERT INTO sessions (id, mode) VALUES ($1, $2)")
            .bind(session.id)
            .bind(session.mode.as_str())
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn record_tx(&self, session_id: Uuid) -> Result<()> {
        sqlx::query("UPDATE sessions SET tx_count = tx_count + 1 WHERE id = $1")
            .bind(session_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn complete(&self, session: &Session) -> Result<()> {
        sqlx::query("UPDATE sessions SET ended_at = NOW(), status = 'completed' WHERE id = $1")
            .bind(session.id)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
