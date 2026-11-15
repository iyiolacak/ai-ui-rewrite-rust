// node --experimental-strip-types site/scripts/chart.test.ts
import assert from 'node:assert/strict';
import { barChart, comparison, ratio } from '../src/lib/bench.ts';

const open = comparison('open-1000');
assert.equal(ratio(open), '24.1');
assert.equal(ratio(comparison('memory-1000')), '33.9');
assert.equal(ratio(comparison('cpu-1000')), '3.6');
const svg = barChart(open);
for (const needle of ['data-bench="open-1000.rust">0.12<', 'data-bench="open-1000.react">2.9<', 'role="img"', 'aria-label=']) {
  assert.ok(svg.includes(needle), `chart lacks ${needle}`);
}
console.log('chart ok');
