pub mod adapters;
pub mod config;
pub mod domain;

mod app;

pub use app::{report, run};
pub use config::Config;

pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,h2=warn,hyper=warn,rustls=warn".into()),
        )
        .with_target(false)
        .init();
}
