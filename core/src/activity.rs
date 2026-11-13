//! Use of the app: how many times the panel opens and how many times it
//! takes you to a conversation, counted per hour. Only counters: nothing about
//! the conversations, projects or folders.
//!
//! The counters always stay on this Mac (Activity view). When a server is set
//! in the settings they are also sent to it, as absolute values per hour: the
//! server replaces what it has for that hour, so a request sent twice never
//! counts twice. Each row has a `version` that grows on every change and the
//! `synced_version` the server last accepted: what differs is still to send,
//! including the hours that changed while a request was in flight.

use std::io::Read;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

use crate::store;

pub const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// History kept on this Mac: enough for a year's comparison.
pub const KEEP_DAYS: i64 = 400;
/// Hours per request.
pub const BATCH: usize = 500;
/// Version of the request body, in `schema`.
pub const PROTOCOL: u32 = 1;

const META_INSTALL_ID: &str = "activity_install_id";
const META_LAST_SYNC: &str = "activity_last_sync_at";
const META_LAST_ATTEMPT: &str = "activity_last_attempt_at";
const META_LAST_ERROR: &str = "activity_last_error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counter {
    /// The panel became visible.
    PanelOpen,
    /// Open conversation or Resume from the panel.
    ConversationOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hour {
    /// Start of the hour, UTC, in ms.
    pub hour_start: i64,
    pub panel_opens: i64,
    pub conversation_opens: i64,
}

/// An hour still to send, with the version it was read at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub hour: Hour,
    pub version: i64,
}

pub fn hour_start(ms: i64) -> i64 {
    ms - ms.rem_euclid(HOUR_MS)
}

pub fn record(conn: &Connection, counter: Counter, now: i64) -> Result<()> {
    let (panel, conversation) = match counter {
        Counter::PanelOpen => (1, 0),
        Counter::ConversationOpen => (0, 1),
    };
    conn.execute(
        "INSERT INTO activity(hour_start, panel_opens, conversation_opens, version) VALUES (?1, ?2, ?3, 1)
         ON CONFLICT(hour_start) DO UPDATE SET
           panel_opens = panel_opens + excluded.panel_opens,
           conversation_opens = conversation_opens + excluded.conversation_opens,
           version = version + 1",
        params![hour_start(now), panel, conversation],
    )?;
    Ok(())
}

