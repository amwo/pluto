use crate::domain::trade::{DexKind, ObservedTrade, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkipReason {
    TooLarge,
    TooSmall,
    UnknownRoute,
    StaleDetection,
    ExistingPosition,
    MintBlocked,
    MaxOpenPositions,
    DailyLossLimit,
    ExitInProgress,
    HighPriceImpact,
    InsufficientBalance,
    PriorityFeeAnomaly,
    TargetColdStreak,
    RiskLimit,
    DecodeUncertain,
    RateLimited,
    NotABuy,
    NoOpenPosition,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::TooLarge => "too_large",
            SkipReason::TooSmall => "too_small",
            SkipReason::UnknownRoute => "unknown_route",
            SkipReason::StaleDetection => "stale_detection",
            SkipReason::ExistingPosition => "existing_position",
            SkipReason::MintBlocked => "mint_blocked",
            SkipReason::MaxOpenPositions => "max_open_positions",
            SkipReason::DailyLossLimit => "daily_loss_limit",
            SkipReason::ExitInProgress => "exit_in_progress",
            SkipReason::HighPriceImpact => "high_price_impact",
            SkipReason::InsufficientBalance => "insufficient_balance",
            SkipReason::PriorityFeeAnomaly => "priority_fee_anomaly",
            SkipReason::TargetColdStreak => "target_cold_streak",
            SkipReason::RiskLimit => "risk_limit",
            SkipReason::DecodeUncertain => "decode_uncertain",
            SkipReason::RateLimited => "rate_limited",
            SkipReason::NotABuy => "not_a_buy",
            SkipReason::NoOpenPosition => "no_open_position",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CopyDecision {
    Copy { size_lamports: u64 },
    Skip(SkipReason),
}

impl CopyDecision {
    pub fn action(&self) -> &'static str {
        match self {
            CopyDecision::Copy { .. } => "copy",
            CopyDecision::Skip(_) => "skip",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FilterParams {
    pub max_target_buy_lamports: u64,
    pub min_target_buy_lamports: u64,
    pub copy_ratio_bps: u32,
    pub max_copy_lamports: u64,
    pub max_detection_delay_ms: i32,
    pub max_open_positions: u32,
    pub max_daily_loss_lamports: u64,
    pub max_priority_fee_lamports: u64,
    pub target_cold_streak_threshold_lamports: i64,
    pub balance_buffer_lamports: u64,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            max_target_buy_lamports: 2_500_000_000,
            min_target_buy_lamports: 50_000_000,
            copy_ratio_bps: 1000,
            max_copy_lamports: 200_000_000,
            max_detection_delay_ms: 5_000,
            max_open_positions: 4,
            max_daily_loss_lamports: 900_000_000,
            max_priority_fee_lamports: 5_000_000,
            target_cold_streak_threshold_lamports: i64::MIN,
            balance_buffer_lamports: 15_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FilterContext {
    pub open_positions: u32,
    pub daily_realized_pnl_lamports: i64,
    pub target_recent_pnl_lamports: i64,
    pub wallet_balance_lamports: u64,
}

pub fn decide(
    trade: &ObservedTrade,
    params: &FilterParams,
    ctx: &FilterContext,
) -> CopyDecision {
    match trade.side {
        Side::Unknown => return CopyDecision::Skip(SkipReason::DecodeUncertain),
        Side::Sell => return CopyDecision::Skip(SkipReason::NotABuy),
        Side::Buy => {}
    }

    if let Some(delay) = trade.detection_delay_ms
        && delay > params.max_detection_delay_ms
    {
        return CopyDecision::Skip(SkipReason::StaleDetection);
    }

    if ctx.daily_realized_pnl_lamports
        <= -i64::try_from(params.max_daily_loss_lamports).unwrap_or(i64::MAX)
    {
        return CopyDecision::Skip(SkipReason::DailyLossLimit);
    }

    if ctx.open_positions >= params.max_open_positions {
        return CopyDecision::Skip(SkipReason::MaxOpenPositions);
    }

    if ctx.target_recent_pnl_lamports < params.target_cold_streak_threshold_lamports {
        return CopyDecision::Skip(SkipReason::TargetColdStreak);
    }

    if trade.priority_fee_lamports > params.max_priority_fee_lamports {
        return CopyDecision::Skip(SkipReason::PriorityFeeAnomaly);
    }

    let target_sol = trade.sol_delta_lamports.unsigned_abs();

    if target_sol > params.max_target_buy_lamports {
        return CopyDecision::Skip(SkipReason::TooLarge);
    }
    if target_sol < params.min_target_buy_lamports {
        return CopyDecision::Skip(SkipReason::TooSmall);
    }

    let supported = trade
        .route
        .iter()
        .any(|d| matches!(d, DexKind::Jupiter | DexKind::PumpSwap | DexKind::PumpFun));
    if !supported {
        return CopyDecision::Skip(SkipReason::UnknownRoute);
    }

    let size_lamports = (target_sol * params.copy_ratio_bps as u64 / 10_000)
        .min(params.max_copy_lamports);

    let required_balance = size_lamports.saturating_add(params.balance_buffer_lamports);
    if ctx.wallet_balance_lamports < required_balance {
        return CopyDecision::Skip(SkipReason::InsufficientBalance);
    }

    CopyDecision::Copy { size_lamports }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::solana::{Pubkey, Signature, Slot};

    fn rich_ctx() -> FilterContext {
        FilterContext {
            open_positions: 0,
            daily_realized_pnl_lamports: 0,
            target_recent_pnl_lamports: 0,
            wallet_balance_lamports: 10_000_000_000,
        }
    }

    fn buy_trade() -> ObservedTrade {
        ObservedTrade {
            signature: Signature::from([0u8; 64]),
            slot: Slot::from(0),
            block_time: None,
            target: Pubkey::from([0u8; 32]),
            side: Side::Buy,
            mint: Some(Pubkey::from([1u8; 32])),
            sol_delta_lamports: -100_000_000,
            token_delta: 1_000,
            route: vec![DexKind::Jupiter],
            jupiter: true,
            pump_swap: false,
            jito_marker: false,
            priority_fee_lamports: 0,
            compute_unit_limit: None,
            detection_delay_ms: Some(100),
        }
    }

    #[test]
    fn copy_when_all_pass() {
        let dec = decide(&buy_trade(), &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_stale_when_delay_exceeds_limit() {
        let mut t = buy_trade();
        t.detection_delay_ms = Some(5_001);
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::StaleDetection)));
    }

    #[test]
    fn allow_when_delay_unknown() {
        let mut t = buy_trade();
        t.detection_delay_ms = None;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_max_open_at_cap() {
        let dec = decide(
            &buy_trade(),
            &FilterParams::default(),
            &FilterContext { open_positions: 4, daily_realized_pnl_lamports: 0, target_recent_pnl_lamports: 0, wallet_balance_lamports: 10_000_000_000 },
        );
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::MaxOpenPositions)));
    }

    #[test]
    fn copy_just_below_cap() {
        let dec = decide(
            &buy_trade(),
            &FilterParams::default(),
            &FilterContext { open_positions: 3, daily_realized_pnl_lamports: 0, target_recent_pnl_lamports: 0, wallet_balance_lamports: 10_000_000_000 },
        );
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn copy_at_delay_boundary() {
        let mut t = buy_trade();
        t.detection_delay_ms = Some(5_000);
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_too_large() {
        let mut t = buy_trade();
        t.sol_delta_lamports = -3_000_000_000;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::TooLarge)));
    }

    #[test]
    fn skip_too_small() {
        let mut t = buy_trade();
        t.sol_delta_lamports = -10_000_000;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::TooSmall)));
    }

    #[test]
    fn skip_unknown_route() {
        let mut t = buy_trade();
        t.route = vec![DexKind::RaydiumAmmV4];
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::UnknownRoute)));
    }

    #[test]
    fn skip_not_a_buy() {
        let mut t = buy_trade();
        t.side = Side::Sell;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::NotABuy)));
    }

    #[test]
    fn skip_decode_uncertain() {
        let mut t = buy_trade();
        t.side = Side::Unknown;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::DecodeUncertain)));
    }

    #[test]
    fn skip_daily_loss_limit_exceeded() {
        let mut ctx = rich_ctx();
        ctx.daily_realized_pnl_lamports = -900_000_001;
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::DailyLossLimit)));
    }

    #[test]
    fn copy_at_daily_loss_boundary() {
        let mut ctx = rich_ctx();
        ctx.daily_realized_pnl_lamports = -899_999_999;
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_at_daily_loss_exact_limit() {
        let mut ctx = rich_ctx();
        ctx.daily_realized_pnl_lamports = -900_000_000;
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::DailyLossLimit)));
    }

    #[test]
    fn skip_priority_fee_anomaly() {
        let mut t = buy_trade();
        t.priority_fee_lamports = 5_000_001;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::PriorityFeeAnomaly)));
    }

    #[test]
    fn copy_at_priority_fee_boundary() {
        let mut t = buy_trade();
        t.priority_fee_lamports = 5_000_000;
        let dec = decide(&t, &FilterParams::default(), &rich_ctx());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn cold_streak_disabled_by_default() {
        let mut ctx = rich_ctx();
        ctx.target_recent_pnl_lamports = -1_000_000_000_000;
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn cold_streak_fires_when_threshold_set() {
        let params = FilterParams {
            target_cold_streak_threshold_lamports: -300_000_000,
            ..FilterParams::default()
        };
        let mut ctx = rich_ctx();
        ctx.target_recent_pnl_lamports = -300_000_001;
        let dec = decide(&buy_trade(), &params, &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::TargetColdStreak)));
    }

    #[test]
    fn skip_insufficient_balance() {
        let mut ctx = rich_ctx();
        ctx.wallet_balance_lamports = 1_000_000;
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::InsufficientBalance)));
    }

    #[test]
    fn balance_just_enough_passes() {
        let params = FilterParams::default();
        let target_sol = 100_000_000u64;
        let copy_size = target_sol * params.copy_ratio_bps as u64 / 10_000;
        let required = copy_size + params.balance_buffer_lamports;
        let mut ctx = rich_ctx();
        ctx.wallet_balance_lamports = required;
        let dec = decide(&buy_trade(), &params, &ctx);
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }
}
