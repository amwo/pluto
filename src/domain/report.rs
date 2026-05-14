use std::fmt;

#[derive(Clone, Debug)]
pub struct DailyReport {
    pub day: String,
    pub total_trades: i64,
    pub buys: i64,
    pub sells: i64,
    pub unknowns: i64,
    pub copies: i64,
    pub skips: i64,
    pub skip_breakdown: Vec<(String, i64)>,
    pub buy_sol_lamports: i64,
    pub sell_sol_lamports: i64,
    pub jupiter_count: i64,
    pub pump_swap_count: i64,
    pub detection_delay_p50_ms: Option<i32>,
    pub detection_delay_p95_ms: Option<i32>,
    pub latency_breakdown: Vec<LatencyStats>,
    pub realized_pnl_lamports: i64,
    pub positions_closed: i64,
    pub positions_wins: i64,
    pub positions_losses: i64,
}

#[derive(Clone, Debug)]
pub struct LatencyStats {
    pub kind: String,
    pub samples: i64,
    pub success_count: i64,
    pub p50_ms: Option<i32>,
    pub p95_ms: Option<i32>,
}

impl DailyReport {
    pub fn net_sol(&self) -> f64 {
        (self.sell_sol_lamports - self.buy_sol_lamports) as f64 / 1e9
    }
}

impl fmt::Display for DailyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "=== Daily Report {} ===", self.day)?;
        writeln!(
            f,
            "Trades:   {} total (buy={} sell={} unknown={})",
            self.total_trades, self.buys, self.sells, self.unknowns
        )?;
        writeln!(
            f,
            "SOL flow: -{:.3} in / +{:.3} out / {:+.3} net",
            self.buy_sol_lamports as f64 / 1e9,
            self.sell_sol_lamports as f64 / 1e9,
            self.net_sol(),
        )?;
        let pct = |n: i64| -> f64 {
            if self.total_trades == 0 {
                0.0
            } else {
                100.0 * n as f64 / self.total_trades as f64
            }
        };
        writeln!(
            f,
            "Routes:   Jupiter={} ({:.0}%)  PumpSwap={} ({:.0}%)",
            self.jupiter_count,
            pct(self.jupiter_count),
            self.pump_swap_count,
            pct(self.pump_swap_count),
        )?;
        writeln!(f, "Decisions:")?;
        writeln!(f, "  copy: {}", self.copies)?;
        writeln!(f, "  skip: {}", self.skips)?;
        for (reason, count) in &self.skip_breakdown {
            writeln!(f, "    {reason}: {count}")?;
        }
        writeln!(
            f,
            "Detection delay: p50={} p95={}",
            self.detection_delay_p50_ms
                .map(|v| format!("{v}ms"))
                .unwrap_or_else(|| "-".to_string()),
            self.detection_delay_p95_ms
                .map(|v| format!("{v}ms"))
                .unwrap_or_else(|| "-".to_string()),
        )?;
        writeln!(f, "Latency:")?;
        for s in &self.latency_breakdown {
            let success_rate = if s.samples == 0 {
                0.0
            } else {
                100.0 * s.success_count as f64 / s.samples as f64
            };
            writeln!(
                f,
                "  {}: n={} ok={:.0}% p50={} p95={}",
                s.kind,
                s.samples,
                success_rate,
                s.p50_ms
                    .map(|v| format!("{v}ms"))
                    .unwrap_or_else(|| "-".to_string()),
                s.p95_ms
                    .map(|v| format!("{v}ms"))
                    .unwrap_or_else(|| "-".to_string()),
            )?;
        }
        writeln!(
            f,
            "Positions: closed={} (win={} loss={}) realized PnL={:+.4} SOL",
            self.positions_closed,
            self.positions_wins,
            self.positions_losses,
            self.realized_pnl_lamports as f64 / 1e9,
        )?;
        Ok(())
    }
}
