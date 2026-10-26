//! Required scenarios from the specification, on the engine rules.

use std::collections::HashMap;

use tracker_core::engine::Tracker;
use tracker_core::event::{EnvInfo, Event, Kind};
use tracker_core::projects::{RepoInfo, StaticInspector};
use tracker_core::store;
use tracker_core::views::{self, DisplayState};

const T0: i64 = 1_800_000_000_000;

struct World {
    t: Tracker,
    seq: u64,
}

fn repo(identity: &str, root: &str, name: &str) -> RepoInfo {
    RepoInfo { identity: identity.into(), worktree_root: root.into(), name: name.into() }
}

impl World {
    fn new() -> Self {
        let mut map = HashMap::new();
        map.insert("/src/shop".to_string(), repo("/src/shop/.git", "/src/shop", "shop"));
        map.insert("/src/shop-wt".to_string(), repo("/src/shop/.git", "/src/shop-wt", "shop"));
        map.insert("/clone/shop".to_string(), repo("/clone/shop/.git", "/clone/shop", "shop"));
        map.insert("/src/blog".to_string(), repo("/src/blog/.git", "/src/blog", "blog"));
        let conn = store::open_in_memory().unwrap();
        World { t: Tracker::new(conn, Box::new(StaticInspector(map))), seq: 0 }
    }

    fn ev(&mut self, session: &str, pid: i64, cwd: &str, kind: Kind, at: i64) -> Event {
        self.seq += 1;
        Event {
            dedup_key: format!("k{}", self.seq),
            tool: "claude".into(),
            namespace: "default".into(),
            native_id: session.into(),
            kind,
            source_at: at,
            cwd: Some(cwd.into()),
            transcript_path: Some(format!("/t/{session}.jsonl")),
            prompt_id: None,
            pid: Some(pid),
            tty: Some("/dev/ttys001".into()),
            env: EnvInfo::detect(Some("com.apple.Terminal"), Some("Apple_Terminal"), false),
            entrypoint: Some("cli".into()),
            prompt_snippet: None,
            detail: None,
            probe: None,
        }
    }

    fn send(&mut self, session: &str, pid: i64, cwd: &str, kind: Kind, at: i64) -> tracker_core::engine::ApplyOutcome {
        let e = self.ev(session, pid, cwd, kind, at);
        self.t.apply(&e, at).unwrap()
    }

    fn id(&self, session: &str) -> i64 {
        self.t
            .conn
            .query_row("SELECT id FROM conversations WHERE native_id = ?1", [session], |r| r.get(0))
            .unwrap()
    }

    fn card(&self, session: &str, now: i64) -> views::ConversationCard {
        views::conversation(&self.t.conn, self.id(session), now).unwrap().unwrap()
    }

    fn visible(&self, now: i64) -> usize {
        views::overview(&self.t.conn, now).unwrap().total_conversations
    }
}

fn start() -> Kind {
    Kind::SessionStart { source: "startup".into() }
}

#[test]
fn three_conversations_two_tools_same_project_stay_distinct() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", start(), T0);
    w.send("b", 101, "/src/shop", start(), T0 + 1);
    w.send("a", 100, "/src/shop", Kind::Prompt, T0 + 2);
    let mut codex = w.ev("c", 102, "/src/shop", start(), T0 + 3);
    codex.tool = "codex".into();
    w.t.apply(&codex, T0 + 3).unwrap();

    let ov = views::overview(&w.t.conn, T0 + 10).unwrap();
    assert_eq!(ov.projects.len(), 1);
    assert_eq!(ov.projects[0].name, "shop");
    assert_eq!(ov.projects[0].counts.total, 3);
    assert_eq!(ov.projects[0].counts.working, 1);
    assert_eq!(w.card("a", T0 + 10).state, DisplayState::Working);
    assert_eq!(w.card("b", T0 + 10).state, DisplayState::Idle);
}

