use anyhow::Result;
use sqlx::PgPool;
use sqlx::Row;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::{ExitReason, Position, PositionStatus, Pubkey};

pub struct Positions<'a> {
    pool: &'a PgPool,
}

impl<'a> Positions<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn open(
        &self,
        session_id: Uuid,
        mint: Pubkey,
        entry_paper_trade_id: i64,
        entry_in_lamports: u64,
        entry_out_amount: u64,
        entry_price: f64,
    ) -> Result<i64> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO positions
                (session_id, mint, entry_paper_trade_id, entry_in_lamports, entry_out_amount, entry_price, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             RETURNING id",
        )
        .bind(session_id)
        .bind(mint.as_bytes().to_vec())
        .bind(entry_paper_trade_id)
        .bind(i64::try_from(entry_in_lamports)?)
        .bind(i64::try_from(entry_out_amount)?)
        .bind(entry_price)
        .bind(PositionStatus::Open.as_str())
        .fetch_one(self.pool)
        .await?;
        Ok(id)
    }

    pub async fn find_open_by_mint(
        &self,
        session_id: Uuid,
        mint: Pubkey,
    ) -> Result<Option<Position>> {
        let row = sqlx::query(
            "SELECT id, session_id, mint, opened_at,
                    entry_paper_trade_id, entry_in_lamports, entry_out_amount, entry_price
             FROM positions
             WHERE session_id = $1 AND mint = $2 AND status = 'open'",
        )
        .bind(session_id)
        .bind(mint.as_bytes().to_vec())
        .fetch_optional(self.pool)
        .await?;
        match row {
            None => Ok(None),
            Some(r) => {
                let mint_bytes: Vec<u8> = r.try_get("mint")?;
                let mint_arr: [u8; 32] = mint_bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("mint length"))?;
                Ok(Some(Position {
                    id: r.try_get("id")?,
                    session_id: r.try_get("session_id")?,
                    mint: Pubkey::from(mint_arr),
                    opened_at: r.try_get::<OffsetDateTime, _>("opened_at")?,
                    entry_paper_trade_id: r.try_get("entry_paper_trade_id")?,
                    entry_in_lamports: u64::try_from(r.try_get::<i64, _>("entry_in_lamports")?)?,
                    entry_out_amount: u64::try_from(r.try_get::<i64, _>("entry_out_amount")?)?,
                    entry_price: r.try_get("entry_price")?,
                }))
            }
        }
    }

    pub async fn close(
        &self,
        position_id: i64,
        exit_paper_trade_id: i64,
        exit_reason: ExitReason,
        realized_pnl_lamports: i64,
        realized_pnl_pct: f64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE positions
             SET status = 'closed',
                 closed_at = NOW(),
                 exit_reason = $2,
                 exit_paper_trade_id = $3,
                 realized_pnl_lamports = $4,
                 realized_pnl_pct = $5
             WHERE id = $1",
        )
        .bind(position_id)
        .bind(exit_reason.as_str())
        .bind(exit_paper_trade_id)
        .bind(realized_pnl_lamports)
        .bind(realized_pnl_pct)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
