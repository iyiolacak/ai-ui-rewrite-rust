#!/usr/bin/env node
// Smoke test of the real landing page's demo embed, in headless Chromium (software WebGL).
// Checks: the demo starts by itself after the page loads (no click); the first frame
// arrives; console errors; and writes a screenshot. Usage:
//   PLAYWRIGHT=/path/to/node_modules/playwright [CHROME=...] node scripts/demo-smoke.mjs [url] [shot-dir]
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';
import { join } from 'node:path';

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT ?? 'playwright');
const url = process.argv[2] ?? 'http://127.0.0.1:4321/';
const shots = process.argv[3] ?? join(process.env.HOME, '.cache/ai-ui-rewrite-rust/tmp/landing-shots');
mkdirSync(shots, { recursive: true });

const browser = await chromium.launch({
  executablePath: process.env.CHROME || undefined,
  args: ['--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'],
});
const problems = [];
const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
const wasmRequests = [];
page.on('request', (r) => { if (/\.wasm|wasm\//.test(r.url())) wasmRequests.push(r.url()); });
page.on('console', (m) => { if (m.type() === 'error') problems.push(`console: ${m.text()}`); });
page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));

await page.goto(url, { waitUntil: 'load' });
await page.waitForTimeout(500);
await page.screenshot({ path: join(shots, 'desktop-top.png'), fullPage: false });
await page.locator('#demo').scrollIntoViewIfNeeded();
await page.screenshot({ path: join(shots, 'desktop-demo-before.png'), fullPage: false });

const t0 = Date.now();
const frame = await page.waitForFunction(() => window.__aui && window.__aui.firstFrame, null, { timeout: 120000 })
  .then(() => Date.now() - t0).catch(() => -1);
await page.waitForTimeout(1500);
await page.screenshot({ path: join(shots, 'desktop-demo-running.png'), fullPage: false });

const result = {
  wasmRequests: wasmRequests.length,
  clickToFirstFrameMs: frame,
  canvas: await page.evaluate(() => {
    const c = document.getElementById('canvas');
    return c ? { cssW: c.clientWidth, cssH: c.clientHeight, w: c.width, h: c.height, dpr: window.devicePixelRatio } : null;
  }),
  problems,
};
console.log(JSON.stringify(result, null, 2));
await browser.close();
process.exit(frame > 0 && wasmRequests.length > 0 && problems.length === 0 ? 0 : 1);
