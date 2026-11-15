#!/usr/bin/env python3
"""Parses the raw benchmark files and writes docs/runs.json (schema 1). Stdlib only.
   python3 benchmarks/results/make-runs-json.py          write docs/runs.json
   python3 benchmarks/results/make-runs-json.py --check  fail if docs/runs.json is out of date
Description of the schema and the runs: docs/RUNS.md. Every raw file is in benchmarks/results/; run 4 is included
only when its folder is complete."""
import json, math, pathlib, re, statistics, sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
RUN1 = ROOT / "benchmarks/results/like-for-like/raw"
RUN2 = ROOT / "benchmarks/results/like-for-like-run2"
RUN3 = ROOT / "benchmarks/results/like-for-like-run3"
RUN4 = ROOT / "benchmarks/results/like-for-like-run4"
HAIKU = ROOT / "benchmarks/results/web-and-tests"
NOTE = []  # printed notes (run 4 skipped etc.)


def tilde(p):
    p = pathlib.Path(p)
    try:
        return str(p.relative_to(ROOT))
    except ValueError:
        return "~/" + str(p.relative_to(pathlib.Path.home()))


def floor1(x):
    return math.floor(x * 10 + 1e-9) / 10


def r3(x):
    return round(x, 3)


def parse_rust(t):
    m = lambda pat: re.search(pat, t)
    stall = m(r"max ([\d.]+)(µs|ms|s)\b")
    return dict(
        open=float(m(r"like for like\) ([\d.]+) ms").group(1)),
        first_sync=float(m(r"import\+first sync ([\d.]+) ms").group(1)),
        cpu=float(m(r"process CPU while streaming (\d+) ms").group(1)),
        rss=float(m(r"rss ([\d.]+) MiB, peak [\d.]+ MiB at the end").group(1)),
        stall=float(stall.group(1)) * {"µs": 0.001, "ms": 1, "s": 1000}[stall.group(2)],
        iters_over_4ms=int(m(r"iterations over 4 ms: (\d+)").group(1)),
    )


def parse_react(t):
    line = [l for l in t.splitlines() if l.startswith("{")][0]
    j = json.loads(line)
    return dict(open=j["open_formatted_ms"], cpu=j["stream_process_cpu_ms"], rss=j["memory_end"]["rss_mib"],
                pss=j["memory_end"]["pss_mib"], stall=j["frame_max_ms"], frames_over_33ms=j["frames_over_33ms"],
                frame_p95=j["frame_p95_ms"], frames=j["frames"], js_heap=j["js_heap_mib"])


def measure(unit, rust, react, better="lower", note=None, react_label=None):
    d = {"unit": unit, "better": better}
    if note:
        d["note"] = note
    if rust is not None:
        d["rust"] = rust
    if react is not None:
        d["react"] = react
    for side, xs in (("rust", rust), ("react", react)):
        if xs is not None:
            d[side + "_median"] = r3(statistics.median(xs))
            d[side + "_min"] = min(xs)
            d[side + "_max"] = max(xs)
    if rust is not None and react is not None:
        ratio = (lambda ru, re_: re_ / ru) if better == "lower" else (lambda ru, re_: ru / re_)  # react/rust, or rust/react when higher is better
        pr = [ratio(r, c) for r, c in zip(rust, react)]
        d["ratio_rule"] = "react/rust (lower is better)" if better == "lower" else "rust/react (higher is better)"
        d["ratio_per_pair_floor"] = [floor1(x) for x in pr]
        d["ratio_floor_median"] = floor1(ratio(statistics.median(rust), statistics.median(react)))
        d["ratio_floor_least_favourable"] = floor1(min(pr))
        d["ratio_floor_most_favourable"] = floor1(max(pr))
        d["ratio_floor_median_of_pair_ratios"] = floor1(statistics.median(pr))
    return d


