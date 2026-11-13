//! Incremental reading of Claude Code transcripts and Codex rollouts
//! (internal, version-dependent formats: see `docs/compatibility.md`).
//!
//! Extracted content is treated as untrusted data: it is only summarized,
//! never executed or interpreted as instructions.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde_json::Value;

/// Maximum bytes read from the file in a single extraction.
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
/// Longest record that is skipped looking for its end: beyond this the
/// transcript is reported as unreadable instead of being scanned forever.
const MAX_SKIP_BYTES: u64 = 1024 * 1024 * 1024;
const USER_MAX_CHARS: usize = 1500;
const ASSISTANT_MAX_CHARS: usize = 1200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extract {
    pub text: String,
    /// Offset up to which content has been consumed (end of a complete line).
    pub end_offset: u64,
    pub file_key: String,
    /// The file was truncated or replaced: reading restarted from zero.
    pub reset: bool,
    /// More content remains beyond this extraction's limit.
    pub more: bool,
    /// Size of a record skipped for exceeding `MAX_READ_BYTES` (its content is never kept).
    pub skipped_bytes: u64,
}

#[cfg(unix)]
pub fn file_key(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", m.dev(), m.ino()))
}

#[cfg(not(unix))]
pub fn file_key(path: &Path) -> Option<String> {
    std::fs::metadata(path).ok().map(|_| path.display().to_string())
}

/// Reads new content starting at `offset`, up to about `max_chars`
/// characters of useful text. Only consumes complete lines.
pub fn read_new(path: &Path, offset: u64, prev_key: Option<&str>, max_chars: usize) -> Result<Extract> {
    read_new_limited(path, offset, prev_key, max_chars, MAX_READ_BYTES, MAX_SKIP_BYTES)
}

/// `read_new` with the window and the longest skippable record as parameters
/// (the tests use small ones to cover the boundaries).
fn read_new_limited(
    path: &Path,
    offset: u64,
    prev_key: Option<&str>,
    max_chars: usize,
    max_read: u64,
    max_skip: u64,
) -> Result<Extract> {
    let key = file_key(path).ok_or_else(|| anyhow::anyhow!("transcript not available"))?;
    let len = std::fs::metadata(path)?.len();
    let reset = prev_key.is_some_and(|k| k != key) || offset > len;
    let start = if reset { 0 } else { offset };

    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    (&mut file).take(max_read).read_to_end(&mut buf)?;

    let mut out = Formatter::default();
    let mut consumed = 0usize;
    let mut more = false;
    for line in buf.split_inclusive(|b| *b == b'\n') {
        if line.last() != Some(&b'\n') {
            break; // partial line: read once it is complete
        }
        if out.len() >= max_chars {
            more = true;
            break;
        }
        consumed += line.len();
        if let Ok(v) = serde_json::from_slice::<Value>(line) {
            out.push_record(&v);
        }
    }
    let mut skipped_bytes = 0;
    if consumed == 0 && buf.len() as u64 == max_read && !buf.contains(&b'\n') {
        // A single record larger than the read window: a partial line that is still
        // being written looks the same, so only the end of the record tells them apart.
        match find_line_end(&mut file, start + max_read, len, start.saturating_add(max_skip))? {
            LineEnd::Found(end) => {
                skipped_bytes = end - start;
                consumed = skipped_bytes as usize;
                log::warn!("transcript record of {skipped_bytes} bytes skipped (over the {max_read} byte limit)");
            }
            LineEnd::TooLong => anyhow::bail!("transcript record too large"),
            LineEnd::Eof => {} // still being written: nothing is consumed until it is complete
        }
    }
    if !more && (start + consumed as u64) < len && (buf.len() as u64 == max_read || skipped_bytes > 0) {
        more = true;
    }
    Ok(Extract { text: out.finish(), end_offset: start + consumed as u64, file_key: key, reset, more, skipped_bytes })
}

enum LineEnd {
    Found(u64),
    /// The file ended before a newline: the record may still be growing.
    Eof,
    /// No newline up to the scan limit.
    TooLong,
}

