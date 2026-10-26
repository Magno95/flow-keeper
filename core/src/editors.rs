//! Editors of the VS Code family (VS Code, Insiders, Cursor, Windsurf).
//!
//! To bring back a conversation open in the integrated terminal we need to
//! know which window it is in. With the embedded extension
//! (`editor-extension/`) each window answers on a local socket and selects
//! the exact terminal; without the extension the window is picked from the
//! conversation's folder, reading the open windows from `storage.json`. In
//! both cases the window is brought to the front by reopening its folder:
//! the editor selects the window that already has it open.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::access::{AccessResult, Target};
use crate::paths::{home, Paths};
use crate::system::run_with_timeout;

const EXT_PACKAGE_JSON: &str = include_str!("../../editor-extension/package.json");
const EXT_MAIN_JS: &str = include_str!("../../editor-extension/extension.js");

/// Folder of the sockets opened by the extension, one entry per window.
const SOCKETS_DIR: &str = "editors";

pub struct Editor {
    pub bundle: &'static str,
    pub name: &'static str,
    /// Data folder under `~/Library/Application Support`.
    data_dir: &'static str,
    /// Extensions folder under the home directory.
    extensions_dir: &'static str,
    cli: &'static str,
    app_name: &'static str,
}

pub const EDITORS: &[Editor] = &[
    Editor { bundle: "com.microsoft.VSCode", name: "VS Code", data_dir: "Code", extensions_dir: ".vscode", cli: "code", app_name: "Visual Studio Code" },
    Editor {
        bundle: "com.microsoft.VSCodeInsiders",
        name: "VS Code Insiders",
        data_dir: "Code - Insiders",
        extensions_dir: ".vscode-insiders",
        cli: "code-insiders",
        app_name: "Visual Studio Code - Insiders",
    },
    Editor { bundle: "com.todesktop.230313mzl4w4u92", name: "Cursor", data_dir: "Cursor", extensions_dir: ".cursor", cli: "cursor", app_name: "Cursor" },
    Editor { bundle: "com.exafunction.windsurf", name: "Windsurf", data_dir: "Windsurf", extensions_dir: ".windsurf", cli: "windsurf", app_name: "Windsurf" },
];

pub fn by_bundle(bundle: &str) -> Option<&'static Editor> {
    EDITORS.iter().find(|e| e.bundle == bundle)
}

fn manifest() -> Value {
    serde_json::from_str(EXT_PACKAGE_JSON).expect("extension package.json")
}

pub fn extension_id() -> String {
    let m = manifest();
    format!("{}.{}", m["publisher"].as_str().unwrap_or_default(), m["name"].as_str().unwrap_or_default())
}

fn extension_version() -> String {
    manifest()["version"].as_str().unwrap_or("0.0.0").to_string()
}

// ---------------------------------------------------------------------------
// Extension install

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorStatus {
    pub bundle: String,
    pub name: String,
    pub extension_installed: bool,
    /// The installed version matches the one embedded in the app.
    pub up_to_date: bool,
}

impl Editor {
    fn app_path(&self) -> Option<PathBuf> {
        let query = format!("kMDItemCFBundleIdentifier == '{}'", self.bundle);
        let found = run_with_timeout(Command::new("/usr/bin/mdfind").arg(query), Duration::from_secs(5))
            .and_then(|out| String::from_utf8(out).ok())
            .and_then(|s| s.lines().map(PathBuf::from).find(|p| p.join("Contents/Info.plist").is_file()));
        found.or_else(|| {
            let app = format!("{}.app", self.app_name);
            [PathBuf::from("/Applications").join(&app), home().join("Applications").join(&app)].into_iter().find(|p| p.is_dir())
        })
    }

    fn cli_path(&self) -> Option<PathBuf> {
        Some(self.app_path()?.join("Contents/Resources/app/bin").join(self.cli)).filter(|p| p.is_file())
    }

    /// Installed version of extension `id`, from the editor's extension registry.
    fn installed_version(&self, id: &str) -> Option<String> {
        let registry = home().join(self.extensions_dir).join("extensions/extensions.json");
        let list: Value = serde_json::from_str(&fs::read_to_string(registry).ok()?).ok()?;
        list.as_array()?
            .iter()
            .find(|e| e["identifier"]["id"].as_str().is_some_and(|i| i.eq_ignore_ascii_case(id)))
            .and_then(|e| e["version"].as_str().map(str::to_string))
    }

