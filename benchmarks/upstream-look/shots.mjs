// Screenshots of upstream's default thread for side-by-side UX review:
// shots/{empty,conversation,stream}.png at 900x760, dark theme.
//   npm ci --ignore-scripts && npm run build
//   PLAYWRIGHT=/path/to/node_modules/playwright [CHROME=/path/to/chrome] node shots.mjs [stream-ms]
import { createServer } from "node:http";
import { readFile, mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { extname, join } from "node:path";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT ?? "playwright");
const DIST = new URL("./dist/", import.meta.url).pathname;
const OUT = new URL("./shots/", import.meta.url).pathname;
const STREAM_MS = Number(process.argv[2] ?? 2500);

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".woff": "font/woff" };
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
const base = `http://127.0.0.1:${server.address().port}/`;
await mkdir(OUT, { recursive: true });

const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME || undefined });
const page = await browser.newPage({ viewport: { width: 900, height: 760 }, colorScheme: "dark" });
for (const scene of ["empty", "conversation", "stream"]) {
  await page.goto(`${base}?scene=${scene}`);
  await page.evaluate(() => document.fonts.ready);
  if (scene === "stream") {
    await page.waitForTimeout(STREAM_MS);
  } else {
    await page.waitForFunction(() => window.__ready === true, null, { timeout: 30_000 });
    await page.waitForTimeout(600);
  }
  await page.screenshot({ path: join(OUT, `${scene}.png`) });
  console.log(`shots/${scene}.png`);
}
await browser.close();
server.close();
