#!/usr/bin/env python3
"""docs/results.json is the data; docs/RESULTS.md carries tables made from it.

    python3 scripts/results.py render   rewrite the tables in docs/RESULTS.md from docs/results.json
    python3 scripts/results.py check    fail if the tables and the JSON disagree, if a ratio is not
                                        floor(react / rust) to one decimal, or if the rows are not ranked

Ratios are computed here, from the stored values, and floored to one decimal (never rounded up). For
a measure where higher is better (frames per second), the ratio is rust / react.
"""
import json
import math
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
JSON = ROOT / "docs" / "results.json"
MD = ROOT / "docs" / "RESULTS.md"
MARK = {
    "vs_react": ("<!-- results:vs_react -->", "<!-- /results:vs_react -->"),
    "rust_only": ("<!-- results:rust_only -->", "<!-- /results:rust_only -->"),
}


def floor1(x):
    # Work in integers so that 50.0 is not floored to 49.9 by binary floating point.
    return math.floor(round(x * 10, 6)) / 10


def ratio(row, rust, react):
    if row.get("better", "lower") == "higher":
        return floor1(rust / react)
    return floor1(react / rust)


def ratios(row):
    """(headline, low, high) from the stored values; the high end pairs the best Rust figure with the
    worst React figure."""
    rl, rh = row["rust_range"]
    cl, ch = row["react_range"]
    head = ratio(row, row["rust"], row["react"])
    if row.get("better", "lower") == "higher":
        lo, hi = ratio(row, rl, ch), ratio(row, rh, cl)
    else:
        lo, hi = ratio(row, rh, cl), ratio(row, rl, ch)
    return head, lo, hi


def fmt(x):
    return f"{x:g}"


def span(lo, hi):
    return fmt(lo) if lo == hi else f"{fmt(lo)} to {fmt(hi)}"


def ranked(rows):
    return sorted(rows, key=lambda r: (0 if r.get("no_ratio") else -ratios(r)[2], r["id"]))


def table(rows, group):
    rows = [r for r in rows if r["group"] == group]
    out = [
        "| Rank | Workload | Measure | Rust | React | Ratio | Range | Source (commit), date | Conditions | What drives it |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for i, r in enumerate(ranked(rows), 1):
        u = r["unit"]
        if r.get("no_ratio"):
            out.append(
                f"| {i} | {r['workload']} | {r['measure']} | {r['rust_text']} | {r['react_text']} | none | none | "
                f"{r['source']}, {r['date']} | {r['conditions']} | {r['drivers']} |"
            )
            continue
        head, lo, hi = ratios(r)
        lb = ">= " if r.get("lower_bound") else ""
        out.append(
            f"| {i} | {r['workload']} | {r['measure']} | {span(*r['rust_range'])} {u} (value {fmt(r['rust'])}) | "
            f"{span(*r['react_range'])} {u} (value {fmt(r['react'])}) | {lb}{fmt(head)}x | {lb}{fmt(lo)}x to {lb}{fmt(hi)}x | "
            f"{r['source']}, {r['date']} | {r['conditions']} | {r['drivers']} |"
        )
    return "\n".join(out)


def render(md, data):
    for group, (a, b) in MARK.items():
        i, j = md.index(a) + len(a), md.index(b)
        md = md[:i] + "\n" + table(data["rows"], group) + "\n" + md[j:]
    return md


def check():
    data = json.loads(JSON.read_text())
    md = MD.read_text()
    bad = []
    seen = set()
    for r in data["rows"]:
        if r["id"] in seen:
            bad.append(f"duplicate id {r['id']}")
        seen.add(r["id"])
        if r.get("no_ratio"):
            continue
        for k in ("rust", "react"):
            lo, hi = r[k + "_range"]
            if not lo <= r[k] <= hi:
                bad.append(f"{r['id']}: {k} value {r[k]} outside its range {lo}..{hi}")
        head, lo, hi = ratios(r)
        if "ratio" in r and r["ratio"] != head:
            bad.append(f"{r['id']}: stored ratio {r['ratio']} != computed {head}")
        if not lo <= head <= hi:
            bad.append(f"{r['id']}: headline {head} outside range {lo}..{hi}")
    if render(md, data) != md:
        bad.append("docs/RESULTS.md tables differ from docs/results.json (run: python3 scripts/results.py render)")
    for msg in bad:
        print("FAIL:", msg)
    if bad:
        sys.exit(1)
    print(f"ok: {len(data['rows'])} rows, tables match docs/results.json, ratios recomputed")


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    if cmd == "render":
        MD.write_text(render(MD.read_text(), json.loads(JSON.read_text())))
    elif cmd == "check":
        check()
    else:
        sys.exit(__doc__)