    pub fn status(&self) -> EditorStatus {
        let version = self.installed_version(&extension_id());
        EditorStatus {
            bundle: self.bundle.into(),
            name: self.name.into(),
            extension_installed: version.is_some(),
            up_to_date: version.as_deref() == Some(extension_version().as_str()),
        }
    }

    fn run_cli(&self, args: &[&str]) -> Result<()> {
        let cli = self.cli_path().ok_or_else(|| anyhow!("{} not found", self.name))?;
        let out = Command::new(&cli).args(args).stdin(std::process::Stdio::null()).output()?;
        if out.status.success() {
            return Ok(());
        }
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        bail!("{} {}: {}", self.cli, args.join(" "), crate::engine::truncate_chars(text.trim(), 400))
    }

    /// Installs (or updates) the embedded extension. `work` is an app folder
    /// where the .vsix package is prepared.
    pub fn install(&self, work: &Path) -> Result<()> {
        let vsix = write_vsix(work)?;
        self.run_cli(&["--install-extension", &vsix.to_string_lossy(), "--force"])
    }

    pub fn uninstall(&self) -> Result<()> {
        self.run_cli(&["--uninstall-extension", &extension_id()])
    }
}

/// Editors present on the Mac, with the extension status.
pub fn detect() -> Vec<EditorStatus> {
    EDITORS.iter().filter(|e| e.app_path().is_some()).map(Editor::status).collect()
}

