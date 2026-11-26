# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later

"""Writes the pack's placeholder textures beside this file: simple shapes for an artist to replace.

    python3 placeholders.py

Each is a 32-bit uncompressed TGA with the shape in its alpha, and in its colour for the additive ones, so the emitter's colours
tint it. The light shaft's TXI makes its quads additive. Seeded, so a run writes the same files.
"""

import math
import os
import random
import struct

HERE = os.path.dirname(os.path.abspath(__file__))


def write_tga(name, w, h, px):
    """px(x, y) -> (r, g, b, a) in 0..1, y up from the bottom row (TGA's default order)."""
    head = struct.pack("<BBBHHBHHHHBB", 0, 0, 2, 0, 0, 0, 0, 0, w, h, 32, 8)
    body = bytearray()
    for y in range(h):
        for x in range(w):
            r, g, b, a = (max(0, min(255, round(c * 255))) for c in px(x, y))
            body += bytes((b, g, r, a))
    with open(os.path.join(HERE, name + ".tga"), "wb") as f:
        f.write(head + body)


def soft(d, r):
    """1 at the centre, 0 at r and beyond, smooth."""
    t = max(0.0, 1.0 - d / r)
    return t * t * (3 - 2 * t)


def frames(size, cell):
    """A 2 by 2 sheet: cell(frame, u, v) with u, v from -1 to 1 in each frame, frames 0 and 1 in the top row."""

    def px(x, y):
        half = size // 2
        col, row = x // half, 1 - y // half
        u = (x % half + 0.5) / half * 2 - 1
        v = (y % half + 0.5) / half * 2 - 1
        return cell(row * 2 + col, u, v)

    return px


# Firefly glow: a bright core in a soft halo, four frames of brightness so a fly flickers as its frames run.
LEVELS = (1.0, 0.65, 0.3, 0.65)


def firefly(i, u, v):
    d = math.hypot(u, v)
    g = LEVELS[i] * (0.75 * soft(d, 0.25) + 0.35 * soft(d, 1.0))
    return (g, g, g, g)


# Soft motes: each frame a few tiny soft specks, so one particle reads as several motes.
rng = random.Random(7)
SPECKS = [[(rng.uniform(-0.7, 0.7), rng.uniform(-0.7, 0.7), rng.uniform(0.1, 0.18), rng.uniform(0.7, 1.0)) for _ in range(3 + i % 2)]
          for i in range(4)]


def motes(i, u, v):
    g = min(1.0, sum(b * soft(math.hypot(u - x, v - y), r) for x, y, r, b in SPECKS[i]))
    return (g, g, g, g)


# Sand streak: a thin horizontal wisp with grain, fading at both ends.
grain = random.Random(11)
NOISE = [[grain.random() for _ in range(64)] for _ in range(64)]


def sand(x, y):
    u, v = (x + 0.5) / 32 - 1, (y + 0.5) / 32 - 1
    band = soft(abs(v), 0.35) * soft(abs(u), 1.0)
    a = band * (0.55 + 0.45 * NOISE[y][x])
    return (1.0, 1.0, 1.0, a)


# Light shaft: brightest at the foot, fading upwards and to both edges. Drawn additively (its TXI), so the colour carries the
# shape; an alpha-blended quad drew in the client as a flat, even band.
def shaft(x, y):
    u, v = (x + 0.5) / 16 - 1, (y + 0.5) / 128
    a = 0.45 * soft(abs(u), 1.0) * (1 - v) ** 1.5 * min(1.0, v * 12)
    return (a, 0.96 * a, 0.85 * a, a)


if __name__ == "__main__":
    write_tga("lspt_firefly", 64, 64, frames(64, firefly))
    write_tga("lspt_motes", 64, 64, frames(64, motes))
    write_tga("lspt_sandstrk", 64, 64, sand)
    write_tga("lspt_shaft", 32, 128, shaft)
    with open(os.path.join(HERE, "lspt_shaft.txi"), "w") as f:
        f.write("blending additive\n")
