//! Incremental reading of Claude Code transcripts and Codex rollouts
//! (internal, version-dependent formats: see `docs/compatibility.md`).
//!
//! Extracted content is treated as untrusted data: it is only summarized,
//! never executed or interpreted as instructions.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;
use regex::Regex;
use serde_json::Value;

/// Maximum bytes read from the file in a single extraction.
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
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
    let key = file_key(path).ok_or_else(|| anyhow::anyhow!("transcript not available"))?;
    let len = std::fs::metadata(path)?.len();
    let reset = prev_key.is_some_and(|k| k != key) || offset > len;
    let start = if reset { 0 } else { offset };

    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(MAX_READ_BYTES).read_to_end(&mut buf)?;

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
    if !more && (start + consumed as u64) < len && buf.len() as u64 == MAX_READ_BYTES {
        more = true;
    }
    Ok(Extract { text: out.finish(), end_offset: start + consumed as u64, file_key: key, reset, more })
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
    let re = Regex::new(r"(?s)<system-reminder>.*?</system-reminder>").ok()?;
    let t = re.replace_all(t, "");
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

/// Filter for the most common secrets. It is an extra safeguard: it does not
/// catch every piece of sensitive data.
pub fn redact(text: &str) -> String {
    static PATTERNS: &[&str] = &[
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        r"\bsk-ant-[A-Za-z0-9_\-]{10,}",
        r"\bsk-[A-Za-z0-9_\-]{20,}",
        r"\bgh[pousr]_[A-Za-z0-9]{20,}",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}",
        r"\bAKIA[0-9A-Z]{16}\b",
        r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
        r#"(?i)\b(password|passwd|pwd|secret|token|api[_-]?key)\s*[:=]\s*["']?[^\s"']{6,}"#,
    ];
    let mut out = text.to_string();
    for p in PATTERNS {
        if let Ok(re) = Regex::new(p) {
            out = re.replace_all(&out, "[secret removed]").to_string();
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
}
