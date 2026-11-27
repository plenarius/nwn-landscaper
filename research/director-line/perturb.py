import json, sys
inp = sys.argv[1]
v = json.load(open(f'{inp}/vale.json')); v['keys'][2]['height'] = 5.5; v['keys'][2]['pitch'] = 84; json.dump(v, open('bad-vale.json', 'w'))
d = json.load(open(f'{inp}/dunes.json')); d['keys'][1]['height'] = 4.2; d['keys'][1]['pitch'] = 86; json.dump(d, open('bad-dunes.json', 'w'))
c = json.load(open(f'{inp}/cave.json')); c['keys'][3]['height'] = 7; c['keys'][3]['pitch'] = 50; json.dump(c, open('bad-cave.json', 'w'))
