#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Checks an installer of the desktop app (.github/workflows/release.yml): installs it as a person would, starts the installed app,
# and waits for its window to load the editor at /?app from the engine inside the app. Then checks that the engine answers (its
# /healthz, the editor's page, the app's own routes), that a second start only brings the first one forward, and that the engine
# stops with the app. Uninstalls and removes what it wrote, whether the check passes or not.
#
#   scripts/app-smoke.sh <installer: the NSIS .exe, the .dmg, the .deb or the .AppImage> [seconds to wait] [screenshot.png]
#
# With APP_SMOKE_SIGNED=1 (a version tag's build) the signatures must hold as well: Authenticode on the installer, the program
# and the uninstaller on Windows; on macOS Gatekeeper must accept the app and the .dmg, each with Apple's ticket stapled to it.
#
# No game here: the app is given a stand-in game folder (an empty data/nwn_base.key), so it starts its engine instead of asking
# for the folder; the engine says it can't build the texture library and serves the editor. On Linux it needs a display: without
# one it runs itself again under xvfb-run, with a session bus for the single-instance check.
set -euo pipefail

installer=${1:?usage: app-smoke.sh <installer> [seconds] [screenshot.png]}
wait=${2:-120}
shot=${3:-}
port=8766

if [ "$(uname -s)" = Linux ] && [ -z "${DISPLAY:-}" ]; then
    exec xvfb-run -a -s "-screen 0 1600x1000x24" dbus-run-session -- "$0" "$@"