/// Offset just after the first newline at or beyond `from`, reading with
/// bounded memory and never past `limit`.
fn find_line_end(file: &mut File, from: u64, len: u64, limit: u64) -> Result<LineEnd> {
    file.seek(SeekFrom::Start(from))?;
    let mut chunk = [0u8; 64 * 1024];
    let mut pos = from;
    let stop = len.min(limit);
    while pos < stop {
        let want = chunk.len().min((stop - pos) as usize);
        let n = file.read(&mut chunk[..want])?;
        if n == 0 {
            break;
        }
        if let Some(i) = chunk[..n].iter().position(|b| *b == b'\n') {
            return Ok(LineEnd::Found(pos + i as u64 + 1));
        }
        pos += n as u64;
    }
    Ok(if pos >= limit && limit < len { LineEnd::TooLong } else { LineEnd::Eof })
}

/// Title the agent gives the session itself: `ai-title` at the end of the
/// Claude Code transcript, or the Codex thread name. Used only as a local
/// provisional title.
pub fn ai_title(path: &Path) -> Option<String> {
    if let Some(name) = crate::codex::thread_name(path) {
        return Some(name);
    }
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(256 * 1024);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    buf.split(|b| *b == b'\n')
        .rev()
        .filter(|l| l.windows(8).any(|w| w == b"ai-title"))
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .find(|v| v.get("type").and_then(Value::as_str) == Some("ai-title"))
        .and_then(|v| v.get("aiTitle").and_then(Value::as_str).map(str::to_string))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

#[derive(Default)]
struct Formatter {
    lines: Vec<String>,
    pending_tools: Vec<String>,
    size: usize,
}

impl Formatter {
    fn len(&self) -> usize {
        self.size
    }

    fn push(&mut self, line: String) {
        if self.lines.last() == Some(&line) {
            return; // cuts down on duplicates
        }
        self.size += line.len() + 1;
        self.lines.push(line);
    }

    fn flush_tools(&mut self) {
        if self.pending_tools.is_empty() {
            return;
        }
        let mut counted: Vec<(String, usize)> = Vec::new();
        for t in self.pending_tools.drain(..) {
            match counted.iter_mut().find(|(n, _)| *n == t) {
                Some((_, c)) => *c += 1,
                None => counted.push((t, 1)),
            }
        }
        let list: Vec<String> = counted
            .into_iter()
            .map(|(n, c)| if c > 1 { format!("{n} ×{c}") } else { n })
            .collect();
        self.push(format!("[Tools used: {}]", list.join(", ")));
    }

    fn push_record(&mut self, v: &Value) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "response_item" {
            if let Some(item) = v.get("payload") {
                self.push_codex_item(item);
            }
            return;
        }
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else { return };
        match kind {
            "user" => {
                if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
                    return;
                }
                let texts: Vec<String> = match content {
                    Value::String(s) => vec![s.clone()],
                    Value::Array(blocks) => blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
                        .collect(),
                    _ => vec![],
                };
                for t in texts {
                    if let Some(clean) = clean_user_text(&t) {
                        self.flush_tools();
                        self.push(format!("User: {}", clip(&clean, USER_MAX_CHARS)));
                    }
                }
            }
            "assistant" => {
                let Value::Array(blocks) = content else { return };
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = b.get("text").and_then(Value::as_str).unwrap_or("").trim();
                            if !text.is_empty() {
                                self.flush_tools();
                                self.push(format!("Assistant: {}", clip(&redact(text), ASSISTANT_MAX_CHARS)));
                            }
                        }
                        Some("tool_use") => {
                            if let Some(name) = b.get("name").and_then(Value::as_str) {
                                self.pending_tools.push(name.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// An item of a Codex rollout. Tool outputs (`*_output`) and `developer`
    /// messages are not read.
    fn push_codex_item(&mut self, item: &Value) {
        let texts = |kind: &str| -> Vec<String> {
            item.get("content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some(kind))
                        .filter_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => match item.get("role").and_then(Value::as_str) {
                Some("user") => {
                    for t in texts("input_text") {
                        if let Some(clean) = clean_codex_user_text(&t) {
                            self.flush_tools();
                            self.push(format!("User: {}", clip(&clean, USER_MAX_CHARS)));
                        }
                    }
                }
                Some("assistant") => {
                    for t in texts("output_text") {
                        let t = t.trim();
                        if !t.is_empty() {
                            self.flush_tools();
                            self.push(format!("Assistant: {}", clip(&redact(t), ASSISTANT_MAX_CHARS)));
                        }
                    }
                }
                _ => {}
            },
            "function_call" | "custom_tool_call" => {
                if let Some(name) = item.get("name").and_then(Value::as_str) {
                    self.pending_tools.push(name.to_string());
                }
            }
            "local_shell_call" => self.pending_tools.push("shell".into()),
            "web_search_call" => self.pending_tools.push("web_search".into()),
            _ => {}
        }
    }

    fn finish(mut self) -> String {
        self.flush_tools();
        self.lines.join("\n")
    }
}

fn clean_user_text(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with("<local-command") || t.starts_with("<command-") || t.starts_with("Caveat:") {
        return None;
    }
    let t = SYSTEM_REMINDER.replace_all(t, "");
    let t = t.trim();
    if t.is_empty() {
        None
    } else {
        Some(redact(t))
    }
}

/// Codex also inserts generated context as user messages (environment,
/// AGENTS.md instructions, images, interrupted turns): they all start with a
/// recognizable tag or heading.
fn clean_codex_user_text(text: &str) -> Option<String> {
    let t = text.trim();
    if t.starts_with('<') || t.starts_with("# AGENTS.md instructions") {
        return None;
    }
    clean_user_text(t)
}

fn clip(s: &str, max: usize) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::engine::truncate_chars(&collapsed, max)
}

/// Patterns of the secrets filter, compiled once. They are constants: an
/// invalid one is a programming error and fails at the first use.
static SECRET_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        r"\bsk-ant-[A-Za-z0-9_\-]{10,}",
        r"\bsk-[A-Za-z0-9_\-]{20,}",
        r"\bgh[pousr]_[A-Za-z0-9]{20,}",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}",
        r"\bAKIA[0-9A-Z]{16}\b",
        r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
        r"(?i)\bbearer\s+[A-Za-z0-9._~+/=\-]{16,}",
        // Credentials in a URL: scheme://user:password@host.
        r"(?i)\b[a-z][a-z0-9+.\-]*://[^\s:/@]+:[^\s@/]{3,}@",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid secrets pattern"))
    .collect()
});

static SYSTEM_REMINDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<system-reminder>.*?</system-reminder>").expect("valid pattern"));

/// `key=value`, `key: value` and JSON (`"key": "value"`, also escaped as
/// `\"key\":\"value\"` inside a string), with prefixed keys such as `access_token`.
/// Only the key and the separator: the value is read by `value_end`.
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)["'\\]*[\w\-]*(?:password|passwd|pwd|secret|token|api[_-]?key|private[_-]?key)["'\\]*\s*[:=]\s*"#,
    )
    .expect("valid pattern")
});

