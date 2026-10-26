// Extension installed by Flow Keeper. Each window opens a local socket in the
// app's data folder; on a "focus" request carrying a conversation's process
// chain, it selects the terminal hosting it and replies with the window's
// folder, which the app brings to the front.
// No data leaves the Mac and the socket accepts only process identifiers.

const vscode = require("vscode");
const fs = require("fs");
const net = require("net");
const os = require("os");
const path = require("path");

const BASE = process.env.FLOW_KEEPER_HOME || path.join(os.homedir(), "Library", "Application Support", "Flow Keeper");
const DIR = path.join(BASE, "editors");
const MAX_REQUEST = 4096;

function windowPath() {
  const file = vscode.workspace.workspaceFile;
  if (file && file.scheme === "file") return file.fsPath;
  const folder = (vscode.workspace.workspaceFolders || []).find((f) => f.uri.scheme === "file");
  return folder ? folder.uri.fsPath : null;
}

function withTimeout(promise, ms) {
  return Promise.race([promise, new Promise((resolve) => setTimeout(() => resolve(undefined), ms))]);
}

async function focus(pids) {
  const terminals = vscode.window.terminals;
  const ids = await Promise.all(terminals.map((t) => withTimeout(t.processId, 500)));
  const index = ids.findIndex((id) => id !== undefined && pids.has(id));
  if (index >= 0) {
    terminals[index].show(false);
    return { found: "terminal", window: windowPath() };
  }
  // Conversation started from an extension (e.g. Claude Code for VS Code) in this window.
  if (pids.has(process.pid)) return { found: "window", window: windowPath() };
  return { found: null };
}

async function handle(line) {
  const req = JSON.parse(line);
  if (req.op !== "focus" || !Array.isArray(req.pids)) return { found: null };
  return focus(new Set(req.pids.filter(Number.isInteger)));
}

function activate(context) {
  if (process.platform !== "darwin") return;
  const sock = path.join(DIR, `${process.pid}.sock`);
  try {
    fs.mkdirSync(DIR, { recursive: true, mode: 0o700 });
    fs.rmSync(sock, { force: true });
  } catch {
    return;
  }
  const server = net.createServer((conn) => {
    let buf = "";
    conn.setEncoding("utf8");
    conn.setTimeout(3000, () => conn.destroy());
    conn.on("error", () => {});
    conn.on("data", (chunk) => {
      buf += chunk;
      const end = buf.indexOf("\n");
      if (end < 0) {
        if (buf.length > MAX_REQUEST) conn.destroy();
        return;
      }
      conn.removeAllListeners("data");
      handle(buf.slice(0, end))
        .catch(() => ({ found: null }))
        .then((res) => conn.end(JSON.stringify(res) + "\n"));
    });
  });
  server.on("error", () => {});
  server.listen(sock, () => {
    try {
      fs.chmodSync(sock, 0o600);
    } catch {}
  });
  context.subscriptions.push({
    dispose() {
      server.close();
      try {
        fs.rmSync(sock, { force: true });
      } catch {}
    },
  });
}

function deactivate() {}

module.exports = { activate, deactivate };