#[test]
fn codex_interrupt_ends_the_turn_without_notifying() {
    let mut w = World::new();
    let mut send = |w: &mut World, kind: Kind, at: i64| {
        let mut e = w.ev("cx", 300, "/src/shop", kind, at);
        e.tool = "codex".into();
        w.t.apply(&e, at).unwrap()
    };
    send(&mut w, start(), T0);
    send(&mut w, Kind::Prompt, T0 + 1);
    let out = send(&mut w, Kind::Interrupt, T0 + 2);
    assert!(out.notify.is_none(), "the user chose to interrupt");
    let c = w.card("cx", T0 + 3);
    assert_eq!(c.agent, "Codex");
    assert_eq!(c.state, DisplayState::Finished);
    assert_eq!(c.state_reason.as_deref(), Some("Interrupted by you"));

    send(&mut w, Kind::Prompt, T0 + 4);
    let out = send(&mut w, Kind::Stop, T0 + 5);
    assert!(out.notify.is_some());
    assert_eq!(w.card("cx", T0 + 6).state_reason.as_deref(), Some("Response complete"));
}

#[test]
fn codex_daemon_hooks_drive_companion_and_notifications() {
    use std::path::Path;
    use tracker_core::{event::{normalize, Normalized}, spool};
    let dir = tempfile::tempdir().unwrap();
    let rollout = dir.path().join("rollout.jsonl");
    std::fs::write(&rollout, serde_json::json!({"type":"session_meta","payload":{
        "originator":"codex-tui","source":"vscode","thread_source":"user"
    }}).to_string()).unwrap();
    let mut world = World::new();
    let steps = [
        ("SessionStart", "", views::Mood::Rest, None),
        ("UserPromptSubmit", "", views::Mood::Working, None),
        ("PermissionRequest", "Bash", views::Mood::Attention, Some(tracker_core::engine::NotifyKind::Attention)),
        ("PostToolUse", "Bash", views::Mood::Working, None),
        ("PreToolUse", "request_user_input", views::Mood::Attention, Some(tracker_core::engine::NotifyKind::Attention)),
        ("PostToolUse", "request_user_input", views::Mood::Working, None),
        ("Stop", "", views::Mood::Rest, Some(tracker_core::engine::NotifyKind::Finished)),
        ("SessionEnd", "", views::Mood::Rest, None),
    ];
    for (i, (name, tool, mood, notify)) in steps.into_iter().enumerate() {
        let at = T0 + i as i64;
        let payload = serde_json::json!({
            "session_id":"codex-daemon-session","transcript_path":rollout,"cwd":"/src/shop",
            "hook_event_name":name,"turn_id":"t1","tool_name":tool
        });
        let bytes = format!("flow-keeper-spool 1\nTOOL=codex\nEVENT={name}\nPID=300\nCODEX_HOST=app-server\n\n{payload}");
        let rec = spool::parse_bytes(Path::new("test.evt"), bytes.as_bytes(), at as i128 * 1_000_000).unwrap();
        let Normalized::Event(e) = normalize(&rec, &Default::default()) else { panic!("CLI hook discarded: {name}") };
        let out = world.t.apply(&e, at).unwrap();
        assert_eq!(views::companion(&world.t.conn, at, false).unwrap().mood, mood, "{name}");
        assert_eq!(out.notify.map(|n| n.kind), notify, "{name}");
    }
    let state: String = world.t.conn.query_row("SELECT state FROM conversations WHERE native_id = 'codex-daemon-session'", [], |r| r.get(0)).unwrap();
    assert_eq!(state, tracker_core::engine::state::CLOSED);
}

#[test]
fn worktrees_share_project_with_distinct_folder_labels_and_clones_do_not() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", start(), T0);
    w.send("b", 101, "/src/shop-wt", start(), T0 + 1);
    w.send("c", 102, "/clone/shop", start(), T0 + 2);
    let ov = views::overview(&w.t.conn, T0 + 10).unwrap();
    assert_eq!(ov.projects.len(), 2, "the separate clone is not merged");
    let pid = w.card("a", T0).project_id;
    assert_eq!(pid, w.card("b", T0).project_id);
    assert_ne!(pid, w.card("c", T0).project_id);
    let detail = views::project_detail(&w.t.conn, pid, T0 + 10).unwrap().unwrap();
    let labels: Vec<_> = detail.conversations.iter().map(|c| c.folder_label.clone()).collect();
    assert!(labels.contains(&Some("shop".into())) && labels.contains(&Some("shop-wt".into())));
}

