use crate::domain::solana::{Signature, Slot};

#[derive(Clone, Copy, Debug)]
pub struct DetectedTx {
    pub slot: Slot,
    pub signature: Signature,
}
