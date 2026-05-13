use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::ObservedTrade;

pub struct ObservedTrades<'a> {
    pool: &'a PgPool,
}

impl<'a> ObservedTrades<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, session_id: Uuid, trade: &ObservedTrade) -> Result<i64> {
        let route: Vec<String> = trade.route.iter().map(|d| d.as_str().to_string()).collect();
        let mint_bytes: Option<Vec<u8>> = trade.mint.as_ref().map(|m| m.as_bytes().to_vec());
        let signature_bytes: Vec<u8> = trade.signature.as_bytes().to_vec();
        let target_bytes: Vec<u8> = trade.target.as_bytes().to_vec();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO observed_trades (
                session_id, slot, signature, target, side, mint,
                sol_delta_lamports, token_delta, route,
                jupiter, pump_swap, jito_marker,
                priority_fee_lamports, compute_unit_limit, detection_delay_ms
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            RETURNING id",
        )
        .bind(session_id)
        .bind(trade.slot.as_u64() as i64)
        .bind(&signature_bytes)
        .bind(&target_bytes)
        .bind(trade.side.as_str())
        .bind(&mint_bytes)
        .bind(trade.sol_delta_lamports)
        .bind(i64::try_from(trade.token_delta).context("token_delta exceeds BIGINT")?)
        .bind(&route)
        .bind(trade.jupiter)
        .bind(trade.pump_swap)
        .bind(trade.jito_marker)
        .bind(i64::try_from(trade.priority_fee_lamports).context("priority_fee_lamports exceeds BIGINT")?)
        .bind(trade.compute_unit_limit.map(|c| c as i32))
        .bind(trade.detection_delay_ms)
        .fetch_one(self.pool)
        .await?;
        Ok(id)
    }
}
