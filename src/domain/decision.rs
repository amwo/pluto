use crate::domain::trade::{DexKind, ObservedTrade, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkipReason {
    TooLarge,
    TooSmall,
    UnknownRoute,
    StaleDetection,
    ExistingPosition,
    HighPriceImpact,
    RiskLimit,
    DecodeUncertain,
    RateLimited,
    NotABuy,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::TooLarge => "too_large",
            SkipReason::TooSmall => "too_small",
            SkipReason::UnknownRoute => "unknown_route",
            SkipReason::StaleDetection => "stale_detection",
            SkipReason::ExistingPosition => "existing_position",
            SkipReason::HighPriceImpact => "high_price_impact",
            SkipReason::RiskLimit => "risk_limit",
            SkipReason::DecodeUncertain => "decode_uncertain",
            SkipReason::RateLimited => "rate_limited",
            SkipReason::NotABuy => "not_a_buy",
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
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            max_target_buy_lamports: 700_000_000,
            min_target_buy_lamports: 50_000_000,
            copy_ratio_bps: 2500,
            max_copy_lamports: 150_000_000,
        }
    }
}

pub fn decide(trade: &ObservedTrade, params: &FilterParams) -> CopyDecision {
    match trade.side {
        Side::Unknown => return CopyDecision::Skip(SkipReason::DecodeUncertain),
        Side::Sell => return CopyDecision::Skip(SkipReason::NotABuy),
        Side::Buy => {}
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
