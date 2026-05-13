use crate::domain::solana::Pubkey;

#[derive(Clone, Debug)]
pub struct Quote {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub in_amount: u64,
    pub out_amount: u64,
    pub other_amount_threshold: u64,
    pub price_impact_bps: i32,
    pub slippage_bps: u32,
    pub route_labels: Vec<String>,
}
