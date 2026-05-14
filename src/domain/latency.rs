#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LatencyKind {
    JupiterQuote,
    JupiterSwap,
    JitoSend,
    JitoConfirm,
}

impl LatencyKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            LatencyKind::JupiterQuote => "jupiter_quote",
            LatencyKind::JupiterSwap => "jupiter_swap",
            LatencyKind::JitoSend => "jito_send",
            LatencyKind::JitoConfirm => "jito_confirm",
        }
    }
}
