use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use memory::audit;
use memory::bootstrap;
use memory::dreaming;
use memory::paths::Layout;
use memory::session::{Event, SessionMeta};
use memory::store::Memory;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_root() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("memtest-sre-{nanos}-{n}"));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn write_session(layout: &Layout, meta: &SessionMeta, events: &[Event]) {
    let dir = layout.sessions_dir().join(&meta.session_id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("meta.json"), serde_json::to_string(meta).unwrap()).unwrap();
    let mut transcript = String::new();
    for event in events {
        transcript.push_str(&serde_json::to_string(event).unwrap());
        transcript.push('\n');
    }
    fs::write(dir.join("transcript.jsonl"), transcript).unwrap();
}

fn msg(role: &str, text: &str) -> Event {
    Event::Message {
        role: role.into(),
        text: text.into(),
    }
}

fn tool(name: &str, status: &str) -> Event {
    Event::ToolCall {
        tool: name.into(),
        input: String::new(),
        status: status.into(),
        output: String::new(),
    }
}

fn tool_calls(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::ToolCall { .. }))
        .count()
}

#[test]
fn sre_scenario_end_to_end() {
    let root = temp_root();
    let layout = Layout::new(&root);
    bootstrap::init(&layout).unwrap();
    let memory = Memory::open(layout.clone());

    assert!(
        memory
            .write("org_knowledge", "runbooks/x.md", "x", None, "a", "s")
            .is_err(),
        "org_knowledge must reject writes"
    );

    let observation = "dispatch アラートは上流 CPU spike の約60秒後に再発する";

    let note_a = "# dispatch triage\n\n\
- CPU spike を確認\n\
- short-circuit: 60秒待って再 spike を確認すれば再調査は不要\n";
    memory
        .write(
            "team_sre",
            "notes/dispatch.md",
            note_a,
            None,
            "sre-agent-a",
            "session-a",
        )
        .unwrap();
    let events_a = vec![
        msg("user", "P1 dispatch alert"),
        tool("fetch-cpu", "ok"),
        tool("list-prs", "ok"),
        tool("describe-pods", "error"),
        tool("describe-pods", "error"),
        tool("fetch-cpu", "ok"),
        msg("observation", observation),
    ];
    write_session(
        &layout,
        &SessionMeta {
            session_id: "session-a".into(),
            agent_id: "sre-agent-a".into(),
            started_at: "2026-05-15T09:00:00Z".into(),
            ended_at: Some("2026-05-15T09:08:00Z".into()),
            store_refs: vec!["org_knowledge".into(), "team_sre".into()],
        },
        &events_a,
    );

    let shared = memory.read("team_sre", "notes/dispatch.md").unwrap();
    assert!(
        shared.contains("short-circuit"),
        "agent B reads agent A's note"
    );
    let events_b = vec![
        msg("user", "P1 dispatch alert"),
        tool("describe-pods", "error"),
        msg("observation", observation),
        msg("refute", "notes/old-knowledge.md"),
    ];
    write_session(
        &layout,
        &SessionMeta {
            session_id: "session-b".into(),
            agent_id: "sre-agent-b".into(),
            started_at: "2026-05-15T09:30:00Z".into(),
            ended_at: Some("2026-05-15T09:33:00Z".into()),
            store_refs: vec!["org_knowledge".into(), "team_sre".into()],
        },
        &events_b,
    );

    assert!(
        tool_calls(&events_b) < tool_calls(&events_a),
        "shared memory cuts agent B's tool calls"
    );

    let dup = "# 重複メモ\n\nupstream CPU spike を確認すること\n";
    for i in 1..=5 {
        memory
            .write(
                "team_sre",
                &format!("notes/dup-{i}.md"),
                dup,
                None,
                "seed",
                "session-seed",
            )
            .unwrap();
    }
    memory
        .write(
            "team_sre",
            "notes/old-knowledge.md",
            "# 古い知識\n\ndispatch アラートは無視してよい\n",
            None,
            "seed",
            "session-seed",
        )
        .unwrap();

    let job = dreaming::run(&memory, "team_sre", "job-test", None).unwrap();
    dreaming::write_job(&layout, &job).unwrap();

    assert_eq!(job.proposal.input_sessions, vec!["session-a", "session-b"]);
    let report = fs::read_to_string(layout.job_dir("job-test").join("report.md")).unwrap();
    assert!(report.contains("60秒"), "report mentions the cross-session pattern");
    assert!(layout.job_dir("job-test").join("diff.patch").exists());

    let applied = dreaming::apply(&memory, "job-test").unwrap();
    assert_eq!(applied, job.proposal.operations.len());

    let notes = memory.list("team_sre").unwrap();
    assert!(notes.contains(&"notes/patterns.md".to_string()));
    assert!(!notes.contains(&"notes/old-knowledge.md".to_string()));
    assert_eq!(
        notes.iter().filter(|n| n.starts_with("notes/dup-")).count(),
        1,
        "5 duplicates consolidated to 1"
    );

    let patterns = memory.read("team_sre", "notes/patterns.md").unwrap();
    assert!(patterns.contains("60秒"));
    assert!(patterns.contains("describe-pods"));

    let dispatch = memory.read("team_sre", "notes/dispatch.md").unwrap();
    assert!(dispatch.contains("verified:"), "surviving note gets a verification note");

    let history = audit::read_all(&layout).unwrap();
    assert!(
        history.iter().any(|e| e.agent_id == "dreamer"),
        "dreaming writes are attributed to the dreamer"
    );
}
