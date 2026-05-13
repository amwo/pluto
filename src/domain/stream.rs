use crate::domain::solana::{Pubkey, Signature, Slot};

#[derive(Debug)]
pub enum Subscription {
    WalletTransactions(Vec<Pubkey>),
}

#[derive(Debug)]
pub enum StreamEvent {
    Tx { slot: Slot, signature: Signature },
    Heartbeat,
}
