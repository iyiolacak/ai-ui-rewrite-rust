// Extra React-side measurements for docs/RESULTS.md, in headless Chromium.
//
//   REACT_DIST=<react-baseline/dist of a build that accepts ?data=> \
//   PLAYWRIGHT=<playwright package> CHROME=<chrome-headless-shell> \
//   node react-extra.mjs MODE HISTORY REPLY_KB [DATA_JSON]
//
// MODE
//   scroll  open the thread, then 600 frames of a 400 px wheel notch per frame
//           (bottom of the thread upward, no streaming). Prints frame intervals
//           and main-thread busy time per frame.
//   typing  stream a reply (REPLY_KB prompt) and, 5 s in, type 100 characters
//           one every 150 ms into the composer. Key to next paint = event time
//           stamp of `input` to the first task after the following animation frame.
//   memory  stream a reply for 300 s and sample resident memory (all Chromium
//           processes, RSS and PSS) and the JS heap every 5 s.
// Prints one JSON line.
import { createServer } from "node:http";
import { readFile, readdir, mkdtemp, mkdir } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { extname, join } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT ?? "playwright");
const MODE = process.argv[2];
const HISTORY = Number(process.argv[3] ?? 1000);
const REPLY_KB = Number(process.argv[4] ?? 9);
const DATA_FILE = process.argv[5] || undefined;
const DIST = process.env.REACT_DIST;
const NOTCH_PX = Number(process.env.NOTCH_PX ?? 400);

const PARAGRAPH =
  "A paragraph of ordinary prose with a `code span`, some *emphasis* and a [link](https://example.com).\n\n";
