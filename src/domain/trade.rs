use crate::domain::solana::{Pubkey, Signature, Slot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
    Unknown,
}

impl Side {
    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
            Side::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DexKind {
    PumpFun,
    PumpSwap,
    RaydiumAmmV4,
    RaydiumCpmm,
    RaydiumClmm,
    Bonk,
    Jupiter,
    Other,
}

impl DexKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DexKind::PumpFun => "pumpfun",
            DexKind::PumpSwap => "pumpswap",
            DexKind::RaydiumAmmV4 => "raydium_amm_v4",
            DexKind::RaydiumCpmm => "raydium_cpmm",
            DexKind::RaydiumClmm => "raydium_clmm",
            DexKind::Bonk => "bonk",
            DexKind::Jupiter => "jupiter",
            DexKind::Other => "other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ObservedTrade {
    pub signature: Signature,
    pub slot: Slot,
    pub block_time: Option<i64>,
    pub target: Pubkey,
    pub side: Side,
    pub mint: Option<Pubkey>,
    pub sol_delta_lamports: i64,
    pub token_delta: i128,
    pub route: Vec<DexKind>,
    pub jupiter: bool,
    pub pump_swap: bool,
    pub jito_marker: bool,
    pub priority_fee_lamports: u64,
    pub compute_unit_limit: Option<u32>,
}
