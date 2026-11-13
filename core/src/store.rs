//! SQLite schema and low-level access.

use std::path::Path;

use anyhow::Result;
pub use rusqlite::Connection;
use rusqlite::{params, OptionalExtension};

const SCHEMA_VERSION: i64 = 5;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS projects(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  repo_identity TEXT UNIQUE,
  recap_excluded INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

-- Explicit folder -> project mappings (user rule).
CREATE TABLE IF NOT EXISTS project_folders(
  path TEXT PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS conversations(
  id INTEGER PRIMARY KEY,
  tool TEXT NOT NULL,
  namespace TEXT NOT NULL,
  native_id TEXT NOT NULL,
  project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
  project_confirmed INTEGER NOT NULL DEFAULT 0,
  cwd TEXT,
  worktree_root TEXT,
  transcript_path TEXT,
  provisional_title TEXT,
  title TEXT,
  user_title TEXT,
  summary TEXT,
  summary_at INTEGER,
  revision INTEGER NOT NULL DEFAULT 0,
  recap_revision INTEGER NOT NULL DEFAULT 0,
  recap_status TEXT NOT NULL DEFAULT 'pending',
  recap_due_at INTEGER,
  recap_attempts INTEGER NOT NULL DEFAULT 0,
  recap_error TEXT,
  state TEXT NOT NULL,
  state_detail TEXT,
  state_at INTEGER NOT NULL,
  last_activity_at INTEGER NOT NULL,
  first_seen_at INTEGER NOT NULL,
  hidden INTEGER NOT NULL DEFAULT 0,
  hidden_at INTEGER,
  -- Attention request already seen: reset on every state change.
  attention_seen INTEGER NOT NULL DEFAULT 0,
  -- Finished turn not looked at yet: cleared on opening it or its project,
  -- and on every state change.
  unread INTEGER NOT NULL DEFAULT 0,
  usage_synced_at INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER NOT NULL DEFAULT 0,
  current_instance_id INTEGER,
  UNIQUE(tool, namespace, native_id)
);
CREATE INDEX IF NOT EXISTS conversations_project ON conversations(project_id);

CREATE TABLE IF NOT EXISTS instances(
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  pid INTEGER,
  tty TEXT,
  env_app TEXT NOT NULL,
  env_guess INTEGER NOT NULL DEFAULT 0,
  bundle_id TEXT,
  term_program TEXT,
  term_session TEXT,
  tmux INTEGER NOT NULL DEFAULT 0,
  entrypoint TEXT,
  cwd TEXT,
  started_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  ended_at INTEGER,
  end_reason TEXT
);
CREATE INDEX IF NOT EXISTS instances_conversation ON instances(conversation_id);

CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY,
  dedup_key TEXT NOT NULL UNIQUE,
  conversation_id INTEGER REFERENCES conversations(id) ON DELETE CASCADE,
  instance_id INTEGER,
  kind TEXT NOT NULL,
  detail TEXT,
  prompt_id TEXT,
  source_at INTEGER NOT NULL,
  ingested_at INTEGER NOT NULL,
  effect TEXT
);
CREATE INDEX IF NOT EXISTS events_conversation ON events(conversation_id, source_at);

CREATE TABLE IF NOT EXISTS recaps(
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  source_revision INTEGER NOT NULL,
  theme TEXT,
  summary TEXT,
  relevant INTEGER NOT NULL,
  cursor_from INTEGER NOT NULL,
  cursor_to INTEGER NOT NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  prompt_version INTEGER NOT NULL,
  cost_usd REAL NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

-- Every provider call, reserved before it is made: the budget counts calls,
-- not only the summaries that were saved.
CREATE TABLE IF NOT EXISTS recap_calls(
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER REFERENCES conversations(id) ON DELETE SET NULL,
  cost_usd REAL NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS recap_calls_created ON recap_calls(created_at);

CREATE TABLE IF NOT EXISTS cursors(
  conversation_id INTEGER PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  file_key TEXT,
  offset INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS notifications(
  id INTEGER PRIMARY KEY,
  conversation_id INTEGER REFERENCES conversations(id) ON DELETE CASCADE,
  event_id INTEGER,
  kind TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  read INTEGER NOT NULL DEFAULT 0,
  UNIQUE(event_id, kind)
);

CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);

-- Use of the app per hour (UTC start, ms): only counters (`activity.rs`).
-- `version` grows on every change, `synced_version` is the last one the
-- remote server accepted.
CREATE TABLE IF NOT EXISTS activity(
  hour_start INTEGER PRIMARY KEY,
  panel_opens INTEGER NOT NULL DEFAULT 0,
  conversation_opens INTEGER NOT NULL DEFAULT 0,
  version INTEGER NOT NULL DEFAULT 0,
  synced_version INTEGER NOT NULL DEFAULT 0
);

-- Tokens per conversation, model and hour (UTC start, ms), copied from the
-- transcripts before the agents delete them (`usage.rs`). No foreign key: the
-- usage outlives the conversation.
CREATE TABLE IF NOT EXISTS token_usage(
  conversation_id INTEGER NOT NULL,
  model TEXT NOT NULL,
  hour_start INTEGER NOT NULL,
  subagent INTEGER NOT NULL,
  input INTEGER NOT NULL DEFAULT 0,
  cache_write INTEGER NOT NULL DEFAULT 0,
  cache_write_1h INTEGER NOT NULL DEFAULT 0,
  cache_read INTEGER NOT NULL DEFAULT 0,
  output INTEGER NOT NULL DEFAULT 0,
  reasoning INTEGER NOT NULL DEFAULT 0,
  requests INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(conversation_id, model, hour_start, subagent)
);
CREATE INDEX IF NOT EXISTS token_usage_hour ON token_usage(hour_start);

CREATE VIRTUAL TABLE IF NOT EXISTS conversation_search
  USING fts5(title, summary, tokenize = 'unicode61 remove_diacritics 2');
"#;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        crate::paths::ensure_private_dir(dir)?;
    }
    let conn = Connection::open(path)?;
    configure(&conn)?;
    for suffix in ["", "-wal", "-shm"] {
        crate::paths::restrict_file(Path::new(&format!("{}{suffix}", path.display())));
    }
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.execute_batch(SCHEMA)?;
    migrate(conn)?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

/// Columns added after the first version: `CREATE TABLE IF NOT EXISTS` does
/// not add them to existing databases.
fn migrate(conn: &Connection) -> Result<()> {
    // `usage_synced_at`: last read of the transcript's usage; `started_at`: first
    // record of the transcript, 0 while unknown.
    for column in ["attention_seen", "unread", "usage_synced_at", "started_at"] {
        let present: bool = conn.query_row(
            "SELECT count(*) > 0 FROM pragma_table_info('conversations') WHERE name = ?1",
            [column],
            |r| r.get(0),
        )?;
        if !present {
            conn.execute_batch(&format!("ALTER TABLE conversations ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0"))?;
        }
    }
    // `cache_write_1h` (the part of `cache_write` priced as the 1-hour cache):
    // the transcripts still there are read again to fill it, in the same
    // transaction so an interruption cannot skip the reading.
    let has_1h: bool = conn.query_row(
        "SELECT count(*) > 0 FROM pragma_table_info('token_usage') WHERE name = 'cache_write_1h'",
        [],
        |r| r.get(0),
    )?;
    if !has_1h {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "ALTER TABLE token_usage ADD COLUMN cache_write_1h INTEGER NOT NULL DEFAULT 0;
             UPDATE conversations SET usage_synced_at = 0;",
        )?;
        tx.commit()?;
    }
    // The call ledger starts from the summaries already saved, so the budget
    // window does not reset when updating. Marker check, copy and marker are one
    // transaction: an interruption in between cannot copy the history twice.
    let tx = conn.unchecked_transaction()?;
    if meta(&tx, "recap_calls_seeded")?.is_none() {
        tx.execute(
            "INSERT INTO recap_calls(conversation_id, cost_usd, created_at)
             SELECT conversation_id, cost_usd, created_at FROM recaps",
            [],
        )?;
        set_meta(&tx, "recap_calls_seeded", "1")?;
    }
    tx.commit()?;
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn delete_meta(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    Ok(())
}

/// Updates the search index for a conversation.
pub fn index_conversation(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM conversation_search WHERE rowid = ?1", [id])?;
    conn.execute(
        "INSERT INTO conversation_search(rowid, title, summary)
         SELECT id, COALESCE(user_title, title, provisional_title, ''), COALESCE(summary, '')
         FROM conversations WHERE id = ?1",
        [id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_created_and_fts5_is_available() {
        let conn = open_in_memory().unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM conversation_search WHERE conversation_search MATCH 'x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        set_setting(&conn, "a", "1").unwrap();
        set_setting(&conn, "a", "2").unwrap();
        assert_eq!(get_setting(&conn, "a").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn call_ledger_starts_from_the_summaries_saved_before_it_existed() {
        let conn = open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO conversations(id, tool, namespace, native_id, state, state_at, last_activity_at, first_seen_at)
             VALUES (1, 'claude', 'd', 's', 'idle', 0, 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recaps(conversation_id, source_revision, relevant, cursor_from, cursor_to, provider, model,
               prompt_version, cost_usd, created_at) VALUES (1, 1, 1, 0, 1, 'p', 'm', 2, 0.5, 1000)",
            [],
        )
        .unwrap();
        // A database from before the ledger: no table content and no seed marker.
        conn.execute("DELETE FROM recap_calls", []).unwrap();
        conn.execute("DELETE FROM meta WHERE key = 'recap_calls_seeded'", []).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let (n, cost): (i64, f64) =
            conn.query_row("SELECT count(*), SUM(cost_usd) FROM recap_calls", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((n, cost), (1, 0.5), "seeded once");
    }

    #[test]
    fn existing_database_gains_new_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE conversations(id INTEGER PRIMARY KEY, project_id INTEGER, state TEXT NOT NULL);
             INSERT INTO conversations(id, state) VALUES (1, 'attention');",
        )
        .unwrap();
        configure(&conn).unwrap();
        let seen: bool = conn.query_row("SELECT attention_seen FROM conversations WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert!(!seen);
        let unread: bool = conn.query_row("SELECT unread FROM conversations WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert!(!unread);
        let version: String = conn.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn saved_usage_gains_the_hour_cache_and_is_read_again_once() {
        let conn = open_in_memory().unwrap();
        // A database from before `cache_write_1h`.
        conn.execute_batch(
            "INSERT INTO conversations(id, tool, namespace, native_id, state, state_at, last_activity_at, first_seen_at,
               usage_synced_at) VALUES (1, 'claude', 'd', 's', 'idle', 0, 0, 0, 500);
             ALTER TABLE token_usage DROP COLUMN cache_write_1h;",
        )
        .unwrap();
        let synced = |conn: &Connection| -> i64 {
            conn.query_row("SELECT usage_synced_at FROM conversations WHERE id = 1", [], |r| r.get(0)).unwrap()
        };
        migrate(&conn).unwrap();
        assert_eq!(synced(&conn), 0);
        conn.execute("UPDATE conversations SET usage_synced_at = 500", []).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(synced(&conn), 500, "read again only once");
    }

    #[test]
    fn an_interrupted_ledger_migration_does_not_copy_the_history_twice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let conn = open(&path).unwrap();
        conn.execute(
            "INSERT INTO conversations(id, tool, namespace, native_id, state, state_at, last_activity_at, first_seen_at)
             VALUES (1, 'claude', 'd', 's', 'idle', 0, 0, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recaps(conversation_id, source_revision, relevant, cursor_from, cursor_to, provider, model,
               prompt_version, cost_usd, created_at) VALUES (1, 1, 1, 0, 1, 'p', 'm', 2, 0.42, 100)",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM recap_calls", []).unwrap();
        conn.execute("DELETE FROM meta WHERE key = 'recap_calls_seeded'", []).unwrap();
        // Fault injected on the marker write, after the copy.
        conn.execute_batch(
            "CREATE TRIGGER fail_seed BEFORE INSERT ON meta WHEN NEW.key = 'recap_calls_seeded'
             BEGIN SELECT RAISE(ABORT, 'synthetic fault'); END;",
        )
        .unwrap();
        assert!(migrate(&conn).is_err());
        let copied: i64 = conn.query_row("SELECT count(*) FROM recap_calls", [], |r| r.get(0)).unwrap();
        assert_eq!(copied, 0, "the copy was rolled back with the marker");
        conn.execute_batch("DROP TRIGGER fail_seed").unwrap();
        drop(conn);
        let conn = open(&path).unwrap();
        let (n, cost): (i64, f64) =
            conn.query_row("SELECT count(*), SUM(cost_usd) FROM recap_calls", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((n, cost), (1, 0.42));
    }
}
