use std::time::Duration;

use anyhow::{Context, Result};
use rand::seq::SliceRandom;
use serde_json::Value;
use solana_sdk::pubkey::Pubkey as SdkPubkey;
use std::str::FromStr;

const TIP_FLOOR_URL: &str = "https://bundles.jito.wtf/api/v1/bundles/tip_floor";

const JITO_TIP_ACCOUNTS: &[&str] = &[
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pivKeVBBjNS1c8xiKhSj",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

pub const JITODONTFRONT_PROGRAM: &str = "jitodontfront111111111111111111111111111123";

pub struct TipPolicy {
    client: reqwest::Client,
}

#[derive(Clone, Debug)]
pub struct TipFloor {
    pub p25_lamports: u64,
    pub p50_lamports: u64,
    pub p75_lamports: u64,
    pub p95_lamports: u64,
}

impl Default for TipPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl TipPolicy {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .expect("reqwest client"),
        }
    }

    pub fn random_tip_account(&self) -> SdkPubkey {
        let mut rng = rand::thread_rng();
        let s = JITO_TIP_ACCOUNTS
            .choose(&mut rng)
            .copied()
            .unwrap_or(JITO_TIP_ACCOUNTS[0]);
        SdkPubkey::from_str(s).expect("hardcoded tip account")
    }

    pub async fn current_floor(&self) -> Result<TipFloor> {
        let response: Value = self
            .client
            .get(TIP_FLOOR_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let arr = response.as_array().context("tip_floor: expected array")?;
        let row = arr.first().context("tip_floor: empty array")?;
        Ok(TipFloor {
            p25_lamports: sol_field_to_lamports(row, "landed_tips_25th_percentile"),
            p50_lamports: sol_field_to_lamports(row, "landed_tips_50th_percentile"),
            p75_lamports: sol_field_to_lamports(row, "landed_tips_75th_percentile"),
            p95_lamports: sol_field_to_lamports(row, "landed_tips_95th_percentile"),
        })
    }
}

fn sol_field_to_lamports(v: &Value, field: &str) -> u64 {
    let sol = v[field].as_f64().unwrap_or(0.0);
    (sol * 1e9) as u64
}

pub fn jitodontfront_program() -> SdkPubkey {
    SdkPubkey::from_str(JITODONTFRONT_PROGRAM).expect("hardcoded jitodontfront program")
}
