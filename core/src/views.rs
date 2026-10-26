//! Reads for the interface. The displayed state comes from the saved
//! operational state plus elapsed time; it is never inferred by the LLM.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{params, Connection, Row};
use serde::Serialize;

use crate::engine::state;

/// After a turn ends with no new activity, the conversation becomes idle.
pub const IDLE_AFTER_MS: i64 = 30 * 60_000;
/// A running turn with no signal at all for this long is no longer certain.
pub const RUNNING_SILENCE_MS: i64 = 20 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayState {
    Working,
    Waiting,
    Finished,
    Idle,
    Closed,
    Unknown,
}

impl DisplayState {
    pub fn label(self) -> &'static str {
        match self {
            DisplayState::Working => "Working",
            DisplayState::Waiting => "Waiting for you",
            DisplayState::Finished => "Finished",
            DisplayState::Idle => "No recent activity",
            DisplayState::Closed => "Closed",
            DisplayState::Unknown => "Status unavailable",
        }
    }

    fn open(self) -> bool {
        !matches!(self, DisplayState::Closed)
    }
}

pub fn display_state(op_state: &str, last_activity_at: i64, now: i64) -> DisplayState {
    let silent = now - last_activity_at;
    match op_state {
        state::RUNNING if silent > RUNNING_SILENCE_MS => DisplayState::Unknown,
        state::RUNNING => DisplayState::Working,
        state::ATTENTION => DisplayState::Waiting,
        state::TURN_ENDED if silent > IDLE_AFTER_MS => DisplayState::Idle,
        state::TURN_ENDED => DisplayState::Finished,
        state::IDLE => DisplayState::Idle,
        state::CLOSED => DisplayState::Closed,
        _ => DisplayState::Unknown,
    }
}

/// Short, verified reason for the state, in plain language.
pub fn state_reason(op_state: &str, detail: Option<&str>, display: DisplayState) -> Option<String> {
    let detail = detail.unwrap_or("");
    let text = match display {
        DisplayState::Waiting => match detail {
            d if d.starts_with("permission:") => format!("Asking for permission to {}", tool_label(&d[11..])),
            "permission" => "Asking for permission".into(),
            "question" => "Has a question for you".into(),
            "plan" => "Waiting for plan approval".into(),
            d if d.starts_with("error:") => error_label(&d[6..]).into(),
            _ => "Needs your input".into(),
        },
        DisplayState::Finished if detail == "interrupted" => "Interrupted by you".into(),
        DisplayState::Finished => "Response complete".into(),
        DisplayState::Closed => match detail {
            "clear" => "Replaced by a new conversation (/clear)".into(),
            "process_exit" => "The process ended".into(),
            "resume" => "Session moved to another conversation".into(),
            "logout" => "Closed after logout".into(),
            _ => "Session closed in the terminal".into(),
        },
        DisplayState::Idle if op_state == state::IDLE && detail == "session_resume" => "Resumed, waiting for a message".into(),
        DisplayState::Idle if op_state == state::IDLE => "Open, waiting for a message".into(),
        DisplayState::Unknown => "No recent signal from the agent".into(),
        _ => return None,
    };
    Some(text)
}

fn tool_label(tool: &str) -> String {
    match tool {
        "Bash" | "shell" | "exec_command" | "local_shell" => "run a command in the terminal".into(),
        "apply_patch" => "edit files".into(),
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "edit files".into(),
        "WebFetch" | "WebSearch" => "access the web".into(),
        t if t.starts_with("mcp__") => "use an external tool".into(),
        t => format!("use {t}"),
    }
}

