# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
"""MkDocs hooks for the handbook site (mkdocs.yml).

- Links from a page to a file outside docs/ (the README, packs/) point at that file on GitHub, so the documents keep the
  relative links that work on GitHub and lychee checks, and the strict build still fails on a link that leads nowhere.
- The showcase page's `<!-- showcase -->` marker becomes a gallery of cards drawn from docs/assets/showcase/index.json (the
  editor's File, Open a showcase reads the same list).
"""

import html
import json
import os
import re
import urllib.parse

LINK = re.compile(r"\]\((?!https?:|mailto:|#)([^)\s]+)\)")
VIDEO = (".mp4", ".webm", ".mov")
SHOWCASE = "assets/showcase"


def on_page_markdown(markdown, page, config, files):
    docs = os.path.abspath(config["docs_dir"])
    root = os.path.dirname(docs)
    blob = config["repo_url"].rstrip("/") + "/blob/master/"
    here = os.path.dirname(os.path.join(docs, page.file.src_path))

    def outside(m):
        target, _, anchor = m.group(1).partition("#")
        path = os.path.normpath(os.path.join(here, target))
        if path.startswith(docs + os.sep) or not path.startswith(root + os.sep):
            return m.group(0)
        url = blob + os.path.relpath(path, root).replace(os.sep, "/") + (f"#{anchor}" if anchor else "")
        return f"]({url})"

    markdown = LINK.sub(outside, markdown)
    if "<!-- showcase -->" in markdown:
        with open(os.path.join(docs, SHOWCASE, "index.json"), encoding="utf-8") as f:
            entries = [site_paths(e) for e in json.load(f)]
        markdown = markdown.replace("<!-- showcase -->", gallery(entries, "../" * page.url.count("/")))
    return markdown


def site_paths(e):
    """Entry `e` of the showcase's index.json with its paths (relative to that folder) as paths from the site's root."""
    return {k: f"{SHOWCASE}/{v}" if k in ("still", "clip", "area") and v and not re.match(r"[a-z]+:", v) else v for k, v in e.items()}


def gallery(entries, up):
    """One card per entry: the clip (a video file plays in place; any other link opens), its still, a title and a line.
    `up` leads from the page's address back to the site's root, where the stills' paths start."""
    cards = []
    for e in entries:
        title = html.escape(e["title"])
        still = html.escape(up + e["still"]) if e.get("still") else ""
        clip = e.get("clip", "")
        if clip and not re.match(r"[a-z]+:", clip):
            clip = up + clip
        text = html.escape(e.get("text", ""))
        if clip.lower().endswith(VIDEO):
            media = (
                f'<video controls preload="none" playsinline poster="{still}">'
                f'<source src="{html.escape(clip)}">{title}</video>'
            )
        elif still:
            media = f'<img src="{still}" alt="{title}" loading="lazy">'
            if clip:
                media = f'<a class="no-lightbox" href="{html.escape(clip)}">{media}</a>'
        else:
            media = '<div class="showcase-empty">Clip to come</div>'
        caption = f"<figcaption><strong>{title}</strong><br>{text}{actions(e, up)}</figcaption>"
        cards.append(f'<figure class="showcase-card">{media}{caption}</figure>')
    return '\n<div class="showcase">\n' + "\n".join(cards) + "\n</div>\n"


def actions(e, up):
    """A card's links for its `area`: open it in /try/ (hidden until /try/ says it opens a linked area: docs/javascripts/
    showcase.js) and download it. /try/ reads `area` from its own address, so the path starts one folder up from it."""
    area = e.get("area")
    if not area:
        return ""
    name = html.escape(os.path.basename(area))
    open_url = html.escape(f"{up}try/?area={urllib.parse.quote('../' + area)}")
    return (
        '<span class="showcase-actions">'
        f'<a class="showcase-open" href="{open_url}" hidden>Open in Landscaper</a>'
        f'<a href="{html.escape(up + area)}" download="{name}">Download</a>'
        "</span>"
    )
