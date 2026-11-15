// Exercise the actual Wasm adapter in a secure loopback origin using a mocked
// clipboard: no system clipboard is touched.
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const moduleName = process.env.AI_UI_PLAYWRIGHT_MODULE;
const { chromium } = await import(moduleName ? pathToFileURL(moduleName).href : "playwright");
const dir = path.resolve(process.argv[2]);
const server = http.createServer(async (req, res) => {
  try {
    if (req.url === "/") return res.end('<script type="module">import init, {Fixture} from "/test.js"; await init(); window.Fixture = Fixture;</script>');
    const name = req.url === "/test.js" ? "test.js" : req.url === "/test_bg.wasm" ? "test_bg.wasm" : null;
    if (!name) return res.writeHead(404).end();
    return res.writeHead(200, {"content-type": name.endsWith(".wasm") ? "application/wasm" : "text/javascript"}).end(await fs.readFile(path.join(dir, name)));
  } catch { return res.writeHead(500).end(); }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
let browser;
try {
  browser = await chromium.launch({ headless: true, executablePath: process.env.AI_UI_BROWSER_EXECUTABLE || undefined });
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.waitForFunction(() => !!window.Fixture);
  const observed = await page.evaluate(async () => {
    let settle;
    let calls = [];
    let mode = "pending";
    let reentrant;
    let fixture;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText(text) {
        calls.push(text);
        if (mode === "throw") throw Error("private platform error");
        if (mode === "not-promise") return undefined;
        if (mode === "reentrant") reentrant = fixture.copy("nested");
        return new Promise((resolve, reject) => { settle = mode === "reject" ? reject : resolve; });
      }
    }});
    fixture = new window.Fixture();
    const first = fixture.copy("one");
    const callsAfterFirst = [...calls]; // The call occurred synchronously.
    const busy = fixture.copy("two");
    settle();
    await new Promise(resolve => setTimeout(resolve, 0));
    const resultAfterFirst = fixture.results();
    mode = "reject";
    const second = fixture.copy("three");
    settle(Error("private rejection"));
    await new Promise(resolve => setTimeout(resolve, 0));
    const resultAfterSecond = fixture.results();
    mode = "throw";
    const thrown = fixture.copy("four");
    mode = "not-promise";
    const invalid = fixture.copy("five");
    const tooLarge = fixture.copy("x".repeat(1024 * 1024 + 1));
    mode = "reentrant";
    const third = fixture.copy("six");
    fixture.shutdown();
    const cancelled = fixture.copy("seven");
    settle();
    await new Promise(resolve => setTimeout(resolve, 0));
    return {first, callsAfterFirst, busy, resultAfterFirst, second, resultAfterSecond, thrown, invalid, tooLarge, third, reentrant, cancelled, final: fixture.results(), calls};
  });
  assert.deepEqual(observed, {
    first: "accepted", callsAfterFirst: ["one"], busy: "Clipboard is busy", resultAfterFirst: "ok",
    second: "accepted", resultAfterSecond: "ok,Clipboard write was rejected", thrown: "Clipboard write was rejected",
    invalid: "Clipboard is unavailable", tooLarge: "Text is too large to copy", third: "accepted", reentrant: "Clipboard is busy", cancelled: "Clipboard request was cancelled",
    final: "ok,Clipboard write was rejected,ok", calls: ["one", "three", "four", "five", "six"]
  });
  const reads = await page.evaluate(async () => {
    let settle;
    let mode = "pending";
    let calls = 0;
    let reentrant;
    let fixture;
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      readText() {
        calls++;
        if (mode === "throw") throw Error("private platform error");
        if (mode === "not-promise") return undefined;
        if (mode === "reentrant") reentrant = fixture.read_text();
        return new Promise((resolve, reject) => { settle = mode === "reject" ? reject : resolve; });
      },
      writeText() { throw Error("write should not start while a read is pending"); }
    }});
    fixture = new window.Fixture();
    const first = fixture.read_text();
    const callsAfterFirst = calls;
    const readBusy = fixture.read_text();
    const writeBusy = fixture.copy("ignored");
    settle("line\r\n雪👩‍💻"); await new Promise(resolve => setTimeout(resolve, 0));
    const afterFirst = fixture.results();
    mode = "reject"; const second = fixture.read_text(); settle(Error("private rejection"));
    await new Promise(resolve => setTimeout(resolve, 0));
    mode = "throw"; const thrown = fixture.read_text();
    mode = "not-promise"; const invalid = fixture.read_text();
    mode = "pending"; const large = fixture.read_text(); settle("x".repeat(4 * 1024 * 1024 + 1));
    await new Promise(resolve => setTimeout(resolve, 0));
    mode = "reentrant"; const third = fixture.read_text();
    fixture.shutdown(); const cancelled = fixture.read_text(); settle("last");
    await new Promise(resolve => setTimeout(resolve, 0));
    return {first, callsAfterFirst, readBusy, writeBusy, afterFirst, second, thrown, invalid, large, third, reentrant, cancelled, final: fixture.results(), calls};
  });
  assert.deepEqual(reads, {
    first: "accepted", callsAfterFirst: 1, readBusy: "Clipboard is busy", writeBusy: "Clipboard is busy",
    afterFirst: "read:line\r\n雪👩‍💻", second: "accepted", thrown: "Clipboard read was rejected",
    invalid: "Clipboard is unavailable", large: "accepted", third: "accepted", reentrant: "Clipboard is busy",
    cancelled: "Clipboard request was cancelled", final: "read:line\r\n雪👩‍💻,Clipboard read was rejected,Clipboard text is too large,read:last", calls: 6
  });
  console.log("PASS browser clipboard: reads and writes, synchronous admission, shared pending slot, resolve/reject, limits and shutdown");
} finally {
  if (browser) await browser.close();
  await new Promise(resolve => server.close(resolve));
}
