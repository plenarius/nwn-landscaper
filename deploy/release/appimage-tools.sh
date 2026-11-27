#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The tools Tauri builds the AppImage with, at fixed versions checked against the checksums below
# (.github/workflows/release.yml). Tauri downloads them into its cache when they aren't there, one of them from a release that
# moves ("continuous"), and appimagetool then downloads the AppImage's runtime, the program at the start of every AppImage, from
# another. With these in place, every release builds from the same bits.
#
#   deploy/release/appimage-tools.sh fetch <Tauri's tool cache>   puts them there; prints the runtime's path, for LDAI_RUNTIME_FILE
#   deploy/release/appimage-tools.sh check <Tauri's tool cache> <the AppImage>
#                                    fails if Tauri fetched anything of its own, or the AppImage doesn't start with the runtime
#
# Tauri's cache is `tauri` in XDG_CACHE_HOME. The names are the ones Tauri's command line (deploy/release/tauri-cli.sh) looks for:
# a new one may want another linuxdeploy, which `check` then finds downloaded.
set -euo pipefail

# <name in the cache> <url> <sha256>
tools=(
    "AppRun-x86_64 https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64 f30140a43a0a59e46db21bdefdf749b9e9f2c6946e92afabbacf98b8ae73fb4f"
    "linuxdeploy-07333c6-x86_64.AppImage https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy-07333c6/linuxdeploy-x86_64.AppImage 36a2d7e274d12e1050d0e9ecfe11d339ed54720b2bec464c286d53f8b07f5c62"
    "linuxdeploy-plugin-appimage.AppImage https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/1-alpha-20250213-1/linuxdeploy-plugin-appimage-x86_64.AppImage 992d502a248e14ab185448ddf6f6e7d25558cb84d4623c354c3af350c25fccb3"
    "type2-runtime-x86_64 https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"
)
# What Tauri writes there itself, from its own program: the linuxdeploy plugins that are scripts.
own=(linuxdeploy-plugin-gtk.sh linuxdeploy-plugin-gstreamer.sh)

what=${1:?usage: appimage-tools.sh fetch|check <cache> [AppImage]}
cache=${2:?usage: appimage-tools.sh fetch|check <cache> [AppImage]}

case "$what" in
    fetch)
        mkdir -p "$cache"
        for t in "${tools[@]}"; do
            read -r name url sum <<<"$t"
            curl -fsSL -o "$cache/$name" "$url"
            echo "$sum  $cache/$name" | sha256sum -c - >&2
            chmod +x "$cache/$name"
        done
        echo "$cache/type2-runtime-x86_64"
        ;;
    check)
        appimage=${3:?usage: appimage-tools.sh check <cache> <AppImage>}
        expected=("${own[@]}")
        for t in "${tools[@]}"; do expected+=("${t%% *}"); done
        for f in "$cache"/*; do
            name=$(basename "$f")
            printf '%s\n' "${expected[@]}" | grep -qxF "$name" || { echo "appimage-tools: Tauri fetched $name itself" >&2; exit 1; }
        done
        runtime="$cache/type2-runtime-x86_64"
        size=$(wc -c <"$runtime")
        # Byte for byte, but for the sections appimagetool fills in: the AppImage's digest, update information and signature.
        ranges=$(readelf -S -W "$runtime" | awk '$2 ~ /^\.(digest_md5|upd_info|sha256_sig|sig_key)$/ { print $5, $6 }' |
            while read -r off len; do echo "$((16#$off)) $((16#$len))"; done)
        [ "$(wc -l <<<"$ranges")" = 4 ] || { echo "appimage-tools: the runtime's sections aren't as expected" >&2; exit 1; }
        outside=$( (cmp -l -n "$size" "$runtime" "$appimage" || true) | awk -v r="$ranges" '
            BEGIN { n = split(r, v, /[ \n]/); for (i = 1; i < n; i += 2) { from[i] = v[i]; to[i] = v[i] + v[i + 1] } }
            { o = $1 - 1; inside = 0; for (i in from) if (o >= from[i] && o < to[i]) inside = 1; if (!inside) bad++ }
            END { print bad + 0 }')
        [ "$outside" = 0 ] || { echo "appimage-tools: $appimage doesn't start with the pinned runtime ($outside bytes differ)" >&2; exit 1; }
        echo "appimage-tools: $(basename "$appimage") was built with the pinned tools and starts with the pinned runtime"
        ;;
    *) echo "appimage-tools: fetch or check, not $what" >&2; exit 2 ;;
esac
