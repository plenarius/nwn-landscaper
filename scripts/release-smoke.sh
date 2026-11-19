#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Checks an unpacked release package (.github/workflows/release.yml): starts its program with `serve`, which finds the editor in
# web/ beside it as a double-click does, and fetches the page, two of its scripts and /healthz. Stops the program and removes what
# it wrote, whether the check passes or not.
#
#   scripts/release-smoke.sh <unpacked package folder> [port] [seconds to wait for it]
#
# Without a game install the program still serves the editor (it says what it couldn't find). With one (NWN_ROOT), the first
# start builds the starter texture library before it listens: give it a few minutes.
set -euo pipefail

dir=${1:?usage: release-smoke.sh <unpacked package folder> [port] [seconds]}
port=${2:-18765}
wait=${3:-120}
exe="$dir/landscaper"
[ -f "$exe.exe" ] && exe="$exe.exe"
[ -x "$exe" ] || { echo "release-smoke: no landscaper program in $dir" >&2; exit 1; }
[ -f "$dir/web/index.html" ] || { echo "release-smoke: no web/index.html in $dir" >&2; exit 1; }

"$exe" --version

# The data folder (texture library, exports) in a scratch folder of its own.
data=$(mktemp -d)
log="$data/serve.log"
XDG_DATA_HOME="$data" "$exe" serve --port "$port" >"$log" 2>&1 &
pid=$!
stop() {
    kill "$pid" 2>/dev/null || true
    case "${OSTYPE:-}" in msys* | cygwin*) taskkill //F //IM landscaper.exe >/dev/null 2>&1 || true ;; esac
    wait "$pid" 2>/dev/null || true
    echo "--- what it printed:"
    cat "$log"
    rm -rf "$data"
}
trap stop EXIT

url="http://127.0.0.1:$port"
for _ in $(seq 1 "$wait"); do
    curl -fsS -o /dev/null "$url/healthz" 2>/dev/null && break
    kill -0 "$pid" 2>/dev/null || { echo "release-smoke: the program stopped" >&2; exit 1; }
    sleep 1
done
curl -fsS "$url/healthz"
echo
curl -fsS "$url/" | grep -q 'type="importmap"' || { echo "release-smoke: / is not the editor's page" >&2; exit 1; }
curl -fsS -o /dev/null "$url/js/app.js"
curl -fsS -o /dev/null "$url/vendor/three/three.module.js"
echo "release-smoke: $url serves the editor"
