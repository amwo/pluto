use std::collections::HashMap;
use std::sync::OnceLock;

use crate::domain::solana::Pubkey;
use crate::domain::trade::DexKind;

const PUMPFUN: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const PUMPSWAP: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const RAYDIUM_AMM_V4: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
const RAYDIUM_CPMM: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
const RAYDIUM_CLMM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const BONK: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
const JUPITER_V6: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";

const JITO_DONTFRONT_PREFIX: &str = "jitodontfront";

fn registry() -> &'static HashMap<[u8; 32], DexKind> {
    static MAP: OnceLock<HashMap<[u8; 32], DexKind>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        for (b58, kind) in [
            (PUMPFUN, DexKind::PumpFun),
            (PUMPSWAP, DexKind::PumpSwap),
            (RAYDIUM_AMM_V4, DexKind::RaydiumAmmV4),
            (RAYDIUM_CPMM, DexKind::RaydiumCpmm),
            (RAYDIUM_CLMM, DexKind::RaydiumClmm),
            (BONK, DexKind::Bonk),
            (JUPITER_V6, DexKind::Jupiter),
        ] {
            let pk = Pubkey::from_base58(b58).expect("known program id");
            m.insert(*pk.as_bytes(), kind);
        }
        m
    })
}

pub fn classify(program_id: &Pubkey) -> DexKind {
    *registry()
        .get(program_id.as_bytes())
        .unwrap_or(&DexKind::Other)
}

pub fn is_compute_budget(program_id: &Pubkey) -> bool {
    static CB: OnceLock<Pubkey> = OnceLock::new();
    let cb = CB.get_or_init(|| Pubkey::from_base58(COMPUTE_BUDGET).expect("compute budget"));
    program_id.as_bytes() == cb.as_bytes()
}

pub fn looks_like_jito_marker(account: &Pubkey) -> bool {
    account.to_string().starts_with(JITO_DONTFRONT_PREFIX)
}
