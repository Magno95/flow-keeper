# Flow Keeper

![FloK, your desktop flow companion](companion-ivory-animations/00-base.png)

Keep your flow, even when your work is spread across several terminals. **FloK** is your little desktop companion: it keeps your Claude Code and Codex CLI sessions in view, so you can see who's working, who's finished and who needs you without checking every terminal.

Flow Keeper is a macOS app that groups those conversations by project and gives each one a topic and a short summary. Find the session you need, jump back to its terminal, and pick up where you left off.

First version: terminal sessions of Claude Code and Codex CLI only, macOS only. Desktop apps and editor extensions are not tracked.

## Installation

Requires **macOS 13 or later** (Apple Silicon or Intel) and an authenticated [GitHub CLI](https://cli.github.com). With [Homebrew](https://brew.sh) installed:

```sh
brew install gh
gh auth login
```

Then install FloK:

```sh
gh api -H "Accept: application/vnd.github.raw" \
  repos/Magno95/flow-keeper/contents/scripts/install.sh | bash
```

The script downloads the latest release, verifies its SHA-256 checksum, installs the app in `~/Applications`, and launches it. A release installation does not require Rust or Node.js. Install Claude Code or Codex CLI separately to track their sessions.

If no release is available, the script falls back to a source build. This requires Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs), and Node.js 22.12 or later. You can also request that path explicitly with `./scripts/install.sh --from-source` from a clone.

On first launch, the “Connect your agents” setup connects Claude Code and Codex with one click each. The app registers the hooks plugin, which ships inside the bundle, with Claude Code from a local folder (`claude plugin marketplace add <data folder>/claude-plugin`). No GitHub access is needed, you don't type any `/plugin` commands, and your `settings.json` is left untouched. Every time the app updates, the plugin is realigned automatically. You can disconnect it from Settings → “Disconnect Claude Code”.

Codex works the same way (`codex plugin marketplace add <data folder>/codex-plugin`, then `codex plugin add`), plus one step that's up to you: Codex only runs a plugin's hooks after you approve them. Open Codex, type `/hooks` and approve the Flow Keeper ones. You only need to do this once: app updates don't require a new approval. The doctor reminds you until they're approved.

**macOS signing.** The app is signed ad hoc, rather than notarized with an Apple Developer ID. The installer removes the downloaded bundle's quarantine attribute and signs the installed copy locally. Use the installation command above for this installation path.

Useful options (from a clone: `./scripts/install.sh …`):

| Option | Effect |
| --- | --- |
| `--from-source` | Builds on your Mac (requires Rust and Node.js) |
| `--version v0.1.0` | Installs a specific release |
| `--no-open` | Doesn't launch the app when done |

To use the same steps from a local checkout:

```sh
git clone https://github.com/Magno95/flow-keeper.git
cd flow-keeper
./scripts/install.sh
```

Alternatively, without the app (for example to try just the hooks), you can install the plugin by hand from the repository's marketplace: `/plugin marketplace add Magno95/flow-keeper` and then `/plugin install flow-keeper@flow-keeper`.

### From AI Tracker to Flow Keeper

Up to 0.4 the app was called AI Tracker. On first launch the new app moves itself from `AI Tracker.app` to `Flow Keeper.app`, brings along your data, API key and launch at login, and replaces the plugin and editor extension with the new ones. What's left to do by hand, once:

- macOS asks again for notification and terminal control permissions, because the app has a new identifier.
- In Codex the hooks must be approved again with `/hooks`: trust is tied to the plugin name.
- Sessions already open during the update must be reopened to be tracked by the new plugin.

As long as 0.4 installations exist, the `release` workflow also archives the bundle under the old name (`AI Tracker.app`): it's the only one 0.4 knows how to install.

### Updating and uninstalling

The app checks for a new release on its own: shortly after launch and then every 30 minutes, using your GitHub CLI (`gh api …/releases/latest`), so you need `gh auth login` as for installation. When it finds one, the “new me available!” bubble appears above FloK. It stays there until you click it; the click dismisses it for that version and opens Settings → Updates. From there, “Update now” downloads the bundle with `gh`, verifies its SHA-256 fingerprint and version, then the app quits. The `install.sh --bundle` shipped inside the app replaces the bundle and reopens it. If the download fails, the app stays open as it is. The installation log is in `logs/update.log` inside the data folder. “Check for updates” forces a check right away.

Alternatively, just rerun the installation command: it replaces the app and updates the plugin.

```sh
./scripts/uninstall.sh          # removes the app, launch at login, local plugins and marketplaces; keeps your data
./scripts/uninstall.sh --purge  # also deletes conversations, settings and API key
```

