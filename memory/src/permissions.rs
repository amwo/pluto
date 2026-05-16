use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    ReadOnly,
    ReadWrite,
}

impl Mode {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "read_only" => Ok(Self::ReadOnly),
            "read_write" => Ok(Self::ReadWrite),
            other => bail!("unknown permission mode: {other}"),
        }
    }

    pub fn writable(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

#[derive(Debug, Clone)]
pub struct Permissions {
    pub store_id: String,
    pub mode: Mode,
    pub owners: Vec<String>,
}

impl Permissions {
    pub fn load(store_dir: &Path) -> Result<Self> {
        let file = store_dir.join("permissions.yaml");
        let text = fs::read_to_string(&file)
            .with_context(|| format!("missing permissions file: {}", file.display()))?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self> {
        let mut store_id = None;
        let mut mode = None;
        let mut owners = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .with_context(|| format!("invalid permissions line: {line}"))?;
            let value = value.trim();
            match key.trim() {
                "store_id" => store_id = Some(value.to_string()),
                "mode" => mode = Some(Mode::parse(value)?),
                "owners" => owners = parse_list(value),
                other => bail!("unknown permissions key: {other}"),
            }
        }
        Ok(Self {
            store_id: store_id.context("permissions missing store_id")?,
            mode: mode.context("permissions missing mode")?,
            owners,
        })
    }
}

fn parse_list(raw: &str) -> Vec<String> {
    raw.trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}
