use anyhow::Result;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    pluto::init();
    let cfg = pluto::Config::from_env()?;
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("report") => pluto::report(cfg, args.get(2).cloned()).await,
        _ => pluto::run(cfg).await,
    }
}
