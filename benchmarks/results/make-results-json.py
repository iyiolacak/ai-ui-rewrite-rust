#!/usr/bin/env python3
"""Writes docs/results.json. Published rows are copied from docs/STATUS.md (2026-11-10); new rows come from
the raw runs summarised in benchmarks/results/summary-run1.json (raw files: ~/.cache/ai-ui-rewrite-rust/results/results-run1/)."""
import json, pathlib, re, statistics
root = pathlib.Path(__file__).resolve().parents[2]
S = "docs/STATUS.md at 2cbedad (6add923 React comparison, 54c371f scale, 48bc8ec real window); raw runs were not kept"
N = "benchmarks/results/summary-run1.json (raw: results-run1/a-2000, 3 Rust runs, 2 of 3 React runs finished); hist build = 2cbedad + origin/demo/long-threads loader + benchmarks/results/rust-bench-extras.patch"
FX = "words.py seed 7 fixture, 2,000 messages, 12.2 MB of text, long structured replies, 9 KB prompt, 60 s window, headless Chromium vs --bench-render, 3 interleaved pairs, machine load 6-25"
def row(id, workload, measure, unit, rust, react, source, date, conditions, drivers, rr=None, cr=None, **kw):
    d = dict(id=id, group="vs_react", workload=workload, measure=measure, unit=unit, rust=rust, react=react,
             rust_range=rr or [rust, rust], react_range=cr or [react, react], source=source, date=date,
             conditions=conditions, drivers=drivers)
    d.update(kw)
    return d
rows = [
 row("fx2000-open", "2,000 long structured messages (12.2 MB)", "open the thread", "ms", 527.1, 42090, N, "2026-11-10", FX + ". Rust = import incl. JSON parse + first sync + Markdown formatted (open_ms + formatted_after_s of each raw run: 485.5, 527.1, 528.3 ms; first sync alone was 375.5 to 417.1 ms and is not comparable); React = navigation until all messages are in the DOM (that old harness did not check Markdown, so this is if anything a lower bound for React). Third React run did not finish within the 120 s harness limit and is excluded from the range, so the range understates React.", "React renders every message as DOM and parses all Markdown on the main thread; Rust formats on worker threads.", rr=[485.5, 528.3], cr=[42090, 96584]),
 row("fx2000-stall", "2,000 long structured messages", "longest single iteration or frame while streaming", "ms", 41.5, 1633.2, N, "2026-11-10", FX + ". Rust = longest iteration (app work + layout + software paint, unpaced); React = longest frame interval. Single-event stalls are noisy: all runs shown in the range.", "React main thread saturated (61 s busy in a 60 s window).", rr=[35.0, 74.8], cr=[1633.2, 4583.1]),
 row("fx2000-mem", "2,000 long structured messages", "resident memory at the end", "MiB", 88.1, 2946.3, N, "2026-11-10", FX + ". React = RSS summed over all Chromium processes.", "Rust keeps parsed text once and instantiates Slint rows; React keeps DOM, JS heap (1.47 GiB in run 1) and Markdown tree.", rr=[87.8, 88.4], cr=[2946.3, 3449.1]),
 row("fx2000-cpu", "2,000 long structured messages", "process CPU while streaming 60 s", "ms", 9130, 60260, N, "2026-11-10", FX + ". Rust includes software raster; React plain CSS, whole Chromium process tree.", "React re-renders and re-lays out the long thread on each streamed update.", rr=[8780, 9750], cr=[60260, 65430]),
 row("fx5000-open", "5,000 long structured messages (29.7 MB)", "open the thread", "ms", 1256.7, 120000, "benchmarks/results/summary-run1.json (raw: results-run1/a-5000, 3/3 pairs)", "2026-11-10", "Same fixture family, 3 interleaved pairs. React did NOT have all 5,000 messages in the DOM within the harness's 120 s limit in 3 of 3 runs, so 120 s is a floor for React, not a measurement; Rust = import + first sync + Markdown formatted, value its slowest of 3 runs (807 to 1257 ms; first sync alone was 697 to 1117 ms). The ratio is therefore a lower bound.", "As above, growing with thread size.", rr=[807.3, 1256.7], cr=[120000, 120000], lower_bound=True),
 dict(id="gpu-cpu", group="rust_only", workload="1000 messages, ~9 KB reply, real GPU window (FemtoVG-WGPU, Intel UHD, Wayland, 30 s)", measure="process CPU", unit="s",
      rust=14.1, react=25.5, rust_range=[11.8, 14.1], react_range=[23.1, 25.5], better="lower", source=S, date="2026-11-10",
      conditions="Rust now (femtovg pipeline-cache patch) vs Rust before; NOT a React comparison. Interleaved runs.", drivers="femtovg recompiled pipelines every few frames."),
 dict(id="gpu-fps", group="rust_only", workload="same", measure="median frame rate", unit="fps",
      rust=141, react=105, rust_range=[141, 142], react_range=[105, 108], better="higher", source=S, date="2026-11-10",
      conditions="Rust now vs Rust before; display limit 142 fps.", drivers="same"),
]
L = "benchmarks/results/like-for-like/raw (5 interleaved pairs, release build of 424a4fb plus the open reporting of this change; summary: benchmarks/results/like-for-like/summarize.py)"
LC = "Median of 5 interleaved pairs (range = min to max over the 5 runs), machine load about 4-6, 60 s window, headless Chromium vs --bench-render. "
def ll(id, workload, measure, unit, rust, react, rr, cr, cond, drv, **kw):
    return row(id, workload, measure, unit, rust, react, L, "2026-11-11", LC + cond, drv, rr=rr, cr=cr, **kw)
