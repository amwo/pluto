use std::collections::{HashMap, HashSet};

use anyhow::Result;
use yellowstone_grpc_proto::prelude::{
    CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions, SubscribeUpdate,
    subscribe_update::UpdateOneof,
};

use super::decode;
use crate::domain::{Commitment, Pubkey, StreamEvent, Subscription};

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

pub(super) fn collect_targets(subscriptions: &[Subscription]) -> HashSet<Pubkey> {
    let mut s = HashSet::new();
    for sub in subscriptions {
        match sub {
            Subscription::WalletTransactions(wallets) => {
                s.extend(wallets.iter().copied());
            }
        }
    }
    s
}

pub(super) fn map_update(
    update: SubscribeUpdate,
    targets: &HashSet<Pubkey>,
) -> Option<Result<StreamEvent>> {
    match update.update_oneof {
        Some(UpdateOneof::Transaction(t)) => {
            let trade = decode::decode(t, targets)?;
            Some(Ok(StreamEvent::Trade(Box::new(trade))))
        }
        Some(UpdateOneof::Ping(_)) | Some(UpdateOneof::Pong(_)) => Some(Ok(StreamEvent::Heartbeat)),
        Some(_) | None => None,
    }
}
