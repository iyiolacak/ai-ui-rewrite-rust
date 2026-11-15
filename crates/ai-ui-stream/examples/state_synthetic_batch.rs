//! Synthetic host-only probe; this does not measure UI or real workloads.
use ai_ui_stream::state::{Limits, Operation, State};
use std::time::Instant;

fn main() {
    let mut state = State::new(
        serde_json::json!({ "large": vec![0; 99_990], "text": "" }),
        Limits::default(),
    )
    .unwrap();
    let operations = vec![
        Operation::AppendText {
            path: vec!["text".into()],
            value: "x".into(),
        };
        256
    ];
    let started = Instant::now();
    state.apply(&operations).unwrap();
    println!("{} ms", started.elapsed().as_millis());
}
