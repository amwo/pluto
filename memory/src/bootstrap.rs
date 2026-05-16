use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::paths::Layout;

const P1_RUNBOOK: &str = "# P1 Dispatch アラート Runbook\n\n\
- オーナー: platform-team\n\
- SLO: 5 分以内に一次対応\n\n\
## 一次対応\n\
1. CPU 使用率を確認する\n\
2. 直近の PR / デプロイを確認する\n\
3. team_sre/notes を先に読んでから調査を開始する\n";

const CRON: &str = "jobs:\n\
\x20 - name: nightly-dreaming\n\
\x20   schedule: \"0 3 * * *\"\n\
\x20   store: team_sre\n\
\x20   apply: false\n\
\x20   command: \"dreamer --store team_sre\"\n";

pub fn init(layout: &Layout) -> Result<()> {
    for dir in [
        layout.stores_dir(),
        layout.sessions_dir(),
        layout.jobs_dir(),
        layout.objects_dir(),
    ] {
        fs::create_dir_all(dir)?;
    }

    store(layout, "org_knowledge", "read_only", "platform-team")?;
    store(layout, "team_sre", "read_write", "sre-team")?;
    store(layout, "codebase", "read_write", "pluto-dev")?;

    fs::create_dir_all(layout.store_dir("team_sre").join("notes"))?;
    fs::create_dir_all(layout.store_dir("org_knowledge").join("runbooks"))?;
    write_if_absent(
        &layout
            .store_dir("org_knowledge")
            .join("runbooks")
            .join("p1-dispatch.md"),
        P1_RUNBOOK,
    )?;
    write_if_absent(
        &layout.store_dir("org_knowledge").join("README.md"),
        "# org_knowledge\n\n読み取り専用ストア。runbook・SLO・オーナー情報を置く。\n",
    )?;
    write_if_absent(&layout.dreaming_dir().join("cron.yaml"), CRON)?;
    Ok(())
}

fn store(layout: &Layout, id: &str, mode: &str, owner: &str) -> Result<()> {
    fs::create_dir_all(layout.store_dir(id))?;
    write_if_absent(
        &layout.store_dir(id).join("permissions.yaml"),
        &format!("store_id: {id}\nmode: {mode}\nowners: [{owner}]\n"),
    )
}

fn write_if_absent(path: &Path, content: &str) -> Result<()> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    Ok(())
}
