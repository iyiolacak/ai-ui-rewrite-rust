#!/usr/bin/env python3
"""Write the "web" section of data/benchmarks.json from measure-web.mjs output.
   usage: node scripts/measure-web.mjs 3 10000 > web.json; python3 scripts/fill-web.py web.json"""
import json, statistics, sys, os
m = json.load(open(sys.argv[1]))
path = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'data', 'benchmarks.json')
med = lambda k, key: statistics.median(r[key] for r in m['runs'][k])
s, n = m['sizes'], m['history']
rows = [
 ('wasm-raw', 'WebAssembly file, raw', round(s['raw'] / 1048576, 2), 'MiB', f"{s['raw']} bytes, release build, not processed with wasm-opt (not installed)"),
 ('wasm-gzip', 'WebAssembly file, gzip -9', round(s['gzip'] / 1048576, 2), 'MiB', f"{s['gzip']} bytes"),
 ('wasm-br', 'WebAssembly file, brotli', round(s['brotli'] / 1048576, 2), 'MiB', f"{s['brotli']} bytes"),
]
for k, label in [('empty', 'empty thread'), ('history', f'{n}-message history')]:
    rows += [
     (f'{k}-paint', f'First paint, {label}', round(med(k, 'firstPaintMs')), 'ms', 'from starting the module load to the first frame drawn (the history is built before it)'),
     (f'{k}-mem', f'WebAssembly memory, {label}', round(med(k, 'wasmMemoryMiB'), 1), 'MiB', 'linear memory size (it only grows), after 3 s and 10 s of streaming'),
     (f'{k}-fps', f'Frames per second while streaming, {label}', round(med(k, 'fps'), 1), 'fps', 'frames drawn during 10 s of a streamed reply (18 ms per word)'),
    ]
d = json.load(open(path))
d['web'] = {
 'conditions': 'Headless Chromium 1248 (Playwright 1.64.1), software WebGL (SwiftShader, no GPU), viewport 1000x700, one shared 12-core Linux machine under load; median of 3 fresh runs per row; values rounded to the shown precision. The GPU is emulated on a busy CPU, so frame rates are a floor, not what a real browser GPU gives. The demo model replies one word every 18 ms.',
 'source': 'site/scripts/measure-web.mjs, 2026-11-10',
 'rows': [dict(id=a, label=b, value=c, unit=u, note=nt) for a, b, c, u, nt in rows]}
json.dump(d, open(path, 'w'), indent=2)