def build_measures(R, C, rust_only_note=None):
    g = lambda L, k: [x[k] for x in L]
    ms = {
        "open_like_for_like": measure("ms", g(R, "open"), g(C, "open"),
            note="Rust: until the last message's Markdown is formatted and painted. React: all messages in the DOM with Markdown rendered plus two frames."),
        "open_first_sync_rust_only": measure("ms", g(R, "first_sync"), None,
            note="Rust history import + first sync (rows visible, Markdown not yet formatted). NOT comparable with React's open time."),
        "cpu_60s": measure("ms", g(R, "cpu"), g(C, "cpu"),
            note="Process CPU while streaming 60 s. Load-sensitive; not a headline figure. Rust includes software raster; React is the whole Chromium process tree."),
        "stall": measure("ms", g(R, "stall"), g(C, "stall"),
            note="Longest loop iteration (Rust, unpaced) vs longest animation-frame interval (React). Single-event maximum, noisy."),
        "memory_end_rss": measure("MiB", g(R, "rss"), g(C, "rss"),
            note="Resident memory at the end. React: RSS summed over all Chromium processes."),
        "memory_end_pss": measure("MiB", g(R, "rss"), g(C, "pss"),
            note="Conservative: Rust RSS against React PSS (discounts shared pages). React only for PSS; Rust value is its RSS."),
        "rust_iterations_over_4ms": measure("count", g(R, "iters_over_4ms"), None, note="Rust loop iterations over 4 ms in the 60 s window."),
        "react_frames_over_33ms": measure("count", None, g(C, "frames_over_33ms"), note="React frames over 33 ms in the 60 s window."),
        "react_frame_p95": measure("ms", None, g(C, "frame_p95"), note="React 95th percentile frame interval."),
    }
    return ms


def loads_from_uptime(path):
    """job2-style: 'load before pair N:  ... load average: a, b, c' lines."""
    bef, aft = {}, {}
    for l in pathlib.Path(path).read_text().splitlines():
        m = re.match(r"load (before|after) pair (\d+):.*load average: ([\d.]+)", l)
        if m:
            (bef if m.group(1) == "before" else aft)[int(m.group(2))] = float(m.group(3))
    n = max(bef)
    return [bef[i] for i in range(1, n + 1)], [aft[i] for i in range(1, n + 1)]


def read_power(path):
    t = pathlib.Path(path).read_text()
    prof = re.search(r"Current active profile: (\S+)", t)
    counts = re.findall(r"^\s*\d+ (\S+)\s*$", t, re.M)
    ac = re.search(r"AC online: (\d)", t)
    epp = re.search(r"epp policy0: (\S+)", t)
    return dict(profile=prof.group(1) if prof else None, counts=counts, epp=epp.group(1) if epp else None,
                ac=ac.group(1) if ac else None, raw=t.strip().splitlines())


def after_path(folder):
    """The power snapshot taken after a run: power-after.txt, or power-final.txt (run 4's name)."""
    for name in ("power-after.txt", "power-final.txt"):
        f = pathlib.Path(folder, name)
        if f.exists():
            return f
    return None


def mhz_per_pair(folder):
    mk = pathlib.Path(folder, "markers.txt")
    fq = pathlib.Path(folder, "freq.txt")
    if not (mk.exists() and fq.exists()):
        return None
    sec = lambda s: sum(int(x) * k for x, k in zip(s.split(":"), (3600, 60, 1)))
    span = {}
    for l in mk.read_text().splitlines():
        m = re.match(r"pair (\d+) (start|end) (\S+)", l)
        if m:
            span.setdefault(int(m.group(1)), {})[m.group(2)] = sec(m.group(3))
    # freq.txt rows are "HH:MM:SS MHz" (run 3) or "epoch HH:MM:SS MHz" (run 4): take the last two fields.
    samples = [(sec(parts[-2]), int(parts[-1])) for parts in (l.split() for l in fq.read_text().splitlines() if l.strip())]
    out = []
    for i in sorted(span):
        v = [f for t, f in samples if span[i]["start"] <= t <= span[i]["end"]]
        out.append(round(sum(v) / len(v)) if v else None)
    return out


def run_from_folder(rid, label, date, rust_files, react_files, workload, power, ac, conditions, load_b=None, load_a=None, mhz=None, sources=None):
    R = [parse_rust(pathlib.Path(f).read_text()) for f in rust_files]
    C = [parse_react(pathlib.Path(f).read_text()) for f in react_files]
    run = dict(id=rid, label=label, date=date, pairs=len(R), workload=workload, power=power, ac=ac, conditions=conditions,
               load_before_each_pair=load_b if load_b else "not recorded",
               load_after_each_pair=load_a if load_a else "not recorded",
               mean_cpu_mhz_per_pair=mhz if mhz else "not recorded",
               sources=sources, measures=build_measures(R, C))
    return run


NR = {"tuned_profile": "not recorded", "governor": "not recorded", "epp": "not recorded"}


