//! Token usage of the conversations, read from the Claude Code transcript or
//! the Codex rollout (internal formats: see `docs/compatibility.md`) and saved
//! in the store.
//!
//! Claude Code writes `message.usage` on every assistant record; a response
//! with several content blocks spans several records with the same
//! `message.id`, so each response is counted once. Subagents write their own
//! transcripts next to the session's. Codex writes cumulative totals in its
//! `token_count` events: each increase is the usage of that request.
//!
//! The agents delete old transcripts (Claude Code after 30 days by default),
//! so the usage is copied in `token_usage`, per conversation, model and hour.
//! Every read replaces a figure only with a higher one: a transcript that is
//! gone or shorter never lowers what was saved.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::activity::hour_start;
use crate::pricing;

/// A conversation is read again until this long after its last activity:
/// the transcript gets the last response a moment after the event.
const SETTLE_MS: i64 = 60_000;
/// Model of the records that don't say it.
const UNKNOWN_MODEL: &str = "unknown";

/// Tokens split as the agents report them. `input` excludes cached tokens
/// for both agents (Codex counts them inside `input_tokens`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub cache_write: u64,
    /// Part of `cache_write` kept for an hour, which costs more (Claude Code).
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    /// Part of `output` spent on reasoning, when the agent reports it.
    pub reasoning: u64,
}

impl Tokens {
    fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cache_write_1h += o.cache_write_1h;
        self.cache_read += o.cache_read;
        self.output += o.output;
        self.reasoning += o.reasoning;
    }

    pub fn total(&self) -> u64 {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Whole conversation, subagents included.
    pub tokens: Tokens,
    /// Share of `tokens` spent by subagents (Claude Code only).
    pub subagents: Option<Tokens>,
    /// Model responses counted.
    pub requests: u64,
    pub models: Vec<String>,
    /// The transcript can't be read anymore: the figures are the saved ones.
    pub from_history: bool,
}

/// Usage of one model in one hour, by the session or by its subagents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub model: String,
    /// Start of the hour, UTC, in ms; 0 when the records have no time.
    pub hour_start: i64,
    pub subagent: bool,
    pub tokens: Tokens,
    pub requests: u64,
}

/// What a transcript says about its usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub buckets: Vec<Bucket>,
    /// Time of the first record: when the conversation started.
    pub started_at: Option<i64>,
}

pub fn read(tool: &str, transcript: &Path) -> Result<Snapshot> {
    let buckets = match tool {
        "codex" => read_codex(transcript)?,
        _ => read_claude(transcript)?,
    };
    Ok(Snapshot { buckets, started_at: first_timestamp(transcript) })
}

/// The usage of a whole conversation from its buckets.
pub fn summarize(buckets: &[Bucket]) -> Usage {
    let mut u = Usage::default();
    let mut models = BTreeSet::new();
    for b in buckets {
        u.tokens.add(&b.tokens);
        u.requests += b.requests;
        if b.subagent {
            u.subagents.get_or_insert_with(Tokens::default).add(&b.tokens);
        }
        if b.model != UNKNOWN_MODEL {
            models.insert(b.model.clone());
        }
    }
    u.models = models.into_iter().collect();
    u
}

