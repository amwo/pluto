use anyhow::{Context, Result};
use base64::Engine;
use solana_sdk::hash::Hash;
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::message::v0::Message as MessageV0;
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey as SdkPubkey;
#[allow(deprecated)]
use solana_sdk::system_instruction;
use solana_sdk::transaction::VersionedTransaction;
use std::str::FromStr;

use crate::adapters::signer::Signer;
use crate::adapters::tip::jitodontfront_program;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const SPL_TOKEN_CLOSE_ACCOUNT_DISCRIMINATOR: u8 = 9;

pub fn build_tip_and_dontfront_tx(
    payer: &Signer,
    tip_account: &SdkPubkey,
    tip_lamports: u64,
    blockhash: Hash,
) -> Result<String> {
    let payer_pk = payer.sdk_pubkey();
    let tip_ix = system_instruction::transfer(&payer_pk, tip_account, tip_lamports);
    let dontfront_ix = Instruction {
        program_id: jitodontfront_program(),
        accounts: vec![],
        data: vec![],
    };
    let message = MessageV0::try_compile(&payer_pk, &[tip_ix, dontfront_ix], &[], blockhash)
        .context("compile tip+dontfront message")?;
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[payer.keypair()])
        .context("sign tip+dontfront tx")?;
    let bytes = bincode::serialize(&tx).context("serialize tip+dontfront tx")?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

pub fn build_close_ata_tx(
    payer: &Signer,
    ata: &SdkPubkey,
    blockhash: Hash,
) -> Result<String> {
    let payer_pk = payer.sdk_pubkey();
    let token_program = SdkPubkey::from_str(TOKEN_PROGRAM).expect("token program");
    let close_ix = Instruction {
        program_id: token_program,
        accounts: vec![
            AccountMeta::new(*ata, false),
            AccountMeta::new(payer_pk, false),
            AccountMeta::new_readonly(payer_pk, true),
        ],
        data: vec![SPL_TOKEN_CLOSE_ACCOUNT_DISCRIMINATOR],
    };
    let message = MessageV0::try_compile(&payer_pk, &[close_ix], &[], blockhash)
        .context("compile close-ata message")?;
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[payer.keypair()])
        .context("sign close-ata tx")?;
    let bytes = bincode::serialize(&tx).context("serialize close-ata tx")?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

pub fn derive_ata(owner: &SdkPubkey, mint: &SdkPubkey) -> SdkPubkey {
    let token_program = SdkPubkey::from_str(TOKEN_PROGRAM).expect("token program");
    let ata_program = SdkPubkey::from_str(ATA_PROGRAM).expect("ata program");
    SdkPubkey::find_program_address(
        &[
            owner.as_ref(),
            token_program.as_ref(),
            mint.as_ref(),
        ],
        &ata_program,
    )
    .0
}

pub fn signature_b58_from_b64_tx(tx_b64: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(tx_b64.as_bytes())
        .context("base64 decode")?;
    let tx: VersionedTransaction =
        bincode::deserialize(&bytes).context("deserialize tx")?;
    let sig = tx
        .signatures
        .first()
        .context("tx has no signatures")?;
    Ok(bs58::encode(sig).into_string())
}
