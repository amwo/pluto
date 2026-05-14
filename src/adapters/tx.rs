use anyhow::{Result, bail};
use base64::Engine;

use crate::adapters::signer::Signer;

#[derive(Clone, Debug)]
pub struct SignedTx {
    pub tx_b64: String,
    pub signature_b58: String,
}

pub fn sign_versioned_tx_b64(tx_b64: &str, signer: &Signer) -> Result<SignedTx> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(tx_b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("base64 decode: {e}"))?;
    if bytes.is_empty() {
        bail!("empty tx bytes");
    }

    let (sig_count, sig_header_len) = decode_short_vec_u16(&bytes)?;
    if sig_count == 0 {
        bail!("tx has no signature slots");
    }
    if sig_count != 1 {
        bail!(
            "tx requires {sig_count} signers but pluto only provides 1; \
             multi-signer txs (Jupiter Z / RFQ) are not supported in self-managed live mode"
        );
    }
    let msg_start = sig_header_len + (sig_count as usize) * 64;
    if msg_start > bytes.len() {
        bail!("tx truncated before message body");
    }

    let signature = signer.sign(&bytes[msg_start..]);

    let mut signed = bytes.clone();
    signed[sig_header_len..sig_header_len + 64].copy_from_slice(&signature);

    Ok(SignedTx {
        tx_b64: base64::engine::general_purpose::STANDARD.encode(&signed),
        signature_b58: bs58::encode(signature).into_string(),
    })
}

fn decode_short_vec_u16(bytes: &[u8]) -> Result<(u16, usize)> {
    let mut value: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &b) in bytes.iter().enumerate().take(3) {
        value |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 == 0 {
            if value > u16::MAX as u32 {
                bail!("short_vec overflow");
            }
            return Ok((value as u16, i + 1));
        }
        shift += 7;
    }
    bail!("short_vec malformed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_vec_one_byte() {
        let (v, len) = decode_short_vec_u16(&[0x01, 0x00]).unwrap();
        assert_eq!(v, 1);
        assert_eq!(len, 1);
    }

    #[test]
    fn short_vec_two_byte() {
        let (v, len) = decode_short_vec_u16(&[0x80, 0x01, 0x00]).unwrap();
        assert_eq!(v, 128);
        assert_eq!(len, 2);
    }

    #[test]
    fn short_vec_zero() {
        let (v, len) = decode_short_vec_u16(&[0x00]).unwrap();
        assert_eq!(v, 0);
        assert_eq!(len, 1);
    }
}