/// Hours from `since` on, oldest first.
pub fn hours(conn: &Connection, since: i64) -> Result<Vec<Hour>> {
    let mut stmt = conn.prepare(
        "SELECT hour_start, panel_opens, conversation_opens FROM activity WHERE hour_start >= ?1 ORDER BY hour_start",
    )?;
    let rows = stmt.query_map([hour_start(since)], |r| {
        Ok(Hour { hour_start: r.get(0)?, panel_opens: r.get(1)?, conversation_opens: r.get(2)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// First hour still in the history, to say since when the app counts.
pub fn first_hour(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT MIN(hour_start) FROM activity", [], |r| r.get(0))?)
}

pub fn pending(conn: &Connection, limit: usize) -> Result<Vec<Pending>> {
    let mut stmt = conn.prepare(
        "SELECT hour_start, panel_opens, conversation_opens, version FROM activity
         WHERE version > synced_version ORDER BY hour_start LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        Ok(Pending {
            hour: Hour { hour_start: r.get(0)?, panel_opens: r.get(1)?, conversation_opens: r.get(2)? },
            version: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn pending_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT count(*) FROM activity WHERE version > synced_version", [], |r| r.get(0))?)
}

/// The server accepted these hours as they were read: an hour that changed in
/// the meantime has a higher version and stays pending.
pub fn mark_synced(conn: &Connection, sent: &[Pending]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for p in sent {
        tx.execute(
            "UPDATE activity SET synced_version = ?2 WHERE hour_start = ?1 AND synced_version < ?2",
            params![p.hour.hour_start, p.version],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn prune(conn: &Connection, now: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM activity WHERE hour_start < ?1", [now - KEEP_DAYS * DAY_MS])?)
}

/// Random identifier of this installation, created on first use. It says
/// which rows belong together, not who you are.
pub fn install_id(conn: &Connection) -> Result<String> {
    if let Some(id) = store::meta(conn, META_INSTALL_ID)? {
        return Ok(id);
    }
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    store::set_meta(conn, META_INSTALL_ID, &id)?;
    Ok(id)
}

/// Request body, protocol version 1 (described in the README).
pub fn payload(install_id: &str, app_version: &str, rows: &[Pending], now: i64) -> Value {
    let hours: Vec<Value> = rows
        .iter()
        .map(|p| {
            json!({
                "hourStart": p.hour.hour_start,
                "utcOffsetMinutes": utc_offset_minutes(p.hour.hour_start),
                "panelOpens": p.hour.panel_opens,
                "conversationOpens": p.hour.conversation_opens,
            })
        })
        .collect();
    json!({
        "schema": PROTOCOL,
        "installId": install_id,
        "app": "flow-keeper",
        "appVersion": app_version,
        "platform": std::env::consts::OS,
        "sentAt": now,
        "hours": hours,
    })
}

/// Offset of the local time zone at `ms`, so the server can tell the local hour.
#[cfg(unix)]
fn utc_offset_minutes(ms: i64) -> i64 {
    let t = (ms / 1000) as libc::time_t;
    // SAFETY: `localtime_r` only writes into the `tm` it is given.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return 0;
    }
    tm.tm_gmtoff as i64 / 60
}

#[cfg(not(unix))]
fn utc_offset_minutes(_ms: i64) -> i64 {
    0
}

/// Sends one request. Any 2xx answer means the hours are saved; redirects
/// are not followed, so the token only goes to the address that was set.
pub fn send(endpoint: &str, token: &str, body: &Value) -> Result<()> {
    use ureq::tls::{RootCerts, TlsConfig};
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(4)))
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .max_redirects(0)
        // The system's certificates: a server signed by a private
        // authority is trusted as it is in the browser.
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into();
    let mut req = agent.post(endpoint).header("content-type", "application/json");
    if !token.is_empty() {
        req = req.header("authorization", &format!("Bearer {token}"));
    }
    let resp = req.send_json(body).map_err(|e| anyhow!("server unreachable: {e}"))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        bail!("the server answered {status}");
    }
    Ok(())
}

pub fn finish_attempt(conn: &Connection, now: i64, error: Option<&str>) -> Result<()> {
    store::set_meta(conn, META_LAST_ATTEMPT, &now.to_string())?;
    match error {
        None => {
            store::set_meta(conn, META_LAST_SYNC, &now.to_string())?;
            store::delete_meta(conn, META_LAST_ERROR)
        }
        Some(e) => store::set_meta(conn, META_LAST_ERROR, e),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub pending_hours: i64,
    pub last_sync_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    /// Error of the last attempt; `None` when it went well.
    pub last_error: Option<String>,
}

pub fn status(conn: &Connection) -> Result<SyncStatus> {
    let ms = |key| -> Result<Option<i64>> { Ok(store::meta(conn, key)?.and_then(|v| v.parse().ok())) };
    Ok(SyncStatus {
        pending_hours: pending_count(conn)?,
        last_sync_at: ms(META_LAST_SYNC)?,
        last_attempt_at: ms(META_LAST_ATTEMPT)?,
        last_error: store::meta(conn, META_LAST_ERROR)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_762_000_000_000;

    #[test]
    fn counters_add_up_per_hour() {
        let conn = store::open_in_memory().unwrap();
        record(&conn, Counter::PanelOpen, T0).unwrap();
        record(&conn, Counter::PanelOpen, T0 + 1000).unwrap();
        record(&conn, Counter::ConversationOpen, T0 + 2000).unwrap();
        record(&conn, Counter::PanelOpen, T0 + HOUR_MS).unwrap();
        let h = hours(&conn, 0).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].hour_start, h[0].panel_opens, h[0].conversation_opens), (hour_start(T0), 2, 1));
        assert_eq!((h[1].panel_opens, h[1].conversation_opens), (1, 0));
        assert_eq!(h[0].hour_start % HOUR_MS, 0);
        assert_eq!(first_hour(&conn).unwrap(), Some(hour_start(T0)));
        assert_eq!(hours(&conn, T0 + HOUR_MS).unwrap().len(), 1);
    }

    #[test]
    fn an_hour_that_changes_while_it_is_sent_stays_pending() {
        let conn = store::open_in_memory().unwrap();
        record(&conn, Counter::PanelOpen, T0).unwrap();
        record(&conn, Counter::PanelOpen, T0 + HOUR_MS).unwrap();
        let batch = pending(&conn, BATCH).unwrap();
        assert_eq!(batch.len(), 2);
        // A click arrives during the request.
        record(&conn, Counter::ConversationOpen, T0 + HOUR_MS).unwrap();
        mark_synced(&conn, &batch).unwrap();
        let left = pending(&conn, BATCH).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].hour.panel_opens, left[0].hour.conversation_opens), (1, 1), "sent again with the new total");
        mark_synced(&conn, &left).unwrap();
        assert_eq!(pending_count(&conn).unwrap(), 0);
        // An old confirmation never brings a row back to pending or forward past its version.
        mark_synced(&conn, &batch).unwrap();
        assert_eq!(pending_count(&conn).unwrap(), 0);
    }

    #[test]
    fn batches_start_from_the_oldest_hours() {
        let conn = store::open_in_memory().unwrap();
        for i in 0..5 {
            record(&conn, Counter::PanelOpen, T0 + i * HOUR_MS).unwrap();
        }
        let first = pending(&conn, 2).unwrap();
        assert_eq!(first.iter().map(|p| p.hour.hour_start).collect::<Vec<_>>(), [hour_start(T0), hour_start(T0) + HOUR_MS]);
        mark_synced(&conn, &first).unwrap();
        assert_eq!(pending_count(&conn).unwrap(), 3);
    }

    #[test]
    fn old_history_is_pruned() {
        let conn = store::open_in_memory().unwrap();
        record(&conn, Counter::PanelOpen, T0 - (KEEP_DAYS + 1) * DAY_MS).unwrap();
        record(&conn, Counter::PanelOpen, T0 - DAY_MS).unwrap();
        assert_eq!(prune(&conn, T0).unwrap(), 1);
        assert_eq!(hours(&conn, 0).unwrap().len(), 1);
    }

    #[test]
    fn the_install_id_is_random_and_stable() {
        let a = store::open_in_memory().unwrap();
        let b = store::open_in_memory().unwrap();
        let id = install_id(&a).unwrap();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(install_id(&a).unwrap(), id);
        assert_ne!(install_id(&b).unwrap(), id);
    }

    #[test]
    fn the_payload_carries_only_counters() {
        let conn = store::open_in_memory().unwrap();
        record(&conn, Counter::ConversationOpen, T0).unwrap();
        let rows = pending(&conn, BATCH).unwrap();
        let body = payload("abc", "1.2.3", &rows, T0);
        assert_eq!(body["schema"], 1);
        assert_eq!(body["installId"], "abc");
        assert_eq!(body["appVersion"], "1.2.3");
        let hour = &body["hours"][0];
        assert_eq!(hour["hourStart"], hour_start(T0));
        assert_eq!((hour["panelOpens"].as_i64(), hour["conversationOpens"].as_i64()), (Some(0), Some(1)));
        assert!(hour["utcOffsetMinutes"].is_i64());
        let mut keys: Vec<_> = hour.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["conversationOpens", "hourStart", "panelOpens", "utcOffsetMinutes"]);
    }

    /// Local server answering `status` once; returns the address and the request it received.
    fn serve_once(status: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/usage", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut stream = stream;
            write!(stream, "HTTP/1.1 {status}\r\nlocation: http://127.0.0.1:9/elsewhere\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .unwrap();
            head + &String::from_utf8(body).unwrap()
        });
        (url, handle)
    }

    #[test]
    fn the_request_carries_the_body_and_the_token() {
        let (url, server) = serve_once("204 No Content");
        let body = json!({ "schema": 1, "hours": [] });
        send(&url, "s3cret", &body).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /v1/usage HTTP/1.1"), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer s3cret"), "{request}");
        let sent: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(sent, body);
    }

    #[test]
    fn errors_and_redirects_leave_the_hours_pending() {
        for status in ["500 Internal Server Error", "401 Unauthorized", "302 Found"] {
            let (url, server) = serve_once(status);
            let err = send(&url, "", &json!({})).unwrap_err().to_string();
            assert!(err.contains(&status[..3]), "{status}: {err}");
            let request = server.join().unwrap();
            assert!(!request.to_ascii_lowercase().contains("authorization"), "no token, no header");
        }
        // Nobody listening: fails without waiting for the global timeout.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let started = std::time::Instant::now();
        assert!(send(&format!("http://127.0.0.1:{port}/"), "", &json!({})).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn status_reports_the_last_attempt() {
        let conn = store::open_in_memory().unwrap();
        record(&conn, Counter::PanelOpen, T0).unwrap();
        finish_attempt(&conn, T0, Some("server unreachable")).unwrap();
        let s = status(&conn).unwrap();
        assert_eq!((s.pending_hours, s.last_sync_at, s.last_attempt_at), (1, None, Some(T0)));
        assert_eq!(s.last_error.as_deref(), Some("server unreachable"));
        finish_attempt(&conn, T0 + 1, None).unwrap();
        let s = status(&conn).unwrap();
        assert_eq!((s.last_sync_at, s.last_error), (Some(T0 + 1), None));
    }
}
