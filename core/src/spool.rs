//! Reading the spool written by the hooks (`plugin/hooks/capture.sh`).
//!
//! Format of an `.evt` file:
//! ```text
//! flow-keeper-spool 1
//! EVENT=Stop
//! PID=123
//! TERM_PROGRAM=iTerm.app
//!
//! {"session_id": "...", ...}      <- first 8 KB of the hook payload
//! ```
//! The file is deleted only after the event has been saved to the store.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const SPOOL_MAGIC: &str = "flow-keeper-spool 1";
/// Header written by the AI Tracker hooks (up to 0.4), still active in
/// sessions opened before the update.
const LEGACY_SPOOL_MAGIC: &str = "ai-tracker-spool 1";

#[derive(Debug, Clone)]
pub struct SpoolRecord {
    pub file: PathBuf,
    /// Time at the source: the file's mtime (written by the hook), in nanoseconds.
    pub source_ns: i128,
    pub header: HashMap<String, String>,
    pub payload: Payload,
    /// Content fingerprint, used as the deduplication key.
    pub digest: String,
}

impl SpoolRecord {
    pub fn source_ms(&self) -> i64 {
        (self.source_ns / 1_000_000) as i64
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.header.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }
}

/// The hook's JSON payload. It may be truncated: in that case text fields
/// are extracted from the available prefix.
#[derive(Debug, Clone)]
pub struct Payload {
    json: Option<Value>,
    raw: String,
}

impl Payload {
    pub fn parse(raw: &str) -> Self {
        let json = serde_json::from_str::<Value>(raw.trim()).ok();
        Payload { json, raw: raw.to_string() }
    }

    pub fn is_complete(&self) -> bool {
        self.json.is_some()
    }

    pub fn str(&self, key: &str) -> Option<String> {
        if let Some(json) = &self.json {
            return json.get(key).and_then(Value::as_str).map(str::to_string);
        }
        let pattern = format!(r#""{}"\s*:\s*"((?:[^"\\]|\\.)*)""#, regex::escape(key));
        let re = Regex::new(&pattern).ok()?;
        let captured = re.captures(&self.raw)?.get(1)?.as_str();
        serde_json::from_str::<String>(&format!("\"{captured}\"")).ok()
    }

    /// Prefix of a text field even when the payload is cut off midway
    /// through the value (e.g. a very long prompt).
    pub fn str_prefix(&self, key: &str, max_chars: usize) -> Option<String> {
        if let Some(value) = self.str(key) {
            return Some(value.chars().take(max_chars).collect());
        }
        if self.json.is_some() {
            return None;
        }
        let marker = format!("\"{key}\"");
        let start = self.raw.find(&marker)? + marker.len();
        let rest = self.raw[start..].trim_start().strip_prefix(':')?.trim_start();
        let body = rest.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') | Some('t') | Some('r') => out.push(' '),
                    Some(other) => out.push(other),
                    None => break,
                },
                _ => out.push(c),
            }
            if out.chars().count() >= max_chars {
                break;
            }
        }
        Some(out)
    }
}

pub fn parse_bytes(file: &Path, bytes: &[u8], source_ns: i128) -> Result<SpoolRecord> {
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text
        .split_once("\n\n")
        .with_context(|| format!("spool without separator: {}", file.display()))?;
    let mut lines = head.lines();
    if !matches!(lines.next(), Some(SPOOL_MAGIC | LEGACY_SPOOL_MAGIC)) {
        bail!("unrecognized spool format: {}", file.display());
    }
    let header = lines
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let digest = hex(&Sha256::digest(bytes));
    Ok(SpoolRecord {
        file: file.to_path_buf(),
        source_ns,
        header,
        payload: Payload::parse(body),
        digest,
    })
}

pub fn read_record(file: &Path) -> Result<SpoolRecord> {
    let bytes = fs::read(file)?;
    let source_ns = mtime_ns(file)?;
    parse_bytes(file, &bytes, source_ns)
}

/// Pending files, in the order the hooks wrote them.
pub fn pending(dir: &Path) -> Vec<(i128, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(i128, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "evt"))
        .filter_map(|p| mtime_ns(&p).ok().map(|t| (t, p)))
        .collect();
    files.sort();
    files
}

/// Moves an unreadable file to `rejected/` so it does not block the queue.
pub fn reject(dir: &Path, file: &Path) {
    let target = dir.join("rejected");
    if fs::create_dir_all(&target).is_ok() {
        if let Some(name) = file.file_name() {
            let _ = fs::rename(file, target.join(name));
            return;
        }
    }
    let _ = fs::remove_file(file);
}

fn mtime_ns(path: &Path) -> Result<i128> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(modified.duration_since(UNIX_EPOCH)?.as_nanos() as i128)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_and_payload() {
        let raw = b"flow-keeper-spool 1\nEVENT=Stop\nPID=42\nTERM_PROGRAM=iTerm.app\n\n{\"session_id\":\"s1\",\"cwd\":\"/tmp\"}\n";
        let rec = parse_bytes(Path::new("x.evt"), raw, 1_000_000_000).unwrap();
        assert_eq!(rec.header("EVENT"), Some("Stop"));
        assert_eq!(rec.header("PID"), Some("42"));
        assert_eq!(rec.payload.str("session_id").as_deref(), Some("s1"));
        assert!(rec.payload.is_complete());
        assert_eq!(rec.source_ms(), 1000);
    }

    #[test]
    fn truncated_payload_still_yields_fields() {
        let raw = r#"{"session_id":"abc","transcript_path":"/a \"b\".jsonl","cwd":"/w","prompt":"hello how are you this is a really long prompt that gets trunc"#;
        let p = Payload::parse(raw);
        assert!(!p.is_complete());
        assert_eq!(p.str("session_id").as_deref(), Some("abc"));
        assert_eq!(p.str("transcript_path").as_deref(), Some("/a \"b\".jsonl"));
        assert_eq!(p.str("prompt"), None);
        assert_eq!(p.str_prefix("prompt", 9).as_deref(), Some("hello how"));
    }

    #[test]
    fn rejects_unknown_format() {
        assert!(parse_bytes(Path::new("x"), b"hello\n\n{}", 0).is_err());
    }
}
