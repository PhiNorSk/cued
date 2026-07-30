#!/bin/bash
# Cued installer for macOS (Apple Silicon).
#
#   curl -fsSL https://raw.githubusercontent.com/PhiNorSk/cued/main/install.sh | bash
#
# Downloads the latest release .dmg and copies Cued.app into /Applications.
# Because the download happens via curl (not a browser), macOS attaches no
# quarantine flag and the app opens without a Gatekeeper prompt.
set -euo pipefail

REPO="PhiNorSk/cued"
APP_NAME="Cued"
DEST="/Applications"

fail() {
  echo "✗ $1" >&2
  exit 1
}

[[ "$(uname -s)" == "Darwin" ]] || fail "This installer is for macOS. Windows builds: https://github.com/$REPO/releases"
[[ "$(uname -m)" == "arm64" ]] || fail "Cued currently ships for Apple Silicon only (this Mac is $(uname -m))."
[[ -w "$DEST" ]] || fail "$DEST is not writable by your user. Move the app there manually from the .dmg on the Releases page."

echo "Looking up the latest Cued release..."
TAG="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | sed -n 's/^ *"tag_name": *"\([^"]*\)".*/\1/p')"
[[ -n "$TAG" ]] || fail "Could not determine the latest release. Check https://github.com/$REPO/releases"
VERSION="${TAG#v}"

WORKDIR="$(mktemp -d)"
MOUNT_POINT=""
cleanup() {
  [[ -n "$MOUNT_POINT" ]] && hdiutil detach "$MOUNT_POINT" -quiet 2>/dev/null || true
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

DMG_URL="https://github.com/$REPO/releases/download/$TAG/${APP_NAME}_${VERSION}_aarch64.dmg"
echo "Downloading Cued ${VERSION}..."
curl -fL --progress-bar -o "$WORKDIR/cued.dmg" "$DMG_URL"

MOUNT_POINT="$(hdiutil attach -nobrowse -readonly "$WORKDIR/cued.dmg" \
  | awk -F'\t' '$NF ~ /^\/Volumes\// { mp = $NF } END { print mp }')"
[[ -d "$MOUNT_POINT/$APP_NAME.app" ]] || fail "$APP_NAME.app not found inside the disk image."

# Replace any existing install (quit it first so the copy isn't blocked).
osascript -e "quit app \"$APP_NAME\"" 2>/dev/null || true
rm -rf "${DEST:?}/$APP_NAME.app"
cp -R "$MOUNT_POINT/$APP_NAME.app" "$DEST/"

hdiutil detach "$MOUNT_POINT" -quiet
MOUNT_POINT=""

# Belt and braces: strip a quarantine flag if one ever ends up on the bundle.
xattr -dr com.apple.quarantine "$DEST/$APP_NAME.app" 2>/dev/null || true

echo "✓ Cued $VERSION installed to $DEST/$APP_NAME.app"
echo "  Launch it from Spotlight or run: open -a $APP_NAME"
