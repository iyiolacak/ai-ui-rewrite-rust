// Everything numeric on the page is derived here from data/benchmarks.json, which scripts/fill-bench.py writes from docs/runs.json.
import data from '../../data/benchmarks.json' with { type: 'json' };

export interface Comparison {
  id: string; title: string; unit: string; workload: string;
  rust: number; react: number; rustNote?: string; reactNote?: string; source: string;
  run: string; rangeLow: number; rangeHigh: number; runs: number;
}

export const method = data.method;
export const limits = data.limits;

export function comparison(id: string): Comparison {
  const c = (data.comparisons as Comparison[]).find((x) => x.id === id);
  if (!c) throw new Error(`benchmarks.json has no comparison "${id}"`);
  return c;
}

/** How many times better Rust is, rounded down to one decimal (never up). */
export function ratio(c: Comparison): string {
  return (Math.floor((c.react / c.rust) * 10 + 1e-9) / 10).toFixed(1);
}

/** The ratio rounded down to a whole number, for the headline only (16.6 reads as 16, never 17). The charts show the one-decimal `ratio`. */
export function ratioWhole(c: Comparison): string {
  return String(Math.floor(c.react / c.rust + 1e-9));
}

/** The range of the run medians' ratios, e.g. "16.6× to 27.9×". */
export function range(c: Comparison): string {
  return `${c.rangeLow.toFixed(1)}× to ${c.rangeHigh.toFixed(1)}×`;
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
const value = (path: string, v: string | number) => `<tspan data-bench="${path}">${v}</tspan>`;

/** Two horizontal bars drawn as SVG, scaled to the larger value. `verb` completes "16.6× faster" / "28.2× less memory". Numbers carry data-bench for the test. */
export function barChart(c: Comparison, verb = 'faster'): string {
  const W = 480, left = 64, span = 330, max = Math.max(c.rust, c.react);
  const bar = (y: number, cls: string, name: string, v: number, key: string) => {
    const w = Math.max((v / max) * span, 2);
    return `<text class="bar-label" x="0" y="${y + 12}">${name}</text>` +
      `<rect class="${cls}" x="${left}" y="${y}" width="${w.toFixed(1)}" height="14" rx="2"/>` +
      `<text class="bar-value" x="${(left + w + 8).toFixed(1)}" y="${y + 12}">${value(`${c.id}.${key}`, v)} ${esc(c.unit)}</text>`;
  };
  const label = `${c.title}: Rust ${c.rust} ${c.unit}, React ${c.react} ${c.unit}, ${ratio(c)}× ${verb}`;
  return `<svg class="chart" viewBox="0 0 ${W} 120" role="img" aria-label="${esc(label)}" width="100%">` +
    bar(6, 'rust', 'Rust', c.rust, 'rust') + bar(36, 'other', 'React', c.react, 'react') +
    `<text class="strong" x="0" y="96">${value(`${c.id}.ratio`, ratio(c))}× ${esc(verb)}</text></svg>`;
}
