use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::domain::Pubkey;

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub url: String,
    pub username: String,
    pub password: String,
}

pub struct Http {
    client: reqwest::Client,
    endpoint: Endpoint,
}

impl Http {
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
        }
    }

    pub async fn get_slot(&self) -> Result<u64> {
        let response: Value = self
            .client
            .post(&self.endpoint.url)
            .basic_auth(&self.endpoint.username, Some(&self.endpoint.password))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getSlot",
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        response["result"]
            .as_u64()
            .context("missing slot in response")
    }

    pub async fn get_balance(&self, pubkey: &Pubkey) -> Result<u64> {
        let response: Value = self
            .client
            .post(&self.endpoint.url)
            .basic_auth(&self.endpoint.username, Some(&self.endpoint.password))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getBalance",
                "params": [pubkey.to_string()],
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        response["result"]["value"]
            .as_u64()
            .context("missing balance in response")
    }

    pub async fn get_signature_status(&self, signature: &str) -> Result<SignatureStatus> {
        let response: Value = self
            .client
            .post(&self.endpoint.url)
            .basic_auth(&self.endpoint.username, Some(&self.endpoint.password))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getSignatureStatuses",
                "params": [[signature], { "searchTransactionHistory": false }],
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let entry = &response["result"]["value"][0];
        if entry.is_null() {
            return Ok(SignatureStatus::Pending);
        }
        if let Some(err) = entry.get("err")
            && !err.is_null()
        {
            return Ok(SignatureStatus::Failed(err.to_string()));
        }
        let confirmation = entry["confirmationStatus"].as_str().unwrap_or("");
        Ok(match confirmation {
            "finalized" => SignatureStatus::Finalized,
            "confirmed" => SignatureStatus::Confirmed,
            "processed" => SignatureStatus::Processed,
            _ => SignatureStatus::Pending,
        })
    }
}

#[derive(Clone, Debug)]
pub enum SignatureStatus {
    Pending,
    Processed,
    Confirmed,
    Finalized,
    Failed(String),
}

impl SignatureStatus {
    pub fn is_landed(&self) -> bool {
        matches!(
            self,
            SignatureStatus::Processed | SignatureStatus::Confirmed | SignatureStatus::Finalized
        )
    }
}
