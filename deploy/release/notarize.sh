#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sends a signed file (a .zip of the program, or the .dmg) to Apple's notary service and waits for its answer
# (.github/workflows/release.yml, version tags). Fails, with Apple's log, unless Apple accepts it.
#
#   deploy/release/notarize.sh <file>     with NOTARY_KEY_PATH (the .p8), NOTARY_KEY_ID and NOTARY_ISSUER_ID set
set -euo pipefail

file=${1:?usage: notarize.sh <file>}
auth=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID")
answer=$(mktemp)
xcrun notarytool submit "$file" "${auth[@]}" --wait --timeout 1h --output-format json >"$answer" || true
cat "$answer"; echo
# notarytool's exit code doesn't always tell a refusal; the status does.
status=$(plutil -extract status raw -o - "$answer" 2>/dev/null || echo none)
if [ "$status" != Accepted ]; then
    id=$(plutil -extract id raw -o - "$answer" 2>/dev/null || true)
    [ -z "$id" ] || xcrun notarytool log "$id" "${auth[@]}" || true
    echo "notarization of $(basename "$file"): $status" >&2
    exit 1
fi
