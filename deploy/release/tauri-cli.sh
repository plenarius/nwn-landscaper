#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Installs Tauri's command line (`cargo tauri`), which builds the desktop app's installers (.github/workflows/release.yml), from
# Tauri's own release for this system, checked against the checksum below, into cargo's folder of programs. Its prebuilt
# program, not `cargo install`, which would build it for minutes on every runner.
set -euo pipefail

version=2.12.4
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) asset=cargo-tauri-x86_64-unknown-linux-gnu.tgz sum=820ae3c7580ca637cad15f65638960ee2a96e5a491e224691c86d6f25e6c9e8a ;;
    Darwin-arm64) asset=cargo-tauri-aarch64-apple-darwin.zip sum=f71a34ada728ca2d1a51c28d6d21639e1e080898ec69ec8d49c5da3b7e8bc1be ;;
    MINGW*-x86_64 | MSYS*-x86_64) asset=cargo-tauri-x86_64-pc-windows-msvc.zip sum=3e8687fb184bd7f10ebd8642f7715f79f4a605c0694c142a21507e8a24dc0f8b ;;
    *) echo "tauri-cli: no Tauri command line for $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

bin=$(dirname "$(command -v cargo)")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL -o "$work/$asset" "https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v$version/$asset"
if command -v sha256sum >/dev/null; then got=$(sha256sum "$work/$asset"); else got=$(shasum -a 256 "$work/$asset"); fi
[ "${got%% *}" = "$sum" ] || { echo "tauri-cli: $asset's checksum is ${got%% *}, not $sum" >&2; exit 1; }
case "$asset" in
    *.tgz) tar -xzf "$work/$asset" -C "$work" ;;
    *) unzip -q "$work/$asset" -d "$work" ;;
esac
for f in cargo-tauri cargo-tauri.exe; do [ -f "$work/$f" ] && install -m 755 "$work/$f" "$bin/$f"; done
cargo tauri --version