const prompt = PARAGRAPH.repeat(Math.floor((REPLY_KB * 1024) / PARAGRAPH.length));
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };
const server = createServer(async (req, res) => {
  const path = new URL(req.url, "http://x").pathname;
  const file = path === "/benchmark-data.json" && DATA_FILE ? DATA_FILE : join(DIST, path === "/" ? "index.html" : path);
  try {
    res.writeHead(200, { "content-type": types[extname(file)] ?? "application/json" });
    res.end(await readFile(file));
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;
const cache = join(homedir(), ".cache/ai-ui-rewrite-rust");
await mkdir(cache, { recursive: true });
const profile = await mkdtemp(join(cache, "chromium-"));

async function processTree() {
  const parent = new Map();
  const roots = new Set();
  for (const pid of await readdir("/proc")) {
    if (!/^\d+$/.test(pid)) continue;
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      parent.set(pid, stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      if (readFileSync(`/proc/${pid}/cmdline`, "utf8").includes(profile)) roots.add(pid);
    } catch {}
  }
  const ours = (pid) => {
    for (let p = pid; p && p !== "0" && p !== "1"; p = parent.get(p)) if (roots.has(p)) return true;
    return false;
  };
  return [...parent.keys()].filter(ours);
}
async function memory() {
  let rss = 0, pss = 0;
  for (const pid of await processTree()) {
    try {
      rss += Number(/VmRSS:\s+(\d+)/.exec(readFileSync(`/proc/${pid}/status`, "utf8"))?.[1] ?? 0);
      pss += Number(/^Pss:\s+(\d+)/m.exec(readFileSync(`/proc/${pid}/smaps_rollup`, "utf8"))?.[1] ?? 0);
    } catch {}
  }
  return { rss_mib: +(rss / 1024).toFixed(1), pss_mib: +(pss / 1024).toFixed(1) };
}
async function cpuMs() {
  let ticks = 0;
  for (const pid of await processTree()) {
    try {
      const f = readFileSync(`/proc/${pid}/stat`, "utf8");
      const x = f.slice(f.lastIndexOf(")") + 2).split(" ");
      ticks += Number(x[11]) + Number(x[12]);
    } catch {}
  }
  return ticks * 10;
}

const context = await chromium.launchPersistentContext(profile, {
  headless: true,
  executablePath: process.env.CHROME || undefined,
  viewport: { width: 900, height: 760 },
});
const page = context.pages()[0] ?? (await context.newPage());
const cdp = await context.newCDPSession(page);
await cdp.send("Performance.enable");
const metrics = async () =>
  Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
const navigated = Date.now();
await page.goto(`http://127.0.0.1:${port}/` + (DATA_FILE ? "?data=/benchmark-data.json" : `?history=${HISTORY}`));
await page.waitForFunction((n) => document.querySelectorAll("[data-role]").length >= n, HISTORY, {
  timeout: 300_000,
  polling: 100,
});
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const openMs = Date.now() - navigated;
await page.waitForTimeout(2000);
const pct = (arr, p) => {
  const s = [...arr].sort((a, b) => a - b);
  return +(s[Math.floor((s.length - 1) * p)] ?? 0).toFixed(2);
};
const out = { engine: "react", mode: MODE, history: HISTORY, reply_kb: REPLY_KB, open_ms: openMs, data: DATA_FILE ?? null };
const startFrames = () =>
  page.evaluate(() => {
    window.__frames = [];
    let last = performance.now();
    const tick = (t) => {
      window.__frames.push(t - last);
      last = t;
      if (!window.__stop) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });

if (MODE === "scroll") {
  // Start at the bottom: the thread scrolls to the newest message on load.
  await page.mouse.move(450, 400);
  await startFrames();
  const before = await metrics();
  const t0 = Date.now();
  for (let i = 0; i < 600; i++) {
    await page.mouse.wheel(0, -NOTCH_PX);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
  }
  const wall = Date.now() - t0;
  const after = await metrics();
  const frames = await page.evaluate(() => { window.__stop = true; return window.__frames.slice(1); });
  const scrolled = await page.evaluate(() => {
    for (let e = document.querySelector("[data-role]"); e; e = e.parentElement) {
      if (e.scrollHeight > e.clientHeight + 10 && /auto|scroll/.test(getComputedStyle(e).overflowY))
        return { scrollTop: Math.round(e.scrollTop), scrollHeight: e.scrollHeight, from_bottom_px: Math.round(e.scrollHeight - e.clientHeight - e.scrollTop) };
    }
    return null;
  });
  Object.assign(out, {
    notches: 600, notch_px: NOTCH_PX, wall_ms: wall,
    frames: frames.length,
    frame_interval_p50_ms: pct(frames, 0.5), frame_interval_p95_ms: pct(frames, 0.95),
    frame_interval_p99_ms: pct(frames, 0.99), frame_interval_max_ms: pct(frames, 1),
    main_thread_ms: Math.round((after.TaskDuration - before.TaskDuration) * 1000),
    main_thread_ms_per_frame: +(((after.TaskDuration - before.TaskDuration) * 1000) / frames.length).toFixed(2),
    scroller_after: scrolled,
  });
} else if (MODE === "typing") {
  await page.evaluate(() => {
    window.__lat = [];
    const input = document.querySelector("[aria-label='Message input']");
    input.addEventListener("input", (e) => {
      const t0 = e.timeStamp;
      requestAnimationFrame(() => setTimeout(() => window.__lat.push(performance.now() - t0), 0));
    });
  });
  const input = page.getByLabel("Message input");
  await input.fill(prompt);
  // Keep the long draft out of the measurement: send it, then type a new draft while it streams.
  await input.press("Enter");
  await startFrames();
  const cpuBefore = await cpuMs();
  await page.waitForTimeout(5000);
  await input.click();
  for (let i = 0; i < 100; i++) {
    await page.keyboard.type("x");
    await page.waitForTimeout(150);
  }
  await page.waitForTimeout(500);
  const lat = await page.evaluate(() => window.__lat);
  const frames = await page.evaluate(() => { window.__stop = true; return window.__frames.slice(1); });
  const typed = await input.inputValue();
  Object.assign(out, {
    keys_typed: 100, keys_landed: typed.length, samples: lat.length,
    key_to_paint_p50_ms: pct(lat, 0.5), key_to_paint_p95_ms: pct(lat, 0.95), key_to_paint_max_ms: pct(lat, 1),
    key_to_paint_all_ms: lat.map((x) => +x.toFixed(1)),
    frame_interval_p95_ms: pct(frames, 0.95), frame_interval_max_ms: pct(frames, 1),
    process_cpu_ms: (await cpuMs()) - cpuBefore,
  });
} else if (MODE === "memory") {
  const input = page.getByLabel("Message input");
  await input.fill(prompt);
  const samples = [];
  const sample = async (t) => {
    const m = await memory();
    const heap = (await metrics()).JSHeapUsedSize / 2 ** 20;
    samples.push({ t_s: t, ...m, js_heap_mib: +heap.toFixed(1) });
  };
  await sample(0);
  const cpuBefore = await cpuMs();
  const start = Date.now();
  await input.press("Enter");
  for (let t = 5; t <= 300; t += 5) {
    await page.waitForTimeout(Math.max(0, start + t * 1000 - Date.now()));
    await sample(t);
  }
  const chars = await page.evaluate(
    () => [...document.querySelectorAll("[data-role=assistant]")].at(-1)?.textContent?.length ?? 0,
  );
  Object.assign(out, { window_s: 300, samples, process_cpu_ms: (await cpuMs()) - cpuBefore, last_message_chars: chars });
} else {
  throw new Error("MODE is scroll, typing or memory");
}
console.log(JSON.stringify(out));
await context.close();
server.close();
