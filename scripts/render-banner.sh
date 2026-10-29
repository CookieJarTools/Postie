#!/usr/bin/env bash
# Renders scripts/banner/banner.html to docs/banner.png with headless Firefox.
#
#   scripts/render-banner.sh
#
# Laid out at 1280x640 and drawn at 2x (the page zooms itself for #2x), so it stays sharp on
# high-DPI screens. A throwaway Firefox profile keeps your own profile out of it.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT

mkdir -p "$root/docs"
firefox --headless --no-remote --profile "$profile" --window-size=2560,1280 \
  --screenshot "$root/docs/banner.png" "file://$root/scripts/banner/banner.html#2x" > /dev/null 2>&1
echo "Wrote docs/banner.png"