#[test]
fn turn_end_does_not_close_and_verified_close_stays_visible() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", start(), T0);
    w.send("a", 100, "/src/shop", Kind::Prompt, T0 + 1);
    w.send("a", 100, "/src/shop", Kind::Stop, T0 + 2);
    let c = w.card("a", T0 + 3);
    assert_eq!(c.state, DisplayState::Finished);
    assert_eq!(c.state_reason.as_deref(), Some("Response complete"));
    assert_eq!(w.card("a", T0 + 31 * 60_000).state, DisplayState::Idle);

    w.send("a", 100, "/src/shop", Kind::SessionEnd { reason: "prompt_input_exit".into() }, T0 + 4);
    assert_eq!(w.card("a", T0 + 5).state, DisplayState::Closed);
    assert_eq!(w.visible(T0 + 5), 1);
}

#[test]
fn clear_closes_old_identity_and_starts_a_separate_one() {
    let mut w = World::new();
    w.send("old", 100, "/src/shop", start(), T0);
    w.send("old", 100, "/src/shop", Kind::Prompt, T0 + 1);
    w.send("old", 100, "/src/shop", Kind::SessionEnd { reason: "clear".into() }, T0 + 2);
    w.send("new", 100, "/src/shop", Kind::SessionStart { source: "clear".into() }, T0 + 3);
    assert_eq!(w.card("old", T0 + 4).state, DisplayState::Closed);
    assert_eq!(w.card("new", T0 + 4).state, DisplayState::Idle);
    assert_ne!(w.id("old"), w.id("new"));
}

#[test]
fn attention_is_set_by_permission_and_resolved_by_progress() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    let out = w.send("a", 100, "/src/shop", Kind::Permission { tool: Some("Bash".into()) }, T0 + 1);
    assert!(out.notify.is_some(), "a verified request triggers a notification");
    let c = w.card("a", T0 + 2);
    assert_eq!(c.state, DisplayState::Waiting);
    assert_eq!(c.state_reason.as_deref(), Some("Asking for permission to run a command in the terminal"));
    // The duplicate Notification does not trigger a second notification.
    let dup = w.send("a", 100, "/src/shop", Kind::AttentionNotice { notification_type: "permission_prompt".into() }, T0 + 2);
    assert!(dup.notify.is_none());
    // Inactivity does not clear an open request.
    assert_eq!(w.card("a", T0 + 3 * 3_600_000).state, DisplayState::Waiting);
    w.send("a", 100, "/src/shop", Kind::ToolActivity, T0 + 3);
    assert_eq!(w.card("a", T0 + 4).state, DisplayState::Working);
}

#[test]
fn dismissed_attention_leaves_waiting_and_badge_until_the_state_changes() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    w.send("a", 100, "/src/shop", Kind::StopFailure { error: Some("overloaded".into()) }, T0 + 1);
    let id = w.id("a");
    assert_eq!(views::overview(&w.t.conn, T0 + 2).unwrap().waiting.len(), 1);

    w.t.dismiss_attention(id).unwrap();
    let ov = views::overview(&w.t.conn, T0 + 3).unwrap();
    assert!(ov.waiting.is_empty());
    assert_eq!(ov.projects[0].counts.waiting, 0);
    let c = views::companion(&w.t.conn, T0 + 3, false).unwrap();
    assert_eq!((c.mood, c.waiting), (views::Mood::Rest, 0));
    // The card stays honest about the real state.
    let card = w.card("a", T0 + 3);
    assert_eq!(card.state, DisplayState::Waiting);
    assert!(card.attention_seen);

    // A new request after resuming is highlighted again.
    w.send("a", 100, "/src/shop", Kind::Prompt, T0 + 10);
    assert!(!w.card("a", T0 + 11).attention_seen);
    w.send("a", 100, "/src/shop", Kind::Permission { tool: Some("Bash".into()) }, T0 + 12);
    assert_eq!(views::overview(&w.t.conn, T0 + 13).unwrap().waiting.len(), 1);
    assert_eq!(views::companion(&w.t.conn, T0 + 13, false).unwrap().waiting, 1);
}

#[test]
fn dismiss_attention_ignores_conversations_that_are_not_waiting() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    w.t.dismiss_attention(w.id("a")).unwrap();
    w.send("a", 100, "/src/shop", Kind::Question { tool: "AskUserQuestion".into() }, T0 + 1);
    assert_eq!(views::overview(&w.t.conn, T0 + 2).unwrap().waiting.len(), 1);
}

