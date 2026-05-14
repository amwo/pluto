use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::{Pubkey, Quote};

pub struct DryTrades<'a> {
    pool: &'a PgPool,
}

#[derive(Clone, Debug)]
pub struct DryTradeRecord<'a> {
    pub copy_decision_id: Option<i64>,
    pub input_mint: &'a Pubkey,
    pub output_mint: &'a Pubkey,
    pub in_amount: u64,
    pub slippage_bps: u32,
    pub quote: Option<&'a Quote>,
    pub quote_latency_ms: i32,
    pub error: Option<&'a str>,
}

impl<'a> DryTrades<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert(&self, session_id: Uuid, record: DryTradeRecord<'_>) -> Result<i64> {
        let input_mint = record.input_mint.as_bytes().to_vec();
        let output_mint = record.output_mint.as_bytes().to_vec();
        let in_amount: i64 = i64::try_from(record.in_amount).context("in_amount exceeds BIGINT")?;
        let out_amount: Option<i64> = record
            .quote
            .map(|q| i64::try_from(q.out_amount).context("out_amount exceeds BIGINT"))
            .transpose()?;
        let other_amount_threshold: Option<i64> = record
            .quote
            .map(|q| {
                i64::try_from(q.other_amount_threshold)
                    .context("other_amount_threshold exceeds BIGINT")
            })
            .transpose()?;
        let price_impact_bps: Option<i32> = record.quote.map(|q| q.price_impact_bps);
        let route_labels: Option<Vec<String>> = record.quote.map(|q| q.route_labels.clone());

        let id: i64 = sqlx::query_scalar(
            "INSERT INTO dry_trades (
                session_id, copy_decision_id, input_mint, output_mint,
                in_amount, out_amount, other_amount_threshold,
                price_impact_bps, slippage_bps, route_labels,
                quote_latency_ms, error
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id",
        )
        .bind(session_id)
        .bind(record.copy_decision_id)
        .bind(&input_mint)
        .bind(&output_mint)
        .bind(in_amount)
        .bind(out_amount)
        .bind(other_amount_threshold)
        .bind(price_impact_bps)
        .bind(record.slippage_bps as i32)
        .bind(&route_labels)
        .bind(record.quote_latency_ms)
        .bind(record.error)
        .fetch_one(self.pool)
        .await?;
        Ok(id)
    }
}
