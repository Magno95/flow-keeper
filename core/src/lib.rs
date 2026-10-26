//! Tracker core: event ingestion, state, visibility, recap.
//! It does not depend on the desktop shell, so the rules can be tested in isolation.

pub mod access;
pub mod codex;
pub mod connect;
pub mod doctor;
pub mod editors;
pub mod engine;
pub mod event;
pub mod legacy;
pub mod logging;
pub mod paths;
pub mod projects;
pub mod recap;
pub mod settings;
pub mod spool;
pub mod store;
pub mod system;
pub mod transcript;
pub mod updates;
pub mod views;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
