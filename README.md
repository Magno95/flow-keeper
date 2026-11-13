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

To let a coding agent do it, clone the repository (`gh repo clone Magno95/flow-keeper`) and ask the agent to follow [`INSTALL.md`](./INSTALL.md).

The script downloads the latest release, verifies its SHA-256 checksum, installs the app in `~/Applications`, and launches it. A release installation does not require Rust or Node.js. Install Claude Code or Codex CLI separately to track their sessions.

If no release is available, the script falls back to a source build. This requires Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs), and Node.js 22.12 or later. You can also request that path explicitly with `./scripts/install.sh --from-source` from a clone.

On first launch, “Connect your agents” guides you through agents, editors, summaries, notifications, a test, and a final review. Connect only the agents you use. Start new agent sessions after connecting so they load the plugin. The app registers the hooks plugin, which ships inside the bundle, with Claude Code from a local folder (`claude plugin marketplace add <data folder>/claude-plugin`). No GitHub access is needed, you don't type any `/plugin` commands, and your `settings.json` is left untouched. Every time the app updates, the plugin is realigned automatically. You can disconnect it from Settings → “Disconnect Claude Code”.

Codex works the same way (`codex plugin marketplace add <data folder>/codex-plugin`, then `codex plugin add`), plus one step that's up to you: Codex only runs a plugin's hooks after you approve them. Open Codex, type `/hooks` and approve the Flow Keeper ones. You only need to do this once: app updates don't require a new approval. The doctor reminds you until they're approved.

For VS Code, VS Code Insiders, Cursor or Windsurf, the optional Editors step installs the bundled Flow Keeper extension. In each editor window, open the Command Palette (⇧⌘P) and run “Developer: Reload Window”. The extension helps “Open conversation” select the exact integrated terminal; collection also works without it. Installation alone does not verify activation: after setup, open a tracked conversation and check that it selects your terminal.

The test step checks new conversations separately for Claude Code and Codex. The optional automatic Claude test uses a small Haiku request on your account. The final review lists connected, verified and pending items; finishing does not imply every integration was tested. To complete skipped steps, return to Settings → Connect your agents. If you only use Codex, choose an API key or Later for summaries, which otherwise require Claude Code.

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

As long as 0.4 installations exist, the release also archives the bundle under the old name (`AI Tracker.app`): it's the only one 0.4 knows how to install.

### Updating and uninstalling

The app checks for a new release on its own: shortly after launch and then every 30 minutes, using your GitHub CLI (`gh api …/releases`), so you need `gh auth login` as for installation. When it finds one, the “new me available!” bubble appears above FloK. It stays there until you click it; the click dismisses it for that version and opens Settings → Updates, which lists the main changes of each release after yours. From there, “Update now” downloads the bundle with `gh`, verifies its SHA-256 fingerprint and version, then the app quits. The `install.sh --bundle` shipped inside the app replaces the bundle and reopens it. If the download fails, the app stays open as it is. The installation log is in `logs/update.log` inside the data folder. “Check for updates” forces a check right away.

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

### Tokens

The chart button next to Settings opens Analytics. Its Tokens tab shows the tokens of all your conversations today, in the last 7, 30 or 90 days or since Flow Keeper started saving them, compared with the previous period, per day (per hour today, per month over longer spans), by project and by model. Today is compared with yesterday up to the same hour. All tokens, output, input, cache write and cache read each have their own figure, and each model lists the four types. The estimated cost, shown first, applies the Claude API list prices to the Claude models' tokens, pricing each cache write by how long it's kept (5 minutes or 1 hour). It is an estimate: Codex and other agents aren't counted, nor fast mode, US-only inference, web searches or the discounts of a plan.

Claude Code and Codex delete old transcripts (Claude Code after 30 days by default), and with them the only record of what a conversation used. So a minute after each reply Flow Keeper copies the conversation's usage from its transcript into its database, per model and hour, and keeps it: a conversation's details and Analytics show the saved figures even when the transcript is gone. A transcript that disappears or gets shorter never lowers what was saved. Only the counts are kept, no content. On the first launch of this version, the conversations Flow Keeper already tracks are read while their transcripts still exist; conversations from before you installed Flow Keeper aren't counted.

