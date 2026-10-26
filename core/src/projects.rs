//! Repository detection for grouping into projects.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// Local identity of the repository: the canonical git common dir, shared
    /// by worktrees of the same clone and different across separate clones.
    pub identity: String,
    /// Root of the worktree the conversation runs in.
    pub worktree_root: String,
    pub name: String,
}

pub trait RepoInspector: Send + Sync {
    fn inspect(&self, cwd: &str) -> Option<RepoInfo>;
}

/// Real implementation based on `git`, with a per-folder cache.
pub struct GitInspector {
    git: String,
    cache: Mutex<HashMap<String, Option<RepoInfo>>>,
}

impl GitInspector {
    pub fn new(git: impl Into<String>) -> Self {
        GitInspector { git: git.into(), cache: Mutex::new(HashMap::new()) }
    }
}

impl RepoInspector for GitInspector {
    fn inspect(&self, cwd: &str) -> Option<RepoInfo> {
        if let Some(hit) = self.cache.lock().ok()?.get(cwd) {
            return hit.clone();
        }
        let info = git_inspect(&self.git, cwd);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(cwd.to_string(), info.clone());
        }
        info
    }
}

fn git_inspect(git: &str, cwd: &str) -> Option<RepoInfo> {
    if !Path::new(cwd).is_dir() {
        return None;
    }
    let out = Command::new(git)
        .args(["-C", cwd, "rev-parse", "--path-format=absolute", "--git-common-dir", "--show-toplevel"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let common = lines.next()?.trim().to_string();
    let toplevel = lines.next()?.trim().to_string();
    let identity = std::fs::canonicalize(&common)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(common);
    Some(RepoInfo { name: repo_name(&identity, &toplevel), identity, worktree_root: toplevel })
}

/// Repository name: the folder containing `.git`, otherwise the worktree.
pub fn repo_name(identity: &str, toplevel: &str) -> String {
    let id = Path::new(identity);
    if id.file_name().is_some_and(|n| n == ".git") {
        if let Some(parent) = id.parent().and_then(|p| p.file_name()) {
            return parent.to_string_lossy().to_string();
        }
    }
    if let Some(stem) = id.file_name().map(|n| n.to_string_lossy().to_string()) {
        if let Some(bare) = stem.strip_suffix(".git") {
            return bare.to_string();
        }
    }
    crate::paths::short_label(toplevel)
}

/// Test inspector with canned answers.
#[derive(Default)]
pub struct StaticInspector(pub HashMap<String, RepoInfo>);

impl RepoInspector for StaticInspector {
    fn inspect(&self, cwd: &str) -> Option<RepoInfo> {
        self.0.get(cwd).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_common_dir() {
        assert_eq!(repo_name("/src/shop/.git", "/src/shop-wt"), "shop");
        assert_eq!(repo_name("/srv/mirror.git", "/x"), "mirror");
    }

    #[test]
    fn worktrees_share_identity() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("app");
        let run = |args: &[&str], cwd: &Path| {
            assert!(Command::new("git").args(args).current_dir(cwd).output().unwrap().status.success(), "{args:?}");
        };
        std::fs::create_dir(&main).unwrap();
        run(&["init", "-q", "-b", "main"], &main);
        run(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "--allow-empty", "-m", "x"], &main);
        let wt = dir.path().join("app-feature");
        run(&["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "feature"], &main);

        let g = GitInspector::new("git");
        let a = g.inspect(main.to_str().unwrap()).unwrap();
        let b = g.inspect(wt.to_str().unwrap()).unwrap();
        assert_eq!(a.identity, b.identity);
        assert_ne!(a.worktree_root, b.worktree_root);
        assert_eq!(a.name, "app");
        assert_eq!(b.name, "app");

        let clone = dir.path().join("app-clone");
        run(&["clone", "-q", main.to_str().unwrap(), clone.to_str().unwrap()], dir.path());
        assert_ne!(g.inspect(clone.to_str().unwrap()).unwrap().identity, a.identity);
        assert!(g.inspect(dir.path().to_str().unwrap()).is_none());
    }
}
