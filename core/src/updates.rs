//! App updates from the GitHub releases.
//!
//! Downloads use GitHub CLI with the user's access, as for installation:
//! no tokens are stored by the app.
//! `gh` does not add the quarantine attribute, so the downloaded bundle opens
//! without Gatekeeper. The bundle is replaced by `scripts/install.sh`
//! (embedded here), after the app has quit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::engine::truncate_chars;
use crate::legacy;

pub const DEFAULT_REPO: &str = "Magno95/flow-keeper";
pub const APP_BUNDLE: &str = "Flow Keeper.app";
pub const INSTALL_SH: &str = include_str!("../../scripts/install.sh");
const ARCHIVE_SUFFIX: &str = ".app.tar.gz";

/// Releases repository; `FLOW_KEEPER_REPO` as in the install script.
pub fn repo() -> String {
    std::env::var("FLOW_KEEPER_REPO").ok().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| DEFAULT_REPO.into())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub tag: String,
    /// Version without the `v` prefix, e.g. "0.2.0".
    pub version: String,
    pub url: String,
    pub published_at: Option<String>,
    /// Name of the bundle archive among the assets.
    pub archive: String,
}

/// Installable release from the `releases/latest` response: no drafts,
/// pre-releases or releases still missing the bundle archive.
pub fn parse_release(v: &Value) -> Option<Release> {
    if v.get("draft").and_then(Value::as_bool).unwrap_or(false)
        || v.get("prerelease").and_then(Value::as_bool).unwrap_or(false)
    {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?.to_string();
    let version = tag.trim_start_matches('v').to_string();
    parse_version(&version)?;
    let archive = v
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(|a| a.as_str().or_else(|| a.get("name")?.as_str()))
        .find(|n| n.ends_with(ARCHIVE_SUFFIX))?
        .to_string();
    Some(Release {
        version,
        url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        published_at: v.get("published_at").and_then(Value::as_str).map(String::from),
        archive,
        tag,
    })
}

/// "1.2.3" (with or without `v`) as numbers; the pre-release suffix is ignored.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

struct Gh<'a> {
    bin: &'a Path,
    path_env: &'a str,
}

impl Gh<'_> {
    fn run(&self, args: &[&str], timeout: Duration) -> Result<Vec<u8>> {
        let child = Command::new(self.bin)
            .args(args)
            .env("PATH", self.path_env)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("could not start GitHub CLI")?;
        let out = wait(child, timeout)?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        Err(anyhow!(gh_error(&String::from_utf8_lossy(&out.stderr))))
    }
}

fn wait(mut child: std::process::Child, timeout: Duration) -> Result<Output> {
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("GitHub is not responding (timed out after {}s)", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Readable message for the most common `gh` errors.
fn gh_error(stderr: &str) -> String {
    let s = stderr.trim();
    let lower = s.to_lowercase();
    if lower.contains("gh auth login") || lower.contains("not logged") || lower.contains("authentication") {
        "GitHub CLI has no access: run “gh auth login” in the terminal".into()
    } else if lower.contains("could not resolve") || lower.contains("connect") || lower.contains("timeout") {
        "GitHub is unreachable: check your connection".into()
    } else if lower.contains("404") || lower.contains("not found") {
        "No release found (or no access to the repository)".into()
    } else if s.is_empty() {
        "GitHub CLI did not respond".into()
    } else {
        truncate_chars(s, 300)
    }
}

/// Latest published release; `None` if there is no installable one.
pub fn latest(gh: &Path, path_env: &str, repo: &str) -> Result<Option<Release>> {
    let out = Gh { bin: gh, path_env }.run(
        &[
            "api",
            &format!("repos/{repo}/releases/latest"),
            "--jq",
            "{tag_name, html_url, published_at, draft, prerelease, assets: [.assets[].name]}",
        ],
        Duration::from_secs(30),
    )?;
    let v: Value = serde_json::from_slice(&out).context("invalid response from GitHub")?;
    Ok(parse_release(&v))
}

/// Downloads and verifies the release bundle into `dir` (emptied first).
/// Returns the path of the extracted bundle: `Flow Keeper.app`, or
/// `AI Tracker.app` in the releases that still have to update 0.4.
pub fn download(gh: &Path, path_env: &str, repo: &str, release: &Release, dir: &Path) -> Result<PathBuf> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    let checksum = format!("{}.sha256", release.archive);
    let dir_arg = dir.to_string_lossy();
    Gh { bin: gh, path_env }.run(
        &[
            "release", "download", &release.tag, "--repo", repo, "--pattern", &release.archive, "--pattern", &checksum,
            "--dir", &dir_arg,
        ],
        Duration::from_secs(600),
    )?;
    let archive = dir.join(&release.archive);
    let bytes = fs::read(&archive).context("archive not downloaded")?;
    let expected = fs::read_to_string(dir.join(&checksum)).context("release checksum not downloaded")?;
    let expected = expected.split_whitespace().next().unwrap_or_default().to_lowercase();
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if expected != actual {
        bail!("the downloaded archive does not match the release checksum");
    }
    let status = Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("invalid release archive");
    }
    let bundle = [APP_BUNDLE, legacy::APP_BUNDLE].iter().map(|b| dir.join(b)).find(|b| b.is_dir()).unwrap_or_else(|| dir.join(APP_BUNDLE));
    let found = bundle_version(&bundle).ok_or_else(|| anyhow!("bundle not found in the archive"))?;
    if parse_version(&found) != parse_version(&release.version) {
        bail!("the downloaded bundle is version {found}, not {}", release.version);
    }
    Ok(bundle)
}

