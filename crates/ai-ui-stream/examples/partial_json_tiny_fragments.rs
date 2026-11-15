//! Bounded synthetic append workload; not an end-to-end frame-pacing benchmark.
use ai_ui_stream::partial_json::{Limits, Parser};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    run(false);
    run(true);
}

fn run(staged_clone: bool) {
    let mut parser = Parser::new(Limits::default()).unwrap();
    parser.append("{\"value\":\"").unwrap();
    let start = Instant::now();
    for _ in 0..65_536 {
        if staged_clone {
            parser = parser.clone().appended(black_box("x")).unwrap();
        } else {
            parser.append(black_box("x")).unwrap();
        }
    }
    parser.append("\"}").unwrap();
    black_box(parser.snapshot());
    println!(
        "65,536 one-byte appends (staged clone: {staged_clone}): {:?}",
        start.elapsed()
    );
}
