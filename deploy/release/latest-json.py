#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Writes latest.json, which the desktop app's updater reads on the newest published release
# (https://github.com/plenarius/nwn-landscaper/releases/latest/download/latest.json, app/tauri.conf.json), from the update files in
# dist/ (.github/workflows/release.yml). Each system's file and its signature (the .sig Tauri wrote beside it):
#
#   Windows  nwn-landscaper-X.Y.Z-windows-x86_64-setup.exe        the NSIS installer, run again over the installed app
#   macOS    nwn-landscaper-X.Y.Z-macos-universal.app.tar.gz      the app, replaced whole (both Mac architectures)
#   Linux    nwn-landscaper-X.Y.Z-linux-x86_64.AppImage           the AppImage, replaced in place
#
# A .deb has none: its app sends the person to the release page. Each signature is checked first against its public key (the
# keys folder holds one per system, KEY_DIR/<platform>.pub, written by deploy/release/updater-key.sh), with minisign: a release
# signed with a key the app doesn't trust would never update anyone.
#
# Usage: deploy/release/latest-json.py VERSION DIST_DIR KEY_DIR NOTES_FILE > latest.json
import base64
import datetime
import json
import os
import subprocess
import sys
import tempfile

REPO = "https://github.com/plenarius/nwn-landscaper"

version, dist, keys, notes_file = sys.argv[1:5]
# <platform>: (file, the updater's names for it: `{os}-{arch}-{installer}`, then `{os}-{arch}`)
FILES = {
    "windows-x86_64": (f"nwn-landscaper-{version}-windows-x86_64-setup.exe", ["windows-x86_64-nsis", "windows-x86_64"]),
    "macos-universal": (
        f"nwn-landscaper-{version}-macos-universal.app.tar.gz",
        ["darwin-aarch64-app", "darwin-x86_64-app", "darwin-aarch64", "darwin-x86_64"],
    ),
    "linux-x86_64": (f"nwn-landscaper-{version}-linux-x86_64.AppImage", ["linux-x86_64-appimage", "linux-x86_64"]),
}


def checked(path, platform):
    """The signature of `path` (its .sig, as Tauri writes it), once minisign finds it good for the platform's public key."""
    with open(path + ".sig", encoding="ascii") as f:
        sig = f.read().strip()
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "s.minisig"), "wb") as f:
            f.write(base64.b64decode(sig))
        with open(os.path.join(keys, platform + ".pub"), encoding="ascii") as k:
            with open(os.path.join(tmp, "k.pub"), "wb") as f:
                f.write(base64.b64decode(k.read().strip()))
        out = subprocess.run(
            ["minisign", "-V", "-p", os.path.join(tmp, "k.pub"), "-m", path, "-x", os.path.join(tmp, "s.minisig")],
            capture_output=True,
            text=True,
        )
    if out.returncode != 0:
        sys.exit(f"latest-json: {os.path.basename(path)}'s signature doesn't check: {out.stderr.strip() or out.stdout.strip()}")
    # (The trusted comment carries the file's name and, from Tauri's command line, the version the app requires.)
    print(f"latest-json: {os.path.basename(path)}: {out.stdout.strip().splitlines()[-1]}", file=sys.stderr)
    return sig


platforms = {}
for platform, (name, targets) in FILES.items():
    path = os.path.join(dist, name)
    if not os.path.isfile(path) or not os.path.isfile(path + ".sig"):
        sys.exit(f"latest-json: no {name} with its .sig in {dist}")
    entry = {"signature": checked(path, platform), "url": f"{REPO}/releases/download/v{version}/{name}"}
    for t in targets:
        platforms[t] = entry

with open(notes_file, encoding="utf-8") as f:
    notes = f.read().strip()
now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)
json.dump(
    {"version": version, "notes": notes, "pub_date": now.isoformat().replace("+00:00", "Z"), "platforms": platforms},
    sys.stdout,
    indent=2,
)
print()
