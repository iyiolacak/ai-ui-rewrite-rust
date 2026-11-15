//! Cost of the state projection while a reply streams
//! (`cargo test --release -p ai-ui-state --test perf -- --ignored --nocapture`).

use ai_ui_core::adapter::{self, RunStream};
use ai_ui_core::*;
use ai_ui_state::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

fn history(messages: usize) -> Vec<ThreadMessage> {
    (0..messages)
        .map(|i| {
            serde_json::from_value(serde_json::json!({
                "role": if i % 2 == 0 { "user" } else { "assistant" }, "id": format!("m{i}"), "createdAt": 0,
                "content": [{ "type": "text", "text": "history message with a few words in it" }],
                "status": { "type": "complete", "reason": "stop" }
            }))
            .unwrap()
        })
        .collect()
}

fn view(messages: usize, updates: usize, smooth: bool) -> ThreadView {
    let runtime = LocalThreadRuntime::new(
        move |_o: ChatModelRunOptions| -> RunStream {
            let mut text = String::new();
            adapter::from_results((0..updates).map(move |_| {
                text.push_str("token ");
                Ok(ChatModelRunResult::text(text.clone()))
            }))
        },
        LocalRuntimeOptions::default(),
    );
    runtime
        .import(ExportedRepository::from_linear(history(messages)))
        .unwrap();
    ThreadView::new(
        Rc::new(runtime),
        ViewOptions {
            smooth: smooth.then(SmoothOptions::default),
        },
    )
}

#[test]
#[ignore]
fn projection_cost_per_streamed_update() {
    for smooth in [false, true] {
        for messages in [10usize, 100, 1000, 5000] {
            let updates = 1000;
            let view = view(messages, updates, smooth);
            // A UI that re-reads the state on every change.
            let v = view.clone();
            let _sub = view.subscribe(move || {
                let _ = v.state();
            });
            let t = Instant::now();
            futures_executor::block_on(view.send("go").unwrap().unwrap()).unwrap();
            println!(
                "smooth={smooth:<5} {messages:>5} messages: {:>8.1} us per streamed update (runtime + projection)",
                t.elapsed().as_secs_f64() * 1e6 / updates as f64
            );
        }
    }
}

// Both modes maintain the same host row snapshot. The comparison changes only
// how the host discovers changed rows; it includes runtime and view projection,
// but no Markdown work, Slint, layout, rendering, network or process startup.
fn reconcile_host(
    shown: &mut Vec<Arc<MessageState>>,
    state: &ThreadState,
    indices: Option<&[usize]>,
) -> usize {
    if let Some(indices) = indices {
        for &index in indices {
            shown[index] = state.messages[index].clone();
        }
        indices.len()
    } else {
        shown.truncate(state.messages.len());
        for (index, row) in state.messages.iter().enumerate() {
            if index == shown.len() {
                shown.push(row.clone());
            } else if !Arc::ptr_eq(&shown[index], row) {
                shown[index] = row.clone();
            }
        }
        state.messages.len()
    }
}

fn host_sync_sample(messages: usize, updates: usize, changes: bool) -> (f64, usize) {
    let view = view(messages, updates, false);
    let initial = view.state_changes_since(None);
    let cursor = Rc::new(RefCell::new(initial.cursor));
    let shown = Rc::new(RefCell::new(initial.state.messages.clone()));
    drop(initial.state);
    let inspected = Rc::new(Cell::new(0));
    let _subscription = view.subscribe({
        let (view, cursor, shown, inspected) = (
            view.clone(),
            cursor.clone(),
            shown.clone(),
            inspected.clone(),
        );
        move || {
            let visited = if changes {
                let changed = view.state_changes_since(Some(&cursor.borrow()));
                let visited = reconcile_host(
                    &mut shown.borrow_mut(),
                    &changed.state,
                    changed.indices.as_deref(),
                );
                *cursor.borrow_mut() = changed.cursor;
                visited
            } else {
                reconcile_host(&mut shown.borrow_mut(), &view.state(), None)
            };
            inspected.set(inspected.get() + visited);
        }
    });
    let started = Instant::now();
    futures_executor::block_on(view.send("go").unwrap().unwrap()).unwrap();
    let micros = started.elapsed().as_secs_f64() * 1e6 / updates as f64;
    let final_state = view.state();
    assert_eq!(&*shown.borrow(), &final_state.messages);
    assert_eq!(
        final_state.messages.last().unwrap().text,
        "token ".repeat(updates)
    );
    (micros, inspected.get())
}

#[test]
#[ignore = "manual balanced comparison of row scanning and the view change feed"]
fn host_reconciliation_cost_per_streamed_update() {
    let updates = 1000;
    for messages in [1000, 10_000] {
        let mut scan = Vec::new();
        let mut feed = Vec::new();
        // One warmup of each mode, then nine interleaved pairs. Alternate
        // which mode runs first and keep every timed sample in the output.
        for pair in 0..=9 {
            let order = if pair % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            for changes in order {
                let (micros, inspected) = host_sync_sample(messages, updates, changes);
                if pair > 0 {
                    if changes {
                        feed.push(micros);
                    } else {
                        scan.push(micros);
                    }
                    println!(
                        "history={messages} pair={pair} mode={} us_per_update={micros:.3} rows_examined={inspected}",
                        if changes { "feed" } else { "scan" },
                    );
                }
            }
        }
        scan.sort_by(f64::total_cmp);
        feed.sort_by(f64::total_cmp);
        println!(
            "history={messages} updates={updates} median_us_per_update scan={:.3} feed={:.3}; runtime + projection + host reconciliation, no rendering",
            scan[scan.len() / 2],
            feed[feed.len() / 2],
        );
    }
}
