use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::domain::Pubkey;

#[derive(Clone, Debug)]
pub struct HttpEndpoint {
    pub url: String,
    pub username: String,
    pub password: String,
}

pub struct Http {
    client: reqwest::Client,
    endpoint: HttpEndpoint,
}

impl Http {
    pub fn new(endpoint: HttpEndpoint) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
        }
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
}
