import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';
import { assertEquivalent, rustClient, trialOrder } from './compare.mjs';

test('each mode occupies each position exactly three times in nine trials', () => {
  const modes = ['node', 'snapshot', 'patch'];
  const positions = modes.map(() => new Map(modes.map(mode => [mode, 0])));
  for (let trial = 0; trial < 9; trial += 1) {
    const order = trialOrder(trial, modes);
    assert.deepEqual([...order].sort(), [...modes].sort());
    order.forEach((mode, position) => positions[position].set(mode, positions[position].get(mode) + 1));
  }
  for (const position of positions) for (const count of position.values()) assert.equal(count, 3);
});

test('zero, negative and non-finite timings cannot enter ratios', () => {
  const reference = { foldedMessage: {}, sidecars: {}, emissions: 1 };
  for (const elapsed_ms of [0, -1, NaN, Infinity]) {
    assert.throws(() => assertEquivalent(reference, { ...reference, elapsed_ms }, 'test', 'snapshot'));
  }
  assert.doesNotThrow(() => assertEquivalent(reference, { ...reference, elapsed_ms: 0.01 }, 'test', 'snapshot'));
});

test('fold, sidecar and publication mismatches independently reject timings', () => {
  const reference = { foldedMessage: {}, sidecars: {}, emissions: 1 };
  for (const field of ['foldedMessage', 'sidecars', 'emissions']) {
    assert.throws(() => assertEquivalent(reference, { ...reference, [field]: null, elapsed_ms: 1 }, 'test', 'snapshot'));
  }
});

test('a silent replay child times out instead of hanging the driver', async () => {
  const client = rustClient(process.execPath, {
    args: ['-e', 'process.stdin.resume()'], timeoutMs: 30,
  });
  try {
    const outcome = await Promise.race([
      client.replay({}).then(() => 'resolved', error => error.message),
      delay(400).then(() => 'external watchdog fired'),
    ]);
    assert.match(outcome, /^Timed out/);
  } finally {
    client.stop();
  }
});

test('missing binary produces a handled rejection, not an unhandled exit promise', async () => {
  const client = rustClient('/nonexistent-assistant-ui-replay', { timeoutMs: 100 });
  try {
    await assert.rejects(client.replay({}));
    await delay(20); // allow any unhandled promise rejection to reach the test runner
  } finally {
    client.stop();
  }
});
