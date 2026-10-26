//! Migration from AI Tracker, the app's name up to 0.4.
//!
//! On Flow Keeper's first launch the data folder, the keychain key and launch
//! at login are moved over; `connect` and `editors` replace the plugin and the
//! editor extension. Every step is idempotent and, if it fails, leaves things
//! as they were.

use std::fs;
use std::path::Path;

use anyhow::{bail, Result};

use crate::paths::{home, APP_DIR_NAME};

pub const APP_DIR_NAME_OLD: &str = "AI Tracker";
pub const APP_BUNDLE: &str = "AI Tracker.app";
pub const KEYCHAIN_SERVICE: &str = "AI Tracker";
pub const PLUGIN_NAME: &str = "ai-tracker";
pub const LOCAL_MARKETPLACE: &str = "ai-tracker-app";
/// Marketplace of the old repository, for those who installed the plugin by hand.
pub const GITHUB_MARKETPLACE: &str = "ai-task-manager";
pub const EXTENSION_ID: &str = "ai-tracker.ai-tracker-terminal";
/// Variable the old hooks read to ignore internal calls.
pub const INTERNAL_ENV: &str = "AI_TRACKER_INTERNAL";

pub fn local_plugin_id() -> String {
    format!("{PLUGIN_NAME}@{LOCAL_MARKETPLACE}")
}

/// AI Tracker plugin, connected by the app or installed by hand from GitHub.
pub fn is_plugin_id(id: &str) -> bool {
    id.strip_prefix(PLUGIN_NAME).is_some_and(|rest| rest.starts_with('@'))
}

/// Runs the steps that do not need the agents' CLIs. Call it before opening
/// the data folder, and so before logging starts: it returns the messages to
/// log.
pub fn migrate() -> Vec<String> {
    let mut notes = Vec::new();
    if std::env::var_os("FLOW_KEEPER_HOME").is_none() {
        let support = home().join("Library/Application Support");
        match migrate_data_dir(&support.join(APP_DIR_NAME_OLD), &support.join(APP_DIR_NAME)) {
            Ok(true) => notes.push(format!("moved the AI Tracker data folder to {APP_DIR_NAME}")),
            Ok(false) => {}
            Err(err) => notes.push(format!("moving the AI Tracker data folder: {err}")),
        }
    }
    #[cfg(target_os = "macos")]
    notes.extend(migrate_keychain());
    // AI Tracker's launch at login points to a bundle that no longer exists;
    // the app creates the new one according to the setting.
    if fs::remove_file(home().join("Library/LaunchAgents").join(format!("{APP_DIR_NAME_OLD}.plist"))).is_ok() {
        notes.push("removed AI Tracker's launch at login".into());
    }
    notes
}

/// Moves the data folder and leaves a symlink at the old path: hooks already
/// loaded in open sessions keep writing to the spool.
/// `Ok(true)` if it moved something.
pub fn migrate_data_dir(old: &Path, new: &Path) -> Result<bool> {
    let Ok(meta) = fs::symlink_metadata(old) else { return Ok(false) };
    if meta.file_type().is_symlink() {
        return Ok(false);
    }
    // The install script may have already created the new folder just for
    // the manifest: the one with the data is still the old one.
    if new.is_dir() && !new.join("tracker.db").exists() {
        let _ = fs::remove_file(new.join("install-manifest.json"));
        let _ = fs::remove_dir(new);
    }
    if new.exists() {
        bail!("both {} and {} exist: leaving the old folder as it is", old.display(), new.display());
    }
    fs::rename(old, new)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(new, old)?;
    Ok(true)
}

#[cfg(target_os = "macos")]
fn migrate_keychain() -> Option<String> {
    use crate::system::{KEYCHAIN_ACCOUNT, KEYCHAIN_SERVICE as NEW_SERVICE};
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};
    let key = get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).ok()?;
    if get_generic_password(NEW_SERVICE, KEYCHAIN_ACCOUNT).is_err() && set_generic_password(NEW_SERVICE, KEYCHAIN_ACCOUNT, &key).is_err() {
        return Some("AI Tracker API key not copied to the keychain".into());
    }
    let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
    Some(format!("moved the AI Tracker API key to the {APP_DIR_NAME} keychain"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_the_folder_and_leaves_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("AI Tracker"), root.path().join("Flow Keeper"));
        fs::create_dir_all(old.join("spool")).unwrap();
        fs::write(old.join("tracker.db"), "db").unwrap();

        assert!(migrate_data_dir(&old, &new).unwrap());
        assert_eq!(fs::read_to_string(new.join("tracker.db")).unwrap(), "db");
        assert!(fs::symlink_metadata(&old).unwrap().file_type().is_symlink());
        // Old hooks write to the new spool through the symlink.
        fs::write(old.join("spool/1.evt"), "x").unwrap();
        assert!(new.join("spool/1.evt").is_file());

        assert!(!migrate_data_dir(&old, &new).unwrap(), "second time: nothing to do");
    }

    #[test]
    fn ignores_a_new_folder_with_only_the_manifest() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("AI Tracker"), root.path().join("Flow Keeper"));
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("tracker.db"), "db").unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("install-manifest.json"), "{}").unwrap();
        assert!(migrate_data_dir(&old, &new).unwrap());
        assert!(new.join("tracker.db").is_file());
    }

    #[test]
    fn touches_nothing_if_both_exist() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("AI Tracker"), root.path().join("Flow Keeper"));
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("tracker.db"), "db").unwrap();
        assert!(migrate_data_dir(&old, &new).is_err());
        assert!(old.is_dir() && !fs::symlink_metadata(&old).unwrap().file_type().is_symlink());
        assert!(!migrate_data_dir(&root.path().join("missing"), &new).unwrap());
    }
}
