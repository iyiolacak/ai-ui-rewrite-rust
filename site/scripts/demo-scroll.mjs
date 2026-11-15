#!/usr/bin/env node
// Scroll behaviour of the landing demo, in headless Chromium (software WebGL).
// 1. Dock the demo; the intro must end at the very bottom (firstVisible >= messages - 15).
// 2. Wheel up over the canvas: the position must change and then hold (no pull-back).
// 3. Wheel down hard: the end is reached again.
// 3b. The counter (#demo-ordinal) shows on a wheel and during a scrollbar drag; it stays hidden only during the intro.
// 4. Fresh page: a wheel during the intro stops it and the position holds.
// Usage: PLAYWRIGHT=/path/to/node_modules/playwright [CHROME=...] node scripts/demo-scroll.mjs [url] [shot-dir]
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
const log = [];
const fail = (m) => { problems.push(m); console.log('FAIL', m); };
const check = (ok, m) => { console.log(ok ? 'ok  ' : 'FAIL', m); if (!ok) problems.push(m); };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function open() {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1 });
  page.on('console', (m) => { if (m.type() === 'error') fail(`console: ${m.text()}`); });
  page.on('pageerror', (e) => fail(`pageerror: ${e.message}`));
  await page.goto(url, { waitUntil: 'load' });
  await page.waitForFunction(() => window.__aui && window.__aui.firstFrame && window.__aui.messages > 100, null, { timeout: 120000 });
  return page;
}
const aui = (page) => page.evaluate(() => ({ ...window.__aui }));
const counter = (page) => page.evaluate(() => ({
  visible: document.getElementById('demo-ordinal').classList.contains('is-visible'),
  value: Number(document.getElementById('demo-ordinal-value').textContent.replace(/,/g, '')),
}));
const first = async (page) => (await aui(page)).firstVisible;
const canvasCenter = async (page) => page.evaluate(() => {
  const r = document.getElementById('canvas').getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
});
// Scroll the page until the demo is docked (frame has is-docked), in steps like a user.
async function dock(page) {
  for (let i = 0; i < 80; i++) {
    if (await page.evaluate(() => document.body.classList.contains('demo-docked'))) return true;
    await page.mouse.move(5, 450);
    await page.mouse.wheel(0, 120);
    await sleep(60);
  }
  return false;
}

// ---- Run 1: intro to the bottom, wheel up, hold, wheel down.
{
  const page = await open();
  check(await dock(page), 'demo docked');
  const intro = Date.now();
  await page.waitForFunction(() => window.__aui.settled === 1, null, { timeout: 60000 }).catch(() => {});
  console.log('settled after', Date.now() - intro, 'ms');
  await sleep(1500);
  let a = await aui(page);
  console.log('at settle', JSON.stringify({ first: a.firstVisible, messages: a.messages, settled: a.settled }));
  check(a.settled === 1, 'settled');
  check(a.firstVisible >= a.messages - 15, `intro reaches the end (${a.firstVisible} of ${a.messages})`);
  await page.screenshot({ path: join(shots, 'scroll-1-bottom.png') });

  const c = await canvasCenter(page);
  await page.mouse.move(c.x, c.y);
  await sleep(2500); // let the counter fade out after the intro
  const before = await first(page);
  for (let i = 0; i < 6; i++) { await page.mouse.wheel(0, -400); await sleep(80); }
  const wc = await counter(page);
  check(wc.visible, `counter visible during wheel (${wc.value})`);
  await sleep(1500);
  const up1 = await first(page);
  check(up1 < before, `wheel up moved (${before} -> ${up1})`);
  await page.screenshot({ path: join(shots, 'scroll-2-up.png') });
  await sleep(2000);
  const up2 = await first(page);
  check(up2 === up1, `no pull-back (${up1} -> ${up2})`);
  // keep going up, still holds
  for (let i = 0; i < 6; i++) { await page.mouse.wheel(0, -600); await sleep(80); }
  await sleep(1500);
  const up3 = await first(page);
  await sleep(2000);
  const up4 = await first(page);
  check(up3 < up2 && up4 === up3, `second wheel up holds (${up2} -> ${up3} -> ${up4})`);

  for (let i = 0; i < 80; i++) {
    await page.mouse.wheel(0, 4000);
    await sleep(60);
    const f = await first(page);
    const m = (await aui(page)).messages;
    if (f >= m - 15) break;
  }
  await sleep(1500);
  a = await aui(page);
  check(a.firstVisible >= a.messages - 15, `wheel down returns to the end (${a.firstVisible} of ${a.messages})`);
  await page.screenshot({ path: join(shots, 'scroll-3-down.png') });

  // Scrollbar drag: press on the track at the right edge of the canvas, drag up in steps.
  await sleep(2500);
  const r = await page.evaluate(() => { const b = document.getElementById('canvas').getBoundingClientRect(); return { x: b.right - 7, top: b.top, h: b.height }; });
  const startIdx = await first(page);
  await page.mouse.move(r.x, r.top + r.h * 0.6);
  await page.mouse.down();
  let during = null;
  let seen = false;
  for (let i = 0; i < 20; i++) {
    await page.mouse.move(r.x, r.top + r.h * (0.6 - i * 0.025), { steps: 3 });
    await sleep(80);
    const k = await counter(page);
    seen = seen || k.visible;
    if (i === 10) { during = k; await page.screenshot({ path: join(shots, 'scroll-5-drag-counter.png') }); }
  }
  const dragIdx = await first(page);
  check(seen && during?.visible, `counter visible during scrollbar drag (${during?.value})`);
  check(startIdx - dragIdx > 100, `scrollbar drag moved by hundreds (${startIdx} -> ${dragIdx})`);
  await sleep(2500);
  check((await counter(page)).visible, 'counter stays while the pointer is held');
  await page.mouse.up();
  await sleep(2800);
  check(!(await counter(page)).visible, 'counter fades after release');
  await page.close();
}

// ---- Run 2: wheel during the intro.
{
  const page = await open();
  // Dock quickly, then wheel up as soon as the intro starts.
  let docked = false;
  for (let i = 0; i < 80 && !docked; i++) {
    await page.mouse.move(5, 450);
    await page.mouse.wheel(0, 120);
    await sleep(60);
    docked = await page.evaluate(() => document.body.classList.contains('demo-docked'));
  }
  check(docked, 'demo docked (run 2)');
  const c = await canvasCenter(page);
  await page.mouse.move(c.x, c.y);
  await sleep(250);
  for (let i = 0; i < 4; i++) { await page.mouse.wheel(0, -400); await sleep(60); }
  await sleep(1500);
  const p1 = await first(page);
  const m = (await aui(page)).messages;
  await sleep(2500);
  const p2 = await first(page);
  console.log('intro-wheel', JSON.stringify({ p1, p2, messages: m }));
  check(p1 === p2, `position holds after wheel during intro (${p1} -> ${p2})`);
  const k1 = await counter(page);
  await page.mouse.wheel(0, -400);
  await sleep(150);
  const k2 = await counter(page);
  check(k2.visible, `counter shows on the next wheel after a cancelled intro (${k2.value})`);
  check(p2 < m - 15, `intro did not carry on to the end (${p2} of ${m})`);
  await page.screenshot({ path: join(shots, 'scroll-4-intro-cancel.png') });
  await page.close();
}

await browser.close();
console.log(problems.length ? `FAILED (${problems.length})` : 'PASSED');
process.exit(problems.length ? 1 : 0);
