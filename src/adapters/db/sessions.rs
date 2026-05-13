use anyhow::Result;
use uuid::Uuid;

use super::Pool;
use crate::domain::Session;

pub async fn mark_running_as_crashed(pool: &Pool) -> Result<()> {
    sqlx::query("UPDATE sessions SET status = 'crashed' WHERE status = 'running'")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert(pool: &Pool, session: &Session) -> Result<()> {
    sqlx::query("INSERT INTO sessions (id, mode) VALUES ($1, $2)")
        .bind(session.id)
        .bind(session.mode.as_str())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn record_tx(pool: &Pool, session_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE sessions SET tx_count = tx_count + 1 WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn complete(pool: &Pool, session: &Session) -> Result<()> {
    sqlx::query("UPDATE sessions SET ended_at = NOW(), status = 'completed' WHERE id = $1")
        .bind(session.id)
        .execute(pool)
        .await?;
    Ok(())
}
