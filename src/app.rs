use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use uuid::Uuid;

struct ExitInFlightGuard(Arc<AtomicU32>);

impl ExitInFlightGuard {
    fn new(counter: Arc<AtomicU32>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for ExitInFlightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

use crate::adapters::db::dry_trades::DryTradeRecord;
use crate::adapters::{Db, Grpc, Http, Jito, Jupiter, LiveExecutor, Signer, Telegram};
use crate::config::Config;
use crate::domain::{
    Commitment, CopyDecision, ExitParams, ExitReason, FilterContext, FilterParams, LatencyKind,
    Mode, ObservedTrade, Position, Pubkey, Quote, Session, Side, SkipReason, StreamEvent,
    Subscription, decision, exit, slot_clock,
};

const EXIT_CHECK_INTERVAL: Duration = Duration::from_secs(30);

const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const DRY_SLIPPAGE_BPS: u32 = 200;
const MINT_BLOCK_LOSS_THRESHOLD: u32 = 2;
const MINT_BLOCK_TTL_SECS: i64 = 24 * 60 * 60;
const COLD_STREAK_WINDOW_SECS: i64 = 60 * 60;

pub async fn report(cfg: Config, day: Option<String>) -> Result<()> {
    let db = Db::connect(&cfg.database_url).await?;
    let report = db.reports().daily(day.as_deref()).await?;
    print!("{report}");
    Ok(())
}

pub async fn run(cfg: Config) -> Result<()> {
    let telegram = cfg.telegram().map(Telegram::new).map(Arc::new);
    if telegram.is_some() {
        info!("telegram notifications enabled");
    }

    let http = Arc::new(Http::new(cfg.http()));
    let lamports = http.get_balance(&cfg.bot_wallet).await?;
    info!(wallet = %cfg.bot_wallet, sol = lamports as f64 / 1e9, "bot wallet balance");

    let current_slot = http.get_slot().await?;
    slot_clock::init(current_slot);
    info!(slot = current_slot, "slot clock initialized");

    let jupiter = Arc::new(Jupiter::new());
    let wsol = Pubkey::from_base58(WSOL_MINT)?;
    let bot_wallet = Arc::new(cfg.bot_wallet);

    let db = Arc::new(Db::connect(&cfg.database_url).await?);
    db.sessions().mark_running_as_crashed().await?;
    let stuck = db.positions().mark_closing_as_crashed().await?;
    if stuck > 0 {
        warn!(count = stuck, "marked stuck 'closing' positions as 'crashed' — investigate live tx state on chain");
    }

    let live_executor = build_live_executor(&cfg, http.clone())?;
    let live_executor = live_executor.map(Arc::new);
    if cfg.mode == Mode::Live && live_executor.is_none() {
        anyhow::bail!("PLUTO_MODE=live requires SOLANA_SIGNER_SECRET and JITO_BLOCK_ENGINE_URLS");
    }
    if let Some(exec) = &live_executor {
        info!(taker = %exec.taker(), "live executor ready");
    }

    let session = Session::new(cfg.mode);
    db.sessions().insert(&session).await?;
    info!(id = %session.id, mode = session.mode.as_str(), "session started");

    notify(
        telegram.as_deref(),
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
    let exit_gate = Arc::new(Semaphore::new(1));
    let live_exit_in_flight = Arc::new(AtomicU32::new(0));

    loop {
        tokio::select! {
            msg = events.recv() => match msg {
                Some(StreamEvent::Trade(trade)) => {
                    if let Err(e) = handle_trade(
                        &db,
                        &http,
                        &jupiter,
                        live_executor.as_deref(),
                        telegram.as_deref(),
                        &bot_wallet,
                        &wsol,
                        session.id,
                        session.mode,
                        &trade,
                        &filter,
                        &live_exit_in_flight,
                    )
                    .await
                    {
                        warn!(error = %e, "trade handling failed");
                        notify(telegram.as_deref(), &format!("⚠️ trade handling failed\n{e}")).await;
                    }
                }
                Some(StreamEvent::Heartbeat) => {
                    info!("heartbeat");
                }
                None => break,
            },
            _ = exit_tick.tick() => {
                if session.mode == Mode::Observe {
                    continue;
                }
                let Ok(permit) = exit_gate.clone().try_acquire_owned() else {
                    info!("exit check still running, skipping tick");
                    continue;
                };
                let db = db.clone();
                let http = http.clone();
                let jupiter = jupiter.clone();
                let live = live_executor.clone();
                let telegram = telegram.clone();
                let bot_wallet = bot_wallet.clone();
                let session_id = session.id;
                let mode = session.mode;
                let exit_in_flight = live_exit_in_flight.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = check_exits(
                        &db,
                        &http,
                        &jupiter,
                        live.as_deref(),
                        telegram.as_deref(),
                        &bot_wallet,
                        &wsol,
                        session_id,
                        mode,
                        &exit_params,
                        &exit_in_flight,
                    )
                    .await
                    {
                        warn!(error = %e, "exit check failed");
                    }
                });
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    db.sessions().complete(&session).await?;
    notify(
        telegram.as_deref(),
        &format!("🔴 pluto stopped\nsession: {}", session.id),
    )
    .await;
    Ok(())
}

fn build_live_executor(cfg: &Config, http: Arc<Http>) -> Result<Option<LiveExecutor>> {
    let Some(secret) = cfg.signer_secret_b58.as_deref() else {
        return Ok(None);
    };
    let endpoints = cfg.jito();
    if endpoints.is_empty() {
        return Ok(None);
    }
    let signer = Signer::from_base58_secret(secret)?;
    let jito = Jito::new(endpoints);
    let jupiter = Jupiter::new();
    Ok(Some(LiveExecutor::new(jupiter, jito, signer, http)))
}

#[allow(clippy::too_many_arguments)]
async fn execute_live_buy(
    db: &Db,
    http: &Http,
    executor: &LiveExecutor,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    mint: &Pubkey,
    session_id: Uuid,
    dec_id: i64,
    size_lamports: u64,
    trade: &ObservedTrade,
) -> Result<()> {
    let outcome = executor
        .execute_swap(db, session_id, wsol, mint, size_lamports)
        .await?;
    let quote = outcome.quote.clone();

    let dt_id = db
        .dry_trades()
        .insert(
            session_id,
            DryTradeRecord {
                copy_decision_id: Some(dec_id),
                input_mint: wsol,
                output_mint: mint,
                in_amount: size_lamports,
                slippage_bps: quote.slippage_bps,
                quote: Some(&quote),
                quote_latency_ms: outcome.quote_latency_ms,
                error: None,
            },
        )
        .await?;

    let entry_price = price_sol_per_raw(quote.in_amount, quote.out_amount as u128);
    db.positions()
        .open(
            session_id,
            *mint,
            dt_id,
            quote.in_amount,
            quote.out_amount,
            entry_price,
        )
        .await?;

    info!(
        signature = %outcome.signature,
        endpoint = %outcome.endpoint,
        quote_latency_ms = outcome.quote_latency_ms,
        swap_latency_ms = outcome.swap_latency_ms,
        send_latency_ms = outcome.send_latency_ms,
        confirm_latency_ms = ?outcome.confirm_latency_ms,
        "live buy sent"
    );

    let mut msg = format_buy_notification(
        &Jupiter::new(),
        http,
        bot_wallet,
        trade,
        &quote,
        size_lamports,
        outcome.quote_latency_ms,
    )
    .await;
    msg.push_str(&format!("\n🚀 live tx: https://solscan.io/tx/{}", outcome.signature));
    notify(telegram, &msg).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_live_sell(
    db: &Db,
    http: &Http,
    executor: &LiveExecutor,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    mint: &Pubkey,
    session_id: Uuid,
    position: &Position,
    trade: &ObservedTrade,
) -> Result<()> {
    if !db.positions().try_claim_for_close(position.id).await? {
        info!(position_id = position.id, "claim lost; another task is closing");
        return Ok(());
    }

    let outcome = match executor
        .execute_swap(db, session_id, mint, wsol, position.entry_out_amount)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            db.positions().release_claim(position.id).await.ok();
            return Err(e);
        }
    };
    let quote = outcome.quote.clone();

    let dt_id = db
        .dry_trades()
        .insert(
            session_id,
            DryTradeRecord {
                copy_decision_id: None,
                input_mint: mint,
                output_mint: wsol,
                in_amount: position.entry_out_amount,
                slippage_bps: quote.slippage_bps,
                quote: Some(&quote),
                quote_latency_ms: outcome.quote_latency_ms,
                error: None,
            },
        )
        .await?;

    let realized_pnl_lamports = quote.out_amount as i64 - position.entry_in_lamports as i64;
    let realized_pnl_pct =
        realized_pnl_lamports as f64 / position.entry_in_lamports as f64 * 100.0;
    let closed = db
        .positions()
        .close(
            position.id,
            dt_id,
            ExitReason::TargetSellFollow,
            realized_pnl_lamports,
            realized_pnl_pct,
        )
        .await?;
    if !closed {
        info!(position_id = position.id, "close race lost; skipping notify");
        return Ok(());
    }

    update_mint_blocklist(db, position.mint, realized_pnl_lamports).await;
    if let Err(e) = executor.submit_close_ata(db, session_id, mint).await {
        warn!(mint = %mint, error = %e, "ATA close failed (rent stays locked)");
    }

    info!(
        signature = %outcome.signature,
        endpoint = %outcome.endpoint,
        position_id = position.id,
        pnl_sol = realized_pnl_lamports as f64 / 1e9,
        "live sell sent"
    );

    let mut msg = format_sell_notification(
        &Jupiter::new(),
        http,
        bot_wallet,
        trade,
        position,
        &quote,
        realized_pnl_lamports,
        realized_pnl_pct,
        outcome.quote_latency_ms,
    )
    .await;
    msg.push_str(&format!("\n🚀 live tx: https://solscan.io/tx/{}", outcome.signature));
    notify(telegram, &msg).await;
    Ok(())
}

async fn notify(tg: Option<&Telegram>, text: &str) {
    if let Some(tg) = tg
        && let Err(e) = tg.send(text).await
    {
        warn!(error = %e, "telegram send failed");
    }
}

async fn record_latency(
    db: &Db,
    session_id: Uuid,
    kind: LatencyKind,
    elapsed_ms: i32,
    success: bool,
    detail: Option<&str>,
) {
    if let Err(e) = db
        .latency_samples()
        .insert(session_id, kind, elapsed_ms, success, detail)
        .await
    {
        warn!(error = %e, kind = kind.as_str(), "latency sample insert failed");
    }
}

async fn update_mint_blocklist(db: &Db, mint: Pubkey, realized_pnl_lamports: i64) {
    let result = if realized_pnl_lamports < 0 {
        db.mint_blocklist().record_loss(mint).await.map(Some)
    } else {
        db.mint_blocklist().clear(mint).await.map(|_| None)
    };
    match result {
        Ok(Some(count)) => info!(mint = %mint, loss_count = count, "mint loss recorded"),
        Ok(None) => {}
        Err(e) => warn!(mint = %mint, error = %e, "mint blocklist update failed"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_trade(
    db: &Db,
    http: &Http,
    jupiter: &Jupiter,
    live: Option<&LiveExecutor>,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    mode: Mode,
    trade: &ObservedTrade,
    filter: &FilterParams,
    live_exit_in_flight: &Arc<AtomicU32>,
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
                db,
                http,
                jupiter,
                live,
                telegram,
                bot_wallet,
                wsol,
                session_id,
                mode,
                trade,
                trade_id,
                filter,
                live_exit_in_flight,
            )
            .await?
        }
        Side::Sell => {
            handle_sell(
                db,
                http,
                jupiter,
                live,
                telegram,
                bot_wallet,
                wsol,
                session_id,
                mode,
                trade,
                trade_id,
                live_exit_in_flight,
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
    live: Option<&LiveExecutor>,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    mode: Mode,
    trade: &ObservedTrade,
    trade_id: i64,
    filter: &FilterParams,
    live_exit_in_flight: &Arc<AtomicU32>,
) -> Result<()> {
    let Some(mint) = trade.mint else {
        let skip = CopyDecision::Skip(SkipReason::DecodeUncertain);
        db.copy_decisions().insert(session_id, trade_id, &skip).await?;
        info!(signature = %trade.signature, "skip decode_uncertain (no mint)");
        return Ok(());
    };

    if mode != Mode::Observe
        && db
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

    if db
        .mint_blocklist()
        .is_blocked(mint, MINT_BLOCK_LOSS_THRESHOLD, MINT_BLOCK_TTL_SECS)
        .await?
    {
        let skip = CopyDecision::Skip(SkipReason::MintBlocked);
        db.copy_decisions().insert(session_id, trade_id, &skip).await?;
        info!(signature = %trade.signature, mint = %mint, "skip mint_blocked");
        return Ok(());
    }

    if mode == Mode::Live && live_exit_in_flight.load(Ordering::SeqCst) > 0 {
        let skip = CopyDecision::Skip(SkipReason::ExitInProgress);
        db.copy_decisions().insert(session_id, trade_id, &skip).await?;
        info!(signature = %trade.signature, "skip exit_in_progress");
        return Ok(());
    }

    let (open_positions, daily_realized_pnl_lamports) = if mode == Mode::Observe {
        (0, 0)
    } else {
        (
            db.positions().count_open(session_id).await?,
            db.positions().realized_pnl_today().await?,
        )
    };
    let target_recent_pnl_lamports = db
        .observed_trades()
        .target_recent_pnl_lamports(&trade.target, COLD_STREAK_WINDOW_SECS)
        .await?;
    let ctx = FilterContext {
        open_positions,
        daily_realized_pnl_lamports,
        target_recent_pnl_lamports,
    };
    let dec = decision::decide(trade, filter, &ctx);
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
        mode = mode.as_str(),
        "copy"
    );

    if mode == Mode::Observe {
        return Ok(());
    }

    if mode == Mode::Live {
        let Some(executor) = live else {
            warn!("live mode without executor; skipping buy");
            return Ok(());
        };
        return execute_live_buy(
            db,
            http,
            executor,
            telegram,
            bot_wallet,
            wsol,
            &mint,
            session_id,
            dec_id,
            size_lamports,
            trade,
        )
        .await;
    }

    let started = Instant::now();
    let result = jupiter
        .quote(wsol, &mint, size_lamports, DRY_SLIPPAGE_BPS)
        .await;
    let quote_latency_ms = started.elapsed().as_millis() as i32;
    record_latency(
        db,
        session_id,
        LatencyKind::JupiterQuote,
        quote_latency_ms,
        result.is_ok(),
        result.as_ref().err().map(|e| e.to_string()).as_deref(),
    )
    .await;

    match result {
        Ok(quote) => {
            info!(
                in_amount = quote.in_amount,
                out_amount = quote.out_amount,
                price_impact_bps = quote.price_impact_bps,
                quote_latency_ms,
                route = ?quote.route_labels,
                "dry buy quote"
            );
            let dt_id = db
                .dry_trades()
                .insert(
                    session_id,
                    DryTradeRecord {
                        copy_decision_id: Some(dec_id),
                        input_mint: wsol,
                        output_mint: &mint,
                        in_amount: size_lamports,
                        slippage_bps: DRY_SLIPPAGE_BPS,
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
                    dt_id,
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
            warn!(error = %e, quote_latency_ms, "dry buy quote failed");
            let msg = e.to_string();
            db.dry_trades()
                .insert(
                    session_id,
                    DryTradeRecord {
                        copy_decision_id: Some(dec_id),
                        input_mint: wsol,
                        output_mint: &mint,
                        in_amount: size_lamports,
                        slippage_bps: DRY_SLIPPAGE_BPS,
                        quote: None,
                        quote_latency_ms,
                        error: Some(&msg),
                    },
                )
                .await?;
            notify(
                telegram,
                &format!("⚠️ dry buy quote failed\n{msg}\nlatency: {quote_latency_ms} ms"),
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
    live: Option<&LiveExecutor>,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    mode: Mode,
    trade: &ObservedTrade,
    trade_id: i64,
    live_exit_in_flight: &Arc<AtomicU32>,
) -> Result<()> {
    if mode == Mode::Observe {
        return Ok(());
    }
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

    if mode == Mode::Live {
        let Some(executor) = live else {
            warn!("live mode without executor; skipping sell");
            return Ok(());
        };
        let _guard = ExitInFlightGuard::new(live_exit_in_flight.clone());
        return execute_live_sell(
            db,
            http,
            executor,
            telegram,
            bot_wallet,
            wsol,
            &mint,
            session_id,
            &position,
            trade,
        )
        .await;
    }

    let started = Instant::now();
    let result = jupiter
        .quote(&mint, wsol, position.entry_out_amount, DRY_SLIPPAGE_BPS)
        .await;
    let quote_latency_ms = started.elapsed().as_millis() as i32;
    record_latency(
        db,
        session_id,
        LatencyKind::JupiterQuote,
        quote_latency_ms,
        result.is_ok(),
        result.as_ref().err().map(|e| e.to_string()).as_deref(),
    )
    .await;

    match result {
        Ok(quote) => {
            let dt_id = db
                .dry_trades()
                .insert(
                    session_id,
                    DryTradeRecord {
                        copy_decision_id: None,
                        input_mint: &mint,
                        output_mint: wsol,
                        in_amount: position.entry_out_amount,
                        slippage_bps: DRY_SLIPPAGE_BPS,
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
                    dt_id,
                    ExitReason::TargetSellFollow,
                    realized_pnl_lamports,
                    realized_pnl_pct,
                )
                .await?;
            if !closed {
                info!(position_id = position.id, "close race lost; skipping notify");
                return Ok(());
            }

            update_mint_blocklist(db, position.mint, realized_pnl_lamports).await;

            info!(
                position_id = position.id,
                exit_dry_trade_id = dt_id,
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
            warn!(error = %e, quote_latency_ms, "dry sell quote failed");
            let msg = e.to_string();
            db.dry_trades()
                .insert(
                    session_id,
                    DryTradeRecord {
                        copy_decision_id: None,
                        input_mint: &mint,
                        output_mint: wsol,
                        in_amount: position.entry_out_amount,
                        slippage_bps: DRY_SLIPPAGE_BPS,
                        quote: None,
                        quote_latency_ms,
                        error: Some(&msg),
                    },
                )
                .await?;
            notify(
                telegram,
                &format!(
                    "⚠️ dry sell quote failed\nposition: {}\n{msg}\nlatency: {quote_latency_ms} ms",
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
        None => return "💰 dry buy (mint unknown)".to_string(),
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
    live: Option<&LiveExecutor>,
    telegram: Option<&Telegram>,
    bot_wallet: &Pubkey,
    wsol: &Pubkey,
    session_id: Uuid,
    mode: Mode,
    params: &ExitParams,
    live_exit_in_flight: &Arc<AtomicU32>,
) -> Result<()> {
    let positions = db.positions().list_open(session_id).await?;
    for position in positions {
        let started = Instant::now();
        let quote_result = jupiter
            .quote(
                &position.mint,
                wsol,
                position.entry_out_amount,
                DRY_SLIPPAGE_BPS,
            )
            .await;
        let quote_latency_ms = started.elapsed().as_millis() as i32;
        record_latency(
            db,
            session_id,
            LatencyKind::JupiterQuote,
            quote_latency_ms,
            quote_result.is_ok(),
            quote_result.as_ref().err().map(|e| e.to_string()).as_deref(),
        )
        .await;

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
            mode = mode.as_str(),
            "exit triggered"
        );

        let (executed_quote, executed_signature, executed_quote_latency_ms) = if mode == Mode::Live
        {
            let Some(executor) = live else {
                warn!(position_id = position.id, "live mode without executor; aborting exit");
                continue;
            };
            if !db.positions().try_claim_for_close(position.id).await? {
                info!(position_id = position.id, "claim lost; another task is closing");
                continue;
            }
            let _guard = ExitInFlightGuard::new(live_exit_in_flight.clone());
            match executor
                .execute_swap(db, session_id, &position.mint, wsol, position.entry_out_amount)
                .await
            {
                Ok(outcome) => (
                    outcome.quote.clone(),
                    Some(outcome.signature),
                    outcome.quote_latency_ms,
                ),
                Err(e) => {
                    db.positions().release_claim(position.id).await.ok();
                    warn!(position_id = position.id, error = %e, "live exit send failed");
                    continue;
                }
            }
        } else {
            (quote.clone(), None, quote_latency_ms)
        };

        let dt_id = db
            .dry_trades()
            .insert(
                session_id,
                DryTradeRecord {
                    copy_decision_id: None,
                    input_mint: &position.mint,
                    output_mint: wsol,
                    in_amount: position.entry_out_amount,
                    slippage_bps: executed_quote.slippage_bps,
                    quote: Some(&executed_quote),
                    quote_latency_ms: executed_quote_latency_ms,
                    error: None,
                },
            )
            .await?;

        let realized_pnl_lamports =
            executed_quote.out_amount as i64 - position.entry_in_lamports as i64;
        let realized_pnl_pct =
            realized_pnl_lamports as f64 / position.entry_in_lamports as f64 * 100.0;

        let closed = db
            .positions()
            .close(position.id, dt_id, reason, realized_pnl_lamports, realized_pnl_pct)
            .await?;
        if !closed {
            info!(position_id = position.id, "close race lost; skipping notify");
            continue;
        }

        update_mint_blocklist(db, position.mint, realized_pnl_lamports).await;
        if mode == Mode::Live
            && let Some(executor) = live
            && let Err(e) = executor.submit_close_ata(db, session_id, &position.mint).await
        {
            warn!(mint = %position.mint, error = %e, "ATA close failed (rent stays locked)");
        }

        let mut msg = format_exit_notification(
            jupiter,
            http,
            bot_wallet,
            &position,
            &executed_quote,
            reason,
            realized_pnl_lamports,
            realized_pnl_pct,
            quote_latency_ms,
        )
        .await;
        if let Some(sig) = executed_signature {
            msg.push_str(&format!("\n🚀 live tx: https://solscan.io/tx/{sig}"));
        }
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
