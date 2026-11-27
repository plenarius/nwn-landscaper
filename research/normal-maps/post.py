# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
# post.py HAKDIR AREA...: turn #96's relief export into the normal-map spike (throwaway).
# - ls_textures.hak: the normal pictures (nm/ln_*.dds) added; ls_tp_fs and ls_bk_fs read texture1 instead of the brightness.
# - each area hak: a material with `lsRelief` (pebbles 0.15, cobbles 0.12, rock 0.10 in #96) gets `texture1 ln_<texture>`.
import os, re, shutil, subprocess, sys, tempfile, glob
G = "/srv/burst-out/normal-maps"
hakdir, areas = sys.argv[1], sys.argv[2:]
K = float(os.environ.get("NM_K", "1.0"))  # strength on top of the picture's own
BY_RELIEF = {"0.150": "ln_gravelpebbles", "0.120": "ln_floorcobble", "0.100": "ln_rock"}

def unpack(hak):
    d = tempfile.mkdtemp(dir=f"{G}/scratch")
    subprocess.run(["nwn_erf", "-x", "-f", hak], cwd=d, check=True, stdout=subprocess.DEVNULL)
    return d

def pack(d, hak):
    os.remove(hak)
    subprocess.run(f"cd {d} && nwn_erf -e HAK -c -f {hak} * >/dev/null", shell=True, check=True)
    shutil.rmtree(d)

DECL = f"""uniform float lsRelief;
uniform lowp int texture1Bound;
uniform sampler2D texUnit1;
// Normal map, DXT5nm: x in alpha, y in green (tangent space of the projection; spike).
vec2 lsNm(vec2 uv) {{ vec4 t = texture2D(texUnit1, uv); return (vec2(t.a, t.g) * 2.0 - 1.0) * {K:.3f}; }}"""

TP = """#if LIGHTING == 1 && (FRAGMENT_LIGHTING == 1 || FRAGMENT_NORMAL == 1)
    if (texture1Bound != 0)
    {
        // Triplanar normal map: each projection's tangent-space x,y added to the surface normal's (UDN blend), blended as the colour.
        vec3 lsNw = normalize(mat3(m_view_inv) * vFragmentNormal);
        vec2 lsTx = lsNm(lsP.yz), lsTy = lsNm(lsP.xz), lsTz = lsNm(lsP.xy);
        vec3 lsAx = vec3(lsNw.x, lsNw.y + lsTx.x, lsNw.z + lsTx.y);
        vec3 lsAy = vec3(lsNw.x + lsTy.x, lsNw.y, lsNw.z + lsTy.y);
        vec3 lsAz = vec3(lsNw.x + lsTz.x, lsNw.y + lsTz.y, lsNw.z);
        vec3 lsM = normalize(lsAx * lsW.x + lsAy * lsW.y + lsAz * lsW.z);
        vFragmentNormal = lsM * mat3(m_view_inv);
    }
#endif"""

BK = """#if LIGHTING == 1 && (FRAGMENT_LIGHTING == 1 || FRAGMENT_NORMAL == 1)
    if (texture1Bound != 0)
    {
        // The tile's shared library normal at the world position (as the bake drew its colour from above), times its share (blue).
        vec3 lsNw = normalize(mat3(m_view_inv) * vFragmentNormal);
        highp vec3 lsP = (m_view_inv * vec4(vPosView, 1.0)).xyz / 4.0;
        float lsK = texture7Bound != 0 ? texture2D(texUnit7, vShadowUV).b : 1.0;
        vec2 lsT = lsNm(lsP.xy) * lsK;
        vFragmentNormal = normalize(vec3(lsNw.xy + lsT, lsNw.z)) * mat3(m_view_inv);
    }
#endif"""

def relief_block(s):
    # #96's block: from its `#if LIGHTING` line (after the colour) to the matching `#endif`.
    a = s.index("#if LIGHTING == 1 && (FRAGMENT_LIGHTING == 1 || FRAGMENT_NORMAL == 1)")
    b = s.index("#endif", a) + len("#endif")
    return a, b

def patch(s, block):
    assert "FRAGMENT_NORMAL == 1)" in s, s[:300]
    s = s.replace("uniform float lsRelief;", DECL, 1)
    s = re.sub(r"float lsHeight\(vec2 uv\).*\n", "", s, count=1)
    a, b = relief_block(s)
    return s[:a] + block + s[b:]

t = f"{hakdir}/ls_textures.hak"
d = unpack(t)
for f in glob.glob(os.environ.get("NM_DIR", f"{G}/nm") + "/ln_*.dds"):
    shutil.copy(f, d)
for name, block in (("ls_tp_fs", TP), ("ls_bk_fs", BK)):
    p = f"{d}/{name}.shd"
    text = patch(open(p).read(), block)
    open(p, "w").write(text)
pack(d, t)
for a in areas:
    h = f"{hakdir}/{a}.hak"
    d = unpack(h)
    n = 0
    for p in glob.glob(f"{d}/*.mtr"):
        s = open(p).read()
        m = re.search(r"parameter float lsRelief ([0-9.]+)", s)
        if m and m.group(1) in BY_RELIEF:
            s += f"texture1 {BY_RELIEF[m.group(1)]}\n"
            open(p, "w").write(s); n += 1
    pack(d, h)
    print(a, "materials with a normal map:", n)