/// Calls `f` on the JSON records whose line contains `needle`: the check on
/// the raw bytes skips the bulk of the file (tool output, file contents).
fn scan(path: &Path, needles: &[&[u8]], mut f: impl FnMut(Value)) -> Result<()> {
    let mut reader = BufReader::with_capacity(256 * 1024, File::open(path)?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if !needles.iter().any(|n| line.windows(n.len()).any(|w| w == *n)) {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(&line) {
            f(v);
        }
    }
}

fn num(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn record_time(v: &Value) -> Option<i64> {
    v.get("timestamp").and_then(Value::as_str).and_then(parse_time)
}

/// `2026-11-04T15:59:13.264Z` or with an offset (`+01:00`) → ms since the epoch.
pub fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let int = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d) = (int(0..4)?, int(5..7)?, int(8..10)?);
    let (h, mi, sec) = (int(11..13)?, int(14..16)?, int(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut ms = 0;
    if b.get(i) == Some(&b'.') {
        let digits = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        let frac = s.get(i + 1..i + 1 + digits.min(3))?;
        ms = format!("{frac:0<3}").parse().ok()?;
        i += 1 + digits;
    }
    let offset_min = match b.get(i) {
        Some(b'Z') | None => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let oh = int(i + 1..i + 3)?;
            let om = if b.get(i + 3) == Some(&b':') { int(i + 4..i + 6)? } else { int(i + 3..i + 5)? };
            (oh * 60 + om) * if sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    // Days from the civil date (H. Hinnant's algorithm).
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi - offset_min) * 60_000 + sec * 1000 + ms)
}

/// The first record with a time, among the first lines.
fn first_timestamp(path: &Path) -> Option<i64> {
    let reader = BufReader::new(File::open(path).ok()?);
    reader
        .split(b'\n')
        .take(200)
        .map_while(|l| l.ok())
        .filter_map(|l| serde_json::from_slice::<Value>(&l).ok())
        .find_map(|v| record_time(&v))
}

type Buckets = BTreeMap<(String, i64, bool), (Tokens, u64)>;

fn add_to(out: &mut Buckets, model: String, hour: i64, subagent: bool, t: &Tokens) {
    let slot = out.entry((model, hour, subagent)).or_default();
    slot.0.add(t);
    slot.1 += 1;
}

fn claude_file(path: &Path, subagent: bool, out: &mut Buckets) -> Result<()> {
    // The last record of a response carries its final count.
    let mut responses: HashMap<String, (String, i64, Tokens)> = HashMap::new();
    let mut anonymous: Vec<(String, i64, Tokens)> = Vec::new();
    scan(path, &[b"\"usage\""], |v| {
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(msg) = v.get("message") else { return };
        let Some(u) = msg.get("usage").filter(|u| u.is_object()) else { return };
        let model = msg.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()).unwrap_or(UNKNOWN_MODEL);
        if model == "<synthetic>" {
            return; // local message, no model call
        }
        let t = Tokens {
            input: num(u, "input_tokens"),
            cache_write: num(u, "cache_creation_input_tokens"),
            cache_write_1h: u.get("cache_creation").map(|c| num(c, "ephemeral_1h_input_tokens")).unwrap_or(0),
            cache_read: num(u, "cache_read_input_tokens"),
            output: num(u, "output_tokens"),
            reasoning: u.get("output_tokens_details").map(|d| num(d, "thinking_tokens")).unwrap_or(0),
        };
        let hour = record_time(&v).map(hour_start).unwrap_or(0);
        match msg.get("id").and_then(Value::as_str) {
            Some(id) => {
                responses.insert(id.to_string(), (model.to_string(), hour, t));
            }
            None => anonymous.push((model.to_string(), hour, t)),
        }
    })?;
    for (model, hour, t) in responses.into_values().chain(anonymous) {
        add_to(out, model, hour, subagent, &t);
    }
    Ok(())
}

fn into_buckets(map: Buckets) -> Vec<Bucket> {
    map.into_iter()
        .map(|((model, hour_start, subagent), (tokens, requests))| Bucket { model, hour_start, subagent, tokens, requests })
        .collect()
}

/// `<dir>/<session>.jsonl` → `<dir>/<session>/subagents/*.jsonl`.
fn subagent_files(transcript: &Path) -> Vec<PathBuf> {
    let Some(stem) = transcript.file_stem() else { return Vec::new() };
    let dir = transcript.with_file_name(stem).join("subagents");
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
    files.sort();
    files
}

fn read_claude(transcript: &Path) -> Result<Vec<Bucket>> {
    let mut map = Buckets::new();
    claude_file(transcript, false, &mut map)?;
    for f in subagent_files(transcript) {
        // A subagent transcript being written or removed doesn't stop the rest.
        let _ = claude_file(&f, true, &mut map);
    }
    Ok(into_buckets(map))
}

const CODEX_FIELDS: [&str; 5] =
    ["input_tokens", "cached_input_tokens", "cache_write_input_tokens", "output_tokens", "reasoning_output_tokens"];

