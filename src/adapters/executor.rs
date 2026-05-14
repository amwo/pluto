use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::adapters::http::SignatureStatus;
use crate::adapters::tx::sign_versioned_tx_b64;
use crate::adapters::{Db, Http, Jito, Jupiter, Signer};
use crate::domain::{LatencyKind, Pubkey, Quote};

const LIVE_SLIPPAGE_BPS: u32 = 200;
const CONFIRM_POLL_INTERVAL: Duration = Duration::from_millis(400);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);
const BLOCKHASH_MIN_REMAINING: u64 = 30;
const FEE_BUFFER_LAMPORTS: u64 = 10_000_000;

pub struct LiveExecutor {
    jupiter: Jupiter,
    jito: Jito,
    signer: Signer,
    http: std::sync::Arc<Http>,
}

#[derive(Clone, Debug)]
pub struct LiveOutcome {
    pub signature: String,
    pub endpoint: String,
    pub quote: Quote,
    pub quote_latency_ms: i32,
    pub swap_latency_ms: i32,
    pub send_latency_ms: i32,
    pub confirm_latency_ms: Option<i32>,
}

impl LiveExecutor {
    pub fn new(
        jupiter: Jupiter,
        jito: Jito,
        signer: Signer,
        http: std::sync::Arc<Http>,
    ) -> Self {
        Self {
            jupiter,
            jito,
            signer,
            http,
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
        self.preflight_balance(&taker, amount, input_mint).await?;

        let started = Instant::now();
        let order_result = self
            .jupiter
            .quote_v1(input_mint, output_mint, amount, LIVE_SLIPPAGE_BPS)
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
        let built_result = self.jupiter.build_swap_v1(&order, &taker).await;
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

        if let Some(last_valid) = built.last_valid_block_height {
            match self.http.get_block_height().await {
                Ok(current) => {
                    if last_valid <= current.saturating_add(BLOCKHASH_MIN_REMAINING) {
                        anyhow::bail!(
                            "blockhash too old: last_valid={last_valid} current={current} (min remaining {BLOCKHASH_MIN_REMAINING})"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "get_block_height failed; sending without TTL gate");
                }
            }
        }

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

        let confirm_latency_ms = self
            .wait_for_confirmation(db, session_id, &outcome.signature)
            .await;

        Ok(LiveOutcome {
            signature: outcome.signature,
            endpoint: outcome.endpoint,
            quote,
            quote_latency_ms,
            swap_latency_ms,
            send_latency_ms,
            confirm_latency_ms,
        })
    }

    pub fn taker(&self) -> Pubkey {
        self.signer.pubkey()
    }

    async fn preflight_balance(
        &self,
        taker: &Pubkey,
        amount: u64,
        input_mint: &Pubkey,
    ) -> Result<()> {
        let balance = self.http.get_balance(taker).await?;
        let wsol = Pubkey::from_base58("So11111111111111111111111111111111111111112")
            .expect("wsol mint");
        let required = if *input_mint == wsol {
            amount.saturating_add(FEE_BUFFER_LAMPORTS)
        } else {
            FEE_BUFFER_LAMPORTS
        };
        if balance < required {
            anyhow::bail!(
                "wallet balance {balance} lamports below required {required} (amount={amount}, fee_buffer={FEE_BUFFER_LAMPORTS})"
            );
        }
        Ok(())
    }

    async fn wait_for_confirmation(
        &self,
        db: &Db,
        session_id: Uuid,
        signature: &str,
    ) -> Option<i32> {
        let started = Instant::now();
        loop {
            if started.elapsed() >= CONFIRM_TIMEOUT {
                let elapsed_ms = started.elapsed().as_millis() as i32;
                db.latency_samples()
                    .insert(
                        session_id,
                        LatencyKind::JitoConfirm,
                        elapsed_ms,
                        false,
                        Some("confirm timeout"),
                    )
                    .await
                    .ok();
                return None;
            }
            match self.http.get_signature_status(signature).await {
                Ok(SignatureStatus::Failed(err)) => {
                    let elapsed_ms = started.elapsed().as_millis() as i32;
                    db.latency_samples()
                        .insert(
                            session_id,
                            LatencyKind::JitoConfirm,
                            elapsed_ms,
                            false,
                            Some(&err),
                        )
                        .await
                        .ok();
                    return Some(elapsed_ms);
                }
                Ok(status) if status.is_landed() => {
                    let elapsed_ms = started.elapsed().as_millis() as i32;
                    db.latency_samples()
                        .insert(
                            session_id,
                            LatencyKind::JitoConfirm,
                            elapsed_ms,
                            true,
                            None,
                        )
                        .await
                        .ok();
                    return Some(elapsed_ms);
                }
                _ => {}
            }
            tokio::time::sleep(CONFIRM_POLL_INTERVAL).await;
        }
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
