#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Makes the handbook site's /try/ page: the Landscaper running in the browser (docs/DESIGN-web-demo.md, docs/handbook/try.md).
# The engine is the newest release's landscaper-wasm.zip and the editor (nwn-landscaper-web) the same release's tag, so the
# page is what that release is. Until a release carries the zip, the engine is built here (scripts/build-wasm.sh) and the
# editor is its main branch. The editor's scripts/try-site.sh then puts the page together from listed files only, with
# packs/demo; nothing of the game's can reach the site. Needs gh (GH_TOKEN in CI), git, unzip.
#
# Usage: scripts/try-page.sh OUT     (OUT: the folder to make, such as site/try after `mkdocs build`)
# Environment: TRY_EDITOR_REF, an editor branch or tag to use instead (to try a change to the page before its release).
#              NWN_ROOT, a game install to make the stock placeables from when no release has them (below).
set -euo pipefail
if (($# != 1)); then
    sed -n '/^# Usage/,/^set /p' "$0" | sed '$d' >&2
    exit 2
fi
out=$(realpath -m "$1")
cd "$(dirname "$0")/.."
repo=${GITHUB_REPOSITORY:-plenarius/nwn-landscaper}
work=target/try-page
rm -rf "$work" && mkdir -p "$work"

# The newest release (vX.Y.Z) with the engine for the browser.
tag=
for t in $(gh release list --repo "$repo" --exclude-pre-releases --limit 20 --json tagName -q '.[].tagName' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$'); do
    if gh release view "$t" --repo "$repo" --json assets -q '.assets[].name' | grep -qx landscaper-wasm.zip; then
        tag=$t
        break
    fi
done
if [[ -n "$tag" ]]; then
    echo "the engine: $tag's landscaper-wasm.zip"
    gh release download "$tag" --repo "$repo" --pattern landscaper-wasm.zip --dir "$work"
    unzip -q "$work/landscaper-wasm.zip" -d "$work"
    wasm=$work/landscaper-wasm/landscaper.wasm
    editor_ref=$tag
else
    echo "the engine: no release has landscaper-wasm.zip yet, so it is built here (scripts/build-wasm.sh)"
    scripts/build-wasm.sh
    mkdir -p "$work/landscaper-wasm"
    cp target/wasm32-wasip1/wasm/landscaper.wasm LICENSE "$work/landscaper-wasm/"
    cp scripts/wasm/NOTICE.md "$work/landscaper-wasm/README.md"
    wasm=$work/landscaper-wasm/landscaper.wasm
    editor_ref=main
fi
editor_ref=${TRY_EDITOR_REF:-$editor_ref}
echo "the editor: nwn-landscaper-web at $editor_ref"
git clone -q --depth 1 --branch "$editor_ref" https://github.com/plenarius/nwn-landscaper-web "$work/editor"
if [[ ! -x "$work/editor/scripts/try-site.sh" ]]; then
    echo "nwn-landscaper-web at $editor_ref has no scripts/try-site.sh: it is older than the page in the browser" >&2
    exit 1
fi
"$work/editor/scripts/try-site.sh" "$out" "$(realpath "$wasm")" "$PWD/packs/demo"

# The game's stock placeables (src/demostock.rs): the "demo-stock" release's demo-stock.zip, made once with the game on a
# machine that has it (`landscaper demo-stock DIR`, then DIR zipped), since Actions has no game; else made here when
# NWN_ROOT names an install; else none, and the page draws placeables it lacks as boxes. The page fetches a model's files only
# when an area or a panel needs them.
stock=$work/stock
if gh release download demo-stock --repo "$repo" --pattern demo-stock.zip --dir "$work" 2> /dev/null; then
    echo "the stock placeables: the demo-stock release's"
    unzip -q "$work/demo-stock.zip" -d "$stock"
elif [[ -n "${NWN_ROOT:-}" ]]; then
    echo "the stock placeables: made from $NWN_ROOT"
    cargo run -q --release -- demo-stock "$stock"
else
    echo "the stock placeables: none (no demo-stock release, no NWN_ROOT): the page draws them as boxes"
fi
if [[ -f "$stock/index.json" ]]; then
    cp -r "$stock" "$out/try/stock"
fi

# The engine's licence and what else is built into it (the zip's README), beside it: the page hands the engine out.
cp "$work/landscaper-wasm/LICENSE" "$out/try/LICENSE-engine.txt"
cp "$work/landscaper-wasm/README.md" "$out/try/NOTICE-engine.md"

# GitHub Pages takes a site of up to 1 GB; this page is about 14 MB, and 210 MB with the stock placeables. Much more means
# something that shouldn't be there.
size=$(du -sm "$out" | cut -f1)
limit=$([[ -d "$out/try/stock" ]] && echo 300 || echo 40)
if ((size > limit)); then
    echo "$out is $size MB, over the $limit MB this page should ever need" >&2
    exit 1
fi
echo "$out: $size MB, engine $(du -h "$wasm" | cut -f1), editor $editor_ref"
