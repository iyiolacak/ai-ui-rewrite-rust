#!/usr/bin/env node
// Checks that every number on the built page comes from site/data/benchmarks.json.
//   npm run build && PLAYWRIGHT=/path/to/node_modules/playwright [CHROME=...] node scripts/check-numbers.mjs [dist-dir]
// 1. every element marked data-bench shows exactly the value derived from the JSON (derived here, independently of the page code);
// 2. every numeric token in the visible text and chart labels occurs in the JSON text or is a derived ratio.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { dirname, join, extname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT ?? 'playwright');
const here = dirname(fileURLToPath(import.meta.url));
const dist = resolve(process.argv[2] ?? join(here, '..', 'dist'));
const raw = readFileSync(join(here, '..', 'data/benchmarks.json'), 'utf8');
const data = JSON.parse(raw);

const ratio = (c) => (Math.floor((c.react / c.rust) * 10 + 1e-9) / 10).toFixed(1);
const expected = new Map();
for (const c of data.comparisons) {
  expected.set(`${c.id}.rust`, String(c.rust));
  expected.set(`${c.id}.react`, String(c.react));
  expected.set(`${c.id}.ratio`, ratio(c));
  expected.set(`${c.id}.ratio-whole`, String(Math.round(c.react / c.rust)));
}
for (const r of data.web?.rows ?? []) expected.set(`web.${r.id}`, String(r.value));
const required = [
  ...['open-1000', 'memory-1000', 'cpu-1000'].flatMap((id) => ['rust', 'react', 'ratio'].map((k) => `${id}.${k}`)),
  ...['open-1000', 'memory-1000', 'stall-1000'].map((id) => `${id}.ratio-whole`),
];

const allowed = new Set(raw.replace(/(\d),(\d{3})/g, '$1$2').match(/\d+(?:\.\d+)?/g) ?? []);
for (const v of expected.values()) for (const t of v.match(/\d+(?:\.\d+)?/g) ?? []) allowed.add(t);

const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm' };
const server = createServer((req, res) => {
  const p = join(dist, decodeURIComponent(req.url.split('?')[0]).replace(/\/$/, '/index.html'));
  if (!p.startsWith(dist) || !existsSync(p)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(p)] ?? 'application/octet-stream' }).end(readFileSync(p));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const browser = await chromium.launch({ executablePath: process.env.CHROME || undefined });
const page = await browser.newPage();
await page.goto(`http://127.0.0.1:${server.address().port}/`);
const found = await page.$$eval('[data-bench]', (els) => els.map((e) => [e.dataset.bench, e.textContent.trim()]));
// The footer is licence and trademark text (it names "CC BY 4.0"), not a benchmark claim, so it is not scanned.
const text = await page.evaluate(() => ['header', 'main'].map((t) => document.querySelector(t)?.innerText ?? '').join('\n'));
const labels = await page.$$eval('[aria-label]', (els) => els.map((e) => e.getAttribute('aria-label')).join('\n'));
await browser.close(); server.close();

let bad = 0;
const fail = (m) => { console.error('FAIL', m); bad++; };
const seen = new Set();
for (const [key, shown] of found) {
  seen.add(key);
  if (!expected.has(key)) fail(`${key}: no such value in the JSON`);
  else if (expected.get(key) !== shown) fail(`${key}: page shows "${shown}", JSON gives "${expected.get(key)}"`);
}
for (const key of required) if (!seen.has(key)) fail(`${key}: required but not shown`);
for (const tok of new Set(`${text}\n${labels}`.replace(/(\d),(\d{3})/g, '$1$2').match(/\d+(?:\.\d+)?/g) ?? [])) {
  if (!allowed.has(tok)) fail(`number ${tok} on the page is not in the JSON`);
}
console.log(`${found.length} marked values, ${bad} problems`);
process.exit(bad ? 1 : 0);
