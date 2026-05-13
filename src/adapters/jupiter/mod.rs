use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::domain::{Pubkey, Quote};

const BASE_URL: &str = "https://api.jup.ag/swap/v2";
const MIN_INTERVAL: Duration = Duration::from_secs(2);

pub struct Jupiter {
    client: reqwest::Client,
    next_request_at: Mutex<Instant>,
}

impl Default for Jupiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Jupiter {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            next_request_at: Mutex::new(Instant::now()),
        }
    }

    pub async fn quote(
        &self,
        input_mint: &Pubkey,
        output_mint: &Pubkey,
        amount: u64,
        slippage_bps: u32,
    ) -> Result<Quote> {
        self.throttle().await;

        let response: Value = self
            .client
            .get(format!("{BASE_URL}/order"))
            .query(&[
                ("inputMint", input_mint.to_string()),
                ("outputMint", output_mint.to_string()),
                ("amount", amount.to_string()),
                ("slippageBps", slippage_bps.to_string()),
                ("swapMode", "ExactIn".to_string()),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(Quote {
            input_mint: *input_mint,
            output_mint: *output_mint,
            in_amount: parse_u64(&response, "inAmount")?,
            out_amount: parse_u64(&response, "outAmount")?,
            other_amount_threshold: parse_u64(&response, "otherAmountThreshold")?,
            price_impact_bps: parse_price_impact_bps(&response)?,
            slippage_bps,
            route_labels: parse_route_labels(&response),
        })
    }

    async fn throttle(&self) {
        let mut next = self.next_request_at.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep(*next - now).await;
        }
        *next = Instant::now() + MIN_INTERVAL;
    }
}

fn parse_u64(v: &Value, field: &str) -> Result<u64> {
    v[field]
        .as_str()
        .with_context(|| format!("missing {field}"))?
        .parse::<u64>()
        .with_context(|| format!("invalid {field}"))
}

fn parse_price_impact_bps(v: &Value) -> Result<i32> {
    let s = v["priceImpactPct"]
        .as_str()
        .context("missing priceImpactPct")?;
    let pct: f64 = s.parse().context("invalid priceImpactPct")?;
    Ok((pct * 10_000.0).round() as i32)
}

fn parse_route_labels(v: &Value) -> Vec<String> {
    v["routePlan"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|step| step["swapInfo"]["label"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
