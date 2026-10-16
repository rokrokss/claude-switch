#!/usr/bin/env bash
#
# icloud-config.sh — the Tauri config that signs claude-switch for iCloud.
#
# Usage: scripts/icloud-config.sh <profile.provisionprofile> <out dir>
#
# Checks that the Developer ID provisioning profile is claude-switch's and
# allows its iCloud container, then writes into <out dir>:
#   icloud.entitlements  what the app is signed with;
#   icloud.conf.json     points bundle.macOS.entitlements at it and embeds the
#                        profile as Contents/embedded.provisionprofile.
# Prints the path of icloud.conf.json, for `tauri build --config`.
#
# Xcode adds the identity entitlements (application identifier, team,
# Keychain group) when it signs, and the Tauri bundler doesn't: without them
# taskgated refuses the signature at launch. So they are read from the
# profile, after OpenUsage's script/render_icloud_entitlements.sh (MIT,
# https://github.com/robinebers/openusage).

set -euo pipefail

usage="usage: scripts/icloud-config.sh <profile.provisionprofile> <out dir>"
PROFILE="${1:?$usage}"
OUT_DIR="${2:?$usage}"
PROFILE="$(cd "$(dirname "$PROFILE")" && pwd)/$(basename "$PROFILE")"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_ID="$(jq -r .identifier "$ROOT/apps/claude-switch/src-tauri/tauri.conf.json")"
# The container src-tauri/src/usage/sync opens.
CONTAINER="iCloud.$BUNDLE_ID"

fail() {
    echo "icloud-config.sh: $PROFILE $1" >&2
    exit 1
}

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
DECODED="$OUT_DIR/profile.plist"
/usr/bin/security cms -D -i "$PROFILE" > "$DECODED" 2>/dev/null || fail "is not a provisioning profile"
# Only the entitlements convert to JSON: the rest holds dates and data.
ENTITLEMENTS_JSON="$(/usr/bin/plutil -extract Entitlements json -o - "$DECODED")"
TEAM="$(/usr/libexec/PlistBuddy -c 'Print :TeamIdentifier:0' "$DECODED")"
rm -f "$DECODED"
entitlement() { jq -r --arg key "$1" '.[$key] // empty | if type == "array" then .[] else . end' <<<"$ENTITLEMENTS_JSON"; }

APP_ID="$(entitlement com.apple.application-identifier)"
[ "$APP_ID" = "$TEAM.$BUNDLE_ID" ] || fail "is for $APP_ID, not $TEAM.$BUNDLE_ID"
entitlement com.apple.developer.icloud-container-identifiers | grep -qxF "$CONTAINER" ||
    fail "does not allow the iCloud container $CONTAINER"
entitlement com.apple.developer.ubiquity-container-identifiers | grep -qxF "$CONTAINER" ||
    fail "does not allow iCloud Documents in $CONTAINER"
ENVIRONMENT="$(entitlement com.apple.developer.icloud-container-environment)"
[ "$ENVIRONMENT" = "Production" ] || fail "is not a Developer ID profile (iCloud environment: ${ENVIRONMENT:-none})"

cat > "$OUT_DIR/icloud.entitlements" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.application-identifier</key>
	<string>$APP_ID</string>
	<key>com.apple.developer.team-identifier</key>
	<string>$TEAM</string>
	<key>keychain-access-groups</key>
	<array>
		<string>$APP_ID</string>
	</array>
	<key>com.apple.developer.icloud-container-identifiers</key>
	<array>
		<string>$CONTAINER</string>
	</array>
	<key>com.apple.developer.icloud-services</key>
	<array>
		<string>CloudDocuments</string>
	</array>
	<key>com.apple.developer.ubiquity-container-identifiers</key>
	<array>
		<string>$CONTAINER</string>
	</array>
	<key>com.apple.developer.icloud-container-environment</key>
	<string>Production</string>
</dict>
</plist>
PLIST
/usr/bin/plutil -lint -s "$OUT_DIR/icloud.entitlements"

jq -n --arg entitlements "$OUT_DIR/icloud.entitlements" --arg profile "$PROFILE" \
    '{bundle: {macOS: {entitlements: $entitlements, files: {"embedded.provisionprofile": $profile}}}}' \
    > "$OUT_DIR/icloud.conf.json"
echo "$OUT_DIR/icloud.conf.json"
