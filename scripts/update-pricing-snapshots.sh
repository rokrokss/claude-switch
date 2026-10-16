#!/usr/bin/env bash
#
# update-pricing-snapshots.sh — refresh the model prices bundled with claude-switch.
#
# Usage: scripts/update-pricing-snapshots.sh
#
# Fetches the two public price lists and writes them, cut down to their prices,
# to apps/claude-switch/src-tauri/resources/pricing/:
#
#   litellm.json     LiteLLM's model_prices_and_context_window.json
#   models-dev.json  models.dev's api.json
#
# The app prices local spend with these until it has fetched the lists itself
# (it does so hourly and keeps what it fetched), so a stale snapshot only
# matters offline and on first launch. Refresh them before a release.
#
# The format must match `pricing::catalog::from_compact` in
# src-tauri/src/usage/spend/pricing/catalog.rs: rates per million tokens, a
# cache write defaulting to the input rate and a cache read to a tenth of it,
# with `"cre": false` marking a cache read that was filled in rather than
# published. Prices claude-switch sets itself live in
# apps/landing/public/pricing-supplement.json instead.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/apps/claude-switch/src-tauri/resources/pricing"

LITELLM_URL="https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json"
MODELS_DEV_URL="https://models.dev/api.json"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
mkdir "$tmpdir/feeds"

echo "Fetching LiteLLM prices..."
curl -fsSL --max-time 120 "$LITELLM_URL" -o "$tmpdir/feeds/litellm.json"
echo "Fetching models.dev prices..."
curl -fsSL --max-time 120 "$MODELS_DEV_URL" -o "$tmpdir/feeds/models_dev.json"

cat > "$tmpdir/compact.py" << 'PY'
import json
import sys
from datetime import datetime, timezone

feeds, out = sys.argv[1], sys.argv[2]
retrieved_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def number(value):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def per_million(value):
    return value * 1e6 if value is not None else None


def compact(i, o, cw, cr, cr_explicit, **extra):
    model = {"i": i, "o": o, "cw": cw, "cr": cr}
    if not cr_explicit:
        model["cre"] = False
    for key, value in extra.items():
        if value is not None:
            model[key] = value
    return model


def write(name, models):
    if not models:
        sys.exit(f"{name}: the feed had no usable prices; keeping the old snapshot.")
    with open(f"{out}/{name}", "w") as f:
        json.dump({"retrieved_at": retrieved_at, "models": models}, f, sort_keys=True, separators=(",", ":"))
    print(f"{name}: {len(models)} models")


# LiteLLM: prices per token; entries without both an input and an output price are stubs.
with open(f"{feeds}/litellm.json") as f:
    litellm = json.load(f)
models = {}
for key, entry in litellm.items():
    if not isinstance(entry, dict):
        continue
    i, o = number(entry.get("input_cost_per_token")), number(entry.get("output_cost_per_token"))
    if i is None or o is None:
        continue
    cw = number(entry.get("cache_creation_input_token_cost"))
    cr = number(entry.get("cache_read_input_token_cost"))
    specific = entry.get("provider_specific_entry")
    models[key] = compact(
        i * 1e6,
        o * 1e6,
        (cw if cw is not None else i) * 1e6,
        (cr if cr is not None else i * 0.1) * 1e6,
        cr is not None,
        ia=per_million(number(entry.get("input_cost_per_token_above_200k_tokens"))),
        oa=per_million(number(entry.get("output_cost_per_token_above_200k_tokens"))),
        cwa=per_million(number(entry.get("cache_creation_input_token_cost_above_200k_tokens"))),
        cra=per_million(number(entry.get("cache_read_input_token_cost_above_200k_tokens"))),
        fast=number(specific.get("fast")) if isinstance(specific, dict) else None,
    )
write("litellm.json", models)

# models.dev: prices already per million; ids kept bare, the first provider by name wins.
with open(f"{feeds}/models_dev.json") as f:
    models_dev = json.load(f)
models = {}
for provider_name in sorted(models_dev):
    provider = models_dev[provider_name]
    if not isinstance(provider, dict):
        continue
    for model_id, model in (provider.get("models") or {}).items():
        if model_id in models or not isinstance(model, dict):
            continue
        cost = model.get("cost") or {}
        i, o = number(cost.get("input")), number(cost.get("output"))
        if i is None or o is None:
            continue
        cw, cr = number(cost.get("cache_write")), number(cost.get("cache_read"))
        models[model_id] = compact(i, o, cw if cw is not None else i, cr if cr is not None else i * 0.1, cr is not None)
write("models-dev.json", models)
PY

mkdir -p "$OUT"
python3 -I "$tmpdir/compact.py" "$tmpdir/feeds" "$OUT"
ls -lh "$OUT"
