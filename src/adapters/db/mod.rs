pub mod copy_decisions;
pub mod dry_trades;
pub mod latency_samples;
pub mod live_send_attempts;
pub mod mint_blocklist;
pub mod observed_trades;
pub mod positions;
pub mod reports;
pub mod sessions;

use anyhow::Result;
use sqlx::PgPool;

pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPool::connect(database_url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn sessions(&self) -> sessions::Sessions<'_> {
        sessions::Sessions::new(&self.pool)
    }

    pub fn observed_trades(&self) -> observed_trades::ObservedTrades<'_> {
        observed_trades::ObservedTrades::new(&self.pool)
    }

    pub fn copy_decisions(&self) -> copy_decisions::CopyDecisions<'_> {
        copy_decisions::CopyDecisions::new(&self.pool)
    }

    pub fn dry_trades(&self) -> dry_trades::DryTrades<'_> {
        dry_trades::DryTrades::new(&self.pool)
    }

    pub fn positions(&self) -> positions::Positions<'_> {
        positions::Positions::new(&self.pool)
    }

    pub fn mint_blocklist(&self) -> mint_blocklist::MintBlocklist<'_> {
        mint_blocklist::MintBlocklist::new(&self.pool)
    }

    pub fn latency_samples(&self) -> latency_samples::LatencySamples<'_> {
        latency_samples::LatencySamples::new(&self.pool)
    }

    pub fn live_send_attempts(&self) -> live_send_attempts::LiveSendAttempts<'_> {
        live_send_attempts::LiveSendAttempts::new(&self.pool)
    }

    pub fn reports(&self) -> reports::Reports<'_> {
        reports::Reports::new(&self.pool)
    }
}
