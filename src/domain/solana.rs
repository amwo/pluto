use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid base58: {0}")]
    Base58(String),
    #[error("expected {expected} bytes, got {actual}")]
    WrongLength { expected: usize, actual: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Slot(u64);

impl Slot {
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<u64> for Slot {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature([u8; 64]);

#[allow(dead_code)]
impl Signature {
    pub fn try_from_slice(b: &[u8]) -> Result<Self, ParseError> {
        b.try_into().map(Self).map_err(|_| ParseError::WrongLength {
            expected: 64,
            actual: b.len(),
        })
    }

    pub fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl From<[u8; 64]> for Signature {
    fn from(v: [u8; 64]) -> Self {
        Self(v)
    }
}

impl std::fmt::Display for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

impl std::fmt::Debug for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Signature({self})")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pubkey([u8; 32]);

#[allow(dead_code)]
impl Pubkey {
    pub fn from_base58(s: &str) -> Result<Self, ParseError> {
        let mut bytes = [0u8; 32];
        let n = bs58::decode(s)
            .onto(&mut bytes)
            .map_err(|e| ParseError::Base58(e.to_string()))?;
        if n != 32 {
            return Err(ParseError::WrongLength {
                expected: 32,
                actual: n,
            });
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl From<[u8; 32]> for Pubkey {
    fn from(v: [u8; 32]) -> Self {
        Self(v)
    }
}

impl std::fmt::Display for Pubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

impl std::fmt::Debug for Pubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pubkey({self})")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Commitment {
    Processed,
    Confirmed,
    Finalized,
}