fn read_codex(rollout: &Path) -> Result<Vec<Bucket>> {
    let mut map = Buckets::new();
    let mut model = UNKNOWN_MODEL.to_string();
    // Cumulative totals so far, in the order of `CODEX_FIELDS`.
    let mut last = [0u64; 5];
    let mut last_total = 0;
    scan(rollout, &[b"\"token_count\"", b"\"turn_context\""], |v| {
        let p = v.get("payload");
        match v.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                if let Some(m) = p.and_then(|p| p.get("model")).and_then(Value::as_str) {
                    model = m.to_string();
                }
            }
            Some("event_msg") if p.and_then(|p| p.get("type")).and_then(Value::as_str) == Some("token_count") => {
                let Some(u) = p.and_then(|p| p.get("info")).and_then(|i| i.get("total_token_usage")) else { return };
                let total = num(u, "total_tokens");
                // The same totals are repeated when only the rate limits change.
                if total <= last_total {
                    return;
                }
                last_total = total;
                let mut d = [0u64; 5];
                for (i, key) in CODEX_FIELDS.iter().enumerate() {
                    let now = num(u, key);
                    d[i] = now.saturating_sub(last[i]);
                    last[i] = last[i].max(now);
                }
                // `input_tokens` includes the cached ones.
                let t = Tokens {
                    input: d[0].saturating_sub(d[1]),
                    cache_read: d[1],
                    cache_write: d[2],
                    cache_write_1h: 0,
                    output: d[3],
                    reasoning: d[4],
                };
                let hour = record_time(&v).map(hour_start).unwrap_or(0);
                add_to(&mut map, model.clone(), hour, false, &t);
            }
            _ => {}
        }
    })?;
    Ok(into_buckets(map))
}

/// Agent and transcript of a conversation; `None` when it has no transcript yet.
pub fn transcript_of(conn: &Connection, id: i64) -> Result<Option<(String, PathBuf)>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row("SELECT tool, transcript_path FROM conversations WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    Ok(row.and_then(|(tool, path)| path.map(|p| (tool, PathBuf::from(p)))))
}

// --- saved usage ---------------------------------------------------------------

