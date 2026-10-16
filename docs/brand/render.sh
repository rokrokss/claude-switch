#!/usr/bin/env bash
# Renders the README and social images in docs/brand to PNGs.
# Usage: docs/brand/render.sh [name ...]   (no args = render everything)
# Needs Google Chrome, Claude.app in /Applications (its icon is badged the way
# the app badges it), and network for Google Fonts.
set -euo pipefail

CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"

mkdir -p "$DIR/.cache"
sips -s format png -Z 512 /Applications/Claude.app/Contents/Resources/electron.icns --out "$DIR/.cache/claude.png" >/dev/null

# name:query:width:height:scale:output
TARGETS=(
  "banner::1400:560:2:docs/assets/banner.png"
  "banner:?og:1200:630:1:apps/landing/public/og-image.png"
  "hero-dock::1200:540:2:docs/assets/hero-dock.png"
  "how-it-works::1200:556:2:docs/assets/how-it-works.png"
)

for t in "${TARGETS[@]}"; do
  IFS=: read -r name query w h scale out <<<"$t"
  if [[ $# -gt 0 && ! " $* " == *" $name "* ]]; then continue; fi
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars \
    --force-device-scale-factor="$scale" --window-size="$w,$h" \
    --default-background-color=00000000 \
    --allow-file-access-from-files \
    --virtual-time-budget=10000 \
    --screenshot="$ROOT/$out" "file://$DIR/$name.html$query" >/dev/null 2>&1
  echo "rendered $out ($w x $h @${scale}x)"
done
