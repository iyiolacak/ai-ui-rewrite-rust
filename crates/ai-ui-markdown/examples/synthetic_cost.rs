//! Local synthetic baseline, not a device, browser or production benchmark.
use ai_ui_markdown::presentation::project;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let paragraph = "## Result\n\nA **bold** finding with a [reference](https://example.com/).\n\n- First item\n- Second item\n\n```rust\nlet answer = 42;\n```\n\n";
    for repeats in [8, 80, 400] {
        let source = paragraph.repeat(repeats);
        let mut samples = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            let output = project(black_box(&source)).expect("synthetic input fits limits");
            black_box(output);
            samples.push(start.elapsed().as_micros());
        }
        samples.sort_unstable();
        println!(
            "bytes={} iterations={} p50_us={} p95_us={}",
            source.len(),
            samples.len(),
            samples[15],
            samples[28]
        );
    }
    let source = paragraph.repeat(80);
    let increments = 40;
    let start = Instant::now();
    for step in 1..=increments {
        let end = source.len() * step / increments;
        // The fixture contains ASCII only, so each prefix ends on a UTF-8 boundary.
        black_box(project(black_box(&source[..end])).expect("synthetic prefix fits limits"));
    }
    println!(
        "stream_bytes={} snapshots={} total_us={}",
        source.len(),
        increments,
        start.elapsed().as_micros()
    );
}