/// Shortest value that is treated as a credential.
const MIN_SECRET_CHARS: usize = 6;

/// End (byte offset) of the value that starts at `start`, if it is long enough
/// to be a credential. Quoted values run to the closing quote, honouring
/// backslash escapes (so spaces, quotes and backslashes inside are consumed);
/// unquoted ones stop at whitespace or a delimiter.
fn value_end(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start..];
    // Opening quote, possibly escaped itself (`\"value\"` inside a JSON string).
    let backslashes = rest.chars().take_while(|c| *c == '\\').count();
    let quote = rest[backslashes..].chars().next().filter(|c| matches!(c, '"' | '\''));
    let Some(quote) = quote else {
        let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ',' | '}' | ']')).unwrap_or(rest.len());
        return (rest[..end].chars().count() >= MIN_SECRET_CHARS).then_some(start + end);
    };
    let body_start = backslashes + 1;
    let mut chars = rest[body_start..].char_indices().peekable();
    let mut length = 0;
    while let Some((i, c)) = chars.next() {
        match c {
            '\n' => return (length >= MIN_SECRET_CHARS).then_some(start + body_start + i),
            '\\' => {
                // A run of backslashes followed by the quote closes the value when
                // it has the same depth as the opening one; otherwise it is an escape.
                let mut run = 1;
                while chars.peek().is_some_and(|(_, n)| *n == '\\') {
                    chars.next();
                    run += 1;
                }
                match chars.peek() {
                    Some(&(j, n)) if n == quote && run == backslashes => {
                        return (length >= MIN_SECRET_CHARS).then_some(start + body_start + j + n.len_utf8());
                    }
                    Some(_) => {
                        // The escaped character belongs to the value.
                        chars.next();
                        length += run + 1;
                    }
                    None => length += run,
                }
            }
            c if c == quote && backslashes == 0 => {
                return (length >= MIN_SECRET_CHARS).then_some(start + body_start + i + c.len_utf8());
            }
            _ => length += 1,
        }
    }
    // Unterminated: the rest of the text.
    (length >= MIN_SECRET_CHARS).then_some(text.len())
}

