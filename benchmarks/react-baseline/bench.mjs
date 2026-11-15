// Measure upstream assistant-ui (React) on the workload of
// `ai-ui-example-thread --bench N --bench-reply-kb K` in headless Chromium.
//
//   npm ci --ignore-scripts && npm run build
//   PLAYWRIGHT=/path/to/node_modules/playwright node bench.mjs [history] [reply-kb] [window-s]
//
// Prints one JSON line: time to open the thread (all N messages in the DOM: open_ms;
// and with every Markdown table rendered and painted: open_formatted_ms),
// main-thread busy time while the reply streams for `window-s` seconds (the
// Rust bench's fixed 60 s window), frame intervals, and memory (JS heap; RSS
// and PSS summed over every Chromium process of this run).
import { createServer } from "node:http";
import { readFile, readdir, mkdtemp } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { extname, join } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT ?? "playwright");

const HISTORY = Number(process.argv[2] ?? 1000);
const REPLY_KB = Number(process.argv[3] ?? 9);
const WINDOW_S = Number(process.argv[4] ?? 60);
const DIST = new URL("./dist/", import.meta.url).pathname;

// bench.rs long_prompt()
const PARAGRAPH =
  "A paragraph of ordinary prose with a `code span`, some *emphasis* and a [link](https://example.com).\n\n";
const prompt =
  REPLY_KB === 0
    ? "Explain streaming"
    : PARAGRAPH.repeat(Math.floor((REPLY_KB * 1024) / PARAGRAPH.length));

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };
const server = createServer(async (req, res) => {
  const path = new URL(req.url, "http://x").pathname;
  const file = join(DIST, path === "/" ? "index.html" : path);
  try {
    const body = await readFile(file);
    res.writeHead(200, { "content-type": types[extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const port = server.address().port;

const profile = await mkdtemp(join(tmpdir(), "aui-react-bench-"));

/**
 * This run's browser process and all its descendants (renderer, GPU and
 * utility processes do not name the profile on their command line, so they
 * are found through their parents).
 */
async function processTree() {
  const parent = new Map();
  const roots = new Set();
  for (const pid of await readdir("/proc")) {
    if (!/^\d+$/.test(pid)) continue;
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      parent.set(pid, stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
      if (readFileSync(`/proc/${pid}/cmdline`, "utf8").includes(profile)) roots.add(pid);
    } catch {
      // The process exited while we read it.
    }
  }
  const ours = (pid) => {
    for (let p = pid; p && p !== "0" && p !== "1"; p = parent.get(p)) if (roots.has(p)) return true;
    return false;
  };
  return [...parent.keys()].filter(ours);
}

/** Sum VmRSS and Pss (MiB) over the process tree. */
async function memory() {
  let rss = 0;
  let pss = 0;
  const pids = await processTree();
  for (const pid of pids) {
    try {
      const status = readFileSync(`/proc/${pid}/status`, "utf8");
      rss += Number(/VmRSS:\s+(\d+)/.exec(status)?.[1] ?? 0);
      const rollup = readFileSync(`/proc/${pid}/smaps_rollup`, "utf8");
      pss += Number(/^Pss:\s+(\d+)/m.exec(rollup)?.[1] ?? 0);
    } catch {
      // The process exited while we read it.
    }
  }
  return { processes: pids.length, rss_mib: +(rss / 1024).toFixed(1), pss_mib: +(pss / 1024).toFixed(1) };
}

/** CPU time (user + system, every thread) used so far by the process tree, in ms. */
async function cpuMs() {
  let ticks = 0;
  for (const pid of await processTree()) {
    try {
      const stat = readFileSync(`/proc/${pid}/stat`, "utf8");
      const fields = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      ticks += Number(fields[11]) + Number(fields[12]);
    } catch {
      // The process exited while we read it.
    }
  }
  return ticks * 10; // USER_HZ = 100
}

const context = await chromium.launchPersistentContext(profile, {
  headless: true,
  // A Chromium or chrome-headless-shell binary, when Playwright's own is not installed.
  executablePath: process.env.CHROME || undefined,
  viewport: { width: 900, height: 760 },
});
const page = context.pages()[0] ?? (await context.newPage());
const cdp = await context.newCDPSession(page);
await cdp.send("Performance.enable");
const metrics = async () =>
  Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));

const opening = await metrics();
const navigated = Date.now();
await page.goto(`http://127.0.0.1:${port}/?history=${HISTORY}`);
await page.waitForFunction(
  (n) => document.querySelectorAll("[data-role]").length >= n,
  HISTORY,
  { timeout: 600_000, polling: 50 },
);
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const openMs = Date.now() - navigated;
// Like for like with Rust: also wait until every assistant message's Markdown is
// rendered (each history reply has a table) and painted, then report that time.
await page.waitForFunction(
  (n) => document.querySelectorAll("[data-role=assistant] table").length >= n,
  Math.floor(HISTORY / 2),
  { timeout: 600_000, polling: 50 },
);
await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const openFormattedMs = Date.now() - navigated;
const opened = await metrics();
const openedMemory = await memory();
await page.waitForTimeout(1000);

// Frame intervals while streaming.
await page.evaluate(() => {
  const w = window;
  w.__frames = [];
  let last = performance.now();
  const tick = (t) => {
    w.__frames.push(t - last);
    last = t;
    if (!w.__stop) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
});
const input = page.getByLabel("Message input");
await input.fill(prompt);
const before = await metrics();
const cpuBefore = await cpuMs();
await input.press("Enter");
await page.waitForTimeout(WINDOW_S * 1000);
const after = await metrics();
const cpuStream = (await cpuMs()) - cpuBefore;
const frames = await page.evaluate(() => {
  window.__stop = true;
  return window.__frames.slice(1);
});
const endMemory = await memory();
const replyChars = await page.evaluate(
  () => [...document.querySelectorAll("[data-role=assistant]")].at(-1)?.textContent?.length ?? 0,
);

const sorted = [...frames].sort((a, b) => a - b);
const pct = (p) => +(sorted[Math.floor((sorted.length - 1) * p)] ?? 0).toFixed(2);
const busy = (m) => (after[m] - before[m]) * 1000;
console.log(
  JSON.stringify({
    engine: "react",
    history: HISTORY,
    reply_kb: REPLY_KB,
    window_s: WINDOW_S,
    open_ms: openMs,
    open_formatted_ms: openFormattedMs,
    open_main_thread_ms: Math.round((opened.TaskDuration - opening.TaskDuration) * 1000),
    stream_process_cpu_ms: Math.round(cpuStream),
    stream_main_thread_ms: Math.round(busy("TaskDuration")),
    stream_script_ms: Math.round(busy("ScriptDuration")),
    stream_layout_ms: Math.round(busy("LayoutDuration")),
    stream_style_ms: Math.round(busy("RecalcStyleDuration")),
    frames: frames.length,
    frame_p50_ms: pct(0.5),
    frame_p95_ms: pct(0.95),
    frame_p99_ms: pct(0.99),
    frame_max_ms: pct(1),
    frames_over_33ms: frames.filter((f) => f > 33.4).length,
    js_heap_mib: +(after.JSHeapUsedSize / 2 ** 20).toFixed(1),
    memory_opened: openedMemory,
    memory_end: endMemory,
    last_message_chars: replyChars,
  }),
);
await context.close();
server.close();
