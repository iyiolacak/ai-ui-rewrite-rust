//! assistant-ui's default thread in Rust: the built-in demo model, or any AI
//! SDK endpoint.
//!
//! ```text
//! cargo run -p ai-ui-example-thread                      # demo model
//! cargo run -p ai-ui-example-thread -- --api URL         # AI SDK backend
//! cargo run -p ai-ui-example-thread -- --light           # light theme (or --theme system|light|dark)
//! Settings (gear row in the sidebar; Ctrl+= / Ctrl+- / Ctrl+0 for text size) are saved in
//! $XDG_CONFIG_HOME/ai-ui-example-thread/settings.json; `--text-scale F` and `--reading-width compact|default|wide|xwide|full` override a run.
//! cargo run -p ai-ui-example-thread -- --screenshot out.png [--conversation] [--expand-all] [--ask TEXT]... [--decide allow|deny] [--new-thread] [--mid-stream MS] [--size 900x760]
//! Demo scenario in a window: `--history 1000 --threads 40 --long-reply-kb 9 --token-delay-ms 4`
//! GPU window: SLINT_BACKEND=winit-femtovg cargo run -p ai-ui-example-thread --features gpu-window -- [--ask TEXT] [--window-snapshot PATH [--snapshot-delay-ms MS]]
//! ```

mod app;
mod bench;
mod chunks;
mod corpus;
mod delay;
mod demo;
mod markdown;
mod paging;
mod settings;
mod shot;
mod titles;
mod tools;
#[cfg(target_arch = "wasm32")]
mod web;

use ai_ui_core::{
    ChatModelAdapter, LocalRuntimeOptions, LocalThreadRuntime, Spawner, ThreadList, ThreadRuntime,
};
#[cfg(not(target_arch = "wasm32"))]
use ai_ui_data_stream::{DataStreamAdapter, DataStreamOptions};
use ai_ui_state::{ThreadView, ToolPresentation, ViewOptions};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Waker};
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

slint::include_modules!();

struct Args {
    api: Option<String>,
    headers: Vec<(String, String)>,
    light: bool,
    /// Overrides the saved text scale for this run (not saved until changed).
    text_scale: Option<f32>,
    /// Overrides the saved reading width (compact, default, wide, xwide, full) for this run.
    reading_width: Option<usize>,
    /// Overrides the saved theme (system, light, dark) for this run.
    theme: Option<settings::Theme>,
    /// Open the settings popover (for `--screenshot`).
    open_settings: bool,
    screenshot: Option<String>,
    window_snapshot: Option<String>,
    snapshot_delay: Duration,
    /// Stream a reply into a thread with this many messages and report per-frame cost.
    bench: Option<usize>,
    /// In a real window: start with this many messages already in the thread.
    history: usize,
    /// In a real window: open the thread scrolled to its first message (`--start top`), not the newest.
    start_top: bool,
    /// In a real window: list this many stored threads in the sidebar, generated on open.
    threads: usize,
    /// In a real window: ask for a reply of about this many kilobytes of Markdown.
    long_reply_kb: usize,
    /// With `--bench`: ask for a reply of about this many kilobytes of Markdown paragraphs.
    bench_reply_kb: usize,
    /// With `--bench`: also lay out and paint every iteration (software renderer).
    bench_render: bool,
    /// With `--bench N --bench-render`: time text size and width changes on the opened thread instead of streaming.
    bench_settings: bool,
    conversation: bool,
    /// Browser build: fetch this JSON file (a list of messages) as the thread's history.
    thread: Option<String>,
    /// Expand reasoning and tool parts before a `--screenshot`.
    expand_all: bool,
    /// Capture this many milliseconds into a paced streaming reply.
    mid_stream: Option<u64>,
    /// Prompts to run instantly before the screenshot.
    ask: Vec<String>,
    /// Answer a pending approval before the screenshot.
    decide: Option<bool>,
    /// Open a new thread before the screenshot.
    new_thread: bool,
    size: (u32, u32),
    token_delay: Duration,
}

/// The command line; in the browser, the page's query string (`?history=1000&light`
/// is `--history 1000 --light`).
#[cfg(not(target_arch = "wasm32"))]
fn raw_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

