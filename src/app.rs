use anyhow::Result;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::{Db, Grpc, Http};
use crate::config::Config;
use crate::domain::{
    Commitment, CopyDecision, FilterParams, ObservedTrade, Session, StreamEvent, Subscription,
    decision,
};

pub async fn report(cfg: Config, day: Option<String>) -> Result<()> {
    let db = Db::connect(&cfg.database_url).await?;
    let report = db.reports().daily(day.as_deref()).await?;
    print!("{report}");
    Ok(())
}

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
    let filter = FilterParams::default();

    loop {
        tokio::select! {
            msg = events.recv() => match msg {
                Some(StreamEvent::Trade(trade)) => {
                    if let Err(e) = handle_trade(&db, session.id, &trade, &filter).await {
                        warn!(error = %e, "trade handling failed");
                    }
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

async fn handle_trade(
    db: &Db,
    session_id: Uuid,
    trade: &ObservedTrade,
    filter: &FilterParams,
) -> Result<()> {
    info!(
        slot = %trade.slot,
        signature = %trade.signature,
        side = trade.side.as_str(),
        sol = trade.sol_delta_lamports as f64 / 1e9,
        route = ?trade.route,
        "trade observed"
    );
    let trade_id = db.observed_trades().insert(session_id, trade).await?;
    let dec = decision::decide(trade, filter);
    match &dec {
        CopyDecision::Copy { size_lamports } => {
            info!(
                signature = %trade.signature,
                size_sol = *size_lamports as f64 / 1e9,
                "copy"
            );
        }
        CopyDecision::Skip(reason) => {
            info!(
                signature = %trade.signature,
                reason = reason.as_str(),
                "skip"
            );
        }
    }
    db.copy_decisions().insert(session_id, trade_id, &dec).await?;
    db.sessions().record_tx(session_id).await?;
    Ok(())
}
