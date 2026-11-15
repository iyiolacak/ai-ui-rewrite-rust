// node --experimental-strip-types site/scripts/chart.test.ts
import assert from 'node:assert/strict';
import { barChart, comparison, range, ratio, ratioWhole } from '../src/lib/bench.ts';

const open = comparison('open-1000');
assert.equal(ratio(open), '16.6');
assert.equal(ratio(comparison('memory-1000')), '28.2');
assert.equal(ratio(comparison('cpu-1000')), '4.2');
assert.equal(ratio(comparison('stall-1000')), '23.8');
// The headline rounds down: 16.6 reads as 16 and 23.8 as 23, never up.
assert.equal(ratioWhole(open), '16');
assert.equal(ratioWhole(comparison('stall-1000')), '23');
assert.equal(range(open), '16.6× to 27.9×');
const svg = barChart(open);
for (const needle of ['data-bench="open-1000.rust">83.4<', 'data-bench="open-1000.react">1385<', 'role="img"', 'aria-label=']) {
  assert.ok(svg.includes(needle), `chart lacks ${needle}`);
}
console.log('chart ok');
