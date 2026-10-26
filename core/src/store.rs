//! SQLite schema and low-level access.

use std::path::Path;

use anyhow::Result;
pub use rusqlite::Connection;
use rusqlite::{params, OptionalExtension};

const SCHEMA_VERSION: i64 = 2;

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
    let has_seen: bool = conn.query_row(
        "SELECT count(*) > 0 FROM pragma_table_info('conversations') WHERE name = 'attention_seen'",
        [],
        |r| r.get(0),
    )?;
    if !has_seen {
        conn.execute_batch("ALTER TABLE conversations ADD COLUMN attention_seen INTEGER NOT NULL DEFAULT 0")?;
    }
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
        let version: String = conn.query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION.to_string());
    }
}
