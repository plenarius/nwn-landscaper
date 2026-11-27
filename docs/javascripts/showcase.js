// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

// The showcase's "Open in Landscaper" links (scripts/mkdocs_hooks.py) show once /try/ opens an area from its link: its
// try/site.json says so with `areaLinks`, from the editor release that added it.
const open = document.querySelectorAll('.showcase-open');
if (open.length) {
    fetch(new URL('try/site.json', open[0].href), { cache: 'no-cache' })
        .then(res => (res.ok ? res.json() : {}))
        .then(site => {
            if (site.areaLinks === true) for (const a of open) a.hidden = false;
        })
        .catch(() => {});
}
