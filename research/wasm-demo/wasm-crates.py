# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
"""Code size of a wasm file by Rust crate (from the name section), plus data and other sections."""
import sys, re, collections
b = open(sys.argv[1], 'rb').read()
def leb(i):
    r = s = 0
    while True:
        x = b[i]; i += 1; r |= (x & 0x7f) << s; s += 7
        if x < 0x80: return r, i
i = 8; imports = 0; code = []; names = {}; sections = collections.Counter()
while i < len(b):
    sid = b[i]; size, j = leb(i + 1); end = j + size
    if sid == 2:
        n, k = leb(j)
        for _ in range(n):
            for _ in range(2):
                l, k = leb(k); k += l
            kind = b[k]; k += 1
            if kind == 0: _, k = leb(k); imports += 1
            elif kind == 1: k += 1; f, k = leb(k); _, k = leb(k); k = k if not (f & 1) else leb(k)[1]
            elif kind == 2: f, k = leb(k); _, k = leb(k); k = k if not (f & 1) else leb(k)[1]
            elif kind == 3: k += 2
            else: k += 2
    if sid == 10:
        n, k = leb(j)
        for _ in range(n):
            s, k2 = leb(k); code.append(s + (k2 - k)); k = k2 + s
    elif sid == 0:
        l, k = leb(j); nm = b[k:k + l].decode(); k += l
        if nm == 'name':
            while k < end:
                sub = b[k]; ss, k = leb(k + 1); se = k + ss
                if sub == 1:
                    n, k = leb(k)
                    for _ in range(n):
                        idx, k = leb(k); l, k = leb(k); names[idx] = b[k:k + l].decode('utf8', 'replace'); k += l
                k = se
        sections['custom:' + nm] += size
    if sid != 10: sections[{1:'type',2:'import',3:'function',5:'memory',6:'global',7:'export',9:'elem',11:'data',12:'datacount'}.get(sid, f'id{sid}')] += size if sid else 0
    i = end
crates = collections.Counter()
for n, s in enumerate(code):
    nm = names.get(n + imports, '?')
    m = re.match(r'_R[a-zA-Z]+?(?:Cs[0-9A-Za-z_]+_)?(\d+)([A-Za-z_][A-Za-z0-9_]*)', nm)
    crate = None
    if nm.startswith('_R'):
        for mm in re.finditer(r'Cs[0-9A-Za-z_]*?_(\d+)', nm):
            l = int(mm.group(1)); crate = nm[mm.end():mm.end() + l]; break
    elif nm.startswith('_ZN') and re.match(r'_ZN(\d+)', nm) and not re.match(r'_ZN\d+(glslang|spv|std|CScript|CExo|C[A-Z])', nm):
        mm = re.match(r'_ZN(\d+)', nm); l = int(mm.group(1)); crate = nm[mm.end():mm.end() + l]
    else:
        crate = 'C/C++ (glslang, nwscript, zstd, libc)'
    crates[re.sub(r'^(\d+)', '', crate or '?')] += s
total = sum(code)
print(f'file {len(b)/1e6:.2f} MB; code {total/1e6:.2f} MB in {len(code)} functions')
for k, v in sections.most_common():
    if v > 50000: print(f'  section {k}: {v/1e6:.2f} MB')
for k, v in crates.most_common(22): print(f'  {k}: {v/1e6:.2f} MB')