/// After an app update the already installed extension is brought back in sync;
/// the AI Tracker one is replaced.
pub fn sync_installed(work: &Path) {
    for e in EDITORS {
        let s = e.status();
        let legacy = e.installed_version(crate::legacy::EXTENSION_ID).is_some();
        if legacy {
            if let Err(err) = e.run_cli(&["--uninstall-extension", crate::legacy::EXTENSION_ID]) {
                log::warn!("removing the AI Tracker extension from {}: {err}", e.name);
            }
        }
        if (s.extension_installed && !s.up_to_date) || (legacy && !s.extension_installed) {
            if let Err(err) = e.install(work) {
                log::warn!("updating the extension for {}: {err}", e.name);
            }
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Prepares the .vsix package (a zip with the manifest and the `extension/` folder).
fn write_vsix(work: &Path) -> Result<PathBuf> {
    let m = manifest();
    let field = |k: &str| xml_escape(m[k].as_str().unwrap_or_default());
    let stage = work.join("stage");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(stage.join("extension"))?;
    fs::write(stage.join("extension/package.json"), EXT_PACKAGE_JSON)?;
    fs::write(stage.join("extension/extension.js"), EXT_MAIN_JS)?;
    fs::write(
        stage.join("[Content_Types].xml"),
        r#"<?xml version="1.0" encoding="utf-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension=".json" ContentType="application/json"/><Default Extension=".js" ContentType="application/javascript"/><Default Extension=".vsixmanifest" ContentType="text/xml"/></Types>
"#,
    )?;
    fs::write(
        stage.join("extension.vsixmanifest"),
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">
  <Metadata>
    <Identity Language="en-US" Id="{name}" Version="{version}" Publisher="{publisher}" />
    <DisplayName>{display}</DisplayName>
    <Description xml:space="preserve">{description}</Description>
    <Categories>Other</Categories>
    <Properties>
      <Property Id="Microsoft.VisualStudio.Code.Engine" Value="{engine}" />
      <Property Id="Microsoft.VisualStudio.Code.ExtensionKind" Value="ui" />
    </Properties>
  </Metadata>
  <Installation><InstallationTarget Id="Microsoft.VisualStudio.Code" /></Installation>
  <Dependencies />
  <Assets><Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" /></Assets>
</PackageManifest>
"#,
            name = field("name"),
            version = field("version"),
            publisher = field("publisher"),
            display = field("displayName"),
            description = field("description"),
            engine = xml_escape(m["engines"]["vscode"].as_str().unwrap_or("*")),
        ),
    )?;
    let vsix = work.join(format!("{}.vsix", extension_id()));
    let _ = fs::remove_file(&vsix);
    let status = Command::new("/usr/bin/zip").arg("-q").arg("-r").arg("-X").arg(&vsix).arg(".").current_dir(&stage).status()?;
    if !status.success() {
        bail!("could not prepare the extension package");
    }
    Ok(vsix)
}

// ---------------------------------------------------------------------------
// Bringing to the front

/// The process and its ancestors (shell, extension host...).
fn ancestors(pid: i64) -> Vec<i64> {
    let parents: HashMap<i64, i64> = run_with_timeout(Command::new("/bin/ps").args(["-A", "-o", "pid=,ppid="]), Duration::from_secs(3))
        .map(|out| {
            String::from_utf8_lossy(&out)
                .lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace().map(|n| n.parse::<i64>().ok());
                    Some((it.next()??, it.next()??))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut chain = vec![pid];
    while let Some(&parent) = parents.get(chain.last().unwrap()) {
        if parent <= 1 || chain.len() >= 64 || chain.contains(&parent) {
            break;
        }
        chain.push(parent);
    }
    chain
}

#[derive(Debug, PartialEq)]
enum Found {
    Terminal(Option<PathBuf>),
    Window(Option<PathBuf>),
}

fn ask(sock: &Path, request: &str) -> std::io::Result<Option<Found>> {
    let mut stream = UnixStream::connect(sock)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(request.as_bytes())?;
    let mut line = String::new();
    BufReader::new(stream.take(64 * 1024)).read_line(&mut line)?;
    let reply: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    let window = reply["window"].as_str().map(PathBuf::from);
    Ok(match reply["found"].as_str() {
        Some("terminal") => Some(Found::Terminal(window)),
        Some("window") => Some(Found::Window(window)),
        _ => None,
    })
}

/// Asks every window with the extension which one hosts the conversation.
/// Sockets left over from closed windows are removed.
fn ask_windows(dir: &Path, pids: &[i64]) -> Option<Found> {
    let request = format!("{}\n", json!({ "op": "focus", "pids": pids }));
    let socks: Vec<PathBuf> = fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "sock")).collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = socks
            .iter()
            .map(|sock| {
                let request = &request;
                s.spawn(move || match ask(sock, request) {
                    Ok(found) => found,
                    Err(e) => {
                        if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound) {
                            let _ = fs::remove_file(sock);
                        }
                        None
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok().flatten()).next()
    })
}

fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let raw = uri.strip_prefix("file://")?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

/// Open windows according to `storage.json`: for each one, the folders it contains
/// and the path to reopen to bring it to the front (folder or .code-workspace).
fn open_windows(storage: &Path) -> Vec<(Vec<PathBuf>, PathBuf)> {
    let Some(json) = fs::read_to_string(storage).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
        return Vec::new();
    };
    let state = &json["windowsState"];
    let windows = state["openedWindows"].as_array().into_iter().flatten().chain(std::iter::once(&state["lastActiveWindow"]));
    let mut out = Vec::new();
    for w in windows {
        if let Some(folder) = w["folder"].as_str().and_then(file_uri_path) {
            out.push((vec![folder.clone()], folder));
        } else if let Some(config) = w["workspaceIdentifier"]["configPath"].as_str().and_then(file_uri_path) {
            // Workspace files may contain comments: if they cannot be parsed, skip them.
            let Some(ws) = fs::read_to_string(&config).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
                continue;
            };
            let base = config.parent().unwrap_or(Path::new("/"));
            let roots = ws["folders"].as_array().into_iter().flatten().filter_map(|f| f["path"].as_str()).map(|p| base.join(p)).collect();
            out.push((roots, config));
        }
    }
    out
}

/// The window containing `cwd`: the one with the closest folder.
fn window_for(cwd: &Path, windows: &[(Vec<PathBuf>, PathBuf)]) -> Option<PathBuf> {
    windows
        .iter()
        .flat_map(|(roots, open)| roots.iter().filter(|r| cwd.starts_with(r)).map(move |r| (r.components().count(), open)))
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, open)| open.clone())
}

fn raise(bundle: &str, window: &Path) -> bool {
    Command::new("/usr/bin/open").args(["-b", bundle]).arg(window).status().map(|s| s.success()).unwrap_or(false)
}

