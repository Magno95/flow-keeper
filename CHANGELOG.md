# Changelog

Notable changes to Flow Keeper, newest first. Each release lists what
changes for people using the app; release tooling goes under Development.
A release opens with a one-line summary of its main changes, which the app
shows in the update box.

## Unreleased

## 0.15.0 — 2026-11-13

See what each new version brings before updating; Claude Fable 5.2 usage is
now priced.

### Added

- When a new version is available, the update box in Settings lists the main
  changes of each release after the one you have.

### Fixed

- Usage from Claude Fable 5.2 and Claude Mythos 5.2 is now included in the
  estimated cost.

## 0.14.0 — 2026-11-10

### Added

- Drag a project card to the left, or swipe it with two fingers, to uncover a
  button that removes the whole project from view, with Undo. Each conversation
  reappears on its own as soon as it picks up again.

## 0.13.0 — 2026-11-05

### Changed

- First setup includes optional editor extension installation, persistent reload
  instructions, separate conversation checks for each agent, and a final review
  of connections and pending manual steps.
- Codex-only installations no longer require a Claude connection in the overview
  or connection diagnostics. The automatic Claude test is offered only when connected.

### Fixed

- Setup reports API key and settings save failures without claiming success or
  leaving the wizard. Summary configuration must be usable or explicitly skipped.

## 0.12.0 — 2026-11-05

### Added

- Analytics has a Today period: the tokens and estimated cost of the day so
  far, per hour, compared with yesterday up to the same hour.

## 0.11.0 — 2026-11-05

### Added

- Analytics estimates what the Claude models' tokens would cost at the Claude
  API list prices, per day, by project and by model, compared with the
  previous period. Cache writes are priced by how long they're kept; Codex
  isn't counted. On the first launch the transcripts still on disk are read
  again to tell the two kinds of cache write apart.
- Analytics shows input, cache write and cache read tokens next to all and
  output tokens: pick one to see it per day, by project and by model. Each
  model lists the four types.

### Changed

- The database no longer grows without limit: the raw hook events and
  notification records are deleted after 30 days. Conversations, summaries and
  token usage are kept.

## 0.10.0 — 2026-11-05

### Added

- Analytics (the chart button next to Settings) shows the tokens of all your
  conversations over 7, 30 or 90 days or all time, per day, by project and by
  model.
- Token usage is saved by Flow Keeper a minute after each reply, so it stays
  after Claude Code or Codex delete old transcripts. A conversation's details
  show the saved figures when the transcript is gone.
- A conversation's details show when it started.
- Activity, in Analytics → App use: how many times you open the
  panel and go to a conversation, compared with the previous 7, 30 or 90 days,
  per day and by weekday and hour. Only these counts are kept, on your Mac.
- The counts can also be sent to a server of your choice (Settings → Activity).
  When it can't be reached, they wait on your Mac and are sent later. The
  request format is described in the README.

### Development

- `FLOK_ACTIVITY_ENDPOINT` and `FLOK_ACTIVITY_TOKEN`, set when building, give the
  build a default activity server.

### Fixed

- The companion now reacts to the mouse hovering over it even while another app is active.

## 0.9.3 — 2026-10-30

### Fixed

- The companion's problem face no longer shows for several minutes after an app
  update: it now rechecks the Claude/Codex connection status every 5 seconds
  while a problem is detected, instead of waiting up to 10 minutes.

## 0.9.2 — 2026-10-29

### Changed

- A conversation's details (click the card) also show the tokens it has used,
  with a link to the full analytics.
- Every conversation card has an always visible × to remove it from view,
  besides the entry in the “…” menu.

## 0.9.1 — 2026-10-29

### Fixed

- Codex sessions with automatic approval review no longer show “Waiting for
  you” for every command the reviewer checks: those requests are decided by
  Codex, not by you. With manual approvals they are still shown.

## 0.9.0 — 2026-10-28

### Added

- Analytics in the conversation menu (“…”): tokens used by the conversation,
  split into input, cache write, cache read and output (with the reasoning
  share), plus the subagents' share for Claude Code, the number of model
  responses and the models used. Read from the agent's local transcript when
  you open it.

### Fixed

- Two identical prompts in different turns (for example “continue” twice) are
  both counted: the conversation no longer stays on “turn ended” after the
  second one.
- A late event from a previous terminal instance no longer takes over the
  conversation, so it isn't closed while the new instance is still running.
- Excluding a project, or turning summaries off, now takes effect immediately
  even for a summary already in progress: nothing more is read, and no request
  starts after the change is confirmed (the last check happens right before
  each request). If the project is excluded, whatever the running summary
  produces is discarded; if summaries are turned off, the parts already
  finished are kept. A request that has already started cannot be recalled.
- Changing the summary provider or model in Settings stops a running summary
  at the next step and continues it with the new one, instead of finishing with
  the old one.
- The daily summary limit now applies to every call to the model, not only to
  each batch of work: with one call left, at most one is made. Failed or
  unusable responses count too, so retries can't go past the limit. When a
  response was paid for but can't be used (invalid, refused or cut off), its
  cost is still recorded against the monthly budget.
- The filter that hides secrets before text is sent for a summary now also
  catches credentials written as JSON (`{"password": "…"}`), keys such as
  `access_token`, bearer tokens and passwords in URLs, including values
  containing spaces, quotes or backslashes. The project name and
  the previous note go through the same filter, and so do error messages
  before they are saved.
- If the saved settings can't be read (damaged file, unknown value, a value
  out of range), summaries stay off instead of silently turning on with the
  defaults. The damaged record is kept aside, Diagnostics explains what
  happened, and saving the settings again brings summaries back. Daily limit,
  budget, model names and shortcut are now checked when saving; a shortcut
  that can't be registered is refused and the previous one keeps working.
- In Settings, changes are saved one after the other and the page shows what
  was actually saved: if a save fails, the previous value comes back (for
  example “Off” no longer appears selected when it wasn't saved).
- A conversation with a very large line in its transcript (over 8 MiB, for
  example a huge tool result) no longer stops its summary from ever updating:
  the oversized line is skipped without being interpreted or sent, and the
  summary continues with what follows (a line beyond 1 GiB is reported as an
  error instead).
- Commands run by the app (Claude Code for summaries, GitHub CLI for updates,
  plugin and editor checks) no longer time out just because they print a lot
  of output: their output is read while they run. The time limit now also
  covers sending the input, and a stuck command is stopped together with any
  process it started.

## 0.8.0 — 2026-10-26

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

### Development

- `scripts/release.sh` builds the universal bundle locally and publishes the
  release with its checksum; `--build` only builds it into `dist/`.

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
