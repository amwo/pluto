pub mod copy_decisions;
pub mod observed_trades;
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

    pub fn reports(&self) -> reports::Reports<'_> {
        reports::Reports::new(&self.pool)
    }
}
