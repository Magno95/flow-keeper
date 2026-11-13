# Installing Flow Keeper — instructions for a coding agent

You are a coding agent (Claude Code, Codex or similar) running in a clone of
this repository. The user wants Flow Keeper installed on this Mac. Follow
these steps in order. Run the commands yourself; stop and ask the user only
where a step says so.

Flow Keeper is a macOS app that tracks Claude Code and Codex CLI
conversations. Installing it does not require changing this repository.

## Rules

- Do not edit, commit or push anything in this repository.
- Do not edit `~/.claude/settings.json`, Codex settings or any hook
  configuration, and do not run `/plugin` commands. The app connects the
  agents itself, from its own setup screen.
- Do not also install the plugin by hand from the GitHub marketplace: the app
  registers its own copy from the installed bundle and keeps it aligned with
  every update.
- Do not run `gh auth login` yourself: it is interactive. Ask the user to run
  it (in Claude Code they can type `! gh auth login`).

## 1. Check the prerequisites

```sh
uname -s            # must print Darwin: Flow Keeper is macOS only
command -v gh       # GitHub CLI
gh auth status      # must report a logged-in account
gh repo view Magno95/flow-keeper --json name -q .name
```

- `uname` not `Darwin`: stop and tell the user Flow Keeper only runs on macOS.
- `gh` missing: run `brew install gh` (if Homebrew is missing too, ask the
  user to install it from https://brew.sh).
- Not logged in: ask the user to run `gh auth login`, then check again.
- `gh repo view` fails ("Could not resolve" or a timeout): GitHub is not
  reachable. Ask the user to check the connection, then check again. The
  download in step 2 cannot work without it.

## 2. Run the installer

From the root of this clone:

```sh
./scripts/install.sh
```

The script downloads the latest release with `gh`, verifies its SHA-256
checksum, installs `~/Applications/Flow Keeper.app`, signs it ad hoc for this Mac and launches
it. It takes under a minute. If it says the app is already running, it quits
the old version and replaces it: that is expected.

The installed release may be older than the code in this clone. That is fine:
the app updates itself when new releases are published.

If the script reports that no release is available, it falls back to building
from source, which needs Xcode Command Line Tools (`xcode-select --install`),
Rust (https://rustup.rs) and Node.js 22.12 or later (`brew install node`) and
takes several minutes. If either is missing, ask
the user before installing them. On a build failure the script prints the
last lines of the log: report them to the user.

## 3. Verify

```sh
test -d "$HOME/Applications/Flow Keeper.app" && echo installed
pgrep -x flow-keeper >/dev/null && echo running
cat "$HOME/Library/Application Support/Flow Keeper/install-manifest.json"
```

All three must succeed. If the app is not running, start it with
`open "$HOME/Applications/Flow Keeper.app"`.

## 4. Hand over to the user

You cannot click through the app's setup, so tell the user, in their
language:

1. FloK, a small robot, is now on the desktop and the setup screen
   “Connect your agents” is open. Go through it: connect only the agents you
   use (Claude Code, Codex), optionally the editors, then summaries,
   notifications and the test. Skipped steps can be completed later from
   Settings → Connect your agents.
2. macOS will ask for permission to send notifications and to control the
   terminal: allow both.
3. After connecting, start **new** agent sessions: sessions already open,
   including this one, do not load the plugin.
4. Codex only: open Codex, type `/hooks` and approve the Flow Keeper hooks
   (once).
5. Editors (VS Code, Cursor, Windsurf), only if the extension was installed:
   in each window run “Developer: Reload Window” from the Command Palette
   (⇧⌘P).
6. Updates are automatic: the app checks for new releases every 30 minutes
   using the same `gh` login.

## Uninstalling

```sh
./scripts/uninstall.sh          # keeps conversations and settings
./scripts/uninstall.sh --purge  # also deletes them and the API key
```

More details are in [README.md](./README.md#installation).
