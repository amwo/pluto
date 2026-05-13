use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde_json::json;
use tokio::sync::Mutex;

const MIN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub token: String,
    pub chat_id: String,
}

pub struct Telegram {
    client: reqwest::Client,
    endpoint: Endpoint,
    next_send_at: Mutex<Instant>,
}

impl Telegram {
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            client: reqwest::Client::new(),
            endpoint,
            next_send_at: Mutex::new(Instant::now()),
        }
    }

    pub async fn send(&self, text: &str) -> Result<()> {
        self.throttle().await;
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.endpoint.token
        );
        let response = self
            .client
            .post(&url)
            .json(&json!({
                "chat_id": self.endpoint.chat_id,
                "text": text,
                "disable_web_page_preview": true,
            }))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("telegram {status}: {body}"));
        }
        Ok(())
    }

    async fn throttle(&self) {
        let mut next = self.next_send_at.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep(*next - now).await;
        }
        *next = Instant::now() + MIN_INTERVAL;
    }
}
