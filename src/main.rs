use anyhow::Result;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    pluto::init();
    pluto::run(pluto::Config::from_env()?).await
}
