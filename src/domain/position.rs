use time::OffsetDateTime;
use uuid::Uuid;

use crate::domain::solana::Pubkey;

#[derive(Clone, Debug)]
pub struct Position {
    pub id: i64,
    pub session_id: Uuid,
    pub mint: Pubkey,
    pub opened_at: OffsetDateTime,
    pub entry_dry_trade_id: i64,
    pub entry_in_lamports: u64,
    pub entry_out_amount: u64,
    pub entry_price: f64,
    pub peak_price: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionStatus {
    Open,
    Closed,
    Crashed,
}

impl PositionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PositionStatus::Open => "open",
            PositionStatus::Closed => "closed",
            PositionStatus::Crashed => "crashed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitReason {
    TargetSellFollow,
    StopLoss,
    TrailingStop,
    MaxHold,
    HardMaxHold,
}

impl ExitReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExitReason::TargetSellFollow => "target_sell_follow",
            ExitReason::StopLoss => "stop_loss",
            ExitReason::TrailingStop => "trailing_stop",
            ExitReason::MaxHold => "max_hold",
            ExitReason::HardMaxHold => "hard_max_hold",
        }
    }
}
