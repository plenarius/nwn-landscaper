#!/bin/sh
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Copies the showcase (docs/assets/showcase: its list, stills and areas, not its clips) into a packed copy of the editor as
# showcase/, for File → Open a showcase. The files live only here: every package of the editor (release.yml, try-page.sh,
# deploy/self-host/Dockerfile, the forge's image) gets them this way, and scripts/check-editor-files.py fails without them.
#
# Usage: scripts/pack-showcase.sh WEB_DIR
set -eu
if [ $# -ne 1 ]; then
    sed -n '/^# Usage/,/^set /p' "$0" | sed '$d' >&2
    exit 2
fi
src=$(dirname "$0")/../docs/assets/showcase
out=$1/showcase
rm -rf "$out"
mkdir -p "$out/areas"
cp "$src/index.json" "$src"/*.webp "$out/"
cp "$src"/areas/*.json "$out/areas/"
