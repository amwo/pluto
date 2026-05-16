use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Layout;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub agent_id: String,
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub store_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Message {
        role: String,
        text: String,
    },
    ToolCall {
        tool: String,
        #[serde(default)]
        input: String,
        status: String,
        #[serde(default)]
        output: String,
    },
}

#[derive(Debug, Clone)]
pub struct Session {
    pub meta: SessionMeta,
    pub events: Vec<Event>,
}

pub fn load_all(layout: &Layout) -> Result<Vec<Session>> {
    let dir = layout.sessions_dir();
    let mut sessions = Vec::new();
    if !dir.exists() {
        return Ok(sessions);
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let meta_path = entry.path().join("meta.json");
        if !meta_path.exists() {
            continue;
        }
        let meta: SessionMeta = serde_json::from_str(&fs::read_to_string(&meta_path)?)
            .with_context(|| format!("parse {}", meta_path.display()))?;
        let transcript_path = entry.path().join("transcript.jsonl");
        let mut events = Vec::new();
        if transcript_path.exists() {
            for line in fs::read_to_string(&transcript_path)?.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                events.push(serde_json::from_str(line)?);
            }
        }
        sessions.push(Session { meta, events });
    }
    sessions.sort_by(|a, b| a.meta.started_at.cmp(&b.meta.started_at));
    Ok(sessions)
}
