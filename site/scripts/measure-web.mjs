#!/usr/bin/env node
// Measures the WebAssembly demo in headless Chromium (software WebGL, SwiftShader). Prints JSON.
//   PLAYWRIGHT=... node scripts/measure-web.mjs [runs=3] [history=10000]
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { gzipSync, brotliCompressSync } from 'node:zlib';
import { dirname, join, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT ?? 'playwright');
const here = dirname(fileURLToPath(import.meta.url));
const publicDir = join(here, '..', 'public');
const runs = Number(process.argv[2] ?? 3);
const history = Number(process.argv[3] ?? 10000);

const wasm = readFileSync(join(publicDir, 'wasm/ai_ui_example_thread_bg.wasm'));
const sizes = { raw: wasm.length, gzip: gzipSync(wasm, { level: 9 }).length, brotli: brotliCompressSync(wasm).length };
const types = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' };
const server = createServer((req, res) => {
  const path = req.url.split('?')[0];
  const p = path === '/' ? join(here, 'harness.html') : join(publicDir, decodeURIComponent(path));
  if (!existsSync(p)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { 'content-type': types[extname(p)] ?? 'application/octet-stream' }).end(readFileSync(p));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const url = `http://127.0.0.1:${server.address().port}/`;
const browser = await chromium.launch({ executablePath: process.env.CHROME || undefined,
  args: ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] });

async function one(n) {
  const ctx = await browser.newContext({ viewport: { width: 1000, height: 700 } });
  const page = await ctx.newPage();
  const errors = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', (e) => errors.push(String(e)));
  await page.goto(url);
  await page.evaluate((n) => {
    window.__AUI_ARGS = n ? `history=${n}` : '';
    window.__t0 = performance.now();
    import('./wasm/ai_ui_example_thread.js').then((m) => m.default()).catch(() => {});
  }, n);
  await page.waitForFunction(() => window.__aui && window.__aui.firstFrame, null, { timeout: 120000 });
  await page.waitForTimeout(3000);
  const first = await page.evaluate(() => window.__aui.firstFrame - window.__t0);
  await page.click('#canvas');
  await page.keyboard.type('Explain streaming', { delay: 10 });
  const f0 = await page.evaluate(() => window.__aui.frames);
  const s0 = Date.now();
  await page.keyboard.press('Enter');
  await page.waitForTimeout(10000);
  const frames = (await page.evaluate(() => window.__aui.frames)) - f0;
  const secs = (Date.now() - s0) / 1000;
  const mem = await page.evaluate(async () => (await (await import('./wasm/ai_ui_example_thread.js')).default()).memory.buffer.byteLength);
  await ctx.close();
  return { firstPaintMs: first, wasmMemoryMiB: mem / 1048576, fps: frames / secs, errors: errors.slice(0, 3) };
}
const out = { sizes, history, runs: { empty: [], history: [] } };
for (let i = 0; i < runs; i++) { out.runs.empty.push(await one(0)); out.runs.history.push(await one(history)); }
await browser.close(); server.close();
console.log(JSON.stringify(out, null, 1));
