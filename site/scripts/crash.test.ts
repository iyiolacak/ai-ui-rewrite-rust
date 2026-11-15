// node --experimental-strip-types site/scripts/crash.test.ts
import assert from 'node:assert/strict';
import { CrashLine, crashDefaults, type BodyLayout } from '../src/scripts/motion/crashLine.ts';

// "opens 24× faster and uses" over "34× less memory", Work Sans 500 widths in em, as measured in Chromium.
function layout(): BodyLayout[] {
  const rows = [[2.85, 1.72, 2.79, 1.76, 2.18], [1.72, 1.87, 3.89]];
  return rows.flatMap((widths, row) => {
    let left = 0;
    return widths.map((width) => {
      const b = { left, width, row };
      left += width + 0.27;
      return b;
    });
  });
}

function run(fps: number) {
  const line = new CrashLine(layout(), 1);
  line.launch();
  let worstGap = Infinity;
  // Furthest each word goes, how far a wall comes back past its place, when it first moves, and its
  // visible pulses (outward peaks rising over 0.006em, about 0.3px on a 52px headline, above the last trough).
  const push = [0, 0, 0, 0, 0];
  const overshoot = [0, 0, 0, 0, 0];
  const firstMove = [-1, -1, -1, -1, -1];
  const pulses = [0, 0, 0, 0, 0];
  const prev = [0, 0, 0, 0, 0];
  const trough = [0, 0, 0, 0, 0];
  const rising = [false, false, false, false, false];
  const peak = [0, 0, 0, 0, 0];
  let maxOtherRow = 0;
  let t = 0;
  while (!line.settled && t < 5) {
    line.advance(1 / fps);
    t += 1 / fps;
    const x = line.frame();
    for (let i = 0; i < 4; i++) {
      const a = line.bodies[i]!;
      const b = line.bodies[i + 1]!;
      worstGap = Math.min(worstGap, b.left + x[i + 1]! - (a.left + a.width + x[i]!));
    }
    for (let i = 0; i < 5; i++) {
      // "opens" is pushed left; the words after the number are pushed right.
      const outward = i === 0 ? -x[i]! : x[i]!;
      push[i] = Math.max(push[i]!, Math.abs(x[i]!));
      if (i !== 1) overshoot[i] = Math.max(overshoot[i]!, -outward);
      if (firstMove[i] === -1 && Math.abs(x[i]!) > 0.004) firstMove[i] = t;
      if (outward > prev[i]!) {
        rising[i] = true;
        peak[i] = outward;
      } else if (rising[i] && outward < prev[i]!) {
        if (peak[i]! - trough[i]! > 0.006 && peak[i]! > 0.006) pulses[i]!++;
        rising[i] = false;
        trough[i] = outward;
      } else if (!rising[i]) trough[i] = Math.min(trough[i]!, outward);
      prev[i] = outward;
    }
    for (let i = 5; i < 8; i++) maxOtherRow = Math.max(maxOtherRow, Math.abs(x[i]!));
  }
  return { line, t, worstGap, maxOtherRow, push, overshoot, firstMove, pulses };
}

// Sampled at 120 Hz so a pulse between two 60 Hz frames still counts.
const at60 = run(120);
const sourceHits = at60.line.hits.filter((h) => h.left === 1 || h.right === 1);
assert.ok(at60.line.settled, 'the crash comes to rest');
assert.ok(at60.t < 1.5, `settles in under 1.5 s, took ${at60.t.toFixed(2)} s`);
assert.ok(sourceHits.length >= 10, `the number strikes its neighbours at least 10 times, got ${sourceHits.length}`);
// The rattle alternates right, left, right, ...
for (const [k, h] of sourceHits.entries()) {
  assert.equal(h.left === 1 ? 'R' : 'L', k % 2 === 0 ? 'R' : 'L', `strike ${k} is on the wrong side`);
}
// Strikes are far enough apart to read one by one at 60 Hz (two frames or more), and the rattle ends
// by slowing down rather than stopping.
const gaps = sourceHits.slice(1).map((h, k) => h.time - sourceHits[k]!.time);
assert.ok(Math.min(...gaps) >= 0.03, `strikes ${(Math.min(...gaps) * 1000).toFixed(0)} ms apart blur together`);
assert.ok(gaps.at(-1)! > gaps.at(-3)!, 'the rattle slows down at the end');

// Walls: every word on the line moves, "uses" included, on every strike, a little later the further it is.
assert.ok(at60.push[1]! <= 0.2, `the number moves ${at60.push[1]!.toFixed(3)} em, too far to read`);
for (const i of [0, 2, 3, 4]) {
  assert.ok(at60.push[i]! >= 0.03, `word ${i} moves only ${at60.push[i]!.toFixed(3)} em`);
  assert.ok(at60.push[i]! < at60.push[1]! / 2, `word ${i} moves too much for a wall`);
  assert.ok(at60.pulses[i]! >= 5, `word ${i} reacts to only ${at60.pulses[i]} strikes`);
  assert.ok(at60.overshoot[i]! < 0.012, `word ${i} swings back ${at60.overshoot[i]!.toFixed(4)} em past its place`);
}
assert.ok(at60.push[2]! > at60.push[3]! && at60.push[3]! > at60.push[4]!, 'the push fades along the line');
assert.ok(at60.firstMove[2]! < at60.firstMove[3]! && at60.firstMove[3]! < at60.firstMove[4]!, 'the push reaches each word later');
assert.ok(at60.firstMove[4]! - at60.firstMove[2]! >= 2 * crashDefaults.echoDelay - 0.01, 'the delay along the line is visible');
assert.ok(at60.worstGap > 0, `letters never touch, smallest gap ${at60.worstGap.toFixed(4)} em`);
assert.equal(at60.maxOtherRow, 0, 'other rows stay put');
assert.ok(at60.line.frame().every((x) => Math.abs(x) < 0.002), 'returns to the layout');

// Fixed substeps: the same crash at any refresh rate.
for (const fps of [30, 144, 240]) {
  const other = run(fps);
  assert.deepEqual(
    other.line.hits.map((h) => [h.left, h.right]),
    at60.line.hits.map((h) => [h.left, h.right]),
    `${fps} fps produces a different sequence of strikes`,
  );
}

// A long stall replays at most 0.1 s of physics.
const stalled = new CrashLine(layout(), 1);
stalled.launch();
stalled.advance(10);
assert.ok(stalled.time <= 0.1 + crashDefaults.step, 'a stalled frame is clamped');

console.log(
  `crash ok: ${sourceHits.length} strikes in ${(sourceHits.at(-1)!.time * 1000).toFixed(0)} ms, rest at ${at60.t.toFixed(2)} s, ` +
    `push ${at60.push.map((r) => r.toFixed(3)).join(' ')} em, pulses ${at60.pulses.join(' ')}, ` +
    `first move ${at60.firstMove.map((f) => Math.round(f * 1000)).join(' ')} ms, worst overshoot ${Math.max(...at60.overshoot).toFixed(4)} em`,
);
