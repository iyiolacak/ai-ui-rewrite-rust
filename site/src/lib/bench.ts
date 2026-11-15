// Everything numeric on the page is derived here from data/benchmarks.json.
import data from '../../data/benchmarks.json' with { type: 'json' };

export interface Comparison {
  id: string; title: string; unit: string; workload: string;
  rust: number; react: number; rustNote?: string; reactNote?: string; source: string;
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

/** The ratio to the nearest whole number, for the headline only (33.9 reads as 34, 16.7 as 17). The charts keep the stricter one-decimal, round-down `ratio`. */
export function ratioWhole(c: Comparison): string {
  return String(Math.round(c.react / c.rust));
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
const value = (path: string, v: string | number) => `<tspan data-bench="${path}">${v}</tspan>`;

/** Two horizontal bars drawn as SVG, scaled to the larger value. `verb` completes "24.1× faster" / "33.9× less memory". Numbers carry data-bench for the test. */
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
