use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub url: String,
}

pub struct Jito {
    client: reqwest::Client,
    endpoints: Vec<Endpoint>,
}

impl Jito {
    pub fn new(endpoints: Vec<Endpoint>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client");
        Self { client, endpoints }
    }

    pub async fn send_bundle_only(&self, signed_tx_b64: &str) -> Result<SendOutcome> {
        let mut last_err: Option<anyhow::Error> = None;
        for endpoint in &self.endpoints {
            let url = format!("{}/api/v1/transactions?bundleOnly=true", endpoint.url);
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "sendTransaction",
                "params": [
                    signed_tx_b64,
                    { "encoding": "base64", "skipPreflight": true },
                ],
            });
            let res = self.client.post(&url).json(&body).send().await;
            match res {
                Ok(r) if r.status().is_success() => {
                    let v: serde_json::Value = r.json().await.context("jito json parse")?;
                    if let Some(sig) = v["result"].as_str() {
                        return Ok(SendOutcome {
                            signature: sig.to_string(),
                            endpoint: endpoint.url.clone(),
                        });
                    }
                    if let Some(err) = v.get("error") {
                        last_err = Some(anyhow::anyhow!("jito error: {err}"));
                    } else {
                        last_err = Some(anyhow::anyhow!("jito no result"));
                    }
                }
                Ok(r) => {
                    last_err = Some(anyhow::anyhow!("jito http {}", r.status()));
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!("jito transport: {e}"));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no jito endpoints configured")))
    }
}

#[derive(Clone, Debug)]
pub struct SendOutcome {
    pub signature: String,
    pub endpoint: String,
}
