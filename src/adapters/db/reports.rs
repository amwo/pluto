use anyhow::Result;
use sqlx::PgPool;

use crate::domain::{DailyReport, LatencyStats};

pub struct Reports<'a> {
    pool: &'a PgPool,
}

impl<'a> Reports<'a> {
    pub(super) fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn daily(&self, day: Option<&str>) -> Result<DailyReport> {
        let day: String = match day {
            Some(d) => d.to_string(),
            None => sqlx::query_scalar("SELECT to_char((NOW() AT TIME ZONE 'UTC')::date, 'YYYY-MM-DD')")
                .fetch_one(self.pool)
                .await?,
        };

        let trade_row: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                COUNT(*),
                COUNT(*) FILTER (WHERE side = 'buy'),
                COUNT(*) FILTER (WHERE side = 'sell'),
                COUNT(*) FILTER (WHERE side = 'unknown'),
                COALESCE(SUM(-sol_delta_lamports) FILTER (WHERE side = 'buy'), 0)::bigint,
                COALESCE(SUM(sol_delta_lamports) FILTER (WHERE side = 'sell'), 0)::bigint,
                COUNT(*) FILTER (WHERE jupiter),
                COUNT(*) FILTER (WHERE pump_swap)
             FROM observed_trades
             WHERE (received_at AT TIME ZONE 'UTC')::date = $1::date",
        )
        .bind(&day)
        .fetch_one(self.pool)
        .await?;

        let decision_row: (i64, i64) = sqlx::query_as(
            "SELECT
                COUNT(*) FILTER (WHERE action = 'copy'),
                COUNT(*) FILTER (WHERE action = 'skip')
             FROM copy_decisions cd
             JOIN observed_trades ot ON cd.observed_trade_id = ot.id
             WHERE (ot.received_at AT TIME ZONE 'UTC')::date = $1::date",
        )
        .bind(&day)
        .fetch_one(self.pool)
        .await?;

        let skip_breakdown: Vec<(String, i64)> = sqlx::query_as(
            "SELECT cd.skip_reason, COUNT(*)::bigint
             FROM copy_decisions cd
             JOIN observed_trades ot ON cd.observed_trade_id = ot.id
             WHERE (ot.received_at AT TIME ZONE 'UTC')::date = $1::date
               AND cd.action = 'skip'
               AND cd.skip_reason IS NOT NULL
             GROUP BY cd.skip_reason
             ORDER BY COUNT(*) DESC",
        )
        .bind(&day)
        .fetch_all(self.pool)
        .await?;

        let delay_row: (Option<i32>, Option<i32>) = sqlx::query_as(
            "SELECT
                percentile_disc(0.5) WITHIN GROUP (ORDER BY detection_delay_ms),
                percentile_disc(0.95) WITHIN GROUP (ORDER BY detection_delay_ms)
             FROM observed_trades
             WHERE (received_at AT TIME ZONE 'UTC')::date = $1::date
               AND detection_delay_ms IS NOT NULL",
        )
        .bind(&day)
        .fetch_one(self.pool)
        .await?;

        type LatencyRow = (String, i64, i64, Option<i32>, Option<i32>);
        let latency_rows: Vec<LatencyRow> = sqlx::query_as(
            "SELECT
                kind,
                COUNT(*)::bigint,
                COUNT(*) FILTER (WHERE success)::bigint,
                percentile_disc(0.5) WITHIN GROUP (ORDER BY elapsed_ms) FILTER (WHERE success),
                percentile_disc(0.95) WITHIN GROUP (ORDER BY elapsed_ms) FILTER (WHERE success)
             FROM latency_samples
             WHERE (sampled_at AT TIME ZONE 'UTC')::date = $1::date
             GROUP BY kind
             ORDER BY kind",
        )
        .bind(&day)
        .fetch_all(self.pool)
        .await?;

        let latency_breakdown = latency_rows
            .into_iter()
            .map(|(kind, samples, success_count, p50, p95)| LatencyStats {
                kind,
                samples,
                success_count,
                p50_ms: p50,
                p95_ms: p95,
            })
            .collect();

        let positions_row: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
                COUNT(*)::bigint,
                COUNT(*) FILTER (WHERE realized_pnl_lamports > 0)::bigint,
                COUNT(*) FILTER (WHERE realized_pnl_lamports < 0)::bigint,
                COALESCE(SUM(realized_pnl_lamports), 0)::bigint
             FROM positions
             WHERE status = 'closed'
               AND (closed_at AT TIME ZONE 'UTC')::date = $1::date",
        )
        .bind(&day)
        .fetch_one(self.pool)
        .await?;

        Ok(DailyReport {
            day,
            total_trades: trade_row.0,
            buys: trade_row.1,
            sells: trade_row.2,
            unknowns: trade_row.3,
            buy_sol_lamports: trade_row.4,
            sell_sol_lamports: trade_row.5,
            jupiter_count: trade_row.6,
            pump_swap_count: trade_row.7,
            copies: decision_row.0,
            skips: decision_row.1,
            skip_breakdown,
            detection_delay_p50_ms: delay_row.0,
            detection_delay_p95_ms: delay_row.1,
            latency_breakdown,
            realized_pnl_lamports: positions_row.3,
            positions_closed: positions_row.0,
            positions_wins: positions_row.1,
            positions_losses: positions_row.2,
        })
    }
}
