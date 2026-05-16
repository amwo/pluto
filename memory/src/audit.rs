use std::fs;
use std::io::Write;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::paths::Layout;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub store: String,
    pub path: String,
    pub agent_id: String,
    pub session_id: String,
    pub before_hash: String,
    pub after_hash: String,
    pub diff: String,
}

pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn append(layout: &Layout, entry: &AuditEntry) -> Result<()> {
    let log = layout.audit_log();
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .with_context(|| format!("open audit log: {}", log.display()))?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

pub fn read_all(layout: &Layout) -> Result<Vec<AuditEntry>> {
    let log = layout.audit_log();
    if !log.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&log)?;
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        entries.push(serde_json::from_str(line)?);
    }
    Ok(entries)
}

pub fn store_object(layout: &Layout, hash: &str, content: &str) -> Result<()> {
    if hash.is_empty() {
        return Ok(());
    }
    let dir = layout.objects_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(hash);
    if !path.exists() {
        fs::write(&path, content)?;
    }
    Ok(())
}

pub fn load_object(layout: &Layout, hash: &str) -> Result<String> {
    let path = layout.objects_dir().join(hash);
    fs::read_to_string(&path).with_context(|| format!("unknown object hash: {hash}"))
}
