#!/bin/bash
# Builds the release on this Mac and publishes it on GitHub. Same steps as the
# `release` workflow, which keeps the assets uploaded here.
#
#   ./scripts/release.sh            build, tag and publish the version in Cargo.toml
#   ./scripts/release.sh --build    build and archive only
#
# Requires Rust with the aarch64/x86_64-apple-darwin targets, Node.js and `gh`.
#
# FLOK_ACTIVITY_ENDPOINT and FLOK_ACTIVITY_TOKEN, if set, become the build's
# default activity server (see “Activity” in the README).

set -euo pipefail

REPO="${FLOW_KEEPER_REPO:-Magno95/flow-keeper}"
cd "$(dirname "$0")/.."

publish=1
[[ "${1:-}" == "--build" ]] && publish=0

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -1)"
tag="v$version"
bundle="target/universal-apple-darwin/release/bundle/macos"
archive="Flow-Keeper_${version}_universal.app.tar.gz"
# The release notes are the version's section in CHANGELOG.md.
notes="$(awk -v v="$version" '$1 == "##" && $2 == v { on = 1; next } $1 == "##" { on = 0 } on' CHANGELOG.md |
  sed -e '/./,$!d')"

if ((publish)); then
  [[ -z "$(git status --porcelain)" ]] || { echo "Commit your changes first." >&2; exit 1; }
  # The branch that tracks main, on the remote it is published to.
  upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)"
  [[ "${upstream#*/}" == "main" ]] || { echo "Release from the branch that tracks main." >&2; exit 1; }
  remote="${upstream%%/*}"
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null && [[ "$(git rev-parse "$tag^{commit}")" != "$(git rev-parse HEAD)" ]]; then
    echo "The local tag $tag points to another commit." >&2
    exit 1
  fi
  [[ -n "$notes" ]] || { echo "Add a “## $version” section to CHANGELOG.md first." >&2; exit 1; }
  if gh release view "$tag" --repo "$REPO" >/dev/null 2>&1; then
    echo "$tag is already published." >&2
    exit 1
  fi
fi

if [[ -n "${FLOK_ACTIVITY_ENDPOINT:-}" ]]; then
  echo "Activity server of this build: $FLOK_ACTIVITY_ENDPOINT"
else
  echo "No activity server in this build (FLOK_ACTIVITY_ENDPOINT is not set)."
fi

# Paths of this Mac (home, clone) stay out of the binaries' panic messages.
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$HOME=~ --remap-path-prefix=$PWD=."

cargo test -p tracker-core
(cd app && npm ci && npx tauri build --target universal-apple-darwin --bundles app)

# As long as 0.4 installations exist, the bundle must be archived under the
# old name: 0.4 only updates from “AI Tracker.app”. The new app moves
# itself to “Flow Keeper.app” on first launch.
mkdir -p dist
tar -czf "dist/$archive" -C "$bundle" -s '/^Flow Keeper\.app/AI Tracker.app/' "Flow Keeper.app"
(cd dist && shasum -a 256 "$archive" > "$archive.sha256")
echo "Archive: dist/$archive"

((publish)) || exit 0

git push "$remote" HEAD:main
git rev-parse -q --verify "refs/tags/$tag" >/dev/null || git tag "$tag"
git push "$remote" "refs/tags/$tag"
gh release create "$tag" "dist/$archive" "dist/$archive.sha256" \
  --repo "$REPO" --verify-tag --title "Flow Keeper $tag" --notes "$notes"
