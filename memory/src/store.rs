use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use similar::TextDiff;
use walkdir::WalkDir;

use crate::audit::{self, AuditEntry};
use crate::hashing::content_hash;
use crate::paths::Layout;
use crate::permissions::Permissions;

pub struct Memory {
    layout: Layout,
}

pub struct WriteOutcome {
    pub before_hash: String,
    pub after_hash: String,
}

impl Memory {
    pub fn open(layout: Layout) -> Self {
        Self { layout }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn permissions(&self, store: &str) -> Result<Permissions> {
        Permissions::load(&self.layout.store_dir(store))
    }

    fn entry_path(&self, store: &str, rel: &str) -> Result<PathBuf> {
        if Path::new(rel).is_absolute() || rel.split('/').any(|part| part == "..") {
            bail!("invalid memory path: {rel}");
        }
        Ok(self.layout.store_dir(store).join(rel))
    }

    pub fn read(&self, store: &str, rel: &str) -> Result<String> {
        let path = self.entry_path(store, rel)?;
        fs::read_to_string(&path).with_context(|| format!("read memory entry: {}", path.display()))
    }

    pub fn current_hash(&self, store: &str, rel: &str) -> Result<String> {
        let path = self.entry_path(store, rel)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(content_hash(&bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(err) => Err(err).with_context(|| format!("hash entry: {}", path.display())),
        }
    }

    pub fn list(&self, store: &str) -> Result<Vec<String>> {
        let base = self.layout.store_dir(store);
        let mut out = Vec::new();
        for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(&base) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        out.sort();
        Ok(out)
    }

    pub fn write(
        &self,
        store: &str,
        rel: &str,
        content: &str,
        if_hash: Option<&str>,
        agent_id: &str,
        session_id: &str,
    ) -> Result<WriteOutcome> {
        if !self.permissions(store)?.mode.writable() {
            bail!("store '{store}' is read_only");
        }
        let before = self.current_hash(store, rel)?;
        if let Some(expected) = if_hash
            && expected != before
        {
            bail!(
                "write conflict on {store}/{rel}: precondition {expected}, actual {}",
                if before.is_empty() { "<absent>" } else { &before },
            );
        }
        let before_content = if before.is_empty() {
            String::new()
        } else {
            self.read(store, rel).unwrap_or_default()
        };
        let path = self.entry_path(store, rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content)?;
        let after = content_hash(content.as_bytes());

        audit::store_object(&self.layout, &before, &before_content)?;
        audit::store_object(&self.layout, &after, content)?;
        audit::append(
            &self.layout,
            &AuditEntry {
                ts: audit::now(),
                store: store.to_string(),
                path: rel.to_string(),
                agent_id: agent_id.to_string(),
                session_id: session_id.to_string(),
                before_hash: before.clone(),
                after_hash: after.clone(),
                diff: unified_diff(&before_content, content, rel),
            },
        )?;
        Ok(WriteOutcome {
            before_hash: before,
            after_hash: after,
        })
    }

    pub fn delete(&self, store: &str, rel: &str, agent_id: &str, session_id: &str) -> Result<()> {
        if !self.permissions(store)?.mode.writable() {
            bail!("store '{store}' is read_only");
        }
        let before = self.current_hash(store, rel)?;
        if before.is_empty() {
            return Ok(());
        }
        let before_content = self.read(store, rel).unwrap_or_default();
        fs::remove_file(self.entry_path(store, rel)?)?;
        audit::store_object(&self.layout, &before, &before_content)?;
        audit::append(
            &self.layout,
            &AuditEntry {
                ts: audit::now(),
                store: store.to_string(),
                path: rel.to_string(),
                agent_id: agent_id.to_string(),
                session_id: session_id.to_string(),
                before_hash: before,
                after_hash: String::new(),
                diff: unified_diff(&before_content, "", rel),
            },
        )?;
        Ok(())
    }
}

pub fn unified_diff(before: &str, after: &str, label: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let body = diff.unified_diff().to_string();
    if body.is_empty() {
        String::new()
    } else {
        format!("--- a/{label}\n+++ b/{label}\n{body}")
    }
}
