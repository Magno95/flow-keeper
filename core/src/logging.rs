//! Diagnostic file log, with rotation. Records technical events only:
//! never conversation content, prompts or secrets.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_BYTES: u64 = 1024 * 1024;

struct FileLogger {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl FileLogger {
    fn open(path: &Path) -> Option<File> {
        let f = OpenOptions::new().create(true).append(true).open(path).ok()?;
        crate::paths::restrict_file(path);
        Some(f)
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let ours = metadata.target().starts_with("tracker_core") || metadata.target().starts_with("flow_keeper");
        metadata.level() <= log::Level::Warn || (ours && metadata.level() <= log::Level::Info)
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let Ok(mut guard) = self.file.lock() else { return };
        if std::fs::metadata(&self.path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            *guard = FileLogger::open(&self.path);
        }
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{} {:<5} {} {}", crate::now_ms(), record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {
        if let Ok(mut g) = self.file.lock() {
            if let Some(f) = g.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

pub fn init(dir: &Path) {
    let path = dir.join("tracker.log");
    let logger = FileLogger { file: Mutex::new(FileLogger::open(&path)), path };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}
