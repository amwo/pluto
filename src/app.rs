use anyhow::Result;
use tracing::info;

use crate::adapters::{Db, Grpc, Http};
use crate::config::Config;
use crate::domain::{Commitment, DetectedTx, Session, StreamEvent, Subscription};

pub async fn run(cfg: Config) -> Result<()> {
    let http = Http::new(cfg.http());
    let lamports = http.get_balance(&cfg.bot_wallet).await?;
    info!(wallet = %cfg.bot_wallet, sol = lamports as f64 / 1e9, "bot wallet balance");

    let db = Db::connect(&cfg.database_url).await?;
    db.sessions().mark_running_as_crashed().await?;

    let session = Session::new(cfg.mode);
    db.sessions().insert(&session).await?;
    info!(id = %session.id, mode = session.mode.as_str(), "session started");

    let grpc = Grpc::new(cfg.grpc());
    let mut events = grpc.spawn_stream(
        vec![Subscription::WalletTransactions(vec![cfg.target_wallet])],
        Commitment::Processed,
    );

    loop {
        tokio::select! {
            msg = events.recv() => match msg {
                Some(StreamEvent::Tx { slot, signature }) => {
                    let detected = DetectedTx { slot, signature };
                    info!(slot = %detected.slot, signature = %detected.signature, "tx");
                    db.sessions().record_tx(session.id).await?;
                }
                Some(StreamEvent::Heartbeat) => {
                    info!("heartbeat");
                }
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    db.sessions().complete(&session).await?;
    Ok(())
}
