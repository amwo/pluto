use time::OffsetDateTime;

use crate::domain::position::{ExitReason, Position};

#[derive(Clone, Copy, Debug)]
pub struct ExitParams {
    pub stop_loss_pct: f64,
    pub trailing_arm_pct: f64,
    pub trailing_stop_pct: f64,
    pub max_hold_seconds: i64,
    pub hard_max_hold_seconds: i64,
}

impl Default for ExitParams {
    fn default() -> Self {
        Self {
            stop_loss_pct: 18.0,
            trailing_arm_pct: 8.0,
            trailing_stop_pct: 8.0,
            max_hold_seconds: 3600,
            hard_max_hold_seconds: 5400,
        }
    }
}

pub fn should_exit(
    position: &Position,
    current_price: f64,
    now: OffsetDateTime,
    params: &ExitParams,
) -> Option<ExitReason> {
    let age_secs = (now - position.opened_at).whole_seconds();

    if age_secs >= params.hard_max_hold_seconds {
        return Some(ExitReason::HardMaxHold);
    }

    let stop_floor = position.entry_price * (1.0 - params.stop_loss_pct / 100.0);
    if current_price <= stop_floor {
        return Some(ExitReason::StopLoss);
    }

    let armed = position.peak_price
        >= position.entry_price * (1.0 + params.trailing_arm_pct / 100.0);
    let trailing_floor = position.peak_price * (1.0 - params.trailing_stop_pct / 100.0);
    if armed && current_price <= trailing_floor {
        return Some(ExitReason::TrailingStop);
    }

    if age_secs >= params.max_hold_seconds {
        return Some(ExitReason::MaxHold);
    }

    None
}
