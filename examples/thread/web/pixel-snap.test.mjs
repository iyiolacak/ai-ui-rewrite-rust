// Run: node --test examples/thread/web/pixel-snap.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { snapSpan } from "./pixel-snap.js";

const dev = (css, dpr) => Math.round(css * dpr * 1e6) / 1e6;

test("integer dpr and integer box are unchanged", () => {
  const s = snapSpan(100, 800, 2);
  assert.equal(s.offset, 0);
  assert.equal(s.length, 800);
  assert.equal(s.devicePixels, 1600);
});

test("fractional box snaps inward to whole device pixels", () => {
  for (const dpr of [1, 1.25, 1.5, 1.75, 2.5, 12 / 11]) {
    for (const [start, size] of [[16.4, 1071.133], [20.3, 707.667], [0.5, 333.3]]) {
      const s = snapSpan(start, size, dpr);
      assert.ok(s.offset >= -1e-9 && s.offset < 1 / dpr + 1e-9, `offset ${s.offset} @${dpr}`);
      assert.equal(dev(start + s.offset, dpr) % 1, 0, `start aligned @${dpr}`);
      assert.equal(dev(s.length, dpr) % 1, 0, `length whole @${dpr}`);
      assert.ok(start + s.offset + s.length <= start + size + 1e-9, "stays inside");
      assert.ok(size - s.length < 2 / dpr + 1e-9, "loses under two device pixels");
    }
  }
});

test("never collapses to zero", () => {
  assert.equal(snapSpan(0.1, 0.1, 1).devicePixels, 1);
});
