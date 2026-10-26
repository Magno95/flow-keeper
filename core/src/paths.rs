//! The tracker's local paths. Everything stays in the user's folder.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

pub const APP_DIR_NAME: &str = "Flow Keeper";

#[derive(Debug, Clone)]
pub struct Paths {
    pub base: PathBuf,
    pub spool: PathBuf,
    pub db: PathBuf,
    pub logs: PathBuf,
}

impl Paths {
    pub fn at(base: impl Into<PathBuf>) -> Self {
        let base = base.into();
        Paths {
            spool: base.join("spool"),
            db: base.join("tracker.db"),
            logs: base.join("logs"),
            base,
        }
    }

    /// `~/Library/Application Support/Flow Keeper`, overridable with
    /// `FLOW_KEEPER_HOME` (the same value the hook uses for the spool).
    pub fn default_location() -> Self {
        if let Some(custom) = std::env::var_os("FLOW_KEEPER_HOME") {
            return Paths::at(PathBuf::from(custom));
        }
        Paths::at(home().join("Library/Application Support").join(APP_DIR_NAME))
    }

    pub fn ensure(&self) -> Result<()> {
        ensure_private_dir(&self.base)?;
        ensure_private_dir(&self.spool)?;
        ensure_private_dir(&self.logs)?;
        Ok(())
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

/// Short label for a folder (last component), never the full path.
pub fn short_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}
