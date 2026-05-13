use crate::domain::solana::Pubkey;
use crate::domain::trade::ObservedTrade;

#[derive(Debug)]
pub enum Subscription {
    WalletTransactions(Vec<Pubkey>),
}

#[derive(Debug)]
pub enum StreamEvent {
    Trade(Box<ObservedTrade>),
    Heartbeat,
}
