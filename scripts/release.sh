#!/usr/bin/env bash
#
# release.sh — build, notarize and publish a claude-switch release from this Mac.
#
# Usage: scripts/release.sh <notes.md> [--publish]
#
# Builds the universal app and DMG signed with the Developer ID, notarizes the
# DMG and staples the ticket to the DMG and the app, then packs the stapled app
# into claude-switch.app.tar.gz for the updater, signs it with the updater key
# and writes latest.json. Everything lands in
# apps/claude-switch/src-tauri/target/release-assets/v<version>/.
#
# Without --publish it stops there and prints the gh commands; with --publish
# it uploads the DMG, the tarball and latest.json to the v<version> release.
# The version comes from tauri.conf.json. See docs/releasing.md.
#
# With ICLOUD_PROVISIONING_PROFILE pointing at the Developer ID provisioning
# profile, the app is signed for iCloud and can sync usage history between
# Macs; without it, it is built as before and Settings says iCloud isn't
# available in this build.

set -euo pipefail

NOTES_FILE="${1:?usage: scripts/release.sh <notes.md> [--publish]}"
NOTES_FILE="$(cd "$(dirname "$NOTES_FILE")" && pwd)/$(basename "$NOTES_FILE")"
PUBLISH="${2:-}"

REPO=rokrokss/claude-switch
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAURI_DIR="$ROOT/apps/claude-switch/src-tauri"
SIGNING_KEY="${SIGNING_KEY:-$HOME/.tauri/claude-switch.key}"
NOTARY_PROFILE="${NOTARY_PROFILE:-claude-switch}"
export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Hyungrok Kim (87G5R48C73)}"
if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(security find-generic-password -s claude-switch-updater -w)"
fi
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
ICLOUD_PROVISIONING_PROFILE="${ICLOUD_PROVISIONING_PROFILE:-}"

VERSION="$(jq -r .version "$TAURI_DIR/tauri.conf.json")"
TAG="v$VERSION"
BUNDLE="$TAURI_DIR/target/universal-apple-darwin/release/bundle"
APP="$BUNDLE/macos/claude-switch.app"
DMG_NAME="claude-switch_${VERSION}_universal.dmg"
OUT="$TAURI_DIR/target/release-assets/$TAG"
TARBALL="$OUT/claude-switch.app.tar.gz"

green() { printf '\033[32m%s\033[0m\n' "$1"; }

# 1. Build. The updater key is only needed because createUpdaterArtifacts is
#    on; the tarball the bundler makes predates the notarization ticket, so
#    step 3 replaces it.
#    With an iCloud profile, the entitlements it allows are rendered first and
#    merged into the config, which embeds the profile too.
build_config=()
if [ -n "$ICLOUD_PROVISIONING_PROFILE" ]; then
    green "Signing for iCloud with $ICLOUD_PROVISIONING_PROFILE…"
    build_config=(--config "$("$ROOT/scripts/icloud-config.sh" "$ICLOUD_PROVISIONING_PROFILE" "$TAURI_DIR/target/icloud")")
fi
green "Building claude-switch $VERSION…"
# The ${…+…} form keeps bash 3.2 from calling an empty array unbound.
TAURI_SIGNING_PRIVATE_KEY="$SIGNING_KEY" pnpm --filter claude-switch tauri build --target universal-apple-darwin \
    ${build_config[@]+"${build_config[@]}"}

# 2. Notarize the DMG (Apple notarizes the app inside it with it), then staple
#    the ticket to both so each passes Gatekeeper offline.
green "Notarizing $DMG_NAME…"
xcrun notarytool submit "$BUNDLE/dmg/$DMG_NAME" --keychain-profile "$NOTARY_PROFILE" --wait
xcrun stapler staple "$BUNDLE/dmg/$DMG_NAME"
xcrun stapler staple "$APP"

# 3. Pack the stapled app for the updater, which expects claude-switch.app/ at
#    the root of the archive. COPYFILE_DISABLE keeps macOS's ._ files out.
rm -rf "$OUT"
mkdir -p "$OUT"
cp "$BUNDLE/dmg/$DMG_NAME" "$OUT/"
COPYFILE_DISABLE=1 tar -czf "$TARBALL" -C "$BUNDLE/macos" claude-switch.app
pnpm --filter claude-switch tauri signer sign -f "$SIGNING_KEY" "$TARBALL"

# 4. latest.json. The universal tarball serves both architectures.
URL="https://github.com/$REPO/releases/download/$TAG/claude-switch.app.tar.gz"
jq -n \
    --arg version "$VERSION" \
    --rawfile notes "$NOTES_FILE" \
    --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --rawfile signature "$TARBALL.sig" \
    --arg url "$URL" \
    '{version: $version, notes: $notes, pub_date: $pub_date, platforms: {
        "darwin-aarch64": {signature: $signature, url: $url},
        "darwin-x86_64": {signature: $signature, url: $url}}}' \
    > "$OUT/latest.json"

green "Release files for $TAG in $OUT:"
ls -l "$OUT"
echo "SHA-256 $DMG_NAME: $(shasum -a 256 "$OUT/$DMG_NAME" | cut -d' ' -f1)"

# 5. Publish as a draft first, so releases/latest (which the updater reads)
#    only moves once every file is uploaded.
create=(gh release create "$TAG" --repo "$REPO" --draft --target "$(git -C "$ROOT" rev-parse HEAD)"
    --title "claude-switch $TAG" --notes-file "$NOTES_FILE"
    "$OUT/$DMG_NAME" "$TARBALL" "$OUT/latest.json")
publish=(gh release edit "$TAG" --repo "$REPO" --draft=false)
if [ "$PUBLISH" = "--publish" ]; then
    "${create[@]}"
    "${publish[@]}"
    green "Published $TAG."
else
    echo
    echo "Not published. To publish these files (HEAD must be pushed):"
    printf '  '; printf '%q ' "${create[@]}"; echo
    printf '  '; printf '%q ' "${publish[@]}"; echo
fi