#[test]
fn hide_during_activity_then_new_activity_brings_back_same_entry() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    let id = w.id("a");
    w.t.hide(id, T0 + 10).unwrap();
    assert_eq!(w.visible(T0 + 11), 0);
    w.send("a", 100, "/src/shop", Kind::ToolActivity, T0 + 20);
    assert_eq!(w.visible(T0 + 21), 1);
    assert_eq!(w.id("a"), id, "no duplicate");
}

#[test]
fn hidden_closed_conversation_ignores_old_events_scans_and_close_but_returns_on_resume() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    w.send("a", 100, "/src/shop", Kind::Stop, T0 + 1);
    w.send("a", 100, "/src/shop", Kind::SessionEnd { reason: "other".into() }, T0 + 2);
    let id = w.id("a");
    w.t.hide(id, T0 + 100).unwrap();

    // Delayed event from before the removal.
    w.send("a", 100, "/src/shop", Kind::ToolActivity, T0 + 50);
    // Only a close and informational notifications.
    w.send("a", 100, "/src/shop", Kind::SessionEnd { reason: "other".into() }, T0 + 200);
    w.send("a", 100, "/src/shop", Kind::InfoNotice { notification_type: "idle_prompt".into() }, T0 + 201);
    // Liveness sweep and a duplicate of the same file.
    w.t.sweep_liveness(T0 + 300, &|_| Some(false)).unwrap();
    assert_eq!(w.visible(T0 + 301), 0);
    assert_eq!(w.card("a", T0 + 301).state, DisplayState::Closed);

    w.send("a", 200, "/src/shop", Kind::SessionStart { source: "resume".into() }, T0 + 400);
    assert_eq!(w.visible(T0 + 401), 1);
    assert_eq!(w.id("a"), id);
}

#[test]
fn duplicate_hook_event_is_applied_once() {
    let mut w = World::new();
    let e = w.ev("a", 100, "/src/shop", Kind::Prompt, T0);
    assert!(!w.t.apply(&e, T0).unwrap().duplicate);
    assert!(w.t.apply(&e, T0).unwrap().duplicate);
    let n: i64 = w.t.conn.query_row("SELECT count(*) FROM events", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn out_of_order_events_do_not_regress_state() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    w.send("a", 100, "/src/shop", Kind::Stop, T0 + 10);
    w.send("a", 100, "/src/shop", Kind::ToolActivity, T0 + 5);
    assert_eq!(w.card("a", T0 + 11).state, DisplayState::Finished);
}

#[test]
fn resume_in_other_terminal_creates_instance_and_late_close_of_old_one_is_ignored() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", start(), T0);
    w.send("a", 200, "/src/shop", Kind::SessionStart { source: "resume".into() }, T0 + 10);
    w.send("a", 200, "/src/shop", Kind::Prompt, T0 + 11);
    w.send("a", 100, "/src/shop", Kind::SessionEnd { reason: "other".into() }, T0 + 12);
    assert_eq!(w.card("a", T0 + 13).state, DisplayState::Working);
    let instances: i64 = w.t.conn.query_row("SELECT count(*) FROM instances", [], |r| r.get(0)).unwrap();
    assert_eq!(instances, 2);
    // The sweep, too, closes only the old instance.
    w.t.sweep_liveness(T0 + 20, &|pid| Some(pid == 200)).unwrap();
    assert_eq!(w.card("a", T0 + 21).state, DisplayState::Working);
    w.t.sweep_liveness(T0 + 30, &|_| Some(false)).unwrap();
    assert_eq!(w.card("a", T0 + 31).state, DisplayState::Closed);
}

#[test]
fn process_exit_without_session_end_is_a_verified_close() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    assert!(w.t.sweep_liveness(T0 + 5, &|_| None).unwrap().is_empty(), "without a check it does not close");
    assert_eq!(w.t.sweep_liveness(T0 + 6, &|_| Some(false)).unwrap().len(), 1);
    let c = w.card("a", T0 + 7);
    assert_eq!(c.state, DisplayState::Closed);
    assert!(c.can_resume && !c.can_focus);
}