### Activity

Flow Keeper counts how many times the panel opens and how many times it takes you to a conversation (Open conversation or Resume), per hour. Nothing else: no titles, projects, folders or content. The counters stay on your Mac for about a year and are shown in Analytics → App use (the chart button next to Settings): totals compared with the previous period, use per day and by weekday and hour.

They can also be sent to a server you choose (Settings → Activity: address, optional token, “Send activity to this server”). Every 15 minutes, when something changed, the app sends the pending hours. If the server can't be reached, for example because it is only reachable from an internal network, the hours wait on your Mac and are sent at the next attempt (after 5 minutes, then up to every hour). “Send now” sends right away, or only checks the server when nothing is pending.

The request is a `POST` with `Content-Type: application/json` and, if a token is set, `Authorization: Bearer <token>`:

```json
{
  "schema": 1,
  "installId": "3f9c0d2e7a1b4c8d9e0f1a2b3c4d5e6f",
  "app": "flow-keeper",
  "appVersion": "0.9.4",
  "platform": "macos",
  "sentAt": 1762077600000,
  "hours": [
    { "hourStart": 1762074000000, "utcOffsetMinutes": 60, "panelOpens": 4, "conversationOpens": 7 }
  ]
}
```

- `installId` is random, generated by the app on first use: it groups the rows of one installation and says nothing about who you are.
- `hourStart` is the start of the hour in UTC, in milliseconds; `utcOffsetMinutes` is the Mac's offset at that time, to work out the local hour.
- The counts are the hour's **totals**, not increments: the server must save them by replacing what it has for `(installId, hourStart)`. The same hour may arrive more than once (it keeps growing during the hour, or a response got lost), and replacing never counts twice.
- Any `2xx` answer means the hours are saved. Any other answer, including redirects (which are not followed), leaves them pending. At most 500 hours per request; `hours` can be empty when “Send now” only checks the server.
- HTTPS uses the macOS certificates, so a server signed by a private certificate authority trusted on the Mac works.

A build can set a default server with the `FLOK_ACTIVITY_ENDPOINT` and `FLOK_ACTIVITY_TOKEN` environment variables at build time (for example `FLOK_ACTIVITY_ENDPOINT=https://… ./scripts/release.sh`): as long as you haven't changed the activity settings yourself, the app uses that server with sending on, and picks up a new address with the next update. Without them, nothing is sent until you set a server. Once you change something in Settings → Activity, turning it off included, your choice stays.

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

To publish a release: bump the version in `Cargo.toml`, `Cargo.lock`, `app/package.json`, `app/package-lock.json`, `app/src-tauri/tauri.conf.json`, `plugin/.claude-plugin/plugin.json`, `plugin/.codex-plugin/plugin.json` and `.claude-plugin/marketplace.json`, and move the `Unreleased` entries in `CHANGELOG.md` under a `## X.Y.Z — date` heading: that section becomes the release notes. Then push a `vX.Y.Z` tag for that public commit to `Magno95/flow-keeper`. The `release` workflow builds an ad hoc signed universal bundle, only in the `Magno95/flow-keeper` repository. Alternatively, `./scripts/release.sh` builds the bundle on your Mac, pushes `main` and the tag, and publishes the release with the archive and its checksum (`--build` only builds it into `dist/`).

## Privacy

Database, events and settings stay in `~/Library/Application Support/Flow Keeper`, with permissions restricted to your user. With summaries on, parts of your conversations are sent to Claude (with `claude -p` through your Claude Code account, or to the API with your key, stored in the keychain). You can exclude a project from its menu: for that project, state, origin and manual titles remain. The diagnostic log contains no conversation content. The activity counters (see [Activity](#activity)) only leave your Mac if you set a server for them, or if the build you installed sets one: Settings → Activity shows which, and you can turn it off. Check your company policies before turning on summaries for work projects.

## License

Flow Keeper's original code and FloK artwork are available under the [Zero-Clause BSD license (0BSD)](LICENSE). Use, modify and redistribute them for any purpose, including commercial projects, without an attribution requirement. Third-party dependencies retain their own licenses.
