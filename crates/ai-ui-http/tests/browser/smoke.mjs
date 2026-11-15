// Exercise the browser fetch client through DataStreamAdapter and LocalThreadRuntime against a
// loopback AI SDK-style server that streams slowly, so chunking, protocol detection, errors and
// cancellation happen over a real network stream in a real browser.
import assert from "node:assert/strict";
import http from "node:http";
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const moduleName = process.env.AI_UI_PLAYWRIGHT_MODULE;
const { chromium } = await import(moduleName ? pathToFileURL(moduleName).href : "playwright");
const dir = path.resolve(process.argv[2]);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const state = { slowClosed: false, bodies: [] };

const server = http.createServer(async (req, res) => {
  try {
    if (req.method === "GET") {
      if (req.url === "/") return res.end('<script type="module">import init, * as api from "/test.js"; await init(); window.api = api;</script>');
      const name = req.url === "/test.js" ? "test.js" : req.url === "/test_bg.wasm" ? "test_bg.wasm" : null;
      if (!name) return res.writeHead(404).end();
      return res.writeHead(200, { "content-type": name.endsWith(".wasm") ? "application/wasm" : "text/javascript" }).end(await fs.readFile(path.join(dir, name)));
    }
    let body = "";
    for await (const chunk of req) body += chunk;
    state.bodies.push(body);
    if (req.url === "/ui") {
      res.writeHead(200, { "content-type": "text/event-stream", "x-vercel-ai-ui-message-stream": "v1" });
      const events = [{ type: "text-start", id: "t" }, { type: "text-delta", id: "t", delta: "Hel" }, { type: "text-delta", id: "t", delta: "lo " }, { type: "text-delta", id: "t", delta: "browser" }, { type: "text-end", id: "t" }, { type: "finish", finishReason: "stop" }];
      for (const event of events) { res.write(`data: ${JSON.stringify(event)}\n\n`); await sleep(40); }
      return res.end("data: [DONE]\n\n");
    }
    if (req.url === "/data") {
      res.writeHead(200, { "content-type": "text/plain", "x-vercel-ai-data-stream": "v1" });
      for (const line of ['0:"Data "\n', '0:"stream"\n', 'd:{"finishReason":"stop"}\n']) { res.write(line); await sleep(40); }
      return res.end();
    }
    if (req.url === "/slow") {
      res.writeHead(200, { "content-type": "text/event-stream", "x-vercel-ai-ui-message-stream": "v1" });
      res.on("close", () => { state.slowClosed = true; });
      res.write(`data: ${JSON.stringify({ type: "text-start", id: "t" })}\n\n`);
      res.write(`data: ${JSON.stringify({ type: "text-delta", id: "t", delta: "partial" })}\n\n`);
      for (let i = 0; i < 100 && !state.slowClosed; i++) await sleep(50);
      return;
    }
    if (req.url === "/error") return res.writeHead(500, { "content-type": "application/json" }).end('{"error":"boom"}');
    res.writeHead(404).end();
  } catch { try { res.writeHead(500).end(); } catch {} }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, executablePath: process.env.AI_UI_BROWSER_EXECUTABLE || undefined });
  const page = await browser.newPage();
  await page.goto(`${origin}/`);
  await page.waitForFunction(() => !!window.api);
  const run = (fn, ...args) => page.evaluate(([fn, args]) => window.api[fn](...args), [fn, args]);

  assert.equal(await run("chat", `${origin}/ui`), "Hello browser|complete");
  assert.equal(await run("chat", `${origin}/data`), "Data stream|complete");
  const failed = await run("chat", `${origin}/error`);
  assert.match(failed, /\|incomplete\|error:/, failed);
  assert.equal(await run("chat_and_cancel", `${origin}/slow`, 300), "partial|incomplete|done");
  await sleep(300);
  assert.equal(state.slowClosed, true, "cancelling aborts the request: the server saw the connection close");
  assert.ok(state.bodies.every((body) => JSON.parse(body).messages.length >= 1), "request bodies are AI SDK JSON");
  console.log("browser fetch client: ok");
} finally {
  await browser?.close();
  server.close();
}