/// `CFBundleShortVersionString` of a bundle.
pub fn bundle_version(bundle: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(bundle.join("Contents/Info.plist"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// The `.app` bundle containing the executable, if the app runs from a bundle.
pub fn running_bundle(exe: &Path) -> Option<PathBuf> {
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension().is_some_and(|e| e == "app") && bundle.join("Contents/Info.plist").is_file())
        .then(|| bundle.to_path_buf())
}

/// Why the app cannot replace the `bundle` bundle by itself, if it cannot.
pub fn install_blocker(bundle: Option<&Path>) -> Option<String> {
    let Some(bundle) = bundle else {
        return Some("The app is not running from an installed bundle (development)".into());
    };
    // With the old name the app moves itself at launch; if that fails it
    // can still update, and the install fixes it.
    if bundle.file_name().is_none_or(|n| n != APP_BUNDLE && n != legacy::APP_BUNDLE) {
        return Some(format!("The bundle is named {:?}: update with the install script", bundle.file_name().unwrap_or_default()));
    }
    let parent = bundle.parent()?;
    let probe = parent.join(".flow-keeper-write-test");
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            None
        }
        Err(_) => Some(format!("No permission to write to {}", parent.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }

    #[test]
    fn reads_the_installable_release() {
        let v = json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/o/r/releases/tag/v0.2.0",
            "published_at": "2026-10-22T08:37:35Z",
            "draft": false,
            "prerelease": false,
            "assets": ["Flow-Keeper_0.2.0_universal.app.tar.gz.sha256", "Flow-Keeper_0.2.0_universal.app.tar.gz"]
        });
        let r = parse_release(&v).expect("release");
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.tag, "v0.2.0");
        assert_eq!(r.archive, "Flow-Keeper_0.2.0_universal.app.tar.gz");
    }

    #[test]
    fn ignores_drafts_prereleases_and_releases_without_bundle() {
        let base = json!({ "tag_name": "v0.2.0", "assets": [{ "name": "Flow-Keeper_0.2.0_universal.app.tar.gz" }] });
        assert!(parse_release(&base).is_some(), "assets as objects too");
        let mut draft = base.clone();
        draft["draft"] = json!(true);
        assert!(parse_release(&draft).is_none());
        let mut pre = base.clone();
        pre["prerelease"] = json!(true);
        assert!(parse_release(&pre).is_none());
        let mut empty = base.clone();
        empty["assets"] = json!([]);
        assert!(parse_release(&empty).is_none());
    }

    /// With GitHub CLI authenticated: `cargo test -p tracker-core updates -- --include-ignored`.
    #[test]
    #[ignore]
    fn reads_the_latest_release_from_github() {
        let path_env = crate::system::search_path(crate::system::login_shell_path().as_deref());
        let gh = crate::system::which("gh", &path_env).expect("gh in PATH");
        let r = latest(&gh, &path_env, &repo()).expect("response").expect("release");
        assert!(parse_version(&r.version).is_some(), "{r:?}");
    }

    /// Actually downloads the latest release and checks its checksum and version.
    #[test]
    #[ignore]
    fn downloads_and_verifies_the_latest_release() {
        let path_env = crate::system::search_path(crate::system::login_shell_path().as_deref());
        let gh = crate::system::which("gh", &path_env).expect("gh in PATH");
        let r = latest(&gh, &path_env, &repo()).expect("response").expect("release");
        let dir = tempfile::tempdir().unwrap();
        let bundle = download(&gh, &path_env, &repo(), &r, &dir.path().join("x")).expect("download");
        assert_eq!(bundle_version(&bundle).as_deref(), Some(r.version.as_str()));
    }

    #[test]
    fn recognizes_the_running_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join(APP_BUNDLE);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/Info.plist"), "").unwrap();
        let exe = app.join("Contents/MacOS/flow-keeper");
        assert_eq!(running_bundle(&exe), Some(app.clone()));
        assert_eq!(install_blocker(Some(&app)), None);
        assert!(running_bundle(&dir.path().join("target/debug/flow-keeper")).is_none());
        assert!(install_blocker(None).is_some());
    }
}
