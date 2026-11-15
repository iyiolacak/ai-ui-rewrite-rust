#!/usr/bin/env python3
"""Write the "method" and "comparisons" sections of data/benchmarks.json from docs/runs.json. Stdlib only.
   python3 site/scripts/fill-bench.py          rewrite site/data/benchmarks.json
   python3 site/scripts/fill-bench.py --check  exit 1 if it is out of date

docs/runs.json (built from the raw files by benchmarks/results/make-runs-json.py) is the one source for every
benchmark number. For each measure the page shows the median of the run least favourable to Rust, and the range of
the run medians beside it, so the headline holds whichever run you pick."""
import json, math, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
RUNS = HERE.parents[1] / "docs" / "runs.json"
OUT = HERE.parent / "data" / "benchmarks.json"
RUN_IDS = ("run1", "run2", "run3", "run4")  # the four 1000-message runs of 2026-11-11

# id, measure in runs.json, title, unit, scale from runs.json unit, decimals (rust, react), notes
SPECS = [
    ("open-1000", "open_like_for_like", "Open a 1000-message thread", "ms", 1, (1, 0),
     "until the last message's Markdown is formatted and painted; the first rows appear after {first} ms",
     "until every message is in the DOM with its Markdown rendered and painted"),
    ("memory-1000", "memory_end_pss", "Memory at the end of the stream", "MiB", 1, (1, 1),
     "resident memory (RSS) of its one process",
     "proportional memory (PSS) summed over every Chromium process; RSS would count shared pages more than once"),
    ("cpu-1000", "cpu_60s", "CPU over 60 s of streaming", "s", 1 / 1000, (2, 2),
     "the whole process: app work, layout and software paint",
     "the whole Chromium process tree"),
    ("stall-1000", "stall", "Longest stall while streaming", "ms", 1, (1, 1),
     "longest loop iteration: app work, layout and software paint",
     "longest interval between two frames"),
]


def floor1(x):
    return math.floor(x * 10 + 1e-9) / 10


def build(old):
    runs = {r["id"]: r for r in json.loads(RUNS.read_text())["runs"] if r["id"] in RUN_IDS}
    assert set(runs) == set(RUN_IDS), f"docs/runs.json lacks one of {RUN_IDS}"
    comparisons = []
    for cid, key, title, unit, scale, (dr, dc), rust_note, react_note in SPECS:
        ratios = {rid: runs[rid]["measures"][key]["ratio_floor_median"] for rid in RUN_IDS}
        worst = min(RUN_IDS, key=lambda rid: (ratios[rid], rid))
        m = runs[worst]["measures"][key]
        rust, react = round(m["rust_median"] * scale, dr), round(m["react_median"] * scale, dc)
        if dc == 0:
            react = int(react)
        # The page derives the ratio from the shown values; it must equal the run's own floored ratio.
        assert floor1(react / rust) == ratios[worst], (cid, react / rust, ratios[worst])
        first = runs[worst]["measures"]["open_first_sync_rust_only"]["rust_median"]
        comparisons.append(dict(
            id=cid, title=title, unit=unit, better="lower",
            workload="1000 messages, then a 9 KB prompt whose reply streams for 60 s",
            rust=rust, react=react,
            rustNote=rust_note.format(first=first), reactNote=react_note,
            run=runs[worst]["label"], runConditions=runs[worst]["conditions"],
            rangeLow=min(ratios.values()), rangeHigh=max(ratios.values()), runs=len(RUN_IDS),
            source="docs/runs.json (" + ", ".join(runs[worst]["sources"]) + ")"))
    out = dict(old)
    out["method"] = dict(
        summary=("Four independent runs on 2026-11-11, each 5 to 9 interleaved Rust/React pairs, under different power "
                 "settings and machine load. Each chart shows the median of the run least favourable to Rust for that "
                 "measure, with the range of the four run medians. Rust was ahead in every pair of every run."),
        workload=("A 1000-message history, then a 9 KB prompt; the demo model's reply quotes it (about twice as long), "
                  "reasoning then the answer, one word every 18 ms, measured over 60 s. Upstream is the published @assistant-ui/react 0.15.33 and "
                  "@assistant-ui/react-markdown 0.14.25 in headless Chromium; Rust is a release build that rasterises "
                  "in software on the CPU."),
        cpu="Whole process tree, every thread and process.",
        memory="Rust: resident memory of its one process. React: PSS summed over every Chromium process.",
        date="2026-11-11",
        source="docs/runs.json and docs/RUNS.md; raw files in benchmarks/results; harness in benchmarks/react-baseline",
    )
    out["comparisons"] = comparisons
    limits = [l for l in old.get("limits", []) if not l.startswith(("Ratios are", "Rust draws at most"))]
    limits += ["Rust draws at most 60 frames a second, like a display. Absolute numbers depend on CPU speed and load; "
               "React's open time is the more sensitive, so the open ratio is smallest on the fastest runs.",
               "Ratios are React median divided by Rust median, rounded down, never up."]
    out["limits"] = limits
    return out


def main():
    old = json.loads(OUT.read_text())
    text = json.dumps(build(old), indent=1, ensure_ascii=False) + "\n"
    if "--check" in sys.argv:
        if OUT.read_text() != text:
            print("site/data/benchmarks.json is out of date: run python3 site/scripts/fill-bench.py")
            sys.exit(1)
        print("site/data/benchmarks.json is up to date")
    else:
        OUT.write_text(text)
        print("wrote", OUT)


main()
