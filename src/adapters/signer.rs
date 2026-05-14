use anyhow::{Context, Result};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer as SdkSignerTrait;

use crate::domain::Pubkey;

pub struct Signer {
    keypair: Keypair,
    pubkey: Pubkey,
}

impl Signer {
    pub fn from_base58_secret(secret: &str) -> Result<Self> {
        let bytes = bs58::decode(secret)
            .into_vec()
            .context("invalid base58 secret key")?;
        let keypair = Keypair::try_from(bytes.as_slice())
            .map_err(|e| anyhow::anyhow!("invalid solana keypair bytes: {e}"))?;
        let pubkey_bytes: [u8; 32] = keypair.pubkey().to_bytes();
        Ok(Self {
            keypair,
            pubkey: Pubkey::from(pubkey_bytes),
        })
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    pub fn sdk_pubkey(&self) -> solana_sdk::pubkey::Pubkey {
        self.keypair.pubkey()
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.keypair.sign_message(message).into()
    }

    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }
}
