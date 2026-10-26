//! State and visibility rules. The only place that writes conversations,
//! instances and events.

use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::event::{normalize, Event, Kind, NormalizeOptions, Normalized};
use crate::projects::RepoInspector;
use crate::spool;
use crate::store;

/// Wait before the recap after a turn ends (initial debounce).
pub const RECAP_DEBOUNCE_MS: i64 = 60_000;
/// Events older than this when ingested do not trigger notifications.
pub const NOTIFY_MAX_DELAY_MS: i64 = 5 * 60_000;

pub mod state {
    pub const RUNNING: &str = "running";
    pub const ATTENTION: &str = "attention";
    pub const TURN_ENDED: &str = "turn_ended";
    pub const IDLE: &str = "idle";
    pub const CLOSED: &str = "closed";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyKind {
    Attention,
    Finished,
}

impl NotifyKind {
    pub fn code(self) -> &'static str {
        match self {
            NotifyKind::Attention => "attention",
            NotifyKind::Finished => "finished",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotifyRequest {
    pub conversation_id: i64,
    pub notification_id: i64,
    pub kind: NotifyKind,
}

#[derive(Debug, Default)]
pub struct ApplyOutcome {
    pub conversation_id: Option<i64>,
    pub duplicate: bool,
    pub changed: bool,
    pub notify: Option<NotifyRequest>,
}

#[derive(Debug, Default)]
pub struct IngestReport {
    pub applied: usize,
    pub duplicates: usize,
    pub ignored: usize,
    pub rejected: usize,
    pub changed: bool,
    pub notifications: Vec<NotifyRequest>,
    pub probes: Vec<String>,
}

pub struct Tracker {
    pub conn: Connection,
    inspector: Box<dyn RepoInspector>,
    pub normalize_options: NormalizeOptions,
}

#[derive(Debug)]
struct ConvRow {
    id: i64,
    state: String,
    state_detail: Option<String>,
    state_at: i64,
    hidden: bool,
    hidden_at: Option<i64>,
    current_instance_id: Option<i64>,
    provisional_title: Option<String>,
}

impl Tracker {
    pub fn new(conn: Connection, inspector: Box<dyn RepoInspector>) -> Self {
        Tracker { conn, inspector, normalize_options: NormalizeOptions::default() }
    }

    /// Ingests all pending files in the spool, in the order they were written.
    /// A file is removed only after its event has been committed.
    pub fn ingest_spool(&mut self, dir: &Path, now: i64) -> Result<IngestReport> {
        let mut report = IngestReport::default();
        for (_, file) in spool::pending(dir) {
            let rec = match spool::read_record(&file) {
                Ok(rec) => rec,
                Err(err) => {
                    log::warn!("unreadable spool file, discarded: {err}");
                    spool::reject(dir, &file);
                    report.rejected += 1;
                    continue;
                }
            };
            match normalize(&rec, &self.normalize_options) {
                Normalized::Probe { nonce, source_at } => {
                    store::set_meta(&self.conn, &format!("probe:{nonce}"), &source_at.to_string())?;
                    report.probes.push(nonce);
                }
                Normalized::Ignored(_) => report.ignored += 1,
                Normalized::Event(event) => match self.apply(&event, now) {
                    Ok(outcome) => {
                        if outcome.duplicate {
                            report.duplicates += 1;
                        } else {
                            report.applied += 1;
                        }
                        report.changed |= outcome.changed;
                        report.notifications.extend(outcome.notify);
                    }
                    Err(err) => {
                        log::error!("event not applied: {err:#}");
                        store::set_meta(&self.conn, "last_ingest_error", &format!("{now}|{err}"))?;
                        spool::reject(dir, &file);
                        report.rejected += 1;
                        continue;
                    }
                },
            }
            let _ = std::fs::remove_file(&file);
        }
        if report.applied + report.duplicates + report.ignored + report.rejected > 0 {
            store::set_meta(&self.conn, "last_ingest_at", &now.to_string())?;
        }
        if report.applied > 0 {
            store::set_meta(&self.conn, "last_event_at", &now.to_string())?;
        }
        Ok(report)
    }

    pub fn apply(&mut self, e: &Event, now: i64) -> Result<ApplyOutcome> {
        let tx = self.conn.transaction()?;
        let seen: Option<i64> = tx
            .query_row("SELECT id FROM events WHERE dedup_key = ?1", [&e.dedup_key], |r| r.get(0))
            .optional()?;
        if seen.is_some() {
            return Ok(ApplyOutcome { duplicate: true, ..Default::default() });
        }

        let conv = match load_conv(&tx, e)? {
            Some(c) => c,
            None => create_conv(&tx, self.inspector.as_ref(), e)?,
        };
        let mut out = ApplyOutcome { conversation_id: Some(conv.id), changed: true, ..Default::default() };

        let instance = resolve_instance(&tx, conv.id, conv.current_instance_id, e)?;
        let is_current = instance.is_none() || conv.current_instance_id.is_none() || instance == conv.current_instance_id;

        // Events older than the current state do not roll it back.
        let stale = e.source_at < conv.state_at;
        let mut st = conv.state.clone();
        let mut detail = conv.state_detail.clone();
        let mut bump_revision = false;
        let mut recap_due: Option<i64> = None;
        let mut effect = String::new();

        if !matches!(e.kind, Kind::SessionEnd { .. }) {
            if let Some(inst) = instance {
                if conv.current_instance_id != Some(inst) {
                    tx.execute("UPDATE conversations SET current_instance_id = ?1 WHERE id = ?2", params![inst, conv.id])?;
                }
            }
        }

        match &e.kind {
            Kind::SessionStart { source } if source == "compact" => {}
            Kind::SessionStart { source } => {
                st = state::IDLE.into();
                detail = Some(format!("session_{source}"));
            }
            Kind::SessionEnd { reason } => {
                if let Some(inst) = instance {
                    tx.execute(
                        "UPDATE instances SET ended_at = ?1, end_reason = ?2 WHERE id = ?3 AND ended_at IS NULL",
                        params![e.source_at, reason, inst],
                    )?;
                }
                if is_current {
                    st = state::CLOSED.into();
                    detail = Some(reason.clone());
                    recap_due = Some(now.max(e.source_at) + 5_000);
                } else {
                    effect.push_str("instance_end;");
                }
            }
            Kind::Prompt | Kind::ToolActivity | Kind::PermissionDenied => {
                st = state::RUNNING.into();
                detail = None;
            }
            Kind::Question { tool } => {
                st = state::ATTENTION.into();
                detail = Some(if tool == "ExitPlanMode" { "plan".into() } else { "question".into() });
            }
            Kind::Permission { tool } => {
                st = state::ATTENTION.into();
                detail = Some(match tool {
                    Some(t) => format!("permission:{t}"),
                    None => "permission".into(),
                });
            }
            Kind::AttentionNotice { notification_type } => {
                if conv.state != state::ATTENTION || detail.is_none() {
                    detail = Some(match notification_type.as_str() {
                        "permission_prompt" => "permission".into(),
                        _ => "input".into(),
                    });
                }
                st = state::ATTENTION.into();
            }
            Kind::Stop => {
                st = state::TURN_ENDED.into();
                detail = None;
                bump_revision = true;
                recap_due = Some(now.max(e.source_at) + RECAP_DEBOUNCE_MS);
            }
            // Interrupted by the user: end of turn, no notification (see below).
            Kind::Interrupt => {
                st = state::TURN_ENDED.into();
                detail = Some("interrupted".into());
                bump_revision = true;
                recap_due = Some(now.max(e.source_at) + RECAP_DEBOUNCE_MS);
            }
            Kind::StopFailure { error } => {
                st = state::ATTENTION.into();
                detail = Some(format!("error:{}", error.as_deref().unwrap_or("unknown")));
                bump_revision = true;
                recap_due = Some(now.max(e.source_at) + RECAP_DEBOUNCE_MS);
            }
            Kind::InfoNotice { .. } | Kind::Other { .. } => {}
        }

        let state_changes = !stale && (st != conv.state || detail != conv.state_detail);
        if stale {
            effect.push_str("stale;");
        }
        if state_changes {
            tx.execute(
                "UPDATE conversations SET state = ?1, state_detail = ?2, state_at = ?3, attention_seen = 0 WHERE id = ?4",
                params![st, detail, e.source_at, conv.id],
            )?;
            effect.push_str(&format!("state={st};"));
        } else if !stale && !matches!(e.kind, Kind::InfoNotice { .. } | Kind::Other { .. }) {
            tx.execute("UPDATE conversations SET state_at = MAX(state_at, ?1) WHERE id = ?2", params![e.source_at, conv.id])?;
        }

        if e.kind.is_significant() {
            tx.execute(
                "UPDATE conversations SET last_activity_at = MAX(last_activity_at, ?1) WHERE id = ?2",
                params![e.source_at, conv.id],
            )?;
        }

        // Automatic return: only real activity after the removal.
        let mut hidden = conv.hidden;
        if conv.hidden && e.kind.is_significant() && e.source_at > conv.hidden_at.unwrap_or(i64::MAX) {
            tx.execute("UPDATE conversations SET hidden = 0, hidden_at = NULL WHERE id = ?1", [conv.id])?;
            hidden = false;
            effect.push_str("unhidden;");
        }

        if bump_revision {
            tx.execute("UPDATE conversations SET revision = revision + 1 WHERE id = ?1", [conv.id])?;
        }
        if let Some(due) = recap_due {
            tx.execute(
                "UPDATE conversations SET recap_due_at = ?1,
                   recap_status = CASE WHEN recap_status IN ('ready', 'failed') THEN 'stale' ELSE recap_status END
                 WHERE id = ?2",
                params![due, conv.id],
            )?;
        }

        if let Some(cwd) = &e.cwd {
            tx.execute("UPDATE conversations SET cwd = ?1 WHERE id = ?2 AND cwd IS NOT ?1", params![cwd, conv.id])?;
        }
        if let Some(tp) = &e.transcript_path {
            tx.execute(
                "UPDATE conversations SET transcript_path = ?1 WHERE id = ?2 AND transcript_path IS NOT ?1",
                params![tp, conv.id],
            )?;
        }
        if conv.provisional_title.is_none() {
            if let Some(title) = e.prompt_snippet.as_deref().and_then(provisional_title) {
                tx.execute("UPDATE conversations SET provisional_title = ?1 WHERE id = ?2", params![title, conv.id])?;
                store::index_conversation(&tx, conv.id)?;
            }
        }

        tx.execute(
            "INSERT INTO events(dedup_key, conversation_id, instance_id, kind, detail, prompt_id, source_at, ingested_at, effect)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![e.dedup_key, conv.id, instance, e.kind.code(), e.detail, e.prompt_id, e.source_at, now, effect],
        )?;
        let event_id = tx.last_insert_rowid();

        let fresh = now - e.source_at <= NOTIFY_MAX_DELAY_MS;
        if state_changes && !hidden && fresh {
            let kind = match (st.as_str(), &e.kind) {
                (state::ATTENTION, _) => Some(NotifyKind::Attention),
                (state::TURN_ENDED, Kind::Stop) => Some(NotifyKind::Finished),
                _ => None,
            };
            if let Some(kind) = kind {
                tx.execute(
                    "INSERT OR IGNORE INTO notifications(conversation_id, event_id, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![conv.id, event_id, kind.code(), now],
                )?;
                out.notify = Some(NotifyRequest { conversation_id: conv.id, notification_id: tx.last_insert_rowid(), kind });
            }
        }

        tx.commit()?;
        Ok(out)
    }

    /// Closes conversations whose process no longer exists. `alive`
    /// returns `None` when this cannot be checked.
    pub fn sweep_liveness(&mut self, now: i64, alive: &dyn Fn(i64) -> Option<bool>) -> Result<Vec<i64>> {
        let rows: Vec<(i64, i64, i64, bool)> = {
            let mut stmt = self.conn.prepare(
                "SELECT i.id, i.pid, i.conversation_id, c.current_instance_id = i.id
                 FROM instances i JOIN conversations c ON c.id = i.conversation_id
                 WHERE i.ended_at IS NULL AND i.pid IS NOT NULL",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            rows
        };
        let mut changed = Vec::new();
        let tx = self.conn.transaction()?;
        for (instance, pid, conversation, current) in rows {
            if alive(pid) != Some(false) {
                continue;
            }
            tx.execute(
                "UPDATE instances SET ended_at = ?1, end_reason = 'process_exit' WHERE id = ?2",
                params![now, instance],
            )?;
            if current {
                let n = tx.execute(
                    "UPDATE conversations SET state = ?1, state_detail = 'process_exit', state_at = MAX(state_at, ?2),
                       recap_due_at = COALESCE(recap_due_at, ?2)
                     WHERE id = ?3 AND state != ?1",
                    params![state::CLOSED, now, conversation],
                )?;
                if n > 0 {
                    changed.push(conversation);
                }
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn live_pids(&self) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT pid FROM instances WHERE ended_at IS NULL AND pid IS NOT NULL")?;
        let pids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(pids)
    }

    // ---- User commands --------------------------------------------------------

    /// Remove from view: saves the point in the sequence already known.
    pub fn hide(&mut self, id: i64, now: i64) -> Result<()> {
        let last_event: Option<i64> = self
            .conn
            .query_row("SELECT MAX(source_at) FROM events WHERE conversation_id = ?1", [id], |r| r.get(0))?;
        let point = now.max(last_event.unwrap_or(0));
        self.conn
            .execute("UPDATE conversations SET hidden = 1, hidden_at = ?1 WHERE id = ?2", params![point, id])?;
        Ok(())
    }

    /// Manual restore (Undo or the Removed filter).
    pub fn unhide(&mut self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE conversations SET hidden = 0, hidden_at = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Attention request already seen: it leaves “Waiting for you” and the
    /// badge until the state changes again.
    pub fn dismiss_attention(&mut self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET attention_seen = 1 WHERE id = ?1 AND state = ?2",
            params![id, state::ATTENTION],
        )?;
        Ok(())
    }

    pub fn rename(&mut self, id: i64, title: Option<&str>) -> Result<()> {
        let title = title.map(str::trim).filter(|t| !t.is_empty()).map(|t| truncate_chars(t, 80));
        self.conn
            .execute("UPDATE conversations SET user_title = ?1 WHERE id = ?2", params![title, id])?;
        store::index_conversation(&self.conn, id)?;
        Ok(())
    }

    /// Moves a single conversation: does not create rules for the others.
    pub fn move_conversation(&mut self, id: i64, project_id: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE conversations SET project_id = ?1, project_confirmed = 1 WHERE id = ?2",
            params![project_id, id],
        )?;
        Ok(())
    }

    /// Explicitly maps a folder to a project and assigns the conversations
    /// there that are not yet assigned.
    pub fn associate_folder(&mut self, path: &str, project_id: i64) -> Result<usize> {
        let path = path.trim_end_matches('/');
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO project_folders(path, project_id) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET project_id = excluded.project_id",
            params![path, project_id],
        )?;
        let n = tx.execute(
            "UPDATE conversations SET project_id = ?1
             WHERE project_id IS NULL AND project_confirmed = 0 AND (cwd = ?2 OR cwd LIKE ?3 ESCAPE '\\')",
            params![project_id, path, format!("{}/%", like_escape(path))],
        )?;
        tx.commit()?;
        Ok(n)
    }

    pub fn create_project(&mut self, name: &str, now: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO projects(name, created_at) VALUES (?1, ?2)",
            params![truncate_chars(name.trim(), 60), now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_project(&mut self, id: i64, name: Option<&str>, description: Option<Option<&str>>) -> Result<()> {
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
            self.conn
                .execute("UPDATE projects SET name = ?1 WHERE id = ?2", params![truncate_chars(name, 60), id])?;
        }
        if let Some(desc) = description {
            let desc = desc.map(str::trim).filter(|d| !d.is_empty()).map(|d| truncate_chars(d, 160));
            self.conn
                .execute("UPDATE projects SET description = ?1 WHERE id = ?2", params![desc, id])?;
        }
        Ok(())
    }

    pub fn set_project_excluded(&mut self, id: i64, excluded: bool) -> Result<()> {
        self.conn
            .execute("UPDATE projects SET recap_excluded = ?1 WHERE id = ?2", params![excluded, id])?;
        Ok(())
    }

    /// Merges `from` into `into`: conversations and folders move to the target project.
    pub fn merge_projects(&mut self, from: i64, into: i64) -> Result<()> {
        if from == into {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE conversations SET project_id = ?1 WHERE project_id = ?2", params![into, from])?;
        tx.execute("UPDATE project_folders SET project_id = ?1 WHERE project_id = ?2", params![into, from])?;
        // The merged clone's identity stays recognizable as a mapped folder.
        tx.execute(
            "INSERT OR IGNORE INTO project_folders(path, project_id)
             SELECT c.worktree_root, ?1 FROM conversations c WHERE c.project_id = ?1 AND c.worktree_root IS NOT NULL",
            params![into],
        )?;
        tx.execute("DELETE FROM projects WHERE id = ?1", [from])?;
        tx.commit()?;
        Ok(())
    }

    pub fn update_cwd(&mut self, id: i64, cwd: &str) -> Result<()> {
        self.conn.execute("UPDATE conversations SET cwd = ?1 WHERE id = ?2", params![cwd, id])?;
        Ok(())
    }
}

fn load_conv(tx: &Transaction, e: &Event) -> Result<Option<ConvRow>> {
    Ok(tx
        .query_row(
            "SELECT id, state, state_detail, state_at, hidden, hidden_at, current_instance_id, provisional_title
             FROM conversations WHERE tool = ?1 AND namespace = ?2 AND native_id = ?3",
            params![e.tool, e.namespace, e.native_id],
            |r| {
                Ok(ConvRow {
                    id: r.get(0)?,
                    state: r.get(1)?,
                    state_detail: r.get(2)?,
                    state_at: r.get(3)?,
                    hidden: r.get(4)?,
                    hidden_at: r.get(5)?,
                    current_instance_id: r.get(6)?,
                    provisional_title: r.get(7)?,
                })
            },
        )
        .optional()?)
}

fn create_conv(tx: &Transaction, inspector: &dyn RepoInspector, e: &Event) -> Result<ConvRow> {
    let (project_id, worktree_root) = match &e.cwd {
        Some(cwd) => resolve_project(tx, inspector, cwd, e.source_at)?,
        None => (None, None),
    };
    // The initial state is then set by the event itself.
    let initial_at = e.source_at - 1;
    tx.execute(
        "INSERT INTO conversations(tool, namespace, native_id, project_id, cwd, worktree_root, transcript_path,
           state, state_at, last_activity_at, first_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
        params![
            e.tool, e.namespace, e.native_id, project_id, e.cwd, worktree_root, e.transcript_path,
            state::IDLE, initial_at, e.source_at
        ],
    )?;
    let id = tx.last_insert_rowid();
    store::index_conversation(tx, id)?;
    Ok(ConvRow {
        id,
        state: state::IDLE.into(),
        state_detail: None,
        state_at: initial_at,
        hidden: false,
        hidden_at: None,
        current_instance_id: None,
        provisional_title: None,
    })
}

/// Mapping rules: explicitly mapped folder, then repository identity
/// (worktrees included), otherwise Unassigned.
fn resolve_project(
    tx: &Transaction,
    inspector: &dyn RepoInspector,
    cwd: &str,
    now: i64,
) -> Result<(Option<i64>, Option<String>)> {
    let repo = inspector.inspect(cwd);
    let worktree_root = repo.as_ref().map(|r| r.worktree_root.clone());

    let by_folder: Option<i64> = tx
        .query_row(
            "SELECT project_id FROM project_folders
             WHERE ?1 = path OR ?1 LIKE (replace(replace(replace(path, '\\', '\\\\'), '%', '\\%'), '_', '\\_') || '/%') ESCAPE '\\'
             ORDER BY length(path) DESC LIMIT 1",
            [cwd],
            |r| r.get(0),
        )
        .optional()?;
    if by_folder.is_some() {
        return Ok((by_folder, worktree_root));
    }

    let Some(repo) = repo else { return Ok((None, None)) };
    let existing: Option<i64> = tx
        .query_row("SELECT id FROM projects WHERE repo_identity = ?1", [&repo.identity], |r| r.get(0))
        .optional()?;
    let id = match existing {
        Some(id) => id,
        None => {
            tx.execute(
                "INSERT INTO projects(name, repo_identity, created_at) VALUES (?1, ?2, ?3)",
                params![repo.name, repo.identity, now],
            )?;
            tx.last_insert_rowid()
        }
    };
    Ok((Some(id), worktree_root))
}

/// Instance the event belongs to: one per `claude` or `codex` process.
fn resolve_instance(tx: &Transaction, conv: i64, current: Option<i64>, e: &Event) -> Result<Option<i64>> {
    let Some(pid) = e.pid else { return Ok(current) };
    let found: Option<i64> = tx
        .query_row(
            "SELECT id FROM instances WHERE conversation_id = ?1 AND pid = ?2 AND ended_at IS NULL
             ORDER BY id DESC LIMIT 1",
            params![conv, pid],
            |r| r.get(0),
        )
        .optional()?;
    let id = match found {
        Some(id) => {
            tx.execute(
                "UPDATE instances SET last_seen_at = MAX(last_seen_at, ?1), tty = COALESCE(?2, tty) WHERE id = ?3",
                params![e.source_at, e.tty, id],
            )?;
            id
        }
        None => {
            tx.execute(
                "INSERT INTO instances(conversation_id, pid, tty, env_app, env_guess, bundle_id, term_program, term_session, tmux,
                   entrypoint, cwd, started_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                params![
                    conv, pid, e.tty, e.env.app, e.env.is_guess(), e.env.bundle_id, e.env.term_program,
                    e.env.term_session, e.env.tmux, e.entrypoint, e.cwd, e.source_at
                ],
            )?;
            tx.last_insert_rowid()
        }
    };
    Ok(Some(id))
}

/// Provisional title from the first prompt: one line, no local commands.
pub fn provisional_title(prompt: &str) -> Option<String> {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('<'))?;
    if line.starts_with('/') && !line.contains(' ') {
        return None;
    }
    Some(truncate_chars(line, 60))
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    let cut = match cut.rfind(' ') {
        Some(pos) if pos > max / 2 => cut[..pos].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end())
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}
