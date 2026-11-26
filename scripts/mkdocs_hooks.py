# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
"""MkDocs hooks for the handbook site (mkdocs.yml).

- Links from a page to a file outside docs/ (the README, packs/) point at that file on GitHub, so the documents keep the
  relative links that work on GitHub and lychee checks, and the strict build still fails on a link that leads nowhere.
- The showcase page's `<!-- showcase -->` marker becomes a gallery of cards drawn from docs/showcase.yml.
"""

import html
import os
import re

import yaml

LINK = re.compile(r"\]\((?!https?:|mailto:|#)([^)\s]+)\)")
VIDEO = (".mp4", ".webm", ".mov")


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
        with open(os.path.join(docs, "showcase.yml"), encoding="utf-8") as f:
            entries = yaml.safe_load(f) or []
        markdown = markdown.replace("<!-- showcase -->", gallery(entries, "../" * page.url.count("/")))
    return markdown


def gallery(entries, up):
    """One card per entry: the clip (a video file plays in place; any other link opens), its still, a title and a line.
    `up` leads from the page's address back to the site's root, where the stills' paths start."""
    cards = []
    for e in entries:
        title = html.escape(e["title"])
        still = html.escape(up + e["still"]) if e.get("still") else ""
        clip = e.get("clip", "")
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
        cards.append(f'<figure class="showcase-card">{media}<figcaption><strong>{title}</strong><br>{text}</figcaption></figure>')
    return '\n<div class="showcase">\n' + "\n".join(cards) + "\n</div>\n"