#[test]
fn long_silence_while_running_is_not_shown_as_certain() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    assert_eq!(w.card("a", T0 + 25 * 60_000).state, DisplayState::Unknown);
}

#[test]
fn no_repository_goes_to_unassigned_until_folder_is_associated() {
    let mut w = World::new();
    w.send("a", 100, "/Users/me/notes", start(), T0);
    let ov = views::overview(&w.t.conn, T0 + 1).unwrap();
    assert_eq!(ov.projects[0].name, views::UNASSIGNED_NAME);
    let p = w.t.create_project("Notes", T0).unwrap();
    assert_eq!(w.t.associate_folder("/Users/me", p).unwrap(), 1);
    assert_eq!(w.card("a", T0).project_id, Some(p));
    w.send("b", 101, "/Users/me/notes/x", start(), T0 + 2);
    assert_eq!(w.card("b", T0).project_id, Some(p));
}

#[test]
fn moving_one_conversation_does_not_move_the_others() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", start(), T0);
    w.send("b", 101, "/src/shop", start(), T0);
    w.send("x", 102, "/src/blog", start(), T0);
    let blog = w.card("x", T0).project_id;
    w.t.move_conversation(w.id("a"), blog).unwrap();
    w.send("c", 103, "/src/shop", start(), T0 + 1);
    assert_eq!(w.card("a", T0).project_id, blog);
    assert_ne!(w.card("b", T0).project_id, blog);
    assert_ne!(w.card("c", T0).project_id, blog);
}

#[test]
fn old_events_after_restart_do_not_notify() {
    let mut w = World::new();
    w.send("a", 100, "/src/shop", Kind::Prompt, T0);
    let e = w.ev("a", 100, "/src/shop", Kind::Permission { tool: None }, T0 + 1);
    let out = w.t.apply(&e, T0 + 60 * 60_000).unwrap();
    assert!(out.notify.is_none());
    assert_eq!(w.card("a", T0 + 2).state, DisplayState::Waiting);
}

#[test]
fn search_finds_titles_and_excludes_hidden_by_default() {
    let mut w = World::new();
    let mut e = w.ev("a", 100, "/src/shop", Kind::Prompt, T0);
    e.prompt_snippet = Some("Fix the user login".into());
    w.t.apply(&e, T0).unwrap();
    assert_eq!(views::search(&w.t.conn, "login", false, T0).unwrap().len(), 1);
    assert_eq!(views::search(&w.t.conn, "logi", false, T0).unwrap().len(), 1);
    w.t.hide(w.id("a"), T0 + 1).unwrap();
    assert_eq!(views::search(&w.t.conn, "login", false, T0).unwrap().len(), 0);
    assert_eq!(views::search(&w.t.conn, "login", true, T0).unwrap().len(), 1);
}

#[test]
fn ingest_from_spool_dir_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, body: &str| std::fs::write(dir.path().join(name), body).unwrap();
    write("1-1-claude-SessionStart.evt", "flow-keeper-spool 1\nEVENT=SessionStart\nPID=4242\nCLAUDE_CODE_ENTRYPOINT=cli\n\n{\"session_id\":\"s1\",\"cwd\":\"/src/shop\",\"source\":\"startup\"}");
    std::thread::sleep(std::time::Duration::from_millis(5));
    write("2-1-claude-UserPromptSubmit.evt", "flow-keeper-spool 1\nEVENT=UserPromptSubmit\nPID=4242\n\n{\"session_id\":\"s1\",\"cwd\":\"/src/shop\",\"prompt\":\"Fix the cart\"}");
    write("3-1-claude-Stop.evt", "garbage");
    write("4-1-claude-Stop.evt", "flow-keeper-spool 1\nEVENT=Stop\nCLAUDE_CODE_ENTRYPOINT=sdk-cli\n\n{\"session_id\":\"p1\"}");
    let mut w = World::new();
    let now = tracker_core::now_ms();
    let report = w.t.ingest_spool(dir.path(), now).unwrap();
    assert_eq!((report.applied, report.rejected, report.ignored), (2, 1, 1));
    assert!(tracker_core::spool::pending(dir.path()).is_empty());
    let c = w.card("s1", now);
    assert_eq!(c.title, "Fix the cart");
    assert!(c.title_is_provisional);
    assert_eq!(c.state, DisplayState::Working);
}
