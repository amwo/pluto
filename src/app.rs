use std::time::{Duration, Instant};

use anyhow::Result;
use time::OffsetDateTime;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::db::paper_trades::PaperTradeRecord;
use crate::adapters::{Db, Grpc, Http, Jupiter, Telegram};
use crate::config::Config;
use crate::domain::{
    Commitment, CopyDecision, ExitParams, ExitReason, FilterParams, ObservedTrade, Position,
    Pubkey, Quote, Session, Side, SkipReason, StreamEvent, Subscription, decision, exit,
    slot_clock,
};

const EXIT_CHECK_INTERVAL: Duration = Duration::from_secs(30);

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
    let exit_params = ExitParams::default();
    let mut exit_tick = tokio::time::interval(EXIT_CHECK_INTERVAL);
    exit_tick.tick().await;

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
            _ = exit_tick.tick() => {
                if let Err(e) = check_exits(&db, &http, &jupiter, telegram.as_ref(), &cfg.bot_wallet, &wsol, session.id, &exit_params).await {
                    warn!(error = %e, "exit check failed");
                }
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

    match trade.side {
        Side::Buy => {
            handle_buy(
                db, http, jupiter, telegram, bot_wallet, wsol, session_id, trade, trade_id, filter,
            )
            .await?
        }
        Side::Sell => {
            handle_sell(
                db, http, jupiter, telegram, bot_wallet, wsol, session_id, trade, trade_id,
            )
            .await?
        }
        Side::Unknown => {
            db.copy_decisions()
                .insert(
                    session_id,
                    trade_id,
                    &CopyDecision::Skip(SkipReason::DecodeUncertain),
                )
                .await?;
            info!(signature = %trade.signature, "skip decode_uncertain");
        }
    }

    db.sessions().record_tx(session_id).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_buy(
    db: &Db,
    http: &Http,
    jupiter: &Jupiter,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    trade: &ObservedTrade,
    trade_id: i64,
    filter: &FilterParams,
) -> Result<()> {
    let Some(mint) = trade.mint else {
        let skip = CopyDecision::Skip(SkipReason::DecodeUncertain);
        db.copy_decisions().insert(session_id, trade_id, &skip).await?;
        info!(signature = %trade.signature, "skip decode_uncertain (no mint)");
        return Ok(());
    };

    if db
        .positions()
        .find_open_by_mint(session_id, mint)
        .await?
        .is_some()
    {
        let skip = CopyDecision::Skip(SkipReason::ExistingPosition);
        db.copy_decisions().insert(session_id, trade_id, &skip).await?;
        info!(signature = %trade.signature, mint = %mint, "skip existing_position");
        return Ok(());
    }

    let dec = decision::decide(trade, filter);
    let dec_id = db.copy_decisions().insert(session_id, trade_id, &dec).await?;

    let CopyDecision::Copy { size_lamports } = dec else {
        if let CopyDecision::Skip(reason) = dec {
            info!(signature = %trade.signature, reason = reason.as_str(), "skip");
        }
        return Ok(());
    };
    info!(
        signature = %trade.signature,
        size_sol = size_lamports as f64 / 1e9,
        "copy"
    );

    let started = Instant::now();
    let result = jupiter
        .quote(wsol, &mint, size_lamports, PAPER_SLIPPAGE_BPS)
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
                "paper buy quote"
            );
            let pt_id = db
                .paper_trades()
                .insert(
                    session_id,
                    PaperTradeRecord {
                        copy_decision_id: Some(dec_id),
                        input_mint: wsol,
                        output_mint: &mint,
                        in_amount: size_lamports,
                        slippage_bps: PAPER_SLIPPAGE_BPS,
                        quote: Some(&quote),
                        quote_latency_ms,
                        error: None,
                    },
                )
                .await?;

            let entry_price = price_sol_per_raw(quote.in_amount, quote.out_amount as u128);
            db.positions()
                .open(
                    session_id,
                    mint,
                    pt_id,
                    quote.in_amount,
                    quote.out_amount,
                    entry_price,
                )
                .await?;

            let msg = format_buy_notification(
                jupiter,
                http,
                bot_wallet,
                trade,
                &quote,
                size_lamports,
                quote_latency_ms,
            )
            .await;
            notify(telegram, &msg).await;
        }
        Err(e) => {
            warn!(error = %e, quote_latency_ms, "paper buy quote failed");
            let msg = e.to_string();
            db.paper_trades()
                .insert(
                    session_id,
                    PaperTradeRecord {
                        copy_decision_id: Some(dec_id),
                        input_mint: wsol,
                        output_mint: &mint,
                        in_amount: size_lamports,
                        slippage_bps: PAPER_SLIPPAGE_BPS,
                        quote: None,
                        quote_latency_ms,
                        error: Some(&msg),
                    },
                )
                .await?;
            notify(
                telegram,
                &format!("⚠️ paper buy quote failed\n{msg}\nlatency: {quote_latency_ms} ms"),
            )
            .await;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_sell(
    db: &Db,
    http: &Http,
    jupiter: &Jupiter,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    trade: &ObservedTrade,
    trade_id: i64,
) -> Result<()> {
    let mint = match trade.mint {
        Some(m) => m,
        None => return Ok(()),
    };
    let position = db
        .positions()
        .find_open_by_mint(session_id, mint)
        .await?;
    let Some(position) = position else {
        db.copy_decisions()
            .insert(
                session_id,
                trade_id,
                &CopyDecision::Skip(SkipReason::NoOpenPosition),
            )
            .await?;
        info!(signature = %trade.signature, "skip no_open_position");
        return Ok(());
    };

    info!(
        signature = %trade.signature,
        position_id = position.id,
        "target sell follow"
    );

    let started = Instant::now();
    let result = jupiter
        .quote(&mint, wsol, position.entry_out_amount, PAPER_SLIPPAGE_BPS)
        .await;
    let quote_latency_ms = started.elapsed().as_millis() as i32;

    match result {
        Ok(quote) => {
            let pt_id = db
                .paper_trades()
                .insert(
                    session_id,
                    PaperTradeRecord {
                        copy_decision_id: None,
                        input_mint: &mint,
                        output_mint: wsol,
                        in_amount: position.entry_out_amount,
                        slippage_bps: PAPER_SLIPPAGE_BPS,
                        quote: Some(&quote),
                        quote_latency_ms,
                        error: None,
                    },
                )
                .await?;

            let realized_pnl_lamports =
                quote.out_amount as i64 - position.entry_in_lamports as i64;
            let realized_pnl_pct =
                realized_pnl_lamports as f64 / position.entry_in_lamports as f64 * 100.0;
            let closed = db
                .positions()
                .close(
                    position.id,
                    pt_id,
                    ExitReason::TargetSellFollow,
                    realized_pnl_lamports,
                    realized_pnl_pct,
                )
                .await?;
            if !closed {
                info!(position_id = position.id, "close race lost; skipping notify");
                return Ok(());
            }

            info!(
                position_id = position.id,
                exit_paper_trade_id = pt_id,
                pnl_sol = realized_pnl_lamports as f64 / 1e9,
                pnl_pct = realized_pnl_pct,
                "position closed"
            );

            let msg = format_sell_notification(
                jupiter,
                http,
                bot_wallet,
                trade,
                &position,
                &quote,
                realized_pnl_lamports,
                realized_pnl_pct,
                quote_latency_ms,
            )
            .await;
            notify(telegram, &msg).await;
        }
        Err(e) => {
            warn!(error = %e, quote_latency_ms, "paper sell quote failed");
            let msg = e.to_string();
            db.paper_trades()
                .insert(
                    session_id,
                    PaperTradeRecord {
                        copy_decision_id: None,
                        input_mint: &mint,
                        output_mint: wsol,
                        in_amount: position.entry_out_amount,
                        slippage_bps: PAPER_SLIPPAGE_BPS,
                        quote: None,
                        quote_latency_ms,
                        error: Some(&msg),
                    },
                )
                .await?;
            notify(
                telegram,
                &format!(
                    "⚠️ paper sell quote failed\nposition: {}\n{msg}\nlatency: {quote_latency_ms} ms",
                    position.id
                ),
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
        None => return "💰 paper buy (mint unknown)".to_string(),
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

    let src_price = price_sol_per_ui_token(
        trade.sol_delta_lamports.unsigned_abs(),
        trade.token_delta.unsigned_abs(),
        decimals,
    );
    let my_price = price_sol_per_ui_token(quote.in_amount, quote.out_amount as u128, decimals);
    let price_diff_pct = pct_diff(src_price, my_price);

    let bal_sol = http
        .get_balance(bot_wallet)
        .await
        .ok()
        .map(|l| l as f64 / 1e9);
    let delay = delay_str(trade.detection_delay_ms);

    let mut out = String::new();
    out.push_str(&format!("🟢 BUY {symbol}\n\n"));
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
    out.push_str(&format!("🏦 Bal: {}\n\n", bal_str(bal_sol)));
    out.push_str(&format!(
        "🔗 https://solscan.io/tx/{}\n",
        trade.signature
    ));
    out.push_str(&format!("(quote latency {quote_latency_ms} ms)"));
    out
}

#[allow(clippy::too_many_arguments)]
async fn format_sell_notification(
    jupiter: &Jupiter,
    http: &Http,
    bot_wallet: &Pubkey,
    trade: &ObservedTrade,
    position: &Position,
    quote: &Quote,
    realized_pnl_lamports: i64,
    realized_pnl_pct: f64,
    quote_latency_ms: i32,
) -> String {
    let token = jupiter.token_info(&position.mint).await;
    let symbol = token
        .as_ref()
        .map(|t| t.symbol.clone())
        .unwrap_or_else(|| short_mint(&position.mint));
    let decimals = token.as_ref().map(|t| t.decimals).unwrap_or(0);

    let tgt_sol = trade.sol_delta_lamports.unsigned_abs() as f64 / 1e9;
    let my_sol = quote.out_amount as f64 / 1e9;

    let tgt_price = price_sol_per_ui_token(
        trade.sol_delta_lamports.unsigned_abs(),
        trade.token_delta.unsigned_abs(),
        decimals,
    );
    let my_price = price_sol_per_ui_token(quote.out_amount, quote.in_amount as u128, decimals);
    let price_diff_pct = pct_diff(tgt_price, my_price);
    let pnl_sol = realized_pnl_lamports as f64 / 1e9;

    let bal_sol = http
        .get_balance(bot_wallet)
        .await
        .ok()
        .map(|l| l as f64 / 1e9);
    let delay = delay_str(trade.detection_delay_ms);

    let mut out = String::new();
    out.push_str(&format!("🔴 SELL {symbol}\n\n"));
    out.push_str(&format!(
        "👛 Target: {:.4} SOL{}\n",
        tgt_sol,
        price_suffix(tgt_price)
    ));
    out.push_str(&format!(
        "🤖 Mine:   {:.4} SOL{}\n\n",
        my_sol,
        price_suffix(my_price)
    ));
    out.push_str(&format!(
        "💰 PnL: {pnl_sol:+.4} SOL ({realized_pnl_pct:+.1}%)\n"
    ));
    out.push_str(&format!(
        "⚖️ Diff: {} | {delay}ms\n",
        price_diff_pct
            .map(|p| format!("{p:+.2}% price"))
            .unwrap_or_else(|| format!("impact {:+.2}%", quote.price_impact_bps as f64 / 100.0)),
    ));
    out.push_str(&format!("🏦 Bal: {}\n\n", bal_str(bal_sol)));
    out.push_str(&format!(
        "🔗 https://solscan.io/tx/{}\n",
        trade.signature
    ));
    out.push_str(&format!("(quote latency {quote_latency_ms} ms)"));
    out
}

#[allow(clippy::too_many_arguments)]
async fn check_exits(
    db: &Db,
    http: &Http,
    jupiter: &Jupiter,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    params: &ExitParams,
) -> Result<()> {
    let positions = db.positions().list_open(session_id).await?;
    for position in positions {
        let started = Instant::now();
        let quote_result = jupiter
            .quote(
                &position.mint,
                wsol,
                position.entry_out_amount,
                PAPER_SLIPPAGE_BPS,
            )
            .await;
        let quote_latency_ms = started.elapsed().as_millis() as i32;

        let quote = match quote_result {
            Ok(q) => q,
            Err(e) => {
                warn!(position_id = position.id, error = %e, "exit price quote failed");
                continue;
            }
        };

        if quote.out_amount == 0 {
            warn!(
                position_id = position.id,
                "exit quote returned zero out_amount; skipping auto close"
            );
            continue;
        }

        let current_price = price_sol_per_raw(quote.out_amount, position.entry_out_amount as u128);
        if current_price > position.peak_price {
            db.positions().update_peak(position.id, current_price).await?;
        }

        let now = OffsetDateTime::now_utc();
        let Some(reason) = exit::should_exit(&position, current_price, now, params) else {
            continue;
        };

        info!(
            position_id = position.id,
            reason = reason.as_str(),
            current_price,
            entry_price = position.entry_price,
            peak_price = position.peak_price,
            "exit triggered"
        );

        let pt_id = db
            .paper_trades()
            .insert(
                session_id,
                PaperTradeRecord {
                    copy_decision_id: None,
                    input_mint: &position.mint,
                    output_mint: wsol,
                    in_amount: position.entry_out_amount,
                    slippage_bps: PAPER_SLIPPAGE_BPS,
                    quote: Some(&quote),
                    quote_latency_ms,
                    error: None,
                },
            )
            .await?;

        let realized_pnl_lamports = quote.out_amount as i64 - position.entry_in_lamports as i64;
        let realized_pnl_pct =
            realized_pnl_lamports as f64 / position.entry_in_lamports as f64 * 100.0;

        let closed = db
            .positions()
            .close(position.id, pt_id, reason, realized_pnl_lamports, realized_pnl_pct)
            .await?;
        if !closed {
            info!(position_id = position.id, "close race lost; skipping notify");
            continue;
        }

        let msg = format_exit_notification(
            jupiter,
            http,
            bot_wallet,
            &position,
            &quote,
            reason,
            realized_pnl_lamports,
            realized_pnl_pct,
            quote_latency_ms,
        )
        .await;
        notify(telegram, &msg).await;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn format_exit_notification(
    jupiter: &Jupiter,
    http: &Http,
    bot_wallet: &Pubkey,
    position: &Position,
    quote: &Quote,
    reason: ExitReason,
    realized_pnl_lamports: i64,
    realized_pnl_pct: f64,
    quote_latency_ms: i32,
) -> String {
    let token = jupiter.token_info(&position.mint).await;
    let symbol = token
        .as_ref()
        .map(|t| t.symbol.clone())
        .unwrap_or_else(|| short_mint(&position.mint));
    let decimals = token.as_ref().map(|t| t.decimals).unwrap_or(0);

    let exit_sol = quote.out_amount as f64 / 1e9;
    let entry_sol = position.entry_in_lamports as f64 / 1e9;
    let exit_price = price_sol_per_ui_token(quote.out_amount, quote.in_amount as u128, decimals);
    let entry_price = price_sol_per_ui_token(
        position.entry_in_lamports,
        position.entry_out_amount as u128,
        decimals,
    );
    let pnl_sol = realized_pnl_lamports as f64 / 1e9;

    let now = OffsetDateTime::now_utc();
    let held_secs = (now - position.opened_at).whole_seconds().max(0);
    let held = format!("{}m{:02}s", held_secs / 60, held_secs % 60);

    let bal_sol = http
        .get_balance(bot_wallet)
        .await
        .ok()
        .map(|l| l as f64 / 1e9);

    let mut out = String::new();
    out.push_str(&format!("🔴 SELL {symbol} ({})\n\n", reason.as_str()));
    out.push_str(&format!(
        "🤖 Exit:  {exit_sol:.4} SOL{}\n",
        price_suffix(exit_price)
    ));
    out.push_str(&format!(
        "Entry: {entry_sol:.4} SOL{}\n",
        price_suffix(entry_price)
    ));
    out.push_str(&format!("Held: {held}\n\n"));
    out.push_str(&format!("💰 PnL: {pnl_sol:+.4} SOL ({realized_pnl_pct:+.1}%)\n"));
    out.push_str(&format!("🏦 Bal: {}\n", bal_str(bal_sol)));
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

fn price_sol_per_raw(sol_lamports: u64, token_raw: u128) -> f64 {
    if token_raw == 0 {
        return 0.0;
    }
    sol_lamports as f64 / token_raw as f64
}

fn pct_diff(base: Option<f64>, observed: Option<f64>) -> Option<f64> {
    match (base, observed) {
        (Some(b), Some(o)) if b > 0.0 => Some((o - b) / b * 100.0),
        _ => None,
    }
}

fn price_suffix(price: Option<f64>) -> String {
    match price {
        Some(p) if p.is_finite() => format!(" @ {p:.9}"),
        _ => String::new(),
    }
}

fn delay_str(delay: Option<i32>) -> String {
    delay
        .map(|d| d.to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn bal_str(bal_sol: Option<f64>) -> String {
    bal_sol
        .map(|b| format!("{b:.3} SOL"))
        .unwrap_or_else(|| "?".to_string())
}
