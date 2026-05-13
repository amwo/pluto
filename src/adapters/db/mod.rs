pub mod sessions;

use anyhow::Result;

pub use sqlx::PgPool as Pool;

pub async fn connect(database_url: &str) -> Result<Pool> {
    let pool = Pool::connect(database_url).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}
