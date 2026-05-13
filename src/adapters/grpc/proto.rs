use std::collections::HashMap;

use anyhow::{Result, anyhow};
use yellowstone_grpc_proto::prelude::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions, SubscribeUpdate,
    SubscribeUpdateTransaction, subscribe_update::UpdateOneof,
};

use crate::domain::{Commitment, Pubkey, Signature, Slot, StreamEvent, Subscription};

pub(super) fn build_request(
    subscriptions: &[Subscription],
    commitment: Commitment,
) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    for (i, sub) in subscriptions.iter().enumerate() {
        match sub {
            Subscription::WalletTransactions(wallets) => {
                transactions.insert(format!("tx_{i}"), wallet_filter(wallets));
            }
        }
    }
    SubscribeRequest {
        transactions,
        commitment: Some(commitment_to_proto(commitment)),
        ..Default::default()
    }
}

fn wallet_filter(wallets: &[Pubkey]) -> SubscribeRequestFilterTransactions {
    SubscribeRequestFilterTransactions {
        vote: Some(false),
        failed: Some(false),
        signature: None,
        account_include: wallets.iter().map(Pubkey::to_string).collect(),
        account_exclude: vec![],
        account_required: vec![],
    }
}

fn commitment_to_proto(c: Commitment) -> i32 {
    match c {
        Commitment::Processed => CommitmentLevel::Processed as i32,
        Commitment::Confirmed => CommitmentLevel::Confirmed as i32,
        Commitment::Finalized => CommitmentLevel::Finalized as i32,
    }
}

pub(super) fn map_update(update: SubscribeUpdate) -> Option<Result<StreamEvent>> {
    match update.update_oneof {
        Some(UpdateOneof::Transaction(t)) => Some(decode_tx(t)),
        Some(UpdateOneof::Ping(_)) | Some(UpdateOneof::Pong(_)) => Some(Ok(StreamEvent::Heartbeat)),
        Some(_) | None => None,
    }
}

fn decode_tx(t: SubscribeUpdateTransaction) -> Result<StreamEvent> {
    let slot = Slot::from(t.slot);
    let bytes = t
        .transaction
        .ok_or_else(|| anyhow!("tx update missing transaction body"))?
        .signature;
    let signature = Signature::try_from_slice(&bytes)?;
    Ok(StreamEvent::Tx { slot, signature })
}
