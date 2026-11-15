//! `--bench N`: stream a reply into a thread of N messages with the offscreen
//! platform and report how long each loop iteration of the app's own work
//! takes (state projection, Slint model updates, Markdown polling, smooth
//! streaming). Rendering is not included unless `--bench-render` is given:
//! then each iteration also lays out and paints the window with the software
//! renderer (at most 60 times a second, like a display), so item creation and
//! layout of long rows are counted too.

use crate::app::App;
use crate::Tasks;
use ai_ui_core::{ExportedRepository, ThreadList, ThreadMessage};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor};
use std::rc::Rc;
use std::task::{Context, Waker};
use std::time::{Duration, Instant};

const REPLY: &str = "Here's a quick answer.\n\nThis reply comes from a local model, streamed token by token.\n\n\
- Edit a message or regenerate an answer\n- Every update is a cumulative result\n- Stop the stream at any time\n\n\
```rust\nlet runtime = LocalThreadRuntime::new(adapter, options);\n```\n\n\
| Layer | Crate |\n|---|---|\n| Runtime | core |\n| UI | slint |\n";

/// A prompt that makes the demo model answer with about `kb` kilobytes of Markdown paragraphs.
pub(crate) fn long_prompt(kb: usize) -> String {
    let paragraph = "A paragraph of ordinary prose with a `code span`, some *emphasis* and a [link](https://example.com).\n\n";
    paragraph.repeat(kb * 1024 / paragraph.len())
}

pub(crate) fn history(messages: usize) -> Vec<ThreadMessage> {
    (0..messages)
        .map(|i| {
            let user = i % 2 == 0;
            serde_json::from_value(serde_json::json!({
                "role": if user { "user" } else { "assistant" }, "id": format!("h{i}"), "createdAt": 0,
                "content": [{ "type": "text", "text": if user { "A question about the thread" } else { REPLY } }],
                "status": { "type": "complete", "reason": "stop" }
            }))
            .expect("valid message")
        })
        .collect()
}

/// A settings change for `--bench-settings`.
pub enum Probe {
    Scale(f32),
    Width(f32),
}

fn pump(tasks: &Tasks, millis: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(millis) {
        slint::platform::update_timers_and_animations();
        tasks.run();
        std::thread::sleep(Duration::from_micros(500));
    }
}

/// A 60 Hz display: a real window and a browser draw at most once per refresh.
const REFRESH: Duration = Duration::from_micros(16_667);

/// Offscreen frame: lay out and paint into a reused buffer when anything
/// changed, at most once per display refresh.
pub struct Render {
    window: Rc<MinimalSoftwareWindow>,
    buffer: Vec<PremultipliedRgbaColor>,
    width: usize,
    last: Option<Instant>,
}

impl Render {
    pub fn new(window: Rc<MinimalSoftwareWindow>, width: u32, height: u32) -> Self {
        Self {
            window,
            buffer: vec![PremultipliedRgbaColor::default(); (width * height) as usize],
            width: width as usize,
            last: None,
        }
    }
    /// Draw now, ignoring the refresh cap: how long layout and paint took.
    fn force(&mut self) -> Duration {
        self.last = None;
        let start = Instant::now();
        self.frame();
        start.elapsed()
    }
    /// Draw if anything changed; how long layout and paint took, if it drew.
    fn frame(&mut self) -> Option<Duration> {
        let start = Instant::now();
        if self.last.is_some_and(|last| start - last < REFRESH) {
            return None;
        }
        let (buffer, width) = (&mut self.buffer, self.width);
        self.window
            .draw_if_needed(|renderer| {
                renderer.render(buffer, width);
            })
            .then(|| {
                self.last = Some(start);
                start.elapsed()
            })
    }
}

fn frame(render: &mut Option<Render>) -> Option<Duration> {
    render.as_mut().and_then(Render::frame)
}

/// Peak and current resident memory from /proc, in MiB (Linux only).
/// CPU time used by the whole process (every thread), in milliseconds (Linux only).
pub(crate) fn cpu_ms() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return f64::NAN;
    };
    // Fields after the command name: utime and stime are the 12th and 13th.
    let rest = &stat[stat.rfind(')').map_or(0, |i| i + 2)..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    };
    // Linux reports in USER_HZ, which is 100 on every mainstream configuration.
    (ticks(11) + ticks(12)) * 10.0
}

pub(crate) fn memory() -> String {
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return "memory n/a".into();
    };
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(f64::NAN, |kb| kb / 1024.0)
    };
    format!(
        "rss {:.1} MiB, peak {:.1} MiB",
        field("VmRSS:"),
        field("VmHWM:")
    )
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p) as usize]
}

