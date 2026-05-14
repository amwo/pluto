use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::FutureExt;
use serde_json::json;

const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(5);
const RACE_STAGGER: Duration = Duration::from_millis(300);

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
            .timeout(ENDPOINT_TIMEOUT)
            .build()
            .expect("reqwest client");
        Self { client, endpoints }
    }

    pub async fn send_bundle_only(&self, signed_tx_b64: &str) -> Result<SendOutcome> {
        if self.endpoints.is_empty() {
            anyhow::bail!("no jito endpoints configured");
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                signed_tx_b64,
                { "encoding": "base64", "skipPreflight": true },
            ],
        });

        let mut futures = futures_util::stream::FuturesUnordered::new();
        for (idx, endpoint) in self.endpoints.iter().enumerate() {
            let client = self.client.clone();
            let url = format!("{}/api/v1/transactions?bundleOnly=true", endpoint.url);
            let endpoint_url = endpoint.url.clone();
            let body = body.clone();
            let stagger = RACE_STAGGER * idx as u32;
            futures.push(
                async move {
                    if !stagger.is_zero() {
                        tokio::time::sleep(stagger).await;
                    }
                    try_send(client, url, body, endpoint_url).await
                }
                .boxed(),
            );
        }

        use futures_util::stream::StreamExt;
        let mut last_err: Option<anyhow::Error> = None;
        while let Some(res) = futures.next().await {
            match res {
                Ok(outcome) => return Ok(outcome),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no jito endpoints succeeded")))
    }
}

async fn try_send(
    client: reqwest::Client,
    url: String,
    body: serde_json::Value,
    endpoint_url: String,
) -> Result<SendOutcome> {
    let res = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("jito transport {endpoint_url}"))?;
    if !res.status().is_success() {
        anyhow::bail!("jito http {}", res.status());
    }
    let bundle_id = res
        .headers()
        .get("x-bundle-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let v: serde_json::Value = res.json().await.context("jito json parse")?;
    if let Some(sig) = v["result"].as_str() {
        Ok(SendOutcome {
            signature: sig.to_string(),
            endpoint: endpoint_url,
            bundle_id,
        })
    } else if let Some(err) = v.get("error") {
        Err(anyhow::anyhow!("jito error: {err}"))
    } else {
        Err(anyhow::anyhow!("jito no result"))
    }
}

impl Jito {
    pub async fn send_bundle(&self, txs_b64: &[String]) -> Result<BundleOutcome> {
        if self.endpoints.is_empty() {
            anyhow::bail!("no jito endpoints configured");
        }
        if txs_b64.is_empty() || txs_b64.len() > 5 {
            anyhow::bail!("bundle must contain 1-5 txs (got {})", txs_b64.len());
        }

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [txs_b64, { "encoding": "base64" }],
        });

        let mut futures = futures_util::stream::FuturesUnordered::new();
        for (idx, endpoint) in self.endpoints.iter().enumerate() {
            let client = self.client.clone();
            let url = format!("{}/api/v1/bundles", endpoint.url);
            let endpoint_url = endpoint.url.clone();
            let body = body.clone();
            let stagger = RACE_STAGGER * idx as u32;
            futures.push(
                async move {
                    if !stagger.is_zero() {
                        tokio::time::sleep(stagger).await;
                    }
                    try_send_bundle(client, url, body, endpoint_url).await
                }
                .boxed(),
            );
        }

        use futures_util::stream::StreamExt;
        let mut last_err: Option<anyhow::Error> = None;
        while let Some(res) = futures.next().await {
            match res {
                Ok(outcome) => return Ok(outcome),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no jito endpoints succeeded for bundle")))
    }
}

async fn try_send_bundle(
    client: reqwest::Client,
    url: String,
    body: serde_json::Value,
    endpoint_url: String,
) -> Result<BundleOutcome> {
    let res = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("jito bundle transport {endpoint_url}"))?;
    if !res.status().is_success() {
        anyhow::bail!("jito bundle http {}", res.status());
    }
    let v: serde_json::Value = res.json().await.context("jito bundle json parse")?;
    if let Some(bundle_id) = v["result"].as_str() {
        Ok(BundleOutcome {
            bundle_id: bundle_id.to_string(),
            endpoint: endpoint_url,
        })
    } else if let Some(err) = v.get("error") {
        Err(anyhow::anyhow!("jito bundle error: {err}"))
    } else {
        Err(anyhow::anyhow!("jito bundle no result"))
    }
}

#[derive(Clone, Debug)]
pub struct SendOutcome {
    pub signature: String,
    pub endpoint: String,
    pub bundle_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BundleOutcome {
    pub bundle_id: String,
    pub endpoint: String,
}