#[cfg(target_arch = "wasm32")]
fn raw_args() -> Vec<String> {
    // The page can set `window.__AUI_ARGS` ("history=1000&light"); otherwise the URL's query.
    let window = web_sys::window();
    let search = window
        .as_ref()
        .and_then(|w| js_sys::Reflect::get(w, &"__AUI_ARGS".into()).ok())
        .and_then(|v| v.as_string())
        .or_else(|| window.as_ref().and_then(|w| w.location().search().ok()))
        .unwrap_or_default();
    let mut out = Vec::new();
    for pair in search
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.push(format!("--{key}"));
        if !value.is_empty() {
            out.push(
                js_sys::decode_uri_component(&value.replace('+', " "))
                    .map(String::from)
                    .unwrap_or_else(|_| value.to_owned()),
            );
        }
    }
    out
}

fn args() -> Result<Args, String> {
    let mut args = Args {
        api: None,
        headers: Vec::new(),
        light: false,
        text_scale: None,
        reading_width: None,
        theme: None,
        open_settings: false,
        screenshot: None,
        window_snapshot: None,
        snapshot_delay: Duration::from_secs(6),
        bench: None,
        history: 0,
        start_top: false,
        threads: 0,
        long_reply_kb: 0,
        bench_reply_kb: 0,
        bench_render: false,
        bench_settings: false,
        conversation: false,
        thread: None,
        expand_all: false,
        mid_stream: None,
        ask: Vec::new(),
        decide: None,
        new_thread: false,
        size: (900, 760),
        token_delay: Duration::from_millis(18),
    };
    let mut iter = raw_args().into_iter();
    while let Some(arg) = iter.next() {
        let mut value = || iter.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--api" => args.api = Some(value()?),
            "--header" => {
                let header = value()?;
                let (name, value) = header
                    .split_once(':')
                    .ok_or("--header expects NAME:VALUE")?;
                args.headers
                    .push((name.trim().to_owned(), value.trim().to_owned()));
            }
            "--light" => args.light = true,
            "--text-scale" => {
                let scale: f32 = value()?.parse().map_err(|_| "bad text scale")?;
                args.text_scale = Some(settings::clamp_scale(scale));
            }
            "--reading-width" => {
                let name = value()?;
                args.reading_width = Some(
                    settings::WIDTH_NAMES
                        .iter()
                        .position(|n| *n == name)
                        .ok_or("--reading-width expects compact, default, wide, xwide or full")?,
                )
            }
            "--theme" => {
                args.theme = Some(
                    settings::Theme::parse(&value()?)
                        .ok_or("--theme expects system, light or dark")?,
                )
            }
            "--open-settings" => args.open_settings = true,
            "--screenshot" => args.screenshot = Some(value()?),
            "--window-snapshot" => args.window_snapshot = Some(value()?),
            "--snapshot-delay-ms" => {
                args.snapshot_delay =
                    Duration::from_millis(value()?.parse().map_err(|_| "bad delay")?)
            }
            "--bench" => args.bench = Some(value()?.parse().map_err(|_| "bad message count")?),
            "--thread" => args.thread = Some(value()?),
            "--start" => {
                args.start_top = match value()?.as_str() {
                    "top" => true,
                    "end" => false,
                    _ => return Err("--start expects top or end".into()),
                }
            }
            "--history" => args.history = value()?.parse().map_err(|_| "bad message count")?,
            "--threads" => args.threads = value()?.parse().map_err(|_| "bad thread count")?,
            "--long-reply-kb" => args.long_reply_kb = value()?.parse().map_err(|_| "bad size")?,
            "--bench-reply-kb" => args.bench_reply_kb = value()?.parse().map_err(|_| "bad size")?,
            "--bench-render" => args.bench_render = true,
            "--bench-settings" => args.bench_settings = true,
            "--conversation" => args.conversation = true,
            "--expand-all" => args.expand_all = true,
            "--new-thread" => args.new_thread = true,
            "--ask" => args.ask.push(value()?),
            "--decide" => {
                args.decide = Some(match value()?.as_str() {
                    "allow" => true,
                    "deny" => false,
                    _ => return Err("--decide expects allow or deny".into()),
                })
            }
            "--mid-stream" => {
                args.mid_stream = Some(value()?.parse().map_err(|_| "bad milliseconds")?)
            }
            "--size" => {
                let size = value()?;
                let (w, h) = size.split_once('x').ok_or("--size expects WIDTHxHEIGHT")?;
                args.size = (
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                );
            }
            "--token-delay-ms" => {
                args.token_delay = Duration::from_millis(value()?.parse().map_err(|_| "bad delay")?)
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

/// What a thread's model and tools need, cloned into the list's factory.
#[derive(Clone)]
struct Config {
    api: Option<String>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    headers: Vec<(String, String)>,
    token_delay: Duration,
    tool_latency: Duration,
}

fn adapter(config: &Config) -> Box<dyn ChatModelAdapter> {
    match &config.api {
        #[cfg(not(target_arch = "wasm32"))]
        Some(api) => {
            let mut options = DataStreamOptions::new(api.clone());
            let headers = config.headers.clone();
            options.headers = Some(Rc::new(move || headers.clone()));
            Box::new(DataStreamAdapter::new(
                ai_ui_http::NativeClient::new(),
                options,
            ))
        }
        // The browser build runs the demo model only.
        _ => Box::new(demo::DemoModel {
            token_delay: config.token_delay,
        }),
    }
}

struct Boxed(Box<dyn ChatModelAdapter>);
impl ChatModelAdapter for Boxed {
    fn run(&self, options: ai_ui_core::ChatModelRunOptions) -> ai_ui_core::RunStream {
        self.0.run(options)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn runtime_options() -> LocalRuntimeOptions {
    LocalRuntimeOptions::default()
}

/// `wasm32-unknown-unknown` has no system clock: use the browser's.
#[cfg(target_arch = "wasm32")]
fn runtime_options() -> LocalRuntimeOptions {
    LocalRuntimeOptions::with_clock(Rc::new(
        || ai_ui_core::Timestamp(js_sys::Date::now() as u64),
    ))
}

/// A new thread: a local runtime with the model and the tools.
fn thread(config: &Config) -> Rc<dyn ThreadRuntime> {
    let runtime = LocalThreadRuntime::new(Boxed(adapter(config)), runtime_options());
    runtime.set_tools(tools::registry(config.tool_latency));
    Rc::new(runtime)
}

type Task = Pin<Box<dyn Future<Output = ()>>>;

/// Background tasks of the thread list. In a window they run on Slint's
/// executor; offscreen there is no event loop, so the example polls them.
#[derive(Clone, Default)]
struct Tasks(Rc<RefCell<VecDeque<Task>>>);

impl Tasks {
    fn spawner(&self, windowed: bool) -> Spawner {
        let queue = self.0.clone();
        Rc::new(move |task| {
            if windowed {
                let _ = slint::spawn_local(task);
            } else {
                queue.borrow_mut().push_back(task);
            }
        })
    }
    fn run(&self) {
        let mut cx = Context::from_waker(Waker::noop());
        // Poll each task present at entry at most once. Pending tasks and any
        // tasks spawned while polling wait for the next event-loop tick.
        let count = self.0.borrow().len();
        for _ in 0..count {
            let next = self.0.borrow_mut().pop_front();
            let Some(mut task) = next else { break };
            // Do not hold the queue borrow across poll: a task may spawn work.
            if task.as_mut().poll(&mut cx).is_pending() {
                self.0.borrow_mut().push_back(task);
            }
        }
    }
}

/// Run timers, animations and background tasks for `millis`.
fn settle(tasks: &Tasks, millis: u64) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(millis) {
        slint::platform::update_timers_and_animations();
        tasks.run();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = args()?;
    let offscreen = (args.screenshot.is_some() || args.bench.is_some())
        .then(|| shot::install(args.size.0, args.size.1));
    let instant = args.screenshot.is_some() && args.mid_stream.is_none();
    let ui = AppWindow::new()?;
    // Screenshots and benchmarks must not depend on a saved file; only a
    // normal window loads and saves settings.
    let persist = if args.screenshot.is_some() || args.bench.is_some() {
        None
    } else {
        settings::config_path()
    };
    let mut initial = persist
        .as_deref()
        .map(settings::load_from)
        .unwrap_or_default();
    if args.light {
        initial.theme = settings::Theme::Light;
    }
    if let Some(theme) = args.theme {
        initial.theme = theme;
    }
    if let Some(scale) = args.text_scale {
        initial.text_scale = scale;
    }
    if let Some(width) = args.reading_width {
        initial.width = width;
    }
    let _settings = settings::install(&ui, initial, persist);
    let config = Config {
        api: args.api.clone(),
        headers: args.headers.clone(),
        token_delay: if instant {
            Duration::ZERO
        } else {
            args.token_delay
        },
        tool_latency: if instant {
            Duration::ZERO
        } else {
            Duration::from_millis(900)
        },
    };
    let tasks = Tasks::default();
    let demo = Rc::new(corpus::DemoThreads::new(args.threads));
    let list = ThreadList::new(
        move |_id| thread(&config),
        tasks.spawner(offscreen.is_none()),
        Some(demo.clone()),
    );
    let view = ThreadView::for_list(&list, ViewOptions::default());
    view.set_tool_presenter("get_weather", |call| {
        let city = call
            .args
            .get("city")
            .and_then(|c| c.as_str())
            .unwrap_or("the city");
        ToolPresentation {
            title: Some(format!("Weather in {city}")),
            detail: None,
            result_text: call.result.as_ref().and_then(|r| {
                Some(format!(
                    "{} C, {}",
                    r.get("temp_c")?,
                    r.get("sky")?.as_str()?
                ))
            }),
        }
    });
    let app = app::App::new(&ui, list.clone(), view);
    app.set_known_counts(move |id| demo.message_count(id));
    app.focus_composer();

    if let Some(messages) = args.bench {
        let render = match (&offscreen, args.bench_render) {
            (Some(window), true) => {
                ui.show()?;
                Some(bench::Render::new(window.clone(), args.size.0, args.size.1))
            }
            _ => None,
        };
        let weak = ui.as_weak();
        let probe = move |change: bench::Probe| {
            if let Some(ui) = weak.upgrade() {
                match change {
                    bench::Probe::Scale(scale) => {
                        ui.global::<AssistantTheme>().set_text_scale(scale)
                    }
                    bench::Probe::Width(px) => ui.set_column_width(px),
                }
            }
        };
        let probe: Option<&dyn Fn(bench::Probe)> = args.bench_settings.then_some(&probe);
        bench::run(
            &app,
            &list,
            &tasks,
            messages,
            args.bench_reply_kb,
            render,
            probe,
        );
        return Ok(());
    }

    if let (Some(path), Some(_window)) = (&args.screenshot, &offscreen) {
        let run_to_end = |run: ai_ui_core::RunFuture| {
            let mut run = run;
            let mut cx = Context::from_waker(Waker::noop());
            for _ in 0..1000 {
                if run.as_mut().poll(&mut cx).is_ready() {
                    break;
                }
            }
            tasks.run();
        };
        if args.conversation {
            for prompt in ["What can you do?", "Show me a table"] {
                if let Some(run) = app.send_for_test(prompt) {
                    run_to_end(run);
                }
            }
        }
        for prompt in &args.ask {
            if let Some(run) = app.send_for_test(prompt) {
                run_to_end(run);
            }
        }
        if let Some(approved) = args.decide {
            if let Some(run) = app.decide_pending(approved) {
                run_to_end(run);
            }
        }
        if args.expand_all {
            app.expand_all_for_test();
        }
        if args.new_thread {
            list.switch_to_new();
        }
        // Give the Markdown worker time to format the replies.
        if args.conversation || !args.ask.is_empty() {
            settle(&tasks, 400);
        }
        if let Some(millis) = args.mid_stream {
            // Drive the paced run and its timers by hand, as an event loop would.
            if let Some(mut run) = app.send_for_test("Explain streaming") {
                let mut cx = Context::from_waker(Waker::noop());
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_millis(millis) {
                    slint::platform::update_timers_and_animations();
                    tasks.run();
                    if run.as_mut().poll(&mut cx).is_ready() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                // Capture while the run is still streaming, then let it go.
                shot::save(ui.window(), path)?;
                drop(run);
                return Ok(());
            }
        }
        if args.open_settings {
            ui.invoke_open_settings();
        }
        shot::save(ui.window(), path)?;
        return Ok(());
    }
    // A real window can start from the same scenarios, and save a snapshot of
    // what the GPU renderer drew before exiting (`--window-snapshot PATH`).
    if args.history > 0 {
        list.main()
            .import(ai_ui_core::ExportedRepository::from_linear(
                corpus::history(0, args.history),
            ))
            .map_err(|e| format!("history: {e}"))?;
    }
    let long_prompt = (args.long_reply_kb > 0).then(|| bench::long_prompt(args.long_reply_kb));
    let prompts: Vec<&str> = if args.conversation {
        vec!["What can you do?", "Show me a table"]
    } else {
        Vec::new()
    }
    .into_iter()
    .chain(args.ask.iter().map(String::as_str))
    .chain(long_prompt.as_deref())
    .collect();
    for prompt in prompts {
        if let Some(run) = app.send_for_test(prompt) {
            slint::spawn_local(async move {
                let _ = run.await;
            })?;
        }
    }
    let _snapshot_timer = args.window_snapshot.as_ref().map(|path| {
        let (path, weak) = (path.clone(), ui.as_weak());
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            args.snapshot_delay,
            move || {
                if let Some(ui) = weak.upgrade() {
                    match shot::save_now(ui.window(), &path) {
                        Ok(()) => println!("snapshot written to {path}"),
                        Err(error) => eprintln!("snapshot failed: {error}"),
                    }
                }
                let _ = slint::quit_event_loop();
            },
        );
        timer
    });
    // Scroll to the first message once the first layout exists; a later user scroll or a send takes over.
    let _top_timer = args.start_top.then(|| {
        let weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(300),
            move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_scroll_top_request(ui.get_scroll_top_request() + 1);
                }
            },
        );
        timer
    });
    #[cfg(target_arch = "wasm32")]
    web::start(&ui, &list, args.thread.clone())?;
    let started = Instant::now();
    ui.run()?;
    if args.window_snapshot.is_some() {
        // A measured window run: what the whole process used, for comparing
        // renderers (SLINT_BACKEND) on the same scenario.
        println!(
            "window run: {:.1} s, process CPU {:.0} ms, {}",
            started.elapsed().as_secs_f64(),
            bench::cpu_ms(),
            bench::memory()
        );
    }
    Ok(())
}

#[cfg(test)]
mod task_tests {
    use super::Tasks;
    use std::cell::{Cell, RefCell};
    use std::future::poll_fn;
    use std::rc::Rc;
    use std::task::Poll;

    #[test]
    fn pending_tasks_are_polled_once_per_tick() {
        let tasks = Tasks::default();
        let polls = Rc::new(Cell::new(0usize));
        let tick = Rc::new(Cell::new(0usize));
        let ready = Rc::new(Cell::new(false));
        tasks.spawner(false)(Box::pin(poll_fn({
            let (polls, tick, ready) = (polls.clone(), tick.clone(), ready.clone());
            move |cx| {
                polls.set(polls.get() + 1);
                // Fail promptly if the pump spins, rather than hanging this test.
                assert_eq!(polls.get(), tick.get(), "polled twice in one tick");
                if ready.get() {
                    Poll::Ready(())
                } else {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        })));

        for current in 1..=3 {
            tick.set(current);
            tasks.run();
            assert_eq!(polls.get(), current);
        }
        ready.set(true);
        tick.set(4);
        tasks.run();
        assert_eq!(polls.get(), 4);
        tasks.run();
        assert_eq!(polls.get(), 4, "completed tasks must leave the queue");
    }

    #[test]
    fn tasks_spawned_during_a_poll_wait_for_the_next_tick() {
        let tasks = Tasks::default();
        let spawn = tasks.spawner(false);
        let events = Rc::new(RefCell::new(Vec::new()));
        let parent_events = events.clone();
        let spawn_child = spawn.clone();
        spawn(Box::pin(async move {
            parent_events.borrow_mut().push("parent");
            let spawn_grandchild = spawn_child.clone();
            spawn_child(Box::pin(async move {
                parent_events.borrow_mut().push("child");
                spawn_grandchild(Box::pin(async move {
                    parent_events.borrow_mut().push("grandchild");
                }));
            }));
        }));
        let peer_events = events.clone();
        spawn(Box::pin(async move {
            peer_events.borrow_mut().push("peer");
        }));

        tasks.run();
        assert_eq!(*events.borrow(), ["parent", "peer"]);
        tasks.run();
        assert_eq!(*events.borrow(), ["parent", "peer", "child"]);
        tasks.run();
        assert_eq!(*events.borrow(), ["parent", "peer", "child", "grandchild"]);
    }

    #[test]
    fn all_ready_tasks_at_entry_finish_in_one_tick() {
        let tasks = Tasks::default();
        let spawn = tasks.spawner(false);
        let completed = Rc::new(RefCell::new(Vec::new()));
        for index in 0..4 {
            let completed = completed.clone();
            spawn(Box::pin(async move {
                completed.borrow_mut().push(index);
            }));
        }

        tasks.run();
        assert_eq!(*completed.borrow(), [0, 1, 2, 3]);
        tasks.run();
        assert_eq!(*completed.borrow(), [0, 1, 2, 3]);
    }
}
