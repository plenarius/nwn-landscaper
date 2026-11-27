# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# Normal maps for the spike (throwaway): python nmgen.py IN.dds|IN.png OUT_STEM STRENGTH [SIGMAS]
# Height from the picture, band-passed at stone scale (not texel by texel, as #96 did), normals from its slope, written as
# a BioWare DXT5 DDS with X in alpha and Y in green ("DXT5nm"), plus PNG previews (RGB normal, lit from a low sun).
import sys, struct, numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter

def read_bioware_dxt1(path):
    b = open(path, 'rb').read()
    w, h, ch, size = struct.unpack('<IIII', b[:16])
    assert ch == 3, ch
    data = np.frombuffer(b[20:20 + w * h // 2], dtype=np.uint8).reshape(h // 4, w // 4, 8)
    c0 = data[..., 0].astype(np.uint16) | (data[..., 1].astype(np.uint16) << 8)
    c1 = data[..., 2].astype(np.uint16) | (data[..., 3].astype(np.uint16) << 8)
    bits = data[..., 4].astype(np.uint32) | (data[..., 5].astype(np.uint32) << 8) | (data[..., 6].astype(np.uint32) << 16) | (data[..., 7].astype(np.uint32) << 24)
    def rgb(c): return np.stack([((c >> 11) & 31) * 255 / 31, ((c >> 5) & 63) * 255 / 63, (c & 31) * 255 / 31], -1)
    p0, p1 = rgb(c0), rgb(c1)
    four = (c0 > c1)[..., None]
    p2 = np.where(four, (2 * p0 + p1) / 3, (p0 + p1) / 2)
    p3 = np.where(four, (p0 + 2 * p1) / 3, 0)
    pal = np.stack([p0, p1, p2, p3], -2)  # bh, bw, 4, 3
    out = np.zeros((h, w, 3))
    for i in range(16):
        idx = (bits >> (2 * i)) & 3
        col = np.take_along_axis(pal, idx[..., None, None].repeat(3, -1), -2)[..., 0, :]
        out[(i // 4)::4, (i % 4)::4] = col
    return out / 255.0

def load(path):
    if path.endswith('.dds'): return read_bioware_dxt1(path)
    return np.asarray(Image.open(path).convert('RGB'), dtype=np.float64) / 255.0

def gauss(a, s): return gaussian_filter(a, s, mode='wrap')

def height(rgb, sigmas):
    lum = rgb @ np.array([0.299, 0.587, 0.114])
    s = lum.shape[0]
    big = s / 8
    hp = lum - gauss(lum, big)
    hp /= np.sqrt(gauss(hp * hp, big)) + 1e-3  # even contrast: a pale stone and a dark one bulge alike
    hp = np.tanh(hp / 1.5)  # stains and glints don't spike
    # Stone-scale shape: mostly the blurred bands (domes), little of the finest (texture grain).
    ws = np.array([1.0 / (k + 1) for k in range(len(sigmas))])[::-1]
    hgt = sum(w * gauss(hp, sg) for w, sg in zip(ws, sigmas)) / ws.sum()
    return hgt

def normals(h, strength):
    dx = (np.roll(h, -1, 1) - np.roll(h, 1, 1)) / 2
    dy = (np.roll(h, -1, 0) - np.roll(h, 1, 0)) / 2
    n = np.stack([-dx * strength, -dy * strength, np.ones_like(h)], -1)
    return n / np.linalg.norm(n, axis=-1, keepdims=True)

def dxt5_alpha(a):  # a: (bh, bw, 16) 0..255
    lo, hi = a.min(-1), a.max(-1)
    pal = np.stack([hi, lo] + [((6 - k) * hi + (k + 1) * lo) / 7 for k in range(6)], -1)  # hi>lo: 8-value mode
    idx = np.abs(a[..., :, None] - pal[..., None, :]).argmin(-1).astype(np.uint64)
    bits = np.zeros(a.shape[:2], dtype=np.uint64)
    for i in range(16): bits |= idx[..., i] << np.uint64(3 * i)
    hi8, lo8 = np.round(hi).astype(np.uint8), np.round(lo).astype(np.uint8)
    same = hi8 == lo8
    bits[same] = 0
    out = np.zeros(a.shape[:2] + (8,), dtype=np.uint8)
    out[..., 0], out[..., 1] = hi8, lo8
    for k in range(6): out[..., 2 + k] = ((bits >> np.uint64(8 * k)) & np.uint64(255)).astype(np.uint8)
    return out

def dxt_green(g):  # colour block carrying green only (6 bits), 4-colour mode
    lo, hi = np.floor(g.min(-1) * 63 / 255), np.ceil(g.max(-1) * 63 / 255)
    hi = np.maximum(hi, lo + 1).clip(0, 63); lo = np.minimum(lo, hi - 1).clip(0, 63)
    c0, c1 = (hi.astype(np.uint16) << 5), (lo.astype(np.uint16) << 5)
    g0, g1 = hi * 255 / 63, lo * 255 / 63
    pal = np.stack([g0, g1, (2 * g0 + g1) / 3, (g0 + 2 * g1) / 3], -1)
    idx = np.abs(g[..., :, None] - pal[..., None, :]).argmin(-1).astype(np.uint32)
    bits = np.zeros(g.shape[:2], dtype=np.uint32)
    for i in range(16): bits |= idx[..., i] << np.uint32(2 * i)
    out = np.zeros(g.shape[:2] + (8,), dtype=np.uint8)
    out[..., 0], out[..., 1], out[..., 2], out[..., 3] = c0 & 255, c0 >> 8, c1 & 255, c1 >> 8
    for k in range(4): out[..., 4 + k] = (bits >> np.uint32(8 * k)) & 255
    return out

def blocks(ch):  # (h, w) -> (h/4, w/4, 16)
    h, w = ch.shape
    return ch.reshape(h // 4, 4, w // 4, 4).transpose(0, 2, 1, 3).reshape(h // 4, w // 4, 16)

def dxt5nm(n):
    x = (n[..., 0] * 0.5 + 0.5) * 255; y = (n[..., 1] * 0.5 + 0.5) * 255
    a, c = dxt5_alpha(blocks(x)), dxt_green(blocks(y))
    return np.concatenate([a, c], -1).tobytes()

def mips(n):
    lv = [n]
    while lv[-1].shape[0] > 1:
        m = lv[-1]; m = (m[0::2, 0::2] + m[1::2, 0::2] + m[0::2, 1::2] + m[1::2, 1::2]) / 4
        lv.append(m)  # not renormalised: shorter where the bumps average out, as the shader's z then says
    return lv

def write_dds(n, path):
    lv = mips(n)
    top = dxt5nm(lv[0])
    out = struct.pack('<IIIIf', n.shape[1], n.shape[0], 4, len(top), 1.0) + top
    for m in lv[1:]:
        if m.shape[0] < 4: m = np.pad(m, ((0, 4 - m.shape[0]), (0, 4 - m.shape[1]), (0, 0)), mode='edge')
        out += dxt5nm(m)
    open(path, 'wb').write(out)
    return len(out)

if __name__ == '__main__':
    src, stem, strength = sys.argv[1], sys.argv[2], float(sys.argv[3])
    sig = [float(v) for v in sys.argv[4].split(',')] if len(sys.argv) > 4 else [1.0, 2.5, 6.0]
    rgb = load(src)
    if sys.argv[1].endswith('.npy'): pass
    h = height(rgb, sig)
    n = normals(h, strength)
    size = write_dds(n, stem + '.dds')
    Image.fromarray(((n * 0.5 + 0.5) * 255).astype(np.uint8)).save(stem + '_n.png')
    L = np.array([-0.5, -0.5, 0.35]); L /= np.linalg.norm(L)
    lit = np.clip(n @ L, 0, 1) * 0.8 + 0.2
    Image.fromarray((np.clip(rgb * lit[..., None] / (0.2 + 0.8 * L[2]), 0, 1) * 255).astype(np.uint8)).save(stem + '_lit.png')
    Image.fromarray((rgb * 255).astype(np.uint8)).save(stem + '_rgb.png')
    print(stem, n.shape, size, 'bytes', 'slope p50/p95', np.percentile(np.hypot(n[..., 0], n[..., 1]) / n[..., 2], [50, 95]).round(2))
