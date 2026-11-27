"""Reference poses from nwn-director's own planner and flightcheck.py, for the JS port's parity test."""
import json, math, runpy, sys
inp, director = sys.argv[1], sys.argv[2]
out = {}
for plan, area in (('vale', 'lsvale'), ('canyon', 'lscanyon'), ('dunes', 'lsdunes'), ('pass', 'lspass'), ('cave', 'lsdescent')):
    sys.argv = ['flightcheck.py', f'{inp}/{area}.json', f'{inp}/{plan}.json'] + (['--cave'] if plan == 'cave' else [])
    import os; os.environ['DIRECTOR_DIR'] = director
    import io, contextlib
    with contextlib.redirect_stdout(io.StringIO()):
        g = runpy.run_path(f'{inp}/flightcheck.py')
    shot = g['shot']
    rows = []
    t = 0.0
    while t <= shot['seconds'] + 1e-6:
        cx, cy = g['carrier'](t)
        c = g['camera_at'](t)
        r = math.radians(c['dir']); horiz = c['dist'] * math.sin(math.radians(c['pitch']))
        gz = g['h'](cx, cy)
        rows.append({'t': t, 'carrier': [cx, cy, gz], 'camera': [cx - math.cos(r) * horiz, cy - math.sin(r) * horiz,
                     gz + shot['pivot'] + c['dist'] * math.cos(math.radians(c['pitch']))], **c})
        t += 0.5
    out[plan] = {'area': area, 'shot': {k: shot[k] for k in ('name', 'at', 'seconds', 'pivot', 'legs', 'cams')},
                 'rows': rows, 'issues': g['issues'], 'minclear': g['minclear']}
json.dump(out, sys.stdout)
