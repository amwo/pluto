mod client;
mod proto;

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tracing::warn;

use client::Client;

use crate::domain::{Commitment, StreamEvent, Subscription};

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub url: String,
    pub username: String,
    pub password: String,
}

pub struct Grpc {
    endpoint: Endpoint,
}

impl Grpc {
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }

    pub fn spawn_stream(
        &self,
        subscriptions: Vec<Subscription>,
        commitment: Commitment,
    ) -> mpsc::Receiver<StreamEvent> {
        let endpoint = self.endpoint.clone();
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut delay = Duration::from_secs(1);
            loop {
                let started = std::time::Instant::now();
                match drain(&endpoint, &subscriptions, commitment, &tx).await {
                    Ok(()) => return,
                    Err(e) => warn!(error = %e, "grpc stream lost"),
                }
                if tx.is_closed() {
                    return;
                }
                if started.elapsed() > Duration::from_secs(60) {
                    delay = Duration::from_secs(1);
                }
                warn!(delay_secs = delay.as_secs(), "reconnecting");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(60));
            }
        });
        rx
    }
}

async fn drain(
    endpoint: &Endpoint,
    subscriptions: &[Subscription],
    commitment: Commitment,
    tx: &mpsc::Sender<StreamEvent>,
) -> Result<()> {
    let mut client = Client::connect(endpoint).await?;
    let mut updates = client.subscribe(subscriptions, commitment).await?;
    while let Some(msg) = updates.next().await {
        let update = msg.context("grpc stream error")?;
        if tx.send(update).await.is_err() {
            return Ok(());
        }
    }
    anyhow::bail!("grpc stream ended")
}
