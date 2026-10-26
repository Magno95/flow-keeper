# Changelog

Notable changes to Flow Keeper, newest first. Each release lists what
changes for people using the app; release tooling goes under Development.

## Unreleased

### Added

- Finished turns you haven't looked at yet get a colored border, on the
  project card and on the conversation card. The border stays until you open
  the project (that marks all of its conversations as read), open or resume
  the conversation, or the conversation starts working again from the
  terminal. When the panel opens, projects with unread results come right
  after those waiting for you, ahead of the working ones.

### Fixed

- With monitors at different scale factors, the panel opens on FloK's
  screen instead of the main one: window positions and sizes are now in
  logical points.

## 0.7.0 — 2026-10-22

### Changed

- FloK's "!" stays on when a turn finishes while the panel is closed, until
  you open the panel.
- The working badge on FloK shows the count inside a rotating ring.

## 0.6.0 — 2026-10-22

### Changed

- The interface is in English.
- Panel toasts can be closed with ✕ or dismissed by dragging them away.

## 0.5.0 — 2026-10-22

### Changed

- The app is now called Flow Keeper (it was AI Tracker), and the companion
  is FloK. On the first launch after updating from 0.4, the app moves itself
  to “Flow Keeper.app” and migrates the data folder, API key, launch at
  login, Claude Code and Codex plugins and the editor extension. Releases
  still archive the bundle as “AI Tracker.app”, the only name 0.4 can
  install.
- The migration also replaces the AI Tracker plugin installed by hand.

## 0.4.0 — 2026-10-22

### Changed

- Agents and editors are shown as colored badges.
- The panel always reopens on “Your conversations”.

## 0.3.0 — 2026-10-22

### Added

- ✕ “Seen” on a request in “Waiting for you”: it leaves the list, the counts
  and the badge until the conversation's state changes.

### Changed

- FloK's badge moves to the bottom right, leaving room for the "!" above
  its head.

### Fixed

- `install.sh` no longer stops with “REPO: unbound variable” when
  `LANG=C.UTF-8`.

## 0.2.0 — 2026-10-22

### Added

- Updates from the app: it checks the latest release with GitHub CLI shortly
  after launch and every 30 minutes, shows the “new me available!” bubble
  and installs the bundle after verifying its checksum and version.
- Extension for VS Code, Insiders, Cursor and Windsurf: “Open conversation”
  selects the exact integrated terminal. Without it, the window of the
  conversation's folder is brought to the front.

## 0.1.1 — 2026-10-22

### Fixed

- Connecting and detecting Codex CLI.

## 0.1.0 — 2026-10-22

First version.

- Tracks Claude Code and Codex CLI conversations on macOS through plugin
  hooks that write to a local spool.
- Rust core: state and visibility engine, SQLite store with search,
  projects by repository, summaries with `claude -p` or the API, focusing
  and resuming sessions, doctor.
- Tauri 2 + Svelte app: the companion robot with its animations, panel,
  tray, notifications and guided setup.
- The app connects its built-in plugin to Claude Code and Codex from a local
  marketplace and keeps it in sync after updates.
- Installation from GitHub without a Developer ID signature.