fi
case "$installer" in /* | [A-Za-z]:*) ;; *) installer="$PWD/$installer" ;; esac
[ -f "$installer" ] || { echo "app-smoke: no installer at $installer" >&2; exit 1; }

scratch=$(mktemp -d)
# (On Windows the app gets the folders as Windows paths; this script reads them as they are too.)
command -v cygpath >/dev/null && scratch=$(cygpath -m "$scratch")
signed=${APP_SMOKE_SIGNED:-}
mkdir -p "$scratch/game/data" "$scratch/share"
: >"$scratch/game/data/nwn_base.key"
# The data folder (the log, settings.json) in the scratch folder, and the stand-in game.
export XDG_DATA_HOME="$scratch/share" NWN_ROOT="$scratch/game"
log="$XDG_DATA_HOME/landscaper/landscaper.log"
url="http://127.0.0.1:$port"
pid=
uninstall=:
exe=
exe_name=
inner=

answers() { curl -fsS -o /dev/null -m 2 "$url/healthz" 2>/dev/null; }
stop() {
    [ -z "$pid" ] || kill "$pid" 2>/dev/null || true
    case "${OSTYPE:-}" in msys* | cygwin*) [ -z "$exe_name" ] || taskkill //F //IM "$exe_name" >/dev/null 2>&1 || true ;; esac
    [ -z "$exe_name" ] || pkill -x "$exe_name" 2>/dev/null || true
    [ -z "$pid" ] || wait "$pid" 2>/dev/null || true
}
finish() {
    status=$?
    stop
    echo "--- the app's log:"
    cat "$log" 2>/dev/null || echo "(none)"
    $uninstall || true
    rm -rf "$scratch"
    exit "$status"
}
trap finish EXIT

answers && { echo "app-smoke: something already answers on $port" >&2; exit 1; }

case "$installer" in
    *.exe)
        # Per user, with no administrator: %LOCALAPPDATA%\NWN Landscaper. (//S: Git's bash would turn /S into a path, and the
        # installer would wait for clicks.)
        timeout 300 "$installer" //S
        dir="$(cygpath -u "$LOCALAPPDATA")/NWN Landscaper"
        for f in "$dir"/*.exe; do case "$f" in *uninstall.exe) ;; *) exe=$f ;; esac; done
        uninstall() { timeout 120 "$dir/uninstall.exe" //S; }
        uninstall=uninstall
        [ -f "$dir/web/index.html" ] || { echo "app-smoke: no web/ beside the program" >&2; exit 1; }
        for f in "$installer" "$exe" "$dir/uninstall.exe"; do
            status=$(powershell -NoProfile -Command "(Get-AuthenticodeSignature '$(cygpath -w "$f")').Status" | tr -d '\r')
            echo "Authenticode, $(basename "$f"): $status"
            [ -z "$signed" ] || [ "$status" = Valid ] || { echo "app-smoke: $(basename "$f") isn't validly signed" >&2; exit 1; }
        done
        ;;
    *.dmg)
        hdiutil attach -nobrowse -readonly -mountpoint "$scratch/dmg" "$installer" >/dev/null
        ditto "$scratch/dmg/NWN Landscaper.app" "$scratch/Applications/NWN Landscaper.app"
        hdiutil detach "$scratch/dmg" >/dev/null
        app="$scratch/Applications/NWN Landscaper.app"
        codesign --verify --deep --strict --verbose=2 "$app"
        for f in "$app"/Contents/MacOS/*; do exe=$f; done
        lipo -info "$exe"
        [ -f "$app/Contents/Resources/web/index.html" ] || { echo "app-smoke: no web/ in the app's resources" >&2; exit 1; }
        if [ -n "$signed" ]; then
            codesign --display --verbose=2 "$app" 2>&1 | grep -E '^(Authority|TeamIdentifier|Timestamp|Runtime)'
            spctl --assess --type execute --verbose=2 "$app"
            spctl --assess --type open --context context:primary-signature --verbose=2 "$installer"
            xcrun stapler validate "$app"
            xcrun stapler validate "$installer"
        fi
        ;;
    *.deb)
        sudo apt-get install -y -qq "$installer" >/dev/null
        package=$(dpkg-deb -f "$installer" Package)
        uninstall() { sudo apt-get remove -y -qq "$package" >/dev/null; }
        uninstall=uninstall
        exe=$(dpkg -L "$package" | grep '^/usr/bin/' | head -n 1)
        dpkg -L "$package" | grep -q '/web/index.html$' || { echo "app-smoke: no web/ in the package" >&2; exit 1; }
        ;;
    *.AppImage)
        cp "$installer" "$scratch/app.AppImage"
        chmod +x "$scratch/app.AppImage"
        exe="$scratch/app.AppImage"
        # No FUSE 2 on the runner: the AppImage unpacks itself and runs from there.
        export APPIMAGE_EXTRACT_AND_RUN=1
        inner=nwn-landscaper
        ;;
    *) echo "app-smoke: what installer is $installer?" >&2; exit 1 ;;
esac
# (The process to stop at the end: the AppImage's is the program inside it.)
[ -n "$exe" ] || { echo "app-smoke: no program in what $(basename "$installer") installed" >&2; exit 1; }
exe_name=${inner:-$(basename "$exe")}
echo "app-smoke: starting $exe"

"$exe" >"$scratch/out.log" 2>&1 &
pid=$!
# The window loads the app's page, which goes to the editor once the engine answers.
for _ in $(seq 1 "$wait"); do
    grep -qF "window: $url/?app" "$log" 2>/dev/null && break
    kill -0 "$pid" 2>/dev/null || { echo "app-smoke: the app stopped" >&2; cat "$scratch/out.log"; exit 1; }
    sleep 1
done
grep -qF "window: $url/?app" "$log" || { echo "app-smoke: the window never loaded the editor at $url/?app" >&2; exit 1; }

# The engine is the Landscaper (its header), serves the editor, and has the app's own route, which `landscaper serve` lacks.
curl -fsS -D - -o /dev/null "$url/healthz" | grep -qi '^x-landscaper:' || { echo "app-smoke: /healthz isn't the Landscaper's" >&2; exit 1; }
page=$(curl -fsS "$url/?app")
grep -q 'type="importmap"' <<<"$page" || { echo "app-smoke: /?app is not the editor's page" >&2; exit 1; }
route=$(curl -sS -X POST -H "X-Landscaper: 1" -H "Content-Type: application/json" -d '{"what":"smoke"}' \
    -w ' %{http_code}' "$url/api/app/open")
case "$route" in *"data, log or folders"*" 400") ;; *) echo "app-smoke: the app's route answered: $route" >&2; exit 1 ;; esac
# The updater's answer for the version menu (no newer release here: nothing to offer, or still asking).
update=$(curl -fsS -H "X-Landscaper: 1" "$url/api/app/update")
case "$update" in *'"state":"'*) echo "app-smoke: the updater says $update" ;; *) echo "app-smoke: /api/app/update answered: $update" >&2; exit 1 ;; esac

if [ -n "$shot" ]; then
    # Time for the view to draw, then the screen, for a person to look at (the check doesn't read it).
    sleep 10
    case "$(uname -s)" in
        Linux) import -window root "$shot" ;;
        Darwin) screencapture -x "$shot" ;;
        *) powershell -NoProfile -Command "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \$i = New-Object System.Drawing.Bitmap \$b.Width, \$b.Height; [System.Drawing.Graphics]::FromImage(\$i).CopyFromScreen(\$b.Location, [System.Drawing.Point]::Empty, \$b.Size); \$i.Save('$(cygpath -w "$shot" 2>/dev/null || echo "$shot")')" ;;
    esac || echo "app-smoke: no screenshot"
fi

# A second start brings the first window forward and ends; it starts no second engine.
second=0
"$exe" >"$scratch/second.log" 2>&1 &
second_pid=$!
for _ in $(seq 1 30); do kill -0 "$second_pid" 2>/dev/null || { second=1; break; }; sleep 1; done
if [ "$second" = 0 ]; then
    kill "$second_pid" 2>/dev/null || true
    echo "app-smoke: a second start kept running" >&2
    exit 1
fi
curl -fsS -o /dev/null -m 2 "http://127.0.0.1:$((port + 1))/healthz" 2>/dev/null && { echo "app-smoke: a second start started a second engine" >&2; exit 1; }

# The engine runs inside the app: it stops with it.
stop
pid=
for _ in $(seq 1 15); do answers || break; sleep 1; done
answers && { echo "app-smoke: the engine kept answering after the app stopped" >&2; exit 1; }
echo "app-smoke: $(basename "$installer") installs, its window loads the editor at $url/?app, and its engine answers"
