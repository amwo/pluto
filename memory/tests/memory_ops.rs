use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use memory::bootstrap;
use memory::history;
use memory::paths::Layout;
use memory::store::Memory;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn fresh() -> Memory {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir: PathBuf = std::env::temp_dir().join(format!("memtest-ops-{nanos}-{n}"));
    let _ = fs::remove_dir_all(&dir);
    let layout = Layout::new(dir);
    bootstrap::init(&layout).unwrap();
    Memory::open(layout)
}

#[test]
fn optimistic_concurrency_rejects_stale_write() {
    let memory = fresh();
    let h1 = memory
        .write("team_sre", "notes/n.md", "v1", None, "a", "s")
        .unwrap()
        .after_hash;

    assert!(
        memory
            .write("team_sre", "notes/n.md", "v2", Some("deadbeef"), "a", "s")
            .is_err(),
        "stale precondition hash must be rejected"
    );

    let h2 = memory
        .write("team_sre", "notes/n.md", "v2", Some(&h1), "a", "s")
        .unwrap()
        .after_hash;
    assert_ne!(h1, h2);
    assert_eq!(memory.read("team_sre", "notes/n.md").unwrap(), "v2");
}

#[test]
fn read_only_store_rejects_write() {
    let memory = fresh();
    assert!(
        memory
            .write("org_knowledge", "notes/n.md", "x", None, "a", "s")
            .is_err()
    );
}

#[test]
fn path_traversal_is_rejected() {
    let memory = fresh();
    assert!(
        memory
            .write("team_sre", "../escape.md", "x", None, "a", "s")
            .is_err()
    );
}

#[test]
fn history_checkout_restores_old_version() {
    let memory = fresh();
    let h1 = memory
        .write("team_sre", "notes/n.md", "v1", None, "a", "s")
        .unwrap()
        .after_hash;
    memory
        .write("team_sre", "notes/n.md", "v2", Some(&h1), "a", "s")
        .unwrap();

    let versions = history::list_versions(&memory, "team_sre", "notes/n.md").unwrap();
    assert_eq!(versions.len(), 2);

    history::checkout(&memory, "team_sre", "notes/n.md", &h1, "a", "s").unwrap();
    assert_eq!(memory.read("team_sre", "notes/n.md").unwrap(), "v1");
}
