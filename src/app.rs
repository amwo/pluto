use std::time::Instant;

use anyhow::Result;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::db::paper_trades::PaperTradeRecord;
use crate::adapters::{Db, Grpc, Http, Jupiter, Telegram};
use crate::config::Config;
use crate::domain::{
    Commitment, CopyDecision, FilterParams, ObservedTrade, Pubkey, Quote, Session, Side,
    StreamEvent, Subscription, decision, slot_clock,
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
    let telegram = cfg.telegram().map(Telegram::new);
    if telegram.is_some() {
        info!("telegram notifications enabled");
    }

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

    notify(
        telegram.as_ref(),
        &format!(
            "🟢 pluto started\nsession: {}\nmode: {}\nbalance: {:.3} SOL",
            session.id,
            session.mode.as_str(),
            lamports as f64 / 1e9
        ),
    )
    .await;

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
                    if let Err(e) = handle_trade(&db, &http, &jupiter, telegram.as_ref(), &cfg.bot_wallet, &wsol, session.id, &trade, &filter).await {
                        warn!(error = %e, "trade handling failed");
                        notify(telegram.as_ref(), &format!("⚠️ trade handling failed\n{e}")).await;
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
    notify(
        telegram.as_ref(),
        &format!("🔴 pluto stopped\nsession: {}", session.id),
    )
    .await;
    Ok(())
}

async fn notify(tg: Option<&Telegram>, text: &str) {
    if let Some(tg) = tg
        && let Err(e) = tg.send(text).await
    {
        warn!(error = %e, "telegram send failed");
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_trade(
    db: &Db,
    http: &Http,
    jupiter: &Jupiter,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
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
                paper_quote(
                    db, http, jupiter, telegram, bot_wallet, session_id, dec_id, trade, &input,
                    &output, *size_lamports,
                )
                .await?;
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

#[allow(clippy::too_many_arguments)]
async fn paper_quote(
    db: &Db,
    http: &Http,
    jupiter: &Jupiter,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    session_id: Uuid,
    copy_decision_id: i64,
    trade: &ObservedTrade,
    input_mint: &Pubkey,
    output_mint: &Pubkey,
    amount: u64,
) -> Result<()> {
    let started = Instant::now();
    let result = jupiter
        .quote(input_mint, output_mint, amount, PAPER_SLIPPAGE_BPS)
        .await;
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
            let msg = format_buy_notification(
                jupiter, http, bot_wallet, trade, &quote, amount, quote_latency_ms,
            )
            .await;
            notify(telegram, &msg).await;
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
            notify(
                telegram,
                &format!("⚠️ paper quote failed\n{msg}\nlatency: {quote_latency_ms} ms"),
            )
            .await;
        }
    }
    Ok(())
}

async fn format_buy_notification(
    jupiter: &Jupiter,
    http: &Http,
    bot_wallet: &Pubkey,
    trade: &ObservedTrade,
    quote: &Quote,
    my_in_lamports: u64,
    quote_latency_ms: i32,
) -> String {
    let mint = match trade.mint {
        Some(m) => m,
        None => return "💰 paper quote (mint unknown)".to_string(),
    };
    let token = jupiter.token_info(&mint).await;
    let symbol = token
        .as_ref()
        .map(|t| t.symbol.clone())
        .unwrap_or_else(|| short_mint(&mint));
    let decimals = token.as_ref().map(|t| t.decimals).unwrap_or(0);

    let src_sol = trade.sol_delta_lamports.unsigned_abs() as f64 / 1e9;
    let my_sol = my_in_lamports as f64 / 1e9;
    let diff_sol = my_sol - src_sol;

    let src_price = price_sol_per_ui_token(trade.sol_delta_lamports.unsigned_abs(), trade.token_delta.unsigned_abs(), decimals);
    let my_price = price_sol_per_ui_token(quote.in_amount, quote.out_amount as u128, decimals);
    let price_diff_pct = match (src_price, my_price) {
        (Some(s), Some(m)) if s > 0.0 => Some((m - s) / s * 100.0),
        _ => None,
    };

    let bal_sol = http
        .get_balance(bot_wallet)
        .await
        .ok()
        .map(|l| l as f64 / 1e9);

    let icon = match trade.side {
        Side::Buy => "🟢 BUY",
        Side::Sell => "🔴 SELL",
        Side::Unknown => "⚪ TRADE",
    };
    let delay = trade
        .detection_delay_ms
        .map(|d| d.to_string())
        .unwrap_or_else(|| "?".to_string());

    let mut out = String::new();
    out.push_str(&format!("{icon} {symbol}\n\n"));
    out.push_str(&format!(
        "👛 Copy: {:.4} SOL{}\n",
        src_sol,
        price_suffix(src_price)
    ));
    out.push_str(&format!(
        "🤖 Mine: {:.4} SOL{}\n\n",
        my_sol,
        price_suffix(my_price)
    ));
    out.push_str(&format!(
        "⚖️ Diff: {:+.4} SOL | {} | {delay}ms\n",
        diff_sol,
        price_diff_pct
            .map(|p| format!("{p:+.2}%"))
            .unwrap_or_else(|| format!("impact {:+.2}%", quote.price_impact_bps as f64 / 100.0)),
    ));
    out.push_str(&format!(
        "🏦 Bal: {}\n\n",
        bal_sol
            .map(|b| format!("{b:.3} SOL"))
            .unwrap_or_else(|| "?".to_string())
    ));
    out.push_str(&format!(
        "🔗 https://solscan.io/tx/{}\n",
        trade.signature
    ));
    out.push_str(&format!("(quote latency {quote_latency_ms} ms)"));
    out
}

fn short_mint(mint: &Pubkey) -> String {
    let s = mint.to_string();
    format!("{}…{}", &s[..4], &s[s.len() - 4..])
}

fn price_sol_per_ui_token(sol_lamports: u64, token_raw: u128, decimals: u8) -> Option<f64> {
    if token_raw == 0 {
        return None;
    }
    let sol = sol_lamports as f64 / 1e9;
    let scale = 10f64.powi(decimals as i32);
    let ui_tokens = token_raw as f64 / scale;
    if ui_tokens == 0.0 {
        return None;
    }
    Some(sol / ui_tokens)
}

fn price_suffix(price: Option<f64>) -> String {
    match price {
        Some(p) if p.is_finite() => format!(" @ {p:.9}"),
        _ => String::new(),
    }
}

