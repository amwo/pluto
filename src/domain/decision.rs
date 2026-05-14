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
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            max_target_buy_lamports: 700_000_000,
            min_target_buy_lamports: 50_000_000,
            copy_ratio_bps: 2500,
            max_copy_lamports: 150_000_000,
            max_detection_delay_ms: 5_000,
            max_open_positions: 4,
            max_daily_loss_lamports: 900_000_000,
            max_priority_fee_lamports: 5_000_000,
            target_cold_streak_threshold_lamports: -300_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FilterContext {
    pub open_positions: u32,
    pub daily_realized_pnl_lamports: i64,
    pub target_recent_pnl_lamports: i64,
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
    CopyDecision::Copy { size_lamports }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::solana::{Pubkey, Signature, Slot};

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
        let dec = decide(&buy_trade(), &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_stale_when_delay_exceeds_limit() {
        let mut t = buy_trade();
        t.detection_delay_ms = Some(5_001);
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::StaleDetection)));
    }

    #[test]
    fn allow_when_delay_unknown() {
        let mut t = buy_trade();
        t.detection_delay_ms = None;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_max_open_at_cap() {
        let dec = decide(
            &buy_trade(),
            &FilterParams::default(),
            &FilterContext { open_positions: 4, daily_realized_pnl_lamports: 0, target_recent_pnl_lamports: 0 },
        );
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::MaxOpenPositions)));
    }

    #[test]
    fn copy_just_below_cap() {
        let dec = decide(
            &buy_trade(),
            &FilterParams::default(),
            &FilterContext { open_positions: 3, daily_realized_pnl_lamports: 0, target_recent_pnl_lamports: 0 },
        );
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn copy_at_delay_boundary() {
        let mut t = buy_trade();
        t.detection_delay_ms = Some(5_000);
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_too_large() {
        let mut t = buy_trade();
        t.sol_delta_lamports = -800_000_000;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::TooLarge)));
    }

    #[test]
    fn skip_too_small() {
        let mut t = buy_trade();
        t.sol_delta_lamports = -10_000_000;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::TooSmall)));
    }

    #[test]
    fn skip_unknown_route() {
        let mut t = buy_trade();
        t.route = vec![DexKind::RaydiumAmmV4];
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::UnknownRoute)));
    }

    #[test]
    fn skip_not_a_buy() {
        let mut t = buy_trade();
        t.side = Side::Sell;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::NotABuy)));
    }

    #[test]
    fn skip_decode_uncertain() {
        let mut t = buy_trade();
        t.side = Side::Unknown;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::DecodeUncertain)));
    }

    #[test]
    fn skip_daily_loss_limit_exceeded() {
        let ctx = FilterContext {
            open_positions: 0,
            daily_realized_pnl_lamports: -900_000_001,
            target_recent_pnl_lamports: 0,
        };
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::DailyLossLimit)));
    }

    #[test]
    fn copy_at_daily_loss_boundary() {
        let ctx = FilterContext {
            open_positions: 0,
            daily_realized_pnl_lamports: -899_999_999,
            target_recent_pnl_lamports: 0,
        };
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_at_daily_loss_exact_limit() {
        let ctx = FilterContext {
            open_positions: 0,
            daily_realized_pnl_lamports: -900_000_000,
            target_recent_pnl_lamports: 0,
        };
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::DailyLossLimit)));
    }

    #[test]
    fn skip_priority_fee_anomaly() {
        let mut t = buy_trade();
        t.priority_fee_lamports = 5_000_001;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::PriorityFeeAnomaly)));
    }

    #[test]
    fn copy_at_priority_fee_boundary() {
        let mut t = buy_trade();
        t.priority_fee_lamports = 5_000_000;
        let dec = decide(&t, &FilterParams::default(), &FilterContext::default());
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }

    #[test]
    fn skip_target_cold_streak() {
        let ctx = FilterContext {
            open_positions: 0,
            daily_realized_pnl_lamports: 0,
            target_recent_pnl_lamports: -300_000_001,
        };
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Skip(SkipReason::TargetColdStreak)));
    }

    #[test]
    fn copy_at_cold_streak_boundary() {
        let ctx = FilterContext {
            open_positions: 0,
            daily_realized_pnl_lamports: 0,
            target_recent_pnl_lamports: -300_000_000,
        };
        let dec = decide(&buy_trade(), &FilterParams::default(), &ctx);
        assert!(matches!(dec, CopyDecision::Copy { .. }));
    }
}
