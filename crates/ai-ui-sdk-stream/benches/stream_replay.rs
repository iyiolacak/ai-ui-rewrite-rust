//! CPU-only replay. It measures no UI, network latency, GPU or display behavior.

#[path = "../examples/support/mod.rs"]
mod support;

use serde_json::json;

#[allow(
    clippy::print_stdout,
    reason = "This benchmark's sole output is its machine-readable report"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wire = String::from("data: {\"type\":\"text-start\",\"id\":\"t\"}\n\n");
    for _ in 0..10_000 {
        wire.push_str("data: {\"type\":\"text-delta\",\"id\":\"t\",\"delta\":\"abcdefghijklmnopqrstuvwx\"}\n\n");
    }
    wire.push_str("data: {\"type\":\"text-end\",\"id\":\"t\"}\n\ndata: [DONE]\n\n");
    let mut patch_times = Vec::new();
    let mut snapshot_times = Vec::new();
    for run in 0..9 {
        let order = if run % 2 == 0 {
            ["patch", "snapshot"]
        } else {
            ["snapshot", "patch"]
        };
        for mode in order {
            let request = json!({"raw_utf8":wire,"chunk_bytes":4096,"mode":mode});
            let result = support::replay(&request)?;
            let time = result["elapsed_ms"]
                .as_f64()
                .ok_or("missing elapsed time")?;
            std::hint::black_box(result);
            if mode == "patch" {
                patch_times.push(time);
            } else {
                snapshot_times.push(time);
            }
        }
    }
    patch_times.sort_by(f64::total_cmp);
    snapshot_times.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({
            "scope":"native CPU only; same Rust implementation, different snapshot demand",
            "runs_per_mode":9,"deltas":10_000,"delta_bytes":24,
            "patch_ms":patch_times,"snapshot_ms":snapshot_times,
            "warning":"This is not a React, browser-rendering, or end-to-end speedup benchmark."
        })
    );
    Ok(())
}
