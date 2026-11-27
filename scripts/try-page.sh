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

# The engine's licence and what else is built into it (the zip's README), beside it: the page hands the engine out.
cp "$work/landscaper-wasm/LICENSE" "$out/try/LICENSE-engine.txt"
cp "$work/landscaper-wasm/README.md" "$out/try/NOTICE-engine.md"

# GitHub Pages takes a site of up to 1 GB; this page is about 14 MB. Much more means something that shouldn't be there.
size=$(du -sm "$out" | cut -f1)
if ((size > 40)); then
    echo "$out is $size MB, over the 40 MB this page should ever need" >&2
    exit 1
fi
echo "$out: $size MB, engine $(du -h "$wasm" | cut -f1), editor $editor_ref"