def run1_runs():
    out = []
    specs = [("1000-9", "run1", "Run 1", dict(messages=1000, prompt_kb=9)),
             ("200-40", "run1-200x40", "Run 1, 200 messages / 40 KB prompt", dict(messages=200, prompt_kb=40)),
             ("10000-9", "run1-10000", "Run 1, 10,000 messages / 9 KB prompt", dict(messages=10000, prompt_kb=9))]
    for key, rid, label, wl in specs:
        rf = [RUN1 / f"stream-{key}-pair{i}-rust.txt" for i in range(1, 6)]
        cf = [RUN1 / f"stream-{key}-pair{i}-react.txt" for i in range(1, 6)]
        lb, la = [], []
        for f in rf:
            t = f.read_text()
            lb.append(float(re.search(r"# load before: ([\d.]+)", t).group(1)))
            la.append(float(re.search(r"# load after: ([\d.]+)", t).group(1)))
        wl.update(window_s=60, comparison="release Rust --bench-render vs headless Chromium, interleaved pairs")
        out.append(run_from_folder(rid, label, "2026-11-11", rf, cf, wl, NR, "unknown",
                                   "Interleaved pairs, load about 4-6, power profile not recorded (probably power saver).",
                                   lb, la, None, [tilde(RUN1)]))
    return out


def run2_run():
    rf = [RUN2 / f"job2-rust-pair{i}.out" for i in range(1, 10)]
    cf = [RUN2 / f"job2-react-pair{i}.json" for i in range(1, 10)]
    lb, la = loads_from_uptime(RUN2 / "job2-load.txt")
    return run_from_folder("run2", "Run 2", "2026-11-11", rf, cf, dict(messages=1000, prompt_kb=9, window_s=60), NR, "unknown",
                           "9 interleaved pairs, load about 3-5, power profile not recorded.", lb, la, None, [tilde(RUN2)])


def power_dict(folder, fallback):
    after = after_path(folder)
    before = pathlib.Path(folder, "power-before.txt")
    p = read_power(after) if after else None
    b = read_power(before) if before.exists() else None
    gov = (p["counts"][0] if p and p["counts"] else fallback.get("governor"))
    epp = (p["epp"] if p and p.get("epp") else (p["counts"][1] if p and len(p["counts"]) > 1 else fallback.get("epp", "not recorded")))
    d = dict(tuned_profile=(p["profile"] if p and p["profile"] else fallback.get("tuned_profile")), governor=gov, epp=epp)
    d["power_after_raw"] = p["raw"] if p else None
    d["power_before_raw"] = b["raw"] if b else None
    return d, (p["ac"] if p else None), (b["ac"] if b else None)


def perf_run(folder, rid, label, fallback, cond):
    pw, ac_a, ac_b = power_dict(folder, fallback)
    ac = "plugged in" if ac_a == "1" and ac_b in ("1", None) else ("unknown" if ac_a is None else "on battery at some point")
    rf = [folder / f"rust-pair{i}.out" for i in range(1, 10)]
    cf = [folder / f"react-pair{i}.json" for i in range(1, 10)]
    lb, la = loads_from_uptime(folder / "load.txt")
    date = "2026-11-11"
    pa = after_path(folder)
    if pa:
        fm = re.search(r"FINAL (\d{4}-\d{2}-\d{2})", pa.read_text())
        if fm:
            date = fm.group(1)
        m = re.search(r"AFTER \w+ (\w+) (\d+) .* (\d{4})", pa.read_text())
        if m:
            mon = {"Jan": 1, "Feb": 2, "Mar": 3, "Apr": 4, "May": 5, "Jun": 6, "Jul": 7, "Aug": 8, "Sep": 9, "Oct": 10, "Nov": 11, "Dec": 12}.get(m.group(1))
            if mon:
                date = f"{m.group(3)}-{mon:02d}-{int(m.group(2)):02d}"
    return run_from_folder(rid, label, date, rf, cf, dict(messages=1000, prompt_kb=9, window_s=60), pw, ac, cond, lb, la,
                           mhz_per_pair(folder), [tilde(folder)])


def complete(folder):
    return (folder.is_dir() and all((folder / f"rust-pair{i}.out").exists() and (folder / f"react-pair{i}.json").exists() for i in range(1, 10))
            and after_path(folder) is not None and (folder / "load.txt").exists())


def web():
    def one(name, extra):
        d = json.loads((HAIKU / name).read_text())
        load = re.findall(r"load average: ([\d.]+)", (HAIKU / extra).read_text())
        runs = {}
        for k, v in d["runs"].items():
            runs[k] = dict(first_paint_ms=[r3(x["firstPaintMs"]) for x in v], wasm_memory_mib=[x["wasmMemoryMiB"] for x in v],
                           fps=[r3(x["fps"]) for x in v], console_errors=[len(x["errors"]) for x in v],
                           first_paint_ms_median=r3(statistics.median(x["firstPaintMs"] for x in v)),
                           fps_median=r3(statistics.median(x["fps"] for x in v)))
        return dict(source=tilde(HAIKU / name), history_messages=d["history"], load_1min_before=float(load[0]) if load else None,
                    load_1min_all=[float(x) for x in load], runs_per_scenario=len(next(iter(d["runs"].values()))), scenarios=runs), d["sizes"]
    j1, sz = one("job1-measure-web.json", "job1-load.txt")
    j4, sz4 = one("job4-measure-web-fixed.json", "job4-load.txt")
    assert sz == sz4
    return dict(
        wasm_bytes=dict(raw=sz["raw"], gzip=sz["gzip"], brotli=sz["brotli"]),
        wasm_mib=dict(raw=r3(sz["raw"] / 1048576), gzip=r3(sz["gzip"] / 1048576), brotli=r3(sz["brotli"] / 1048576)),
        measurements=dict(
            job1=dict(j1, note="fps for the empty thread was recorded as 0 (harness did not drive frames then); use job4 for fps. One 404 console error (favicon-type resource) on the first empty run in both files."),
            job4_fps_fixed=dict(j4, note="fps harness fixed; empty-thread and 10,000-message fps. Browser tab, software-rendered WebAssembly build.")))


