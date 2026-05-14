use anyhow::{Context, Result, bail};
use ed25519_dalek::{Signer as DalekSigner, SigningKey, VerifyingKey};

use crate::domain::Pubkey;

pub struct Signer {
    signing_key: SigningKey,
    pubkey: Pubkey,
}

impl Signer {
    pub fn from_base58_secret(secret: &str) -> Result<Self> {
        let mut bytes = [0u8; 64];
        let n = bs58::decode(secret)
            .onto(&mut bytes[..])
            .context("invalid base58 secret key")?;
        if n != 64 {
            bail!("expected 64-byte secret key, got {n}");
        }
        let signing_key = SigningKey::from_keypair_bytes(&bytes)
            .context("invalid ed25519 keypair bytes")?;
        let verifying: VerifyingKey = signing_key.verifying_key();
        let pubkey = Pubkey::from(verifying.to_bytes());
        Ok(Self { signing_key, pubkey })
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }
}