fn redact_assignments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(m) = ASSIGNMENT.find_at(text, pos) {
        match value_end(text, m.end()) {
            Some(end) => {
                out.push_str(&text[pos..m.start()]);
                out.push_str("[secret removed]");
                pos = end;
            }
            None => {
                // Not a credential (too short): leave it and look further on.
                out.push_str(&text[pos..m.end()]);
                pos = m.end();
            }
        }
    }
    out.push_str(&text[pos..]);
    out
}

/// Filter for the most common secrets. It is an extra safeguard: it does not
/// catch every piece of sensitive data.
pub fn redact(text: &str) -> String {
    let mut out = redact_assignments(text);
    for re in SECRET_PATTERNS.iter() {
        if let std::borrow::Cow::Owned(replaced) = re.replace_all(&out, "[secret removed]") {
            out = replaced;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(v: serde_json::Value) -> String {
        format!("{v}\n")
    }

    #[test]
    fn extracts_dialogue_and_skips_meta_tools_and_partial_lines() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut f = File::create(&p).unwrap();
        f.write_all(line(serde_json::json!({"type":"user","isMeta":true,"message":{"role":"user","content":"meta"}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"user","message":{"role":"user","content":"<command-name>/plugin</command-name>"}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"user","message":{"role":"user","content":"Fix the login, the password=supersecret123"}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read"}]}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read"}]}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"secret output"}]}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Found it: expired token."}]}})).as_bytes()).unwrap();
        f.write_all(line(serde_json::json!({"type":"ai-title","aiTitle":"Broken login"})).as_bytes()).unwrap();
        f.write_all(b"{\"type\":\"user\",\"message\":{\"conte").unwrap();
        drop(f);

        let e = read_new(&p, 0, None, 10_000).unwrap();
        assert_eq!(
            e.text,
            "User: Fix the login, the [secret removed]\n[Tools used: Read ×2]\nAssistant: Found it: expired token."
        );
        let full = std::fs::metadata(&p).unwrap().len();
        assert!(e.end_offset < full, "the partial line is not consumed");
        assert!(!e.reset && !e.more);
        assert_eq!(ai_title(&p).as_deref(), Some("Broken login"));

        let again = read_new(&p, e.end_offset, Some(&e.file_key), 10_000).unwrap();
        assert_eq!(again.text, "");
        assert_eq!(again.end_offset, e.end_offset);
    }

    #[test]
    fn extracts_codex_rollout_dialogue() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rollout.jsonl");
        let item = |payload: serde_json::Value| line(serde_json::json!({"timestamp":"t","type":"response_item","payload":payload}));
        let body = [
            line(serde_json::json!({"type":"session_meta","payload":{"originator":"codex-tui"}})),
            item(serde_json::json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"rules"}]})),
            item(serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /repo\n..."}]})),
            item(serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]})),
            item(serde_json::json!({"type":"message","role":"user","content":[{"type":"input_text","text":"Add the tests, token=abcdef123456"}]})),
            line(serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"Add the tests"}})),
            item(serde_json::json!({"type":"function_call","name":"exec_command","arguments":"{}","call_id":"c1"})),
            item(serde_json::json!({"type":"function_call_output","call_id":"c1","output":"secret output"})),
            item(serde_json::json!({"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch","call_id":"c2"})),
            item(serde_json::json!({"type":"custom_tool_call","name":"apply_patch","input":"*** Begin Patch","call_id":"c3"})),
            item(serde_json::json!({"type":"reasoning","summary":[],"encrypted_content":"x"})),
            item(serde_json::json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"Tests added."}]})),
        ]
        .concat();
        std::fs::write(&p, body).unwrap();
        let e = read_new(&p, 0, None, 10_000).unwrap();
        assert_eq!(
            e.text,
            "User: Add the tests, [secret removed]\n[Tools used: exec_command, apply_patch ×2]\nAssistant: Tests added."
        );
    }

    #[test]
    fn truncated_file_restarts_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        std::fs::write(&p, line(serde_json::json!({"type":"user","message":{"content":"hello"}}))).unwrap();
        let e = read_new(&p, 10_000, None, 1000).unwrap();
        assert!(e.reset);
        assert_eq!(e.text, "User: hello");
    }

    #[test]
    fn respects_char_budget_and_reports_more() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut body = String::new();
        for i in 0..50 {
            body.push_str(&line(serde_json::json!({"type":"user","message":{"content":format!("message number {i}")}})));
        }
        std::fs::write(&p, body).unwrap();
        let e = read_new(&p, 0, None, 200).unwrap();
        assert!(e.more);
        let rest = read_new(&p, e.end_offset, Some(&e.file_key), 100_000).unwrap();
        assert!(!rest.more);
        assert!(rest.text.contains("message number 49"));
        assert!(!rest.text.contains("message number 0\n"));
    }

    #[test]
    fn redacts_credentials_in_common_formats_and_keeps_ordinary_text() {
        // Synthetic values only.
        let secret = "synthetic-password-123";
        for input in [
            format!("password={secret}"),
            format!(r#"{{"password":"{secret}"}}"#),
            format!(r#"{{ "password" : "{secret}", "user": "x" }}"#),
            format!(r#"{{'api_key': '{secret}'}}"#),
            format!("password: {secret}"),
            format!("access_token = {secret}"),
            format!(r#"{{\"client_secret\":\"{secret}\"}}"#),
            format!("Authorization: Bearer {secret}-abcdef"),
            format!("clone https://user:{secret}@example.test/repo.git"),
            format!("my password is fine but token={secret}"),
        ] {
            let out = redact(&input);
            assert!(!out.contains("synthetic-password"), "{input:?} -> {out:?}");
            assert!(out.contains("[secret removed]"), "{input:?} -> {out:?}");
        }
        // The rest of the message survives, with the structure around the value.
        assert_eq!(redact(r#"{"password":"synthetic-password-123","user":"anna"}"#), r#"{[secret removed],"user":"anna"}"#);
        assert_eq!(redact("Fix the login, the password=supersecret123 now"), "Fix the login, the [secret removed] now");
        // Ordinary text and unicode are untouched.
        for plain in [
            "The token budget is shown in the settings",
            "Found it: expired token.",
            "token: short",
            "Reset the password flow, then check the secrets page",
            "Città perché “password” è una parola",
            "https://example.test/path?token-less=true",
        ] {
            assert_eq!(redact(plain), plain);
        }
        assert_eq!(redact("password=pässwörd-123"), "[secret removed]");
        // Quoted values are consumed to the closing quote, whatever they contain.
        for (input, expected) in [
            (r#"{"password":"ab\\synthetic-password-123","user":"anna"}"#, r#"{[secret removed],"user":"anna"}"#),
            (r#"{"password":"say \"hello\" synthetic pass","user":"anna"}"#, r#"{[secret removed],"user":"anna"}"#),
            (r#"{"token": "with spaces inside it", "x": 1}"#, r#"{[secret removed], "x": 1}"#),
            (r#"{\"password\":\"synthetic \\\"deep\\\" value\",\"u\":1}"#, r#"{[secret removed],\"u\":1}"#),
            ("password: 'single quoted value'\nnext line", "[secret removed]\nnext line"),
        ] {
            assert_eq!(redact(input), expected, "{input}");
        }
        assert_eq!(redact(r#"{"password":"short"}"#), r#"{"password":"short"}"#, "too short to be a credential");
    }

    fn write_lines(path: &Path, parts: &[Vec<u8>]) {
        let mut f = File::create(path).unwrap();
        for p in parts {
            f.write_all(p).unwrap();
        }
    }

    fn user_line(text: &str) -> Vec<u8> {
        line(serde_json::json!({"type":"user","message":{"role":"user","content":text}})).into_bytes()
    }

    #[test]
    fn oversized_record_is_skipped_and_the_next_one_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        // An ignorable record over the limit (a tool result), then a useful message.
        let mut big = br#"{"type":"tool_result","blob":""#.to_vec();
        big.extend(std::iter::repeat_n(b'x', MAX_READ_BYTES as usize + 1000));
        big.extend(b"\"}\n");
        write_lines(&p, &[user_line("before"), big.clone(), user_line("after the big one")]);

        let first = read_new(&p, 0, None, 10_000).unwrap();
        assert_eq!(first.text, "User: before");
        let second = read_new(&p, first.end_offset, Some(&first.file_key), 10_000).unwrap();
        assert!(second.skipped_bytes > MAX_READ_BYTES, "{second:?}");
        assert!(second.text.is_empty() && second.more);
        assert!(!second.text.contains('x'));
        let third = read_new(&p, second.end_offset, Some(&second.file_key), 10_000).unwrap();
        assert_eq!((third.text.as_str(), third.more), ("User: after the big one", false));
        assert_eq!(third.end_offset, std::fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn a_large_record_still_being_written_is_not_consumed_or_cut() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut partial = br#"{"type":"tool_result","blob":""#.to_vec();
        partial.extend(std::iter::repeat_n(b'x', MAX_READ_BYTES as usize + 10));
        write_lines(&p, &[user_line("hello"), partial.clone()]);
        let first = read_new(&p, 0, None, 10_000).unwrap();
        let again = read_new(&p, first.end_offset, Some(&first.file_key), 10_000).unwrap();
        assert_eq!((again.end_offset, again.skipped_bytes, again.more), (first.end_offset, 0, true));
        // The record is completed later; the tail is never read as a record of its own.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(b"\"}\n").unwrap();
        f.write_all(&user_line("done")).unwrap();
        drop(f);
        let skipped = read_new(&p, first.end_offset, Some(&first.file_key), 10_000).unwrap();
        assert!(skipped.skipped_bytes > 0);
        let last = read_new(&p, skipped.end_offset, Some(&skipped.file_key), 10_000).unwrap();
        assert_eq!(last.text, "User: done");
    }

    #[test]
    fn the_skip_limit_is_applied_while_scanning() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let (window, skip) = (64u64, 1000u64);
        let record = |n: usize| format!("{{\"blob\":\"{}\"}}\n", "x".repeat(n)).into_bytes();
        // Newline just inside the limit: skipped. Just beyond it: an error, even though a newline exists.
        let inside = record(skip as usize - 20);
        write_lines(&p, &[inside.clone(), user_line("next")]);
        let e = read_new_limited(&p, 0, None, 10_000, window, skip).unwrap();
        assert_eq!(e.skipped_bytes, inside.len() as u64);
        let beyond = record(skip as usize + 50);
        write_lines(&p, &[beyond.clone(), user_line("next")]);
        let err = read_new_limited(&p, 0, None, 10_000, window, skip).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
        // No newline within the limit and the file ends right there: still being written.
        write_lines(&p, &[vec![b'x'; skip as usize - 5]]);
        let e = read_new_limited(&p, 0, None, 10_000, window, skip).unwrap();
        assert_eq!((e.end_offset, e.skipped_bytes), (0, 0));
    }
}
