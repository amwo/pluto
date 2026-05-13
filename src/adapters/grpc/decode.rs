use std::collections::HashSet;

use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, InnerInstructions, SubscribeUpdateTransaction, TokenBalance,
    TransactionStatusMeta,
};

use crate::domain::dex_registry;
use crate::domain::{DexKind, ObservedTrade, Pubkey, Side, Signature, Slot};

pub(super) fn decode(
    update: SubscribeUpdateTransaction,
    targets: &HashSet<Pubkey>,
) -> Option<ObservedTrade> {
    let slot = Slot::from(update.slot);
    let info = update.transaction?;
    let signature = Signature::try_from_slice(&info.signature).ok()?;
    let tx = info.transaction?;
    let msg = tx.message?;
    let meta = info.meta?;

    let accounts = build_accounts(&msg.account_keys, &meta)?;
    let target_idx = accounts.iter().position(|p| targets.contains(p))?;
    let target = accounts[target_idx];

    let sol_delta = sol_delta(target_idx, &meta);
    let (mint, token_delta) = token_delta_for_owner(&target, &meta);
    let side = classify_side(sol_delta, token_delta);
    let (route, jupiter, pump_swap) = extract_route(&msg.instructions, &meta.inner_instructions, &accounts);
    let jito_marker = accounts.iter().any(dex_registry::looks_like_jito_marker);
    let (priority_fee_lamports, compute_unit_limit) =
        extract_compute_budget(&msg.instructions, &accounts);

    Some(ObservedTrade {
        signature,
        slot,
        block_time: None,
        target,
        side,
        mint,
        sol_delta_lamports: sol_delta,
        token_delta,
        route,
        jupiter,
        pump_swap,
        jito_marker,
        priority_fee_lamports,
        compute_unit_limit,
    })
}

fn build_accounts(base: &[Vec<u8>], meta: &TransactionStatusMeta) -> Option<Vec<Pubkey>> {
    let mut acc = Vec::with_capacity(
        base.len() + meta.loaded_writable_addresses.len() + meta.loaded_readonly_addresses.len(),
    );
    for k in base
        .iter()
        .chain(meta.loaded_writable_addresses.iter())
        .chain(meta.loaded_readonly_addresses.iter())
    {
        let bytes: [u8; 32] = k.as_slice().try_into().ok()?;
        acc.push(Pubkey::from(bytes));
    }
    Some(acc)
}

fn sol_delta(target_idx: usize, meta: &TransactionStatusMeta) -> i64 {
    let pre = *meta.pre_balances.get(target_idx).unwrap_or(&0) as i128;
    let post = *meta.post_balances.get(target_idx).unwrap_or(&0) as i128;
    let mut delta = post - pre;
    if target_idx == 0 {
        delta += meta.fee as i128;
    }
    delta as i64
}

fn token_delta_for_owner(target: &Pubkey, meta: &TransactionStatusMeta) -> (Option<Pubkey>, i128) {
    let owner = target.to_string();
    let mut mints: HashSet<&str> = HashSet::new();
    for tb in meta.pre_token_balances.iter().chain(meta.post_token_balances.iter()) {
        if tb.owner == owner {
            mints.insert(tb.mint.as_str());
        }
    }
    for m in mints {
        let pre = amount_for(m, &owner, &meta.pre_token_balances);
        let post = amount_for(m, &owner, &meta.post_token_balances);
        let delta = post - pre;
        if delta != 0 {
            return (Pubkey::from_base58(m).ok(), delta);
        }
    }
    (None, 0)
}

fn amount_for(mint: &str, owner: &str, balances: &[TokenBalance]) -> i128 {
    balances
        .iter()
        .find(|tb| tb.mint == mint && tb.owner == owner)
        .and_then(|tb| tb.ui_token_amount.as_ref())
        .map(|u| u.amount.parse::<i128>().unwrap_or(0))
        .unwrap_or(0)
}

fn classify_side(sol: i64, token: i128) -> Side {
    match (sol.signum(), token.signum()) {
        (-1, 1) => Side::Buy,
        (1, -1) => Side::Sell,
        _ => Side::Unknown,
    }
}

fn extract_route(
    outer: &[CompiledInstruction],
    inner: &[InnerInstructions],
    accounts: &[Pubkey],
) -> (Vec<DexKind>, bool, bool) {
    let mut route = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |idx: u32| {
        if let Some(pk) = accounts.get(idx as usize) {
            let kind = dex_registry::classify(pk);
            if kind != DexKind::Other && seen.insert(kind) {
                route.push(kind);
            }
        }
    };
    for ci in outer {
        push(ci.program_id_index);
    }
    for ii in inner {
        for inst in &ii.instructions {
            push(inst.program_id_index);
        }
    }
    let jupiter = route.contains(&DexKind::Jupiter);
    let pump_swap = route.contains(&DexKind::PumpSwap);
    (route, jupiter, pump_swap)
}

fn extract_compute_budget(
    instructions: &[CompiledInstruction],
    accounts: &[Pubkey],
) -> (u64, Option<u32>) {
    let mut micro_lamports: u64 = 0;
    let mut cu_limit: Option<u32> = None;
    for ci in instructions {
        let Some(pk) = accounts.get(ci.program_id_index as usize) else {
            continue;
        };
        if !dex_registry::is_compute_budget(pk) {
            continue;
        }
        match ci.data.first() {
            Some(2) if ci.data.len() >= 5 => {
                let bytes: [u8; 4] = ci.data[1..5].try_into().unwrap_or([0; 4]);
                cu_limit = Some(u32::from_le_bytes(bytes));
            }
            Some(3) if ci.data.len() >= 9 => {
                let bytes: [u8; 8] = ci.data[1..9].try_into().unwrap_or([0; 8]);
                micro_lamports = u64::from_le_bytes(bytes);
            }
            _ => {}
        }
    }
    let priority_fee_lamports = match cu_limit {
        Some(limit) => (micro_lamports as u128 * limit as u128 / 1_000_000) as u64,
        None => 0,
    };
    (priority_fee_lamports, cu_limit)
}
