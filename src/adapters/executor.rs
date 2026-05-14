use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::adapters::db::live_send_attempts::ClaimResult;
use crate::adapters::http::SignatureStatus;
use crate::adapters::tip::TipPolicy;
use crate::adapters::tx::sign_versioned_tx_b64;
use crate::adapters::tx_builder::{
    build_close_ata_tx, build_tip_and_dontfront_tx, derive_ata, signature_b58_from_b64_tx,
};
use crate::adapters::{Db, Http, Jito, Jupiter, Signer};
use crate::domain::{LatencyKind, Pubkey, Quote};
use solana_sdk::hash::Hash;
use std::str::FromStr;

const LIVE_SLIPPAGE_BPS: u32 = 200;
const CONFIRM_POLL_INTERVAL: Duration = Duration::from_millis(400);
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);
const BLOCKHASH_MIN_REMAINING: u64 = 30;
const FEE_BUFFER_LAMPORTS: u64 = 10_000_000;
const MIN_TIP_LAMPORTS: u64 = 10_000;
const FALLBACK_TIP_LAMPORTS: u64 = 100_000;

pub struct LiveExecutor {
    jupiter: Jupiter,
    jito: Jito,
    signer: Signer,
    http: std::sync::Arc<Http>,
    tip_policy: TipPolicy,
}

#[derive(Clone, Debug)]
pub struct LiveOutcome {
    pub signature: String,
    pub bundle_id: String,
    pub endpoint: String,
    pub tip_lamports: u64,
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
            tip_policy: TipPolicy::new(),
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

        let signed = sign_versioned_tx_b64(&built.tx_b64, &self.signer)?;

        let blockhash_str = self.http.get_latest_blockhash().await?;
        let blockhash = Hash::from_str(&blockhash_str)
            .map_err(|e| anyhow::anyhow!("parse blockhash: {e}"))?;
        let tip_lamports = self.compute_tip().await;
        let tip_account = self.tip_policy.random_tip_account();
        let tip_tx_b64 =
            build_tip_and_dontfront_tx(&self.signer, &tip_account, tip_lamports, blockhash)?;
        let tip_sig_b58 = signature_b58_from_b64_tx(&tip_tx_b64)?;

        match db
            .live_send_attempts()
            .try_claim(&signed.signature_b58, session_id)
            .await?
        {
            ClaimResult::Fresh => {}
            ClaimResult::Duplicate => {
                anyhow::bail!(
                    "duplicate signature {} — already attempted (idempotent abort)",
                    signed.signature_b58
                );
            }
        }
        db.live_send_attempts()
            .try_claim(&tip_sig_b58, session_id)
            .await
            .ok();

        let started = Instant::now();
        let send_result = self
            .jito
            .send_bundle(&[signed.tx_b64.clone(), tip_tx_b64])
            .await;
        let send_latency_ms = started.elapsed().as_millis() as i32;
        let bundle = match send_result {
            Ok(b) => b,
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
                db.live_send_attempts()
                    .complete(&signed.signature_b58, None, None, false, Some(&e.to_string()))
                    .await
                    .ok();
                anyhow::bail!("jito bundle send: {e}");
            }
        };
        db.latency_samples()
            .insert(
                session_id,
                LatencyKind::JitoSend,
                send_latency_ms,
                true,
                Some(&format!("{}#{}", bundle.endpoint, bundle.bundle_id)),
            )
            .await
            .ok();

        let confirm_latency_ms = self
            .wait_for_confirmation(db, session_id, &signed.signature_b58)
            .await;
        db.live_send_attempts()
            .complete(
                &signed.signature_b58,
                Some(&bundle.bundle_id),
                Some(&bundle.endpoint),
                confirm_latency_ms.is_some(),
                None,
            )
            .await
            .ok();

        Ok(LiveOutcome {
            signature: signed.signature_b58,
            bundle_id: bundle.bundle_id,
            endpoint: bundle.endpoint,
            tip_lamports,
            quote,
            quote_latency_ms,
            swap_latency_ms,
            send_latency_ms,
            confirm_latency_ms,
        })
    }

    async fn compute_tip(&self) -> u64 {
        match self.tip_policy.current_floor().await {
            Ok(floor) => floor.p75_lamports.max(MIN_TIP_LAMPORTS),
            Err(_) => FALLBACK_TIP_LAMPORTS,
        }
    }

    pub async fn submit_close_ata(&self, db: &Db, session_id: Uuid, mint: &Pubkey) -> Result<()> {
        let mint_sdk = solana_sdk::pubkey::Pubkey::from(*mint.as_bytes());
        let ata = derive_ata(&self.signer.sdk_pubkey(), &mint_sdk);
        let blockhash_str = self.http.get_latest_blockhash().await?;
        let blockhash = Hash::from_str(&blockhash_str)
            .map_err(|e| anyhow::anyhow!("parse blockhash: {e}"))?;
        let close_tx_b64 = build_close_ata_tx(&self.signer, &ata, blockhash)?;
        let close_sig_b58 = signature_b58_from_b64_tx(&close_tx_b64)?;

        let tip_lamports = self.compute_tip().await;
        let tip_account = self.tip_policy.random_tip_account();
        let tip_tx_b64 =
            build_tip_and_dontfront_tx(&self.signer, &tip_account, tip_lamports, blockhash)?;
        let tip_sig_b58 = signature_b58_from_b64_tx(&tip_tx_b64)?;

        db.live_send_attempts()
            .try_claim(&close_sig_b58, session_id)
            .await
            .ok();
        db.live_send_attempts()
            .try_claim(&tip_sig_b58, session_id)
            .await
            .ok();

        let bundle = self
            .jito
            .send_bundle(&[close_tx_b64, tip_tx_b64])
            .await?;
        db.live_send_attempts()
            .complete(
                &close_sig_b58,
                Some(&bundle.bundle_id),
                Some(&bundle.endpoint),
                true,
                None,
            )
            .await
            .ok();
        Ok(())
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
