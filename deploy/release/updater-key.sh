#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The key the app's updates are signed with, for `cargo tauri build` (.github/workflows/release.yml). Tauri signs each update file
# (the AppImage, the NSIS installer, the macOS .app.tar.gz: createUpdaterArtifacts in deploy/release/tauri.json) with the private
# key in TAURI_SIGNING_PRIVATE_KEY. On a version tag that is the release key, from the release environment's secrets. Any other
# run has no secrets, so this makes a throwaway key, and the same files are made and checked; no app trusts it, as the app trusts
# only the release key's public half (plugins.updater.pubkey in app/tauri.conf.json).
#
#   . deploy/release/updater-key.sh <public key file>
#
# Sourced, after deploy/release/tauri-cli.sh: it sets TAURI_SIGNING_PRIVATE_KEY and its password when they are empty. It writes
# the public key the signatures must check against to <public key file>: the app's own on a version tag, else the throwaway's
# (deploy/release/latest-json.py checks each signature against it).

updater_pub=${1:?usage: . updater-key.sh <public key file>}
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    jq -r .plugins.updater.pubkey app/tauri.conf.json > "$updater_pub"
else
    updater_tmp=$(mktemp -d)
    cargo tauri signer generate --ci --password "" --write-keys "$updater_tmp/throwaway.key" > /dev/null
    TAURI_SIGNING_PRIVATE_KEY=$(cat "$updater_tmp/throwaway.key")
    export TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
    cp "$updater_tmp/throwaway.key.pub" "$updater_pub"
    rm -rf "$updater_tmp"
    echo "updater: signing with a throwaway key (no release key outside version tags)"
fi
