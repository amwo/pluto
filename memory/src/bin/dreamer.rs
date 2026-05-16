use anyhow::Result;
use clap::Parser;
use memory::audit;
use memory::dreaming;
use memory::paths::Layout;
use memory::store::Memory;

#[derive(Parser)]
#[command(name = "dreamer", about = "out-of-band memory dreaming runner")]
struct Cli {
    #[arg(long, default_value = "memory", env = "MEMORY_ROOT")]
    root: String,
    #[arg(long, default_value = "team_sre")]
    store: String,
    #[arg(long)]
    since: Option<String>,
    #[arg(long)]
    job_id: Option<String>,
    #[arg(long)]
    apply: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let memory = Memory::open(Layout::new(&cli.root));
    let job_id = cli
        .job_id
        .unwrap_or_else(|| format!("job-{}", audit::now().replace([':', '.'], "-")));

    let result = dreaming::run(&memory, &cli.store, &job_id, cli.since.as_deref())?;
    dreaming::write_job(memory.layout(), &result)?;

    let job_dir = memory.layout().job_dir(&result.job_id);
    println!(
        "dreaming job {} written ({} operations)",
        result.job_id,
        result.proposal.operations.len()
    );
    println!("  proposal: {}", job_dir.join("proposal.json").display());
    println!("  report:   {}", job_dir.join("report.md").display());

    if cli.apply {
        let applied = dreaming::apply(&memory, &result.job_id)?;
        println!("applied {applied} operations");
    } else {
        println!("review then apply: memctl apply {}", result.job_id);
    }
    Ok(())
}
