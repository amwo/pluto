use std::io::Read;

use anyhow::Result;
use clap::{Parser, Subcommand};
use memory::bootstrap;
use memory::dreaming;
use memory::history;
use memory::paths::Layout;
use memory::store::Memory;

#[derive(Parser)]
#[command(name = "memctl", about = "self-learning agent memory control")]
struct Cli {
    #[arg(long, default_value = "memory", env = "MEMORY_ROOT")]
    root: String,
    #[arg(long, default_value = "operator", env = "MEMORY_AGENT_ID")]
    agent: String,
    #[arg(long, default_value = "manual", env = "MEMORY_SESSION_ID")]
    session: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
    Read {
        store: String,
        path: String,
    },
    Hash {
        store: String,
        path: String,
    },
    List {
        store: String,
    },
    Write {
        store: String,
        path: String,
        #[arg(long)]
        if_hash: Option<String>,
        #[arg(long)]
        file: Option<String>,
    },
    ListVersions {
        store: String,
        path: String,
    },
    Checkout {
        store: String,
        path: String,
        hash: String,
    },
    Apply {
        job_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let layout = Layout::new(&cli.root);
    let memory = Memory::open(layout);

    match cli.command {
        Command::Init => {
            bootstrap::init(memory.layout())?;
            println!("initialized memory root at {}", cli.root);
        }
        Command::Read { store, path } => {
            print!("{}", memory.read(&store, &path)?);
        }
        Command::Hash { store, path } => {
            println!("{}", memory.current_hash(&store, &path)?);
        }
        Command::List { store } => {
            for entry in memory.list(&store)? {
                println!("{entry}");
            }
        }
        Command::Write {
            store,
            path,
            if_hash,
            file,
        } => {
            let content = match file {
                Some(f) => std::fs::read_to_string(f)?,
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };
            let outcome = memory.write(
                &store,
                &path,
                &content,
                if_hash.as_deref(),
                &cli.agent,
                &cli.session,
            )?;
            println!("{}", outcome.after_hash);
        }
        Command::ListVersions { store, path } => {
            for entry in history::list_versions(&memory, &store, &path)? {
                println!(
                    "{}  {}  {}  {}",
                    entry.ts, entry.agent_id, entry.session_id, entry.after_hash
                );
            }
        }
        Command::Checkout { store, path, hash } => {
            history::checkout(&memory, &store, &path, &hash, &cli.agent, &cli.session)?;
            println!("checked out {store}/{path}@{hash}");
        }
        Command::Apply { job_id } => {
            let applied = dreaming::apply(&memory, &job_id)?;
            println!("applied {applied} operations from {job_id}");
        }
    }
    Ok(())
}
