use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::hashing::content_hash;
use crate::paths::Layout;
use crate::session::{Event, Session, load_all};
use crate::store::{Memory, unified_diff};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    Write {
        store: String,
        path: String,
        content: String,
        reason: String,
    },
    Delete {
        store: String,
        path: String,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub job_id: String,
    pub generated_at: String,
    pub input_sessions: Vec<String>,
    pub operations: Vec<Operation>,
}

pub struct JobResult {
    pub job_id: String,
    pub proposal: Proposal,
    pub report: String,
    pub diff: String,
}

fn group_by_session<F>(sessions: &[&Session], extract: F) -> Vec<(String, Vec<String>)>
where
    F: Fn(&Event) -> Option<String>,
{
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for session in sessions {
        for event in &session.events {
            if let Some(key) = extract(event) {
                let bucket = map.entry(key).or_default();
                if !bucket.contains(&session.meta.session_id) {
                    bucket.push(session.meta.session_id.clone());
                }
            }
        }
    }
    map.into_iter().collect()
}

pub fn run(memory: &Memory, store: &str, job_id: &str, since: Option<&str>) -> Result<JobResult> {
    let sessions = load_all(memory.layout())?;
    let relevant: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.meta.store_refs.iter().any(|r| r == store))
        .filter(|s| since.is_none_or(|t| s.meta.started_at.as_str() >= t))
        .collect();
    let input_sessions: Vec<String> = relevant
        .iter()
        .map(|s| s.meta.session_id.clone())
        .collect();

    let cross_patterns: Vec<(String, Vec<String>)> = group_by_session(&relevant, |event| {
        match event {
            Event::Message { role, text } if role == "observation" => Some(text.clone()),
            _ => None,
        }
    })
    .into_iter()
    .filter(|(_, seen)| seen.len() >= 2)
    .collect();

    let repeated_failures: Vec<(String, Vec<String>)> = group_by_session(&relevant, |event| {
        match event {
            Event::ToolCall { tool, status, .. } if status == "error" => Some(tool.clone()),
            _ => None,
        }
    })
    .into_iter()
    .filter(|(_, seen)| seen.len() >= 2)
    .collect();

    let refuted: Vec<String> = relevant
        .iter()
        .flat_map(|s| s.events.iter())
        .filter_map(|event| match event {
            Event::Message { role, text } if role == "refute" => Some(text.clone()),
            _ => None,
        })
        .collect();

    let notes = memory.list(store)?;
    let mut by_content: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for note in &notes {
        let key = content_hash(memory.read(store, note).unwrap_or_default().trim().as_bytes());
        by_content.entry(key).or_default().push(note.clone());
    }
    let duplicates: Vec<Vec<String>> = by_content
        .into_values()
        .filter(|paths| paths.len() >= 2)
        .collect();

    let mut operations: Vec<Operation> = Vec::new();
    let mut report = String::new();
    report.push_str(&format!("# Dreaming レポート: {job_id}\n\n"));
    report.push_str(&format!("- 対象ストア: `{store}`\n"));
    report.push_str(&format!("- 入力セッション: {}\n", input_sessions.join(", ")));

    if !cross_patterns.is_empty() || !repeated_failures.is_empty() {
        let mut patterns = String::from("# 横断パターン (Dreaming 由来)\n\n");
        report.push_str("\n## 横断パターン\n");
        for (text, seen) in &cross_patterns {
            patterns.push_str(&format!(
                "- 観測: {text}\n  - 確認セッション: {}\n",
                seen.join(", ")
            ));
            report.push_str(&format!("- {text} ({} セッションで観測)\n", seen.len()));
        }
        for (tool, seen) in &repeated_failures {
            patterns.push_str(&format!(
                "- 反復失敗: ツール `{tool}` が {} セッションで失敗\n  - 該当: {}\n",
                seen.len(),
                seen.join(", ")
            ));
            report.push_str(&format!(
                "- ツール `{tool}` の反復失敗 ({} セッション)\n",
                seen.len()
            ));
        }
        operations.push(Operation::Write {
            store: store.to_string(),
            path: "notes/patterns.md".to_string(),
            content: patterns,
            reason: "複数セッション共通のパターン/失敗を集約".to_string(),
        });
    }

    let mut deleted: BTreeSet<String> = BTreeSet::new();
    report.push_str("\n## 整理\n");
    for note in &notes {
        if refuted.iter().any(|r| r == note) && deleted.insert(note.clone()) {
            operations.push(Operation::Delete {
                store: store.to_string(),
                path: note.clone(),
                reason: "トランスクリプトで否定された stale エントリ".to_string(),
            });
            report.push_str(&format!("- stale 削除: `{note}`\n"));
        }
    }
    for group in &duplicates {
        for dup in group.iter().skip(1) {
            if deleted.insert(dup.clone()) {
                operations.push(Operation::Delete {
                    store: store.to_string(),
                    path: dup.clone(),
                    reason: format!("`{}` と同一内容の重複", group[0]),
                });
                report.push_str(&format!(
                    "- 重複統合: `{dup}` を削除 (`{}` に集約)\n",
                    group[0]
                ));
            }
        }
    }

    let verify_label = input_sessions.join(", ");
    for note in &notes {
        if deleted.contains(note) || note == "notes/patterns.md" {
            continue;
        }
        let current = memory.read(store, note).unwrap_or_default();
        if current.contains("verified:") {
            continue;
        }
        operations.push(Operation::Write {
            store: store.to_string(),
            path: note.clone(),
            content: format!(
                "{}\n\n---\nverified: セッション {verify_label} で再検証済み (job {job_id})\n",
                current.trim_end()
            ),
            reason: "検証ノートを追記".to_string(),
        });
    }
    report.push_str(&format!(
        "\n- 提案オペレーション総数: {}\n",
        operations.len()
    ));

    let mut diff = String::new();
    for op in &operations {
        let (path, before, after) = match op {
            Operation::Write {
                store: s,
                path,
                content,
                ..
            } => (
                path.clone(),
                memory.read(s, path).unwrap_or_default(),
                content.clone(),
            ),
            Operation::Delete { store: s, path, .. } => (
                path.clone(),
                memory.read(s, path).unwrap_or_default(),
                String::new(),
            ),
        };
        diff.push_str(&unified_diff(&before, &after, &path));
    }

    let proposal = Proposal {
        job_id: job_id.to_string(),
        generated_at: audit::now(),
        input_sessions,
        operations,
    };
    Ok(JobResult {
        job_id: job_id.to_string(),
        proposal,
        report,
        diff,
    })
}

pub fn write_job(layout: &Layout, result: &JobResult) -> Result<()> {
    let dir = layout.job_dir(&result.job_id);
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join("input_sessions.json"),
        serde_json::to_string_pretty(&result.proposal.input_sessions)?,
    )?;
    fs::write(
        dir.join("proposal.json"),
        serde_json::to_string_pretty(&result.proposal)?,
    )?;
    fs::write(dir.join("diff.patch"), &result.diff)?;
    fs::write(dir.join("report.md"), &result.report)?;
    Ok(())
}

pub fn apply(memory: &Memory, job_id: &str) -> Result<usize> {
    let path = memory.layout().job_dir(job_id).join("proposal.json");
    let proposal: Proposal = serde_json::from_str(&fs::read_to_string(&path)?)?;
    for op in &proposal.operations {
        match op {
            Operation::Write {
                store,
                path,
                content,
                ..
            } => {
                memory.write(store, path, content, None, "dreamer", job_id)?;
            }
            Operation::Delete { store, path, .. } => {
                memory.delete(store, path, "dreamer", job_id)?;
            }
        }
    }
    Ok(proposal.operations.len())
}
