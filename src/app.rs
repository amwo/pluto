use anyhow::Result;
use tracing::info;

use crate::adapters::db::{self, sessions};
use crate::adapters::{grpc, http};
use crate::config::Config;
use crate::domain::{Commitment, DetectedTx, Session, StreamEvent, Subscription};

pub async fn run(cfg: Config) -> Result<()> {
    let lamports = http::get_balance(&cfg.http(), &cfg.bot_wallet).await?;
    info!(wallet = %cfg.bot_wallet, sol = lamports as f64 / 1e9, "bot wallet balance");

    let pool = db::connect(&cfg.database_url).await?;
    sessions::mark_running_as_crashed(&pool).await?;

    let session = Session::new(cfg.mode);
    sessions::insert(&pool, &session).await?;
    info!(id = %session.id, mode = session.mode.as_str(), "session started");

    let mut events = grpc::spawn_stream(
        cfg.grpc(),
        vec![Subscription::WalletTransactions(vec![cfg.target_wallet])],
        Commitment::Processed,
    );

    loop {
        tokio::select! {
            msg = events.recv() => match msg {
                Some(StreamEvent::Tx { slot, signature }) => {
                    let detected = DetectedTx { slot, signature };
                    info!(slot = %detected.slot, signature = %detected.signature, "tx");
                    sessions::record_tx(&pool, session.id).await?;
                }
                Some(StreamEvent::Heartbeat) => {
                    info!("heartbeat");
                }
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    sessions::complete(&pool, &session).await?;
    Ok(())
}