pub fn run(
    app: &Rc<App>,
    list: &ThreadList,
    tasks: &Tasks,
    messages: usize,
    reply_kb: usize,
    mut render: Option<Render>,
    probe: Option<&dyn Fn(Probe)>,
) {
    frame(&mut render);
    let load = Instant::now();
    list.main()
        .import(ExportedRepository::from_linear(history(messages)))
        .expect("import history");
    frame(&mut render);
    let loaded = load.elapsed();
    // Wait for the Markdown worker to format the history; this is the cost of
    // opening a long thread, reported on its own.
    let settle = Instant::now();
    while !app.idle() && settle.elapsed() < Duration::from_secs(60) {
        pump(tasks, 5);
        frame(&mut render);
    }
    let settled = settle.elapsed();
    // Like for like with the React baseline: the last message's Markdown is
    // formatted and has been painted at least once (a forced frame, ignoring
    // the refresh cap, when a renderer is present).
    if let Some(render) = render.as_mut() {
        render.force();
    }
    let open_formatted = load.elapsed();
    let opened = memory();
    pump(tasks, 200);
    if let (Some(probe), Some(render)) = (probe, render.as_mut()) {
        // Settings changes on the opened thread: set, then lay out and paint
        // the frame that follows. Only the rows on screen are laid out.
        render.force();
        for (name, change) in [
            ("text scale 1.0 -> 1.3", Probe::Scale(1.3)),
            ("text scale 1.3 -> 1.0", Probe::Scale(1.0)),
            ("text scale 1.0 -> 1.3", Probe::Scale(1.3)),
            ("text scale 1.3 -> 0.9", Probe::Scale(0.9)),
            ("reading width 704 -> 576 px", Probe::Width(576.0)),
            ("reading width 576 -> 704 px", Probe::Width(704.0)),
            ("text scale 0.9 -> 1.0", Probe::Scale(1.0)),
        ] {
            let t = Instant::now();
            probe(change);
            let set = t.elapsed();
            let drawn = render.force();
            println!(
                "  {messages} messages, {name}: set {:.2} ms, layout + paint {:.2} ms, total {:.2} ms; {}",
                set.as_secs_f64() * 1e3,
                drawn.as_secs_f64() * 1e3,
                t.elapsed().as_secs_f64() * 1e3,
                memory()
            );
        }
        println!(
            "{messages} messages: open (formatted and painted, like for like) {:.1} ms",
            open_formatted.as_secs_f64() * 1e3
        );
        println!(
            "{messages} messages: history import+first sync {:.1} ms, formatted after {:.2} s",
            loaded.as_secs_f64() * 1e3,
            settled.as_secs_f64()
        );
        return;
    }
    // The demo model quotes the prompt in its reply, so a long prompt of
    // paragraphs makes a long reply.
    let prompt = if reply_kb == 0 {
        "Explain streaming".to_owned()
    } else {
        long_prompt(reply_kb)
    };
    let Some(mut run) = app.send_for_test(&prompt) else {
        eprintln!("could not start a run");
        return;
    };
    let mut cx = Context::from_waker(Waker::noop());
    let mut busy: Vec<Duration> = Vec::new();
    // Per 10 s of streaming: frames drawn and their layout + paint time.
    let mut buckets: Vec<(u32, Duration)> = Vec::new();
    let cpu_before = cpu_ms();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(60) {
        let t = Instant::now();
        slint::platform::update_timers_and_animations();
        tasks.run();
        let done = run.as_mut().poll(&mut cx).is_ready();
        if let Some(drawn) = frame(&mut render) {
            let bucket = (started.elapsed().as_secs() / 10) as usize;
            if buckets.len() <= bucket {
                buckets.resize(bucket + 1, (0, Duration::ZERO));
            }
            buckets[bucket].0 += 1;
            buckets[bucket].1 += drawn;
        }
        busy.push(t.elapsed());
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let cpu_stream = cpu_ms() - cpu_before;
    if !buckets.is_empty() {
        let line: Vec<String> = buckets
            .iter()
            .enumerate()
            .map(|(i, (n, d))| {
                format!(
                    "{}-{}s {} frames {:.2} ms",
                    i * 10,
                    i * 10 + 10,
                    n,
                    d.as_secs_f64() * 1e3 / f64::from((*n).max(1))
                )
            })
            .collect();
        println!("  layout + paint per drawn frame: {}", line.join(", "));
    }
    pump(tasks, 300);
    let mut sorted = busy.clone();
    sorted.sort();
    let total: Duration = busy.iter().sum();
    println!(
        "{} after opening the thread; {} at the end; process CPU while streaming {:.0} ms",
        opened,
        memory(),
        cpu_stream
    );
    println!(
        "{messages} messages: open (formatted and painted, like for like) {:.1} ms",
        open_formatted.as_secs_f64() * 1e3
    );
    println!(
        "{messages} messages: history import+first sync {:.1} ms, formatted after {:.2} s; reply streamed in {:.2} s over {} iterations",
        loaded.as_secs_f64() * 1e3,
        settled.as_secs_f64(),
        started.elapsed().as_secs_f64(),
        busy.len()
    );
    let label = if render.is_some() {
        "app work + layout + software paint"
    } else {
        "app work"
    };
    println!(
        "  {label} per iteration: p50 {:?}  p95 {:?}  p99 {:?}  max {:?}  total {:.1} ms; iterations over 4 ms: {}",
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or_default(),
        total.as_secs_f64() * 1e3,
        sorted.iter().filter(|d| **d > Duration::from_millis(4)).count()
    );
}
