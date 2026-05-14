use std::time::Instant;

use anyhow::Result;
use uuid::Uuid;

use crate::adapters::tx::sign_versioned_tx_b64;
use crate::adapters::{Db, Jito, Jupiter, Signer};
use crate::domain::{LatencyKind, Pubkey, Quote};

const LIVE_SLIPPAGE_BPS: u32 = 200;

pub struct LiveExecutor {
    jupiter: Jupiter,
    jito: Jito,
    signer: Signer,
}

#[derive(Clone, Debug)]
pub struct LiveOutcome {
    pub signature: String,
    pub endpoint: String,
    pub quote: Quote,
    pub quote_latency_ms: i32,
    pub swap_latency_ms: i32,
    pub send_latency_ms: i32,
}

impl LiveExecutor {
    pub fn new(jupiter: Jupiter, jito: Jito, signer: Signer) -> Self {
        Self {
            jupiter,
            jito,
            signer,
        }
    }

    pub async fn execute_swap(
        &self,
        db: &Db,
        session_id: Uuid,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        amount: u64,
    ) -> Result<LiveOutcome> {
        let taker = self.signer.pubkey();

        let started = Instant::now();
        let order_result = self
            .jupiter
            .quote_raw(input_mint, output_mint, amount, LIVE_SLIPPAGE_BPS, &taker)
            .await;
        let quote_latency_ms = started.elapsed().as_millis() as i32;
        let order = match &order_result {
            Ok(o) => o.clone(),
            Err(e) => {
                let detail = e.to_string();
                db.latency_samples()
                    .insert(
                        session_id,
                        LatencyKind::JupiterQuote,
                        quote_latency_ms,
                        false,
                        Some(&detail),
                    )
                    .await
                    .ok();
                anyhow::bail!("jupiter quote: {detail}");
            }
        };
        db.latency_samples()
            .insert(
                session_id,
                LatencyKind::JupiterQuote,
                quote_latency_ms,
                true,
                None,
            )
            .await
            .ok();

        let quote = quote_from_order(input_mint, output_mint, &order)?;

        let started = Instant::now();
        let built_result = self.jupiter.build_swap(&order).await;
        let swap_latency_ms = started.elapsed().as_millis() as i32;
        let built = match built_result {
            Ok(b) => b,
            Err(e) => {
                db.latency_samples()
                    .insert(
                        session_id,
                        LatencyKind::JupiterSwap,
                        swap_latency_ms,
                        false,
                        Some(&e.to_string()),
                    )
                    .await
                    .ok();
                anyhow::bail!("jupiter swap build: {e}");
            }
        };
        db.latency_samples()
            .insert(
                session_id,
                LatencyKind::JupiterSwap,
                swap_latency_ms,
                true,
                None,
            )
            .await
            .ok();

        let signed_b64 = sign_versioned_tx_b64(&built.tx_b64, &self.signer)?;

        let started = Instant::now();
        let send_result = self.jito.send_bundle_only(&signed_b64).await;
        let send_latency_ms = started.elapsed().as_millis() as i32;
        let outcome = match send_result {
            Ok(o) => o,
            Err(e) => {
                db.latency_samples()
                    .insert(
                        session_id,
                        LatencyKind::JitoSend,
                        send_latency_ms,
                        false,
                        Some(&e.to_string()),
                    )
                    .await
                    .ok();
                anyhow::bail!("jito send: {e}");
            }
        };
        db.latency_samples()
            .insert(
                session_id,
                LatencyKind::JitoSend,
                send_latency_ms,
                true,
                Some(&outcome.endpoint),
            )
            .await
            .ok();

        Ok(LiveOutcome {
            signature: outcome.signature,
            endpoint: outcome.endpoint,
            quote,
            quote_latency_ms,
            swap_latency_ms,
            send_latency_ms,
        })
    }

    pub fn taker(&self) -> Pubkey {
        self.signer.pubkey()
    }
}

fn quote_from_order(
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    order: &crate::adapters::jupiter::SwapResponse,
) -> Result<Quote> {
    let raw = &order.raw_quote;
    let in_amount = parse_u64(raw, "inAmount")?;
    let out_amount = parse_u64(raw, "outAmount")?;
    let other_amount_threshold = parse_u64(raw, "otherAmountThreshold")?;
    let price_impact_bps = parse_price_impact_bps(raw)?;
    Ok(Quote {
        input_mint: *input_mint,
        output_mint: *output_mint,
        in_amount,
        out_amount,
        other_amount_threshold,
        price_impact_bps,
        slippage_bps: LIVE_SLIPPAGE_BPS,
        route_labels: parse_route_labels(raw),
    })
}

fn parse_u64(v: &serde_json::Value, field: &str) -> Result<u64> {
    v[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing {field}"))?
        .parse::<u64>()
        .map_err(|e| anyhow::anyhow!("invalid {field}: {e}"))
}

fn parse_price_impact_bps(v: &serde_json::Value) -> Result<i32> {
    let s = v["priceImpactPct"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing priceImpactPct"))?;
    let pct: f64 = s.parse().map_err(|e| anyhow::anyhow!("invalid priceImpactPct: {e}"))?;
    Ok((pct * 10_000.0).round() as i32)
}

fn parse_route_labels(v: &serde_json::Value) -> Vec<String> {
    v["routePlan"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|step| step["swapInfo"]["label"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