fn window_name(window: &Path) -> String {
    let name = if window.extension().is_some_and(|e| e == "code-workspace") { window.file_stem() } else { window.file_name() };
    name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Brings to the front the conversation open in a VS Code family editor.
/// `None` if the editor is not in this family or the window cannot be found.
pub fn focus(t: &Target, app: &str) -> Option<AccessResult> {
    let editor = by_bundle(t.bundle_id.as_deref()?)?;
    if let Some(pid) = t.pid {
        let dir = Paths::default_location().base.join(SOCKETS_DIR);
        match ask_windows(&dir, &ancestors(pid)) {
            Some(Found::Terminal(Some(window)) | Found::Window(Some(window))) if raise(editor.bundle, &window) => {
                return Some(AccessResult::Focused { app: app.into() });
            }
            // Window without a folder: the terminal is selected, the window has to be picked by hand.
            Some(Found::Terminal(None)) if raise_app(editor.bundle) => {
                return Some(AccessResult::AppActivated {
                    app: app.into(),
                    note: format!("{app} is in front: the terminal is already selected in the conversation's window."),
                });
            }
            _ => {}
        }
    }
    let cwd = Path::new(t.cwd.as_deref()?);
    let storage = home().join("Library/Application Support").join(editor.data_dir).join("User/globalStorage/storage.json");
    let window = window_for(cwd, &open_windows(&storage))?;
    if !raise(editor.bundle, &window) {
        return None;
    }
    let hint = if editor.status().extension_installed {
        String::new()
    } else {
        " Install the extension from Settings to open the terminal directly.".to_string()
    };
    Some(AccessResult::AppActivated {
        app: app.into(),
        note: format!("{app} is in front on the {} window: select the conversation's terminal.{hint}", window_name(&window)),
    })
}

fn raise_app(bundle: &str) -> bool {
    Command::new("/usr/bin/open").args(["-b", bundle]).status().map(|s| s.success()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_is_the_closest_open_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("team.code-workspace");
        fs::write(&ws, r#"{"folders":[{"path":"api"},{"path":"/elsewhere/web"}]}"#).unwrap();
        let storage = root.join("storage.json");
        let state = json!({"windowsState": {
            "lastActiveWindow": {"folder": "file:///Users/me/dev"},
            "openedWindows": [
                {"folder": "file:///Users/me/dev/my%20app"},
                {"backupPath": "/tmp/x"},
                {"workspaceIdentifier": {"id": "1", "configPath": format!("file://{}", ws.display())}},
            ],
        }});
        fs::write(&storage, state.to_string()).unwrap();
        let windows = open_windows(&storage);
        let pick = |cwd: &str| window_for(Path::new(cwd), &windows);
        assert_eq!(pick("/Users/me/dev/my app/src"), Some(PathBuf::from("/Users/me/dev/my app")));
        assert_eq!(pick("/Users/me/dev/other"), Some(PathBuf::from("/Users/me/dev")));
        assert_eq!(pick("/elsewhere/web"), Some(ws.clone()));
        assert_eq!(pick(root.join("api/lib").to_str().unwrap()), Some(ws));
        assert_eq!(pick("/Users/me/dev/my app2"), Some(PathBuf::from("/Users/me/dev")));
        assert_eq!(pick("/opt"), None);
    }

    #[test]
    fn ancestors_start_from_the_process() {
        let chain = ancestors(std::process::id() as i64);
        assert_eq!(chain[0], std::process::id() as i64);
        assert!(chain.len() >= 2);
    }

    #[test]
    fn windows_answer_over_their_socket_and_stale_ones_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("1.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        let live = dir.path().join("2.sock");
        let listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let server = std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["pids"], json!([30, 20]));
            (&conn).write_all(b"{\"found\":\"terminal\",\"window\":\"/Users/me/app\"}\n").unwrap();
        });
        assert_eq!(ask_windows(dir.path(), &[30, 20]), Some(Found::Terminal(Some(PathBuf::from("/Users/me/app")))));
        server.join().unwrap();
        assert!(!stale.exists());
    }

    #[test]
    fn vsix_contains_manifest_and_extension() {
        let dir = tempfile::tempdir().unwrap();
        let vsix = write_vsix(dir.path()).unwrap();
        let out = Command::new("/usr/bin/unzip").arg("-Z1").arg(&vsix).output().unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        for f in ["extension.vsixmanifest", "[Content_Types].xml", "extension/package.json", "extension/extension.js"] {
            assert!(list.lines().any(|l| l == f), "missing {f}: {list}");
        }
        assert_eq!(extension_id(), "flow-keeper.flow-keeper-terminal");
    }
}