def tests():
    summary = (HAIKU / "job3-summary.txt").read_text()
    runs = []
    for i in (1, 2, 3):
        t = (HAIKU / f"job3-test-run{i}.log").read_text()
        tot = dict(passed=0, failed=0, ignored=0)
        for m in re.finditer(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", t):
            tot["passed"] += int(m.group(1)); tot["failed"] += int(m.group(2)); tot["ignored"] += int(m.group(3))
        tot["test_result_lines"] = len(re.findall(r"test result:", t))
        runs.append(dict(run=i, **tot))
    return dict(command="cargo test --workspace --offline --release, 3 sequential runs, all exit 0", date="2026-11-11",
                source=tilde(HAIKU / "job3-summary.txt"), runs=runs, summary_text=summary.strip().splitlines())


def build():
    runs = run1_runs()[:1] + [run2_run()]
    runs.append(perf_run(RUN3, "run3", "Run 3 (performance profile)", dict(tuned_profile="throughput-performance", governor="performance", epp="not recorded"),
                         "9 interleaved pairs, tuned throughput-performance, governor performance, plugged in, load 0.5-2.9."))
    gen = [tilde(RUN1), tilde(RUN2), tilde(RUN3)]
    if complete(RUN4):
        runs.append(perf_run(RUN4, "run4", "Run 4 (balanced profile)", dict(tuned_profile="balanced", governor="powersave", epp="balance_performance"),
                             "9 interleaved pairs, tuned balanced, governor powersave, energy_performance_preference balance_performance, plugged in."))
        gen.append(tilde(RUN4))
    else:
        NOTE.append("run 4 (%s) not complete (need 9 rust + 9 react files, power-after.txt or power-final.txt, load.txt): skipped" % tilde(RUN4))
    runs += run1_runs()[1:]
    results = json.loads((ROOT / "docs/results.json").read_text())
    earlier = [r for r in results["rows"] if r["id"].startswith("fx") or r["id"].startswith("gpu-")]
    gen += [tilde(HAIKU), "docs/results.json (earlier long-fixture and GPU rows, copied as given)"]
    return {
        "schema": 1,
        "generated_from": gen,
        "generated_by": "benchmarks/results/make-runs-json.py",
        "runs": runs,
        "web": web(),
        "tests": tests(),
        "earlier_rows": earlier,
        "notes": [
            "All values are raw per-pair numbers from the stored files; medians, ranges and ratios are computed by make-runs-json.py. Ratios are floored to one decimal, never rounded up.",
            "ratio_floor_median = floor(median(react)/median(rust)); least/most favourable = min/max of the per-pair ratios (ratio_per_pair_floor lists each pair).",
            "Open times differ between runs because of power profile and machine load; React's open time is more CPU-speed sensitive than Rust's. Show ranges, not single numbers.",
            "CPU totals and single-event stalls are load-sensitive and are not headline figures.",
            "Runs 1 and 2 did not record the power profile; run 1 'plugged in' is unknown. Runs 'run1-200x40' and 'run1-10000' are other workloads of run 1 (5 pairs each).",
            "memory_end_pss uses the Rust RSS as the Rust value (Rust has no PSS measurement).",
            "Earlier rows (fx*, gpu-*) keep their own conditions; see their 'conditions' fields.",
        ],
    }


def main():
    text = json.dumps(build(), indent=1, ensure_ascii=False) + "\n"
    out = ROOT / "docs/runs.json"
    for n in NOTE:
        print("note:", n)
    if "--check" in sys.argv:
        if not out.exists() or out.read_text() != text:
            print("docs/runs.json is out of date: run python3 benchmarks/results/make-runs-json.py")
            sys.exit(1)
        print("docs/runs.json is up to date")
    else:
        out.write_text(text)
        print("wrote", out)


main()