fn error_label(kind: &str) -> &'static str {
    match kind {
        "rate_limit" => "Stopped: usage limit reached",
        "authentication_failed" | "oauth_org_not_allowed" => "Stopped: you need to sign in again",
        "billing_error" | "account_on_hold" => "Stopped: problem with the account",
        "overloaded" | "server_error" => "Stopped: service temporarily unavailable",
        _ => "Stopped by an error",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCard {
    pub id: i64,
    pub project_id: Option<i64>,
    pub title: String,
    pub title_is_provisional: bool,
    pub summary: Option<String>,
    /// pending | ready | stale | failed | disabled | excluded | budget
    pub summary_status: String,
    pub summary_at: Option<i64>,
    pub state: DisplayState,
    pub state_label: String,
    pub state_reason: Option<String>,
    pub agent: String,
    pub environment: String,
    pub environment_is_guess: bool,
    pub folder_label: Option<String>,
    pub last_activity_at: i64,
    pub hidden: bool,
    /// Waiting for you, but the user has already seen the request: left out of “Waiting for you” and the badge.
    pub attention_seen: bool,
    /// The turn finished and the user hasn't looked at it yet.
    pub unread: bool,
    pub can_focus: bool,
    pub can_resume: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: usize,
    pub open: usize,
    pub closed: usize,
    pub working: usize,
    pub waiting: usize,
    pub finished: usize,
    pub unread: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCard {
    /// `None` = the Unassigned group.
    pub id: Option<i64>,
    pub name: String,
    pub description: Option<String>,
    pub recap_excluded: bool,
    pub counts: Counts,
    pub last_activity_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub projects: Vec<ProjectCard>,
    pub waiting: Vec<ConversationCard>,
    pub hidden_count: usize,
    pub total_conversations: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetail {
    pub project: ProjectCard,
    pub conversations: Vec<ConversationCard>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOption {
    pub id: i64,
    pub name: String,
}

const CARD_SQL: &str = "
SELECT c.id, c.project_id, c.user_title, c.title, c.provisional_title, c.summary,
       CASE WHEN c.recap_status = 'pending' AND c.recap_due_at IS NULL THEN 'waiting' ELSE c.recap_status END,
       c.summary_at,
       c.state, c.state_detail, c.last_activity_at, c.hidden, c.worktree_root, c.cwd, c.tool,
       i.env_app, i.env_guess, i.ended_at, i.pid, i.tty, i.bundle_id, c.attention_seen, c.unread
FROM conversations c
LEFT JOIN instances i ON i.id = c.current_instance_id
";

struct RawCard {
    card: ConversationCard,
    worktree_root: Option<String>,
}

fn read_card(r: &Row, now: i64) -> rusqlite::Result<RawCard> {
    let user_title: Option<String> = r.get(2)?;
    let title: Option<String> = r.get(3)?;
    let provisional: Option<String> = r.get(4)?;
    let op_state: String = r.get(8)?;
    let detail: Option<String> = r.get(9)?;
    let last_activity_at: i64 = r.get(10)?;
    let worktree_root: Option<String> = r.get(12)?;
    let cwd: Option<String> = r.get(13)?;
    let tool: String = r.get(14)?;
    let env_app: Option<String> = r.get(15)?;
    let env_guess: Option<bool> = r.get(16)?;
    let ended_at: Option<i64> = r.get(17)?;
    let pid: Option<i64> = r.get(18)?;
    let tty: Option<String> = r.get(19)?;
    let bundle_id: Option<String> = r.get(20)?;
    let attention_seen: bool = r.get(21)?;
    let unread: bool = r.get(22)?;

    let display = display_state(&op_state, last_activity_at, now);
    let (title, title_is_provisional) = match (user_title, title, provisional) {
        (Some(t), _, _) => (t, false),
        (None, Some(t), _) => (t, false),
        (None, None, Some(t)) => (t, true),
        (None, None, None) => (
            format!("Conversation in {}", cwd.as_deref().map(crate::paths::short_label).unwrap_or_else(|| "unknown folder".into())),
            true,
        ),
    };
    let live = ended_at.is_none() && pid.is_some() && display != DisplayState::Closed;
    Ok(RawCard {
        card: ConversationCard {
            id: r.get(0)?,
            project_id: r.get(1)?,
            title,
            title_is_provisional,
            summary: r.get(5)?,
            summary_status: r.get(6)?,
            summary_at: r.get(7)?,
            state: display,
            state_label: display.label().into(),
            state_reason: state_reason(&op_state, detail.as_deref(), display),
            agent: agent_label(&tool),
            environment: match env_app.as_deref() {
                // Label saved by versions before the English translation.
                None | Some("Terminale") => "Terminal".into(),
                Some(app) => app.to_string(),
            },
            environment_is_guess: env_guess.unwrap_or(true),
            folder_label: None,
            last_activity_at,
            hidden: r.get(11)?,
            attention_seen: attention_seen && display == DisplayState::Waiting,
            // It outlives Finished: after a while it becomes Idle but stays unread.
            unread: unread && matches!(display, DisplayState::Finished | DisplayState::Idle),
            can_focus: live && (tty.is_some() || bundle_id.is_some()),
            can_resume: !live && cwd.is_some(),
        },
        worktree_root,
    })
}

fn agent_label(tool: &str) -> String {
    match tool {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        other => other.into(),
    }
}

fn load_cards(conn: &Connection, filter: &str, params: &[&dyn rusqlite::ToSql], now: i64) -> Result<Vec<RawCard>> {
    let sql = format!("{CARD_SQL} WHERE {filter}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params, |r| read_card(r, now))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Folder label only when it is needed to tell apart two conversations of the
/// same project in different worktrees.
fn label_worktrees(cards: &mut [RawCard]) {
    let mut roots: HashMap<Option<i64>, Vec<String>> = HashMap::new();
    for c in cards.iter() {
        if let Some(root) = &c.worktree_root {
            let list = roots.entry(c.card.project_id).or_default();
            if !list.contains(root) {
                list.push(root.clone());
            }
        }
    }
    for c in cards.iter_mut() {
        if let Some(root) = &c.worktree_root {
            if roots.get(&c.card.project_id).is_some_and(|r| r.len() > 1) {
                c.card.folder_label = Some(crate::paths::short_label(root));
            }
        }
    }
}

/// Order on open: waiting first, then unread results, working, then by activity.
fn sort_cards(cards: &mut [ConversationCard]) {
    cards.sort_by_key(|c| {
        let rank = match c.state {
            DisplayState::Waiting => 0,
            _ if c.unread => 1,
            DisplayState::Working => 2,
            DisplayState::Finished => 3,
            DisplayState::Unknown => 4,
            DisplayState::Idle => 5,
            DisplayState::Closed => 6,
        };
        (rank, -c.last_activity_at)
    });
}

fn count(cards: &[&ConversationCard]) -> Counts {
    let mut n = Counts { total: cards.len(), ..Default::default() };
    for c in cards {
        if c.state.open() {
            n.open += 1;
        } else {
            n.closed += 1;
        }
        match c.state {
            DisplayState::Working => n.working += 1,
            DisplayState::Waiting if !c.attention_seen => n.waiting += 1,
            DisplayState::Finished => n.finished += 1,
            _ => {}
        }
        if c.unread {
            n.unread += 1;
        }
    }
    n
}

pub const UNASSIGNED_NAME: &str = "Unassigned";

pub fn overview(conn: &Connection, now: i64) -> Result<Overview> {
    let mut raw = load_cards(conn, "c.hidden = 0", &[], now)?;
    label_worktrees(&mut raw);
    let cards: Vec<ConversationCard> = raw.into_iter().map(|r| r.card).collect();

    let mut projects = Vec::new();
    let mut stmt = conn.prepare("SELECT id, name, description, recap_excluded FROM projects")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, bool>(3)?))
    })?;
    for row in rows {
        let (id, name, description, recap_excluded) = row?;
        let mine: Vec<&ConversationCard> = cards.iter().filter(|c| c.project_id == Some(id)).collect();
        if mine.is_empty() {
            continue;
        }
        projects.push(ProjectCard {
            id: Some(id),
            name,
            description,
            recap_excluded,
            last_activity_at: mine.iter().map(|c| c.last_activity_at).max().unwrap_or(0),
            counts: count(&mine),
        });
    }
    let unassigned: Vec<&ConversationCard> = cards.iter().filter(|c| c.project_id.is_none()).collect();
    if !unassigned.is_empty() {
        projects.push(ProjectCard {
            id: None,
            name: UNASSIGNED_NAME.into(),
            description: Some("Conversations outside a recognized repository".into()),
            recap_excluded: false,
            last_activity_at: unassigned.iter().map(|c| c.last_activity_at).max().unwrap_or(0),
            counts: count(&unassigned),
        });
    }
    projects.sort_by_key(|p| {
        let rank = if p.counts.waiting > 0 {
            0
        } else if p.counts.unread > 0 {
            1
        } else if p.counts.working > 0 {
            2
        } else {
            3
        };
        // Same state: Unassigned goes last.
        (rank, p.id.is_none(), -p.last_activity_at)
    });

    let mut waiting: Vec<ConversationCard> =
        cards.iter().filter(|c| c.state == DisplayState::Waiting && !c.attention_seen).cloned().collect();
    waiting.sort_by_key(|c| -c.last_activity_at);

    let hidden_count: usize = conn.query_row("SELECT count(*) FROM conversations WHERE hidden = 1", [], |r| r.get::<_, i64>(0))? as usize;
    Ok(Overview { total_conversations: cards.len(), projects, waiting, hidden_count })
}

pub fn project_detail(conn: &Connection, project_id: Option<i64>, now: i64) -> Result<Option<ProjectDetail>> {
    let mut raw = match project_id {
        Some(id) => load_cards(conn, "c.hidden = 0 AND c.project_id = ?1", &[&id], now)?,
        None => load_cards(conn, "c.hidden = 0 AND c.project_id IS NULL", &[], now)?,
    };
    label_worktrees(&mut raw);
    let mut conversations: Vec<ConversationCard> = raw.into_iter().map(|r| r.card).collect();
    sort_cards(&mut conversations);
    let refs: Vec<&ConversationCard> = conversations.iter().collect();
    let counts = count(&refs);
    let last = conversations.iter().map(|c| c.last_activity_at).max().unwrap_or(0);

    let project = match project_id {
        Some(id) => {
            let row = conn.query_row(
                "SELECT name, description, recap_excluded FROM projects WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, bool>(2)?)),
            );
            let Ok((name, description, recap_excluded)) = row else { return Ok(None) };
            ProjectCard { id: Some(id), name, description, recap_excluded, counts, last_activity_at: last }
        }
        None => ProjectCard {
            id: None,
            name: UNASSIGNED_NAME.into(),
            description: Some("Conversations outside a recognized repository".into()),
            recap_excluded: false,
            counts,
            last_activity_at: last,
        },
    };
    Ok(Some(ProjectDetail { project, conversations }))
}

pub fn conversation(conn: &Connection, id: i64, now: i64) -> Result<Option<ConversationCard>> {
    Ok(load_cards(conn, "c.id = ?1", &[&id], now)?.into_iter().next().map(|r| r.card))
}

pub fn removed(conn: &Connection, now: i64) -> Result<Vec<ConversationCard>> {
    let mut cards: Vec<ConversationCard> =
        load_cards(conn, "c.hidden = 1", &[], now)?.into_iter().map(|r| r.card).collect();
    cards.sort_by_key(|c| -c.last_activity_at);
    Ok(cards)
}

/// Search by topic and summary; removed ones only with an explicit filter.
pub fn search(conn: &Connection, query: &str, include_hidden: bool, now: i64) -> Result<Vec<ConversationCard>> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let fts = terms.join(" ");
    let filter = if include_hidden {
        "c.id IN (SELECT rowid FROM conversation_search WHERE conversation_search MATCH ?1)"
    } else {
        "c.hidden = 0 AND c.id IN (SELECT rowid FROM conversation_search WHERE conversation_search MATCH ?1)"
    };
    let mut raw = load_cards(conn, filter, &[&fts], now)?;
    label_worktrees(&mut raw);
    let mut cards: Vec<ConversationCard> = raw.into_iter().map(|r| r.card).collect();
    cards.sort_by_key(|c| -c.last_activity_at);
    Ok(cards)
}

pub fn project_options(conn: &Connection) -> Result<Vec<ProjectOption>> {
    let mut stmt = conn.prepare("SELECT id, name FROM projects ORDER BY name COLLATE NOCASE")?;
    let rows = stmt
        .query_map([], |r| Ok(ProjectOption { id: r.get(0)?, name: r.get(1)? }))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    Rest,
    Working,
    Attention,
    Problem,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionState {
    pub mood: Mood,
    pub waiting: usize,
    pub working: usize,
}

/// Priority: attention, tracker problem, activity, rest.
pub fn companion(conn: &Connection, now: i64, tracker_problem: bool) -> Result<CompanionState> {
    let mut stmt =
        conn.prepare("SELECT state, last_activity_at, attention_seen FROM conversations WHERE hidden = 0 AND state != ?1")?;
    let mut waiting = 0;
    let mut working = 0;
    for row in stmt.query_map(params![state::CLOSED], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?))
    })? {
        let (s, last, seen) = row?;
        match display_state(&s, last, now) {
            DisplayState::Waiting if !seen => waiting += 1,
            DisplayState::Working => working += 1,
            _ => {}
        }
    }
    let mood = if waiting > 0 {
        Mood::Attention
    } else if tracker_problem {
        Mood::Problem
    } else if working > 0 {
        Mood::Working
    } else {
        Mood::Rest
    };
    Ok(CompanionState { mood, waiting, working })
}
