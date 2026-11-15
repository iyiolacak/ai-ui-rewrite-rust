#!/usr/bin/env python3
"""Read the raw files of run-pairs.sh and print one JSON document: every run, then per-pair ratios and medians.

    python3 summarize.py RAW_DIR [RAW_DIR ...]      (a directory holds stream-*/scroll-*/typing-*/memory-* files)
"""
import json
import pathlib
import re
import statistics
import sys

UNIT = {"ns": 1e-6, "µs": 1e-3, "ms": 1.0, "s": 1e3}


def dur(text):
    m = re.fullmatch(r"([\d.]+)(ns|µs|ms|s)", text)
    return float(m[1]) * UNIT[m[2]]


def rust_run(text):
    out = {"engine": "rust"}
    if m := re.search(r"history import\+first sync ([\d.]+) ms, formatted after ([\d.]+) s; reply streamed in ([\d.]+) s over (\d+) iterations", text):
        out.update(open_ms=float(m[1]), formatted_after_s=float(m[2]), open_formatted_ms=float(m[1]) + 1e3 * float(m[2]),
                   streamed_s=float(m[3]), iterations=int(m[4]))
    elif m := re.search(r"history import\+first sync ([\d.]+) ms, formatted after ([\d.]+) s", text):
        out.update(open_ms=float(m[1]), formatted_after_s=float(m[2]), open_formatted_ms=float(m[1]) + 1e3 * float(m[2]))
    if m := re.search(r"rss ([\d.]+) MiB, peak ([\d.]+) MiB after opening the thread; rss ([\d.]+) MiB, peak ([\d.]+) MiB at the end; process CPU while streaming (\d+) ms", text):
        out.update(rss_opened_mib=float(m[1]), rss_end_mib=float(m[3]), rss_peak_mib=float(m[4]), cpu_ms=float(m[5]))
    if m := re.search(r"per iteration: p50 (\S+)\s+p95 (\S+)\s+p99 (\S+)\s+max (\S+)\s+total ([\d.]+) ms; iterations over 4 ms: (\d+)", text):
        out.update(iter_p50_ms=dur(m[1]), iter_p95_ms=dur(m[2]), iter_p99_ms=dur(m[3]), iter_max_ms=dur(m[4]),
                   iter_total_ms=float(m[5]), iters_over_4ms=int(m[6]))
    if m := re.search(r"SCROLL (\d+) messages: (\d+) notches of ([\d.]+) px.*?frames (\d+);.*?p50 ([\d.]+) ms p95 ([\d.]+) ms p99 ([\d.]+) ms max ([\d.]+) ms; frames over 16.7 ms: (\d+); rss ([\d.]+) MiB", text):
        out.update(notches=int(m[2]), notch_px=float(m[3]), drawn=int(m[4]), frame_work_p50_ms=float(m[5]),
                   frame_work_p95_ms=float(m[6]), frame_work_p99_ms=float(m[7]), frame_work_max_ms=float(m[8]),
                   frames_over_16_7=int(m[9]), rss_after_scroll_mib=float(m[10]))
    if m := re.search(r"KEYS sent (\d+) painted (\d+) key-to-paint p50 ([\d.]+) ms p95 ([\d.]+) ms max ([\d.]+) ms all_ms (\[.*?\])", text):
        out.update(keys_sent=int(m[1]), keys_painted=int(m[2]), key_p50_ms=float(m[3]), key_p95_ms=float(m[4]),
                   key_max_ms=float(m[5]), key_all_ms=json.loads(m[6]))
    samples = [(int(a), float(b), float(c)) for a, b, c in re.findall(r"SAMPLE t=(\d+)s cpu_ms=(\d+) rss ([\d.]+) MiB", text)]
    if samples:
        out["samples"] = [{"t_s": t, "cpu_ms": c, "rss_mib": r} for t, c, r in samples]
    return out


def react_run(text):
    for line in text.splitlines():
        if line.startswith("{"):
            return json.loads(line)
    return {"engine": "react", "failed": True, "tail": text.strip().splitlines()[-3:]}


def load(d):
    runs = {}
    for f in sorted(pathlib.Path(d).glob("*-pair*-*.txt")):
        mode, hist, kb, pair, side = re.fullmatch(r"(\w+)-(\d+)-(\d+)-pair(\d+)-(\w+)\.txt", f.name).groups()
        text = f.read_text()
        run = rust_run(text) if side == "rust" else react_run(text)
        run["exit"] = (re.search(r"# exit (\d+)", text) or [0, "?"])[1]
        run["load_before"] = (re.search(r"# load before: (.*)", text) or [0, ""])[1]
        runs.setdefault((mode, int(hist), int(kb)), {}).setdefault(int(pair), {})[side] = run
    return runs


def react_value(run, key):
    return {
        "open_ms": run.get("open_ms"), "cpu_ms": run.get("stream_process_cpu_ms"),
        "rss_end_mib": (run.get("memory_end") or {}).get("rss_mib"), "rss_peak_mib": (run.get("memory_peak") or {}).get("rss_mib"),
        "pss_end_mib": (run.get("memory_end") or {}).get("pss_mib"), "iter_max_ms": run.get("frame_max_ms"),
        "iter_p95_ms": run.get("frame_p95_ms"),
    }.get(key)


if __name__ == "__main__":
    result = {}
    for d in sys.argv[1:]:
        for (mode, hist, kb), pairs in load(d).items():
            result[f"{pathlib.Path(d).name}/{mode}-{hist}-{kb}"] = {str(p): v for p, v in sorted(pairs.items())}
    print(json.dumps(result, indent=1))
