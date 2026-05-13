use std::time::Instant;

use anyhow::Result;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::db::paper_trades::PaperTradeRecord;
use crate::adapters::{Db, Grpc, Http, Jupiter};
use crate::config::Config;
use crate::domain::{
    Commitment, CopyDecision, FilterParams, ObservedTrade, Pubkey, Session, Side, StreamEvent,
    Subscription, decision, slot_clock,
};

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const PAPER_SLIPPAGE_BPS: u32 = 200;

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

    let current_slot = http.get_slot().await?;
    slot_clock::init(current_slot);
    info!(slot = current_slot, "slot clock initialized");

    let jupiter = Jupiter::new();
    let wsol = Pubkey::from_base58(WSOL_MINT)?;

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
                    if let Err(e) = handle_trade(&db, &jupiter, &wsol, session.id, &trade, &filter).await {
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
    jupiter: &Jupiter,
    wsol: &Pubkey,
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
        detection_delay_ms = ?trade.detection_delay_ms,
        "trade observed"
    );
    let trade_id = db.observed_trades().insert(session_id, trade).await?;
    let dec = decision::decide(trade, filter);
    let dec_id = db.copy_decisions().insert(session_id, trade_id, &dec).await?;

    match &dec {
        CopyDecision::Copy { size_lamports } => {
            info!(
                signature = %trade.signature,
                size_sol = *size_lamports as f64 / 1e9,
                "copy"
            );
            if let Some(mint) = trade.mint {
                let (input, output) = match trade.side {
                    Side::Buy => (*wsol, mint),
                    _ => (mint, *wsol),
                };
                paper_quote(db, jupiter, session_id, dec_id, &input, &output, *size_lamports).await?;
            }
        }
        CopyDecision::Skip(reason) => {
            info!(
                signature = %trade.signature,
                reason = reason.as_str(),
                "skip"
            );
        }
    }
    db.sessions().record_tx(session_id).await?;
    Ok(())
}

async fn paper_quote(
    db: &Db,
    jupiter: &Jupiter,
    session_id: Uuid,
    copy_decision_id: i64,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount: u64,
) -> Result<()> {
    let started = Instant::now();
    let result = jupiter.quote(input_mint, output_mint, amount, PAPER_SLIPPAGE_BPS).await;
    let quote_latency_ms = started.elapsed().as_millis() as i32;

    match result {
        Ok(quote) => {
            info!(
                in_amount = quote.in_amount,
                out_amount = quote.out_amount,
                price_impact_bps = quote.price_impact_bps,
                quote_latency_ms,
                route = ?quote.route_labels,
                "paper quote"
            );
            db.paper_trades()
                .insert(
                    session_id,
                    PaperTradeRecord {
                        copy_decision_id,
                        input_mint,
                        output_mint,
                        in_amount: amount,
                        slippage_bps: PAPER_SLIPPAGE_BPS,
                        quote: Some(&quote),
                        quote_latency_ms,
                        error: None,
                    },
                )
                .await?;
        }
        Err(e) => {
            warn!(error = %e, quote_latency_ms, "paper quote failed");
            let msg = e.to_string();
            db.paper_trades()
                .insert(
                    session_id,
                    PaperTradeRecord {
                        copy_decision_id,
                        input_mint,
                        output_mint,
                        in_amount: amount,
                        slippage_bps: PAPER_SLIPPAGE_BPS,
                        quote: None,
                        quote_latency_ms,
                        error: Some(&msg),
                    },
                )
                .await?;
        }
    }
    Ok(())
}
