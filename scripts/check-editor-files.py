#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Checks a packed copy of the editor (.github/workflows/release.yml): every file index.html names and every relative import in
# its scripts must be in the package. One left out stops the editor while it starts (0.7.0 to 0.7.2 shipped without try/).
#
# Usage: scripts/check-editor-files.py WEB_DIR
import os
import re
import sys

root = sys.argv[1]
missing = []
imports = re.compile(r"""(?:\bfrom\s*|\bimport\s*\(\s*)['"](\.{1,2}/[^'"]+)['"]""")
page_refs = re.compile(r"""\b(?:src|href)="(?![a-z]+:|#|/)([^"?#]+)""")


def need(base, ref, by):
    if not os.path.isfile(os.path.normpath(os.path.join(base, ref))):
        missing.append(f"{by}: {ref}")


with open(os.path.join(root, "index.html"), encoding="utf-8") as f:
    for ref in page_refs.findall(f.read()):
        need(root, ref, "index.html")
for d, _, files in os.walk(root):
    for name in files:
        if name.endswith(".js"):
            path = os.path.join(d, name)
            with open(path, encoding="utf-8") as f:
                for ref in imports.findall(f.read()):
                    need(d, ref, os.path.relpath(path, root))
for m in missing:
    print(f"not in the package: {m}")
sys.exit(1 if missing else 0)
