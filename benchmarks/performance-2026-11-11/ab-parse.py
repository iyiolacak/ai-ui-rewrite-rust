"""Summarize an ab-run.sh log (side a = base, side b = candidate).

For each scenario and metric, prints the medians per side and the median of the
per-pair b/a ratios with their range. Runs of one pair are adjacent in time, so
the paired ratio is the figure to read when the machine's speed drifts during
the session; the medians are kept for reference.

Usage: python3 -I ab-parse.py bench.log
"""
import re
import statistics
import sys

UNIT_MS = {"ns": 1e-6, "µs": 1e-3, "us": 1e-3, "ms": 1.0, "s": 1e3}


def ms(text):
    m = re.fullmatch(r"([\d.]+)(ns|µs|us|ms|s)", text)
    return float(m.group(1)) * UNIT_MS[m.group(2)]


PATTERNS = [
    (r"open \(formatted and painted, like for like\) ([\d.]+) ms",
     lambda m: {"open ms": float(m.group(1))}),
    (r"p50 (\S+)\s+p95 (\S+)\s+p99 (\S+)\s+max \S+\s+total ([\d.]+) ms; iterations over 4 ms: (\d+)",
     lambda m: {"p50 ms": ms(m.group(1)), "p95 ms": ms(m.group(2)), "p99 ms": ms(m.group(3)),
                "app total ms": float(m.group(4)), "iters > 4 ms": int(m.group(5))}),
    (r"rss ([\d.]+) MiB, peak [\d.]+ MiB after opening the thread; rss [\d.]+ MiB, peak ([\d.]+) MiB at the end; "
     r"process CPU while streaming (\d+) ms",
     lambda m: {"open RSS MiB": float(m.group(1)), "end peak MiB": float(m.group(2)),
                "growth MiB": float(m.group(2)) - float(m.group(1)), "stream CPU ms": int(m.group(3))}),
    (r"TIME user=([\d.]+) sys=([\d.]+)",
     lambda m: {"proc CPU s": float(m.group(1)) + float(m.group(2))}),
]

runs = {}  # (scenario, run) -> side -> metric -> value
key = side = None
for line in open(sys.argv[1], encoding="utf-8"):
    if line.startswith("=== "):
        side, scenario, run = [p.strip() for p in line[4:].split("|")]
        key = (scenario, run)
        continue
    if key is None:
        continue
    metrics = runs.setdefault(key, {}).setdefault(side, {})
    for pattern, extract in PATTERNS:
        if m := re.search(pattern, line):
            metrics.update(extract(m))

for scenario in dict.fromkeys(s for s, _ in runs):
    pairs = [v for (s, _), v in runs.items() if s == scenario and "proc CPU s" in v.get("a", {})
             and "proc CPU s" in v.get("b", {})]
    print(f"\n## {scenario}  ({len(pairs)} complete pairs)")
    print(f"{'metric':<15}{'base med':>11}{'cand med':>11}{'paired b/a':>12}  range")
    names = dict.fromkeys(k for p in pairs for k in p["a"])
    for name in names:
        a = [p["a"][name] for p in pairs if name in p["a"] and name in p["b"]]
        b = [p["b"][name] for p in pairs if name in p["a"] and name in p["b"]]
        if not a:
            continue
        ratios = sorted(y / x for x, y in zip(a, b) if x)
        paired = f"{statistics.median(ratios):.3f}  [{ratios[0]:.3f}..{ratios[-1]:.3f}]" if ratios else "-"
        print(f"{name:<15}{statistics.median(a):>11.3f}{statistics.median(b):>11.3f}  {paired}")