Uninstalling removes only what the app added: the `flow-keeper@flow-keeper-app` plugin and the `flow-keeper-app` local marketplace, in Claude Code and in Codex (and the AI Tracker ones, if still present). If you installed the plugin by hand from GitHub, it tells you but leaves it alone.

## How it works

```
Claude Code and Codex hooks (plugin/hooks/capture.sh)
   → local spool: one file per event, only the first 8 KB of the payload
   → local service (Tauri app, Rust): deduplicates, orders and applies events in SQLite
   → incremental transcript reading → topic and summary (claude -p or API)
   → FloK, panel, notifications
```

- **Fast, harmless hooks.** The script writes a file and always exits with 0, in about 30 ms. It uses no network, `jq` or Node. If the app is closed, events wait in the spool and are picked up on restart.
- **States come only from events.** Working, Waiting for you, Finished, Closed and Unverifiable are never inferred by the LLM. A late event never moves the state backwards. Closing is also verified by checking the process.
- **Remove from view.** Hides the entry without touching the agent. The entry only reappears with new real activity after the removal.
- **Seen.** The ✕ in Waiting for you removes the request from the box and from the count on FloK, which sits in the bottom right corner. The conversation stays in its project with its real state and is highlighted again at the next state change.
- **Projects.** They're based on the repository's local identity, so worktrees of the same clone end up together and separate clones stay apart. Folders without a repository go into Unassigned until you assign them.
- **Summaries.** They start 60 s after a turn ends. The model sees the previous summary and only the new text (prompts and replies, never tool output), after a secrets filter. There's a configurable budget, and excluded projects send nothing.

Repository layout:

| Path | Contents |
| --- | --- |
| `plugin/` | Claude Code plugin (`.claude-plugin/`, `hooks/hooks.json`) and Codex plugin (`.codex-plugin/`, `hooks/codex.json`), sharing a single `capture.sh`, embedded in the app (`core/src/connect.rs`) |
| `.claude-plugin/marketplace.json` | Marketplace for installing the plugin by hand from GitHub (Claude Code) |
| `core/` | Rust library: spool, normalization, state engine, SQLite store, views, recap, session access, doctor |
| `app/` | Tauri 2 + Svelte 5 app: FloK (`companion` window), panel, tray, background service |
| `app/scripts/build-sprites.mjs` | Pipeline that cleans up FloK's sheets into frames and icons |
| `scripts/` | Installation and uninstallation |

## Development

```sh
cargo test -p tracker-core           # specification rules and scenarios
cargo test -p tracker-core connect -- --include-ignored   # connection with the real CLIs, in a temporary CLAUDE_CONFIG_DIR / CODEX_HOME
cd app && npm install
npm run dev                          # UI in the browser with mock data (localhost:1420)
FLOW_KEEPER_HOME=/tmp/flow-keeper-dev npx tauri dev   # real app with isolated data
npx tauri build --bundles app        # .app bundle
```

`FLOW_KEEPER_HOME` moves the store and spool, and the hook honors it too. Together with `claude --plugin-dir ./plugin` it lets you try everything without installing anything.

To regenerate FloK's frames from the sheets:

```sh
cd app && node scripts/build-sprites.mjs ../companion-ivory-animations /tmp/previews
npx tauri icon src-tauri/icons/source.png -o src-tauri/icons
```

To publish a release: bump the version in `Cargo.toml`, `Cargo.lock`, `app/package.json`, `app/package-lock.json`, `app/src-tauri/tauri.conf.json`, `plugin/.claude-plugin/plugin.json`, `plugin/.codex-plugin/plugin.json` and `.claude-plugin/marketplace.json`, then push a `vX.Y.Z` tag for that public commit to `Magno95/flow-keeper`. The `release` workflow builds an ad hoc signed universal bundle, only in the `Magno95/flow-keeper` repository. Alternatively, `--from-source` builds locally.

## Privacy

Database, events and settings stay in `~/Library/Application Support/Flow Keeper`, with permissions restricted to your user. With summaries on, parts of your conversations are sent to Claude (with `claude -p` through your Claude Code account, or to the API with your key, stored in the keychain). You can exclude a project from its menu: for that project, state, origin and manual titles remain. The diagnostic log contains no conversation content. Check your company policies before turning on summaries for work projects.

## License

Flow Keeper's original code and FloK artwork are available under the [Zero-Clause BSD license (0BSD)](LICENSE). Use, modify and redistribute them for any purpose, including commercial projects, without an attribution requirement. Third-party dependencies retain their own licenses.
