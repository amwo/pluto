use anyhow::{Context, Result};

use crate::audit::{self, AuditEntry};
use crate::store::Memory;

pub fn list_versions(memory: &Memory, store: &str, rel: &str) -> Result<Vec<AuditEntry>> {
    Ok(audit::read_all(memory.layout())?
        .into_iter()
        .filter(|entry| entry.store == store && entry.path == rel)
        .collect())
}

pub fn checkout(
    memory: &Memory,
    store: &str,
    rel: &str,
    hash: &str,
    agent_id: &str,
    session_id: &str,
) -> Result<()> {
    let content = audit::load_object(memory.layout(), hash)
        .with_context(|| format!("checkout {store}/{rel}@{hash}"))?;
    memory.write(store, rel, &content, None, agent_id, session_id)?;
    Ok(())
}