W1, W2, W3 = "1000 messages, 9 KB prompt", "200 messages, 40 KB prompt", "10,000 messages, 9 KB prompt"
OPEN = "Like for like: Rust = until the last message's Markdown is formatted and painted at least once (import + first sync + formatting on worker threads + a forced frame); React = navigation until all messages are in the DOM with their Markdown rendered (every reply's table present) plus two frames. "
rows += [
 ll("pub-open-1000", W1, "open the thread (like for like: Markdown formatted and painted)", "ms", 105.3, 2940, [100.8, 150.9], [2794, 2987], OPEN + "Rust first sync alone (rows visible, Markdown not yet formatted) was 12.9 ms (11.1 to 28.6) and is NOT comparable to React's figure.", "Rust formats Markdown on worker threads and lays out only visible rows; React builds all DOM and parses all Markdown on the main thread."),
 ll("pub-open-200x40", W2, "open the thread (like for like: Markdown formatted and painted)", "ms", 33.2, 1104, [31.8, 46.6], [1029, 1197], OPEN + "Rust first sync alone: 2.1 ms (1.5 to 6.1), not comparable.", "As above."),
 ll("pub-open-10000", W3, "open the thread (NOT like for like: Rust formatted the newest 1,200 messages, React all)", "ms", 435.5, 26081, [382.9, 472.1], [23591, 27196], OPEN + "Rust first sync alone: 147.8 ms (138.0 to 207.4), not comparable.", "As above, growing with thread size."),
 ll("pub-stall-1000", W1, "longest iteration/frame while streaming", "ms", 34.0, 916.6, [17.5, 35.4], [866.6, 1183.3], "Rust = longest loop iteration (unpaced), React = longest animation-frame interval. React also had 105 frames over 33 ms in run 1.", "Main thread saturation in React."),
 ll("pub-stall-200x40", W2, "longest stall", "ms", 109.9, 2399.9, [36.2, 136.2], [2183.2, 2649.9], "Rust = longest loop iteration (unpaced), React = longest frame interval. Rust varied 36 to 136 ms between runs.", "Long reasoning text: Rust shows it as cached chunks laid out once; React re-wraps and re-lays out."),
 ll("pub-stall-10000", W3, "longest stall", "ms", 41.1, 11599.5, [28.3, 46.0], [9833, 12899.5], "Rust = longest loop iteration (unpaced), React = longest frame interval (median frame 183 ms in run 1).", "Main thread saturation in React."),
 ll("pub-mem-1000", W1, "memory at the end (RSS)", "MiB", 30.1, 1481.5, [29.7, 30.4], [1474.5, 1526.6], "React RSS over all 9 Chromium processes.", "DOM plus JS heap."),
 ll("pub-mem-1000-pss", W1, "memory at the end (Rust RSS vs React PSS)", "MiB", 30.1, 850.3, [29.7, 30.4], [841.8, 895.4], "Conservative: React's PSS, which discounts shared pages.", "As above."),
 ll("pub-mem-200x40", W2, "memory at the end (RSS)", "MiB", 24.6, 1212.7, [24.5, 24.8], [1210, 1268], "React RSS over all Chromium processes.", "As above."),
 ll("pub-mem-200x40-pss", W2, "memory at the end (Rust RSS vs React PSS)", "MiB", 24.6, 584, [24.5, 24.8], [582.4, 640.9], "Conservative PSS comparison.", "As above."),
 ll("pub-mem-10000", W3, "memory at the end (RSS)", "MiB", 63.6, 3717.8, [63.3, 64.0], [3675.4, 3926], "React RSS over all Chromium processes; PSS 3089 MiB (3051 to 3299).", "As above."),
 ll("pub-mem-10000-pss", W3, "memory at the end (Rust RSS vs React PSS)", "MiB", 63.6, 3088.9, [63.3, 64.0], [3051.1, 3298.6], "Conservative PSS comparison.", "As above."),
 ll("pub-cpu-1000", W1, "process CPU while streaming 60 s", "ms", 17160, 73260, [13700, 17650], [73050, 73540], "Rust includes software raster; React is the whole Chromium process tree (main thread about 60 s, saturated).", "React main thread."),
 ll("pub-cpu-200x40", W2, "process CPU while streaming 60 s", "ms", 17020, 51160, [13660, 19450], [41900, 57120], "Rust includes software raster; Rust varied 13.7 to 19.5 s between runs.", "Software raster is included on the Rust side."),
 ll("pub-cpu-10000", W3, "process CPU while streaming 60 s", "ms", 22800, 83500, [20930, 23220], [76060, 84710], "Rust includes software raster; React is the whole Chromium process tree.", "React main thread."),
]

