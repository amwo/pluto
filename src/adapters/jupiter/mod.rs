use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::domain::{Pubkey, Quote};

const API_BASE: &str = "https://api.jup.ag";
const MIN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub struct TokenInfo {
    pub symbol: String,
    pub decimals: u8,
}

pub struct Jupiter {
    client: reqwest::Client,
    next_request_at: Mutex<Instant>,
    token_cache: Mutex<HashMap<[u8; 32], Option<TokenInfo>>>,
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
            token_cache: Mutex::new(HashMap::new()),
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
            .get(format!("{API_BASE}/swap/v2/order"))
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

    pub async fn token_info(&self, mint: &Pubkey) -> Option<TokenInfo> {
        let key = *mint.as_bytes();
        if let Some(cached) = self.token_cache.lock().await.get(&key) {
            return cached.clone();
        }
        let fetched = self.fetch_token_info(mint).await;
        self.token_cache.lock().await.insert(key, fetched.clone());
        fetched
    }

    async fn fetch_token_info(&self, mint: &Pubkey) -> Option<TokenInfo> {
        self.throttle().await;
        let response: Value = self
            .client
            .get(format!("{API_BASE}/tokens/v2/search"))
            .query(&[("query", mint.to_string())])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .await
            .ok()?;
        let needle = mint.to_string();
        let arr = response.as_array()?;
        let row = arr.iter().find(|t| t["id"].as_str() == Some(&needle))?;
        let symbol = row["symbol"].as_str()?.to_string();
        let decimals = row["decimals"].as_u64()? as u8;
        Some(TokenInfo { symbol, decimals })
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