/// Conversations whose transcript may have new usage: never read, or active
/// since the last read. Most recent first.
pub fn due(conn: &Connection, limit: usize) -> Result<Vec<(i64, String, PathBuf)>> {
    let mut stmt = conn.prepare(
        "SELECT id, tool, transcript_path FROM conversations
         WHERE transcript_path IS NOT NULL AND usage_synced_at < last_activity_at + ?1
         ORDER BY last_activity_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![SETTLE_MS, limit as i64], |r| {
        Ok((r.get(0)?, r.get(1)?, PathBuf::from(r.get::<_, String>(2)?)))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Saves what a read found and marks the conversation as read at `now`. A
/// failed read (transcript deleted) leaves the saved usage as it is.
pub fn save(conn: &Connection, id: i64, read: Option<&Snapshot>, now: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    if let Some(s) = read {
        for b in &s.buckets {
            let t = &b.tokens;
            tx.execute(
                "INSERT INTO token_usage(conversation_id, model, hour_start, subagent,
                   input, cache_write, cache_write_1h, cache_read, output, reasoning, requests)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(conversation_id, model, hour_start, subagent) DO UPDATE SET
                   input = MAX(input, excluded.input),
                   cache_write = MAX(cache_write, excluded.cache_write),
                   cache_write_1h = MAX(cache_write_1h, excluded.cache_write_1h),
                   cache_read = MAX(cache_read, excluded.cache_read),
                   output = MAX(output, excluded.output),
                   reasoning = MAX(reasoning, excluded.reasoning),
                   requests = MAX(requests, excluded.requests)",
                params![
                    id,
                    b.model,
                    b.hour_start,
                    b.subagent,
                    t.input as i64,
                    t.cache_write as i64,
                    t.cache_write_1h as i64,
                    t.cache_read as i64,
                    t.output as i64,
                    t.reasoning as i64,
                    b.requests as i64
                ],
            )?;
        }
        if let Some(at) = s.started_at {
            tx.execute(
                "UPDATE conversations SET started_at = ?2 WHERE id = ?1 AND (started_at = 0 OR started_at > ?2)",
                params![id, at],
            )?;
        }
    }
    tx.execute("UPDATE conversations SET usage_synced_at = ?2 WHERE id = ?1", params![id, now])?;
    tx.commit()?;
    Ok(())
}

const SUMS: &str = "SUM(u.input), SUM(u.cache_write), SUM(u.cache_write_1h), SUM(u.cache_read), SUM(u.output),
  SUM(u.reasoning), SUM(u.requests)";
/// Estimated cost in USD of the rows (see `pricing`); after `SUMS`, at `from + 7`.
const COST: &str =
    "COALESCE(SUM(token_cost(u.model, u.input, u.cache_write, u.cache_write_1h, u.cache_read, u.output)), 0)";

fn sums(r: &rusqlite::Row, from: usize) -> rusqlite::Result<(Tokens, u64)> {
    let n = |i: usize| -> rusqlite::Result<u64> { Ok(r.get::<_, Option<i64>>(from + i)?.unwrap_or(0).max(0) as u64) };
    let tokens = Tokens {
        input: n(0)?,
        cache_write: n(1)?,
        cache_write_1h: n(2)?,
        cache_read: n(3)?,
        output: n(4)?,
        reasoning: n(5)?,
    };
    Ok((tokens, n(6)?))
}

/// `token_cost(model, input, cache_write, cache_write_1h, cache_read, output)`
/// for `COST`: NULL for the models without a price.
fn register_cost(conn: &Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    conn.create_scalar_function("token_cost", 6, FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC, |ctx| {
        let n = |i: usize| -> rusqlite::Result<u64> { Ok(ctx.get::<i64>(i)?.max(0) as u64) };
        let t = Tokens {
            input: n(1)?,
            cache_write: n(2)?,
            cache_write_1h: n(3)?,
            cache_read: n(4)?,
            output: n(5)?,
            reasoning: 0,
        };
        Ok(pricing::cost(&ctx.get::<String>(0)?, &t))
    })
}

/// The saved usage of a conversation; `None` when nothing was saved.
pub fn saved(conn: &Connection, id: i64) -> Result<Option<Usage>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT u.model, u.subagent, {SUMS} FROM token_usage u WHERE u.conversation_id = ?1 GROUP BY u.model, u.subagent"
    ))?;
    let buckets = stmt
        .query_map([id], |r| {
            let (tokens, requests) = sums(r, 2)?;
            Ok(Bucket { model: r.get(0)?, hour_start: 0, subagent: r.get(1)?, tokens, requests })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((!buckets.is_empty()).then(|| summarize(&buckets)))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourTokens {
    pub hour_start: i64,
    pub tokens: Tokens,
    /// Estimated, in USD: Claude models only.
    pub cost: f64,
}

/// A project's or a model's part of the usage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Share {
    /// Project id; `None` for Unassigned and for models.
    pub id: Option<i64>,
    pub name: String,
    pub tokens: Tokens,
    pub cost: f64,
    pub requests: u64,
    pub conversations: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// First hour saved: since when the app counts.
    pub first_hour: Option<i64>,
    pub tokens: Tokens,
    pub cost: f64,
    pub requests: u64,
    pub conversations: u64,
    /// Usage between `previous_since` and `previous_until`, for the comparison.
    pub previous: Option<Tokens>,
    pub previous_cost: Option<f64>,
    /// From `since` on, oldest first.
    pub hours: Vec<HourTokens>,
    /// Most used first.
    pub projects: Vec<Share>,
    pub models: Vec<Share>,
    /// Claude models used in the period that have no price: not in `cost`.
    pub unpriced: Vec<String>,
}

/// Usage of all conversations from `since` (ms; `None` = everything saved).
/// The previous period ends before the hour of `previous_until` (`None` = `since`).
pub fn stats(conn: &Connection, since: Option<i64>, previous_since: Option<i64>, previous_until: Option<i64>) -> Result<Stats> {
    register_cost(conn)?;
    let from = since.map(hour_start).unwrap_or(i64::MIN);
    let first_hour: Option<i64> =
        conn.query_row("SELECT MIN(hour_start) FROM token_usage WHERE hour_start > 0", [], |r| r.get(0))?;
    let (tokens, requests, cost, conversations) = conn.query_row(
        &format!("SELECT {SUMS}, {COST}, COUNT(DISTINCT u.conversation_id) FROM token_usage u WHERE u.hour_start >= ?1"),
        [from],
        |r| {
            let (t, n) = sums(r, 0)?;
            Ok((t, n, r.get::<_, f64>(7)?, r.get::<_, i64>(8)? as u64))
        },
    )?;
    let (previous, previous_cost) = match (since, previous_since) {
        (Some(to), Some(prev)) => {
            let (t, c) = conn.query_row(
                &format!("SELECT {SUMS}, {COST} FROM token_usage u WHERE u.hour_start >= ?1 AND u.hour_start < ?2"),
                [hour_start(prev), hour_start(previous_until.unwrap_or(to))],
                |r| Ok((sums(r, 0)?.0, r.get::<_, f64>(7)?)),
            )?;
            (Some(t), Some(c))
        }
        _ => (None, None),
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT u.hour_start, {SUMS}, {COST} FROM token_usage u WHERE u.hour_start >= ?1 AND u.hour_start > 0
         GROUP BY u.hour_start ORDER BY u.hour_start"
    ))?;
    let hours = stmt
        .query_map([from], |r| Ok(HourTokens { hour_start: r.get(0)?, tokens: sums(r, 1)?.0, cost: r.get(8)? }))?
        .collect::<rusqlite::Result<_>>()?;
    let share = |sql: &str| -> Result<Vec<Share>> {
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt
            .query_map([from], |r| {
                let (tokens, requests) = sums(r, 2)?;
                Ok(Share {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    tokens,
                    cost: r.get(9)?,
                    requests,
                    conversations: r.get::<_, i64>(10)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.sort_by_key(|s| std::cmp::Reverse(s.tokens.total()));
        Ok(rows)
    };
    // Left joins: the usage outlives a conversation or project that is gone.
    let projects = share(&format!(
        "SELECT p.id, COALESCE(p.name, 'Unassigned'), {SUMS}, {COST}, COUNT(DISTINCT u.conversation_id)
         FROM token_usage u
         LEFT JOIN conversations c ON c.id = u.conversation_id
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE u.hour_start >= ?1 GROUP BY p.id"
    ))?;
    let models = share(&format!(
        "SELECT NULL, u.model, {SUMS}, {COST}, COUNT(DISTINCT u.conversation_id)
         FROM token_usage u WHERE u.hour_start >= ?1 GROUP BY u.model"
    ))?;
    let unpriced = models
        .iter()
        .filter(|m| m.name.contains("claude-") && pricing::price(&m.name).is_none())
        .map(|m| m.name.clone())
        .collect();
    Ok(Stats {
        first_hour,
        tokens,
        cost,
        requests,
        conversations,
        previous,
        previous_cost,
        hours,
        projects,
        models,
        unpriced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn write(path: &Path, records: &[Value]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = File::create(path).unwrap();
        for r in records {
            writeln!(f, "{r}").unwrap();
        }
    }

    fn assistant(id: &str, output: u64, at: &str) -> Value {
        json!({"type":"assistant","timestamp":at,"message":{"id":id,"model":"claude-opus-5-5","usage":{
            "input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,
            "cache_creation":{"ephemeral_1h_input_tokens":60,"ephemeral_5m_input_tokens":40},
            "output_tokens":output,"output_tokens_details":{"thinking_tokens":5}}}})
    }

    const T0: &str = "2026-11-04T15:58:00.000Z";
    const T1: &str = "2026-11-04T15:59:13.264Z";
    const T2: &str = "2026-11-04T16:20:00.000Z";

    fn ms(t: &str) -> i64 {
        parse_time(t).unwrap()
    }

    fn claude_transcript(dir: &Path) -> PathBuf {
        let t = dir.join("s1.jsonl");
        write(
            &t,
            &[
                json!({"type":"user","timestamp":T0,"message":{"role":"user","content":"hi, the usage is fine"}}),
                assistant("m1", 10, T1),
                assistant("m1", 10, T1),
                assistant("m2", 20, T2),
                json!({"type":"assistant","timestamp":T2,"message":{"id":"m3","model":"<synthetic>","usage":{"input_tokens":0,"output_tokens":0}}}),
            ],
        );
        write(&dir.join("s1/subagents/agent-a.jsonl"), &[assistant("s1", 7, T2)]);
        t
    }

    #[test]
    fn parses_transcript_times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2024-02-29T12:00:00Z"), Some(1_709_208_000_000));
        assert_eq!(parse_time("2026-11-04T16:59:13.264+01:00"), Some(ms(T1)));
        assert_eq!(parse_time("2026-11-04T15:59:13.264123Z"), Some(ms(T1)));
        assert_eq!(ms(T1) - ms(T0), 73_264);
        assert_eq!(parse_time("t"), None);
        assert_eq!(parse_time("2026-13-04T15:59:13Z"), None);
    }

    #[test]
    fn claude_counts_each_response_once_and_adds_subagents() {
        let dir = tempfile::tempdir().unwrap();
        let s = read("claude", &claude_transcript(dir.path())).unwrap();
        assert_eq!(s.started_at, Some(ms(T0)));
        let hour = |t: &str| hour_start(ms(t));
        assert_eq!(
            s.buckets.iter().map(|b| (b.hour_start, b.subagent, b.requests)).collect::<Vec<_>>(),
            vec![(hour(T1), false, 1), (hour(T2), false, 1), (hour(T2), true, 1)]
        );
        let u = summarize(&s.buckets);
        assert_eq!(u.requests, 3);
        assert_eq!(
            u.tokens,
            Tokens { input: 6, cache_write: 300, cache_write_1h: 180, cache_read: 3000, output: 37, reasoning: 15 }
        );
        assert_eq!(
            u.subagents,
            Some(Tokens { input: 2, cache_write: 100, cache_write_1h: 60, cache_read: 1000, output: 7, reasoning: 5 })
        );
        assert_eq!(u.models, vec!["claude-opus-5-5"]);
        assert_eq!(u.tokens.total(), 3343);
    }

    #[test]
    fn codex_splits_the_cumulative_totals_by_request() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("rollout.jsonl");
        let count = |input: u64, cached: u64, output: u64, at: &str| {
            json!({"timestamp":at,"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{
                "input_tokens":input,"cached_input_tokens":cached,"cache_write_input_tokens":0,
                "output_tokens":output,"reasoning_output_tokens":output / 2,"total_tokens":input + output}}}})
        };
        write(
            &t,
            &[
                json!({"timestamp":T0,"type":"session_meta","payload":{}}),
                json!({"timestamp":T1,"type":"turn_context","payload":{"model":"gpt-6-astra"}}),
                json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
                count(1000, 800, 50, T1),
                count(1000, 800, 50, T1),
                count(3000, 2500, 120, T2),
            ],
        );

        let s = read("codex", &t).unwrap();
        assert_eq!(s.started_at, Some(ms(T0)));
        assert_eq!(s.buckets.len(), 2);
        assert_eq!(
            s.buckets[1].tokens,
            Tokens { input: 300, cache_write: 0, cache_write_1h: 0, cache_read: 1700, output: 70, reasoning: 35 }
        );
        let u = summarize(&s.buckets);
        assert_eq!(u.requests, 2);
        assert_eq!(
            u.tokens,
            Tokens { input: 500, cache_write: 0, cache_write_1h: 0, cache_read: 2500, output: 120, reasoning: 60 }
        );
        assert_eq!(u.subagents, None);
        assert_eq!(u.models, vec!["gpt-6-astra"]);
    }

    #[test]
    fn missing_transcript_is_an_error() {
        assert!(read("claude", Path::new("/does/not/exist.jsonl")).is_err());
    }

    fn conversation(conn: &Connection, id: i64, project: Option<i64>, path: &Path, last_activity_at: i64) {
        conn.execute(
            "INSERT INTO conversations(id, tool, namespace, native_id, project_id, transcript_path, state, state_at,
               last_activity_at, first_seen_at)
             VALUES (?1, 'claude', 'n', ?1, ?2, ?3, 'idle', 0, ?4, ?4)",
            params![id, project, path.to_string_lossy(), last_activity_at],
        )
        .unwrap();
    }

    #[test]
    fn saved_usage_survives_the_transcript() {
        let conn = crate::store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = claude_transcript(dir.path());
        let now = ms(T2);
        conversation(&conn, 1, None, &path, now);
        assert_eq!(due(&conn, 10).unwrap().len(), 1);

        save(&conn, 1, Some(&read("claude", &path).unwrap()), now).unwrap();
        let before = saved(&conn, 1).unwrap().unwrap();
        assert_eq!(before.tokens.total(), 3343);
        // Still settling, then done until new activity.
        assert_eq!(due(&conn, 10).unwrap().len(), 1);
        save(&conn, 1, None, now + SETTLE_MS).unwrap();
        assert!(due(&conn, 10).unwrap().is_empty());

        // The agent deletes the transcript, then a shorter one appears: nothing goes down.
        std::fs::remove_dir_all(dir.path()).unwrap();
        assert!(read("claude", &path).is_err());
        save(&conn, 1, None, now + 2 * SETTLE_MS).unwrap();
        write(&path, &[assistant("m1", 10, T1)]);
        save(&conn, 1, Some(&read("claude", &path).unwrap()), now + 3 * SETTLE_MS).unwrap();
        assert_eq!(saved(&conn, 1).unwrap().unwrap(), before);
        let started: i64 = conn.query_row("SELECT started_at FROM conversations WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(started, ms(T0));
    }

    #[test]
    fn stats_by_period_project_and_model() {
        let conn = crate::store::open_in_memory().unwrap();
        conn.execute("INSERT INTO projects(id, name, created_at) VALUES (7, 'Checkout', 0)", []).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = claude_transcript(dir.path());
        conversation(&conn, 1, Some(7), &path, ms(T2));
        conversation(&conn, 2, None, &path, ms(T2));
        let s = read("claude", &path).unwrap();
        save(&conn, 1, Some(&s), ms(T2)).unwrap();
        save(&conn, 2, Some(&s), ms(T2)).unwrap();

        let all = stats(&conn, None, None, None).unwrap();
        assert_eq!(all.tokens.total(), 2 * 3343);
        assert_eq!(all.conversations, 2);
        assert_eq!(all.requests, 6);
        assert_eq!(all.first_hour, Some(hour_start(ms(T1))));
        assert_eq!(all.hours.len(), 2);
        assert_eq!(all.previous, None);
        assert_eq!(all.projects.len(), 2);
        assert!(all.projects.iter().any(|p| p.id == Some(7) && p.name == "Checkout" && p.conversations == 1));
        assert!(all.projects.iter().any(|p| p.id.is_none() && p.name == "Unassigned"));
        assert_eq!(all.models.len(), 1);
        assert_eq!(all.models[0].name, "claude-opus-5-5");

        // Only the second hour, compared with the first.
        let recent = stats(&conn, Some(hour_start(ms(T2))), Some(hour_start(ms(T1))), None).unwrap();
        assert_eq!(recent.hours.len(), 1);
        assert_eq!(recent.tokens.output, 2 * 27);
        assert_eq!(recent.previous.map(|p| p.output), Some(2 * 10));
        // A previous period that ends before its first hour is empty.
        let cut = stats(&conn, Some(hour_start(ms(T2))), Some(hour_start(ms(T1))), Some(hour_start(ms(T1)))).unwrap();
        assert_eq!(cut.previous.map(|p| p.output), Some(0));
    }

    #[test]
    fn stats_estimate_the_cost_of_the_claude_models_only() {
        let conn = crate::store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = claude_transcript(dir.path());
        conversation(&conn, 1, None, &path, ms(T2));
        save(&conn, 1, Some(&read("claude", &path).unwrap()), ms(T2)).unwrap();
        for model in ["gpt-6-astra", "claude-opus-9-9"] {
            conn.execute(
                "INSERT INTO token_usage(conversation_id, model, hour_start, subagent, input, output, requests)
                 VALUES (2, ?1, ?2, 0, 1000000, 1000000, 1)",
                params![model, hour_start(ms(T2))],
            )
            .unwrap();
        }

        let all = stats(&conn, None, None, None).unwrap();
        // Opus 5.5: 6 input, 120 + 180 cache writes (5 minutes, 1 hour), 3000 cache reads, 37 output.
        let opus = (6.0 * 4.0 + 120.0 * 5.0 + 180.0 * 8.0 + 3000.0 * 0.20 + 37.0 * 20.0) / 1e6;
        assert!((all.cost - opus).abs() < 1e-12);
        assert!((all.hours.iter().map(|h| h.cost).sum::<f64>() - opus).abs() < 1e-12);
        assert!((all.projects.iter().map(|p| p.cost).sum::<f64>() - opus).abs() < 1e-12);
        assert_eq!(all.models.iter().find(|m| m.name == "gpt-6-astra").map(|m| m.cost), Some(0.0));
        assert_eq!(all.unpriced, vec!["claude-opus-9-9"]);
    }
}