# Second, independent run (2026-11-11, lighter machine load): parsed from the raw files, never replacing run 1.
R2 = root / "benchmarks" / "results" / "like-for-like-run2"
def _runs():
    out = {k: [] for k in ("ropen", "rcpu", "rrss", "rstall", "copen", "ccpu", "cstall", "crss", "cpss")}
    for i in range(1, 10):
        t = (R2 / f"job2-rust-pair{i}.out").read_text()
        out["ropen"].append(float(re.search(r"like for like\) ([\d.]+) ms", t).group(1)))
        out["rcpu"].append(float(re.search(r"process CPU while streaming (\d+) ms", t).group(1)))
        out["rrss"].append(float(re.search(r"rss ([\d.]+) MiB, peak [\d.]+ MiB at the end", t).group(1)))
        out["rstall"].append(float(re.search(r"max ([\d.]+)ms", t).group(1)))
        j = json.loads((R2 / f"job2-react-pair{i}.json").read_text())
        out["copen"].append(j["open_formatted_ms"]); out["ccpu"].append(j["stream_process_cpu_ms"])
        out["cstall"].append(j["frame_max_ms"]); out["crss"].append(j["memory_end"]["rss_mib"]); out["cpss"].append(j["memory_end"]["pss_mib"])
    return out
def _md(xs, nd=1):
    return round(statistics.median(xs), nd), [round(min(xs), nd), round(max(xs), nd)]
L2 = "benchmarks/results/like-for-like-run2 (9 interleaved pairs, release build, job2-run-pairs.sh; second independent run, summary job2-summary.txt)"
LC2 = ("RUN 2, a second independent run: median of 9 interleaved pairs (range = min to max over the 9 runs), machine load about 3-5 "
       "(lighter than run 1's 4-6), 60 s window, headless Chromium vs --bench-render, 1000 messages, 9 KB prompt. ")
X = _runs()
def l2(id, measure, unit, rk, ck, cond, drv, nd=1):
    r, rr = _md(X[rk], nd); c, cr = _md(X[ck], nd)
    return row(id, W1, measure, unit, r, c, L2, "2026-11-11", LC2 + cond, drv, rr=rr, cr=cr)
rows += [
 l2("run2-open-1000", "open the thread (like for like: Markdown formatted and painted), run 2", "ms", "ropen", "copen", OPEN + "Same definitions as run 1.", "As run 1."),
 l2("run2-stall-1000", "longest iteration/frame while streaming, run 2", "ms", "rstall", "cstall", "Rust = longest loop iteration (unpaced), React = longest animation-frame interval.", "Main thread saturation in React.", 2),
 l2("run2-mem-1000", "memory at the end (RSS), run 2", "MiB", "rrss", "crss", "React RSS over all 9 Chromium processes.", "DOM plus JS heap."),
 l2("run2-mem-1000-pss", "memory at the end (Rust RSS vs React PSS), run 2", "MiB", "rrss", "cpss", "Conservative: React's PSS, which discounts shared pages.", "As above."),
 l2("run2-cpu-1000", "process CPU while streaming 60 s, run 2", "ms", "rcpu", "ccpu", "Rust includes software raster; React is the whole Chromium process tree. CPU is load-sensitive: Rust median 9.7 s here against 17.2 s in run 1 (load 3-5 against 4-6), so this row is not a headline figure.", "React main thread."),
]
(root / "docs" / "results.json").write_text(json.dumps({"schema": 1, "rows": rows}, indent=1) + "\n")
