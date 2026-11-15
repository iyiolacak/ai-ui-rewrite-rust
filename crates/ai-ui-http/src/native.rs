//! A native client. Each request runs on its own thread; the response body
//! is handed over through a bounded queue, so a slow consumer slows reading
//! instead of buffering the whole response.

use ai_ui_core::AbortSignal;
use ai_ui_data_stream::{HttpClient, HttpError, HttpRequest, HttpResponse, ResponseFuture};
use futures_core::Stream;
use std::collections::VecDeque;
use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};

/// Chunks the reader may queue ahead of the consumer.
const QUEUE: usize = 64;
const CHUNK_BYTES: usize = 16 * 1024;

type Received = Result<(u16, Vec<(String, String)>), HttpError>;

#[derive(Default)]
struct State {
    head: Option<Received>,
    chunks: VecDeque<Result<Vec<u8>, HttpError>>,
    finished: bool,
    /// The consumer went away; the reader stops.
    dropped: bool,
    waker: Option<Waker>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    space: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn update(&self, change: impl FnOnce(&mut State)) {
        let waker = {
            let mut state = self.lock();
            change(&mut state);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// A client over `ureq` and rustls, with one worker thread per request.
#[derive(Clone)]
pub struct NativeClient {
    agent: ureq::Agent,
}

impl Default for NativeClient {
    fn default() -> Self {
        // Non-2xx responses are returned so the adapter can report the status.
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl NativeClient {
    /// A client with default settings.
    pub fn new() -> Self {
        Self::default()
    }
    /// Use a configured agent (proxy, timeouts, TLS); keep
    /// `http_status_as_error(false)` so error statuses reach the adapter.
    pub fn with_agent(agent: ureq::Agent) -> Self {
        Self { agent }
    }
}

fn read(agent: ureq::Agent, request: HttpRequest, signal: AbortSignal, shared: Arc<Shared>) {
    let mut builder = agent.post(&request.url);
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    let response = match builder.send(&request.body[..]) {
        Ok(response) => response,
        Err(error) => {
            shared.update(|s| s.head = Some(Err(HttpError(error.to_string()))));
            return;
        }
    };
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
        })
        .collect();
    let status = response.status().as_u16();
    shared.update(|s| s.head = Some(Ok((status, headers))));
    let mut reader = response.into_body().into_reader();
    let mut buffer = vec![0; CHUNK_BYTES];
    loop {
        {
            let mut state = shared.lock();
            while state.chunks.len() >= QUEUE && !state.dropped && !signal.is_aborted() {
                state = shared.space.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.dropped || signal.is_aborted() {
                return;
            }
        }
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => shared.update(|s| s.chunks.push_back(Ok(buffer[..n].to_vec()))),
            Err(error) => {
                shared.update(|s| s.chunks.push_back(Err(HttpError(error.to_string()))));
                break;
            }
        }
    }
    shared.update(|s| s.finished = true);
}

impl HttpClient for NativeClient {
    fn post(&self, request: HttpRequest, signal: AbortSignal) -> ResponseFuture {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        let agent = self.agent.clone();
        let spawned = std::thread::Builder::new()
            .name("ai-ui-http".into())
            .spawn(move || read(agent, request, signal, worker));
        if let Err(error) = spawned {
            return Box::pin(std::future::ready(Err(HttpError(format!(
                "could not start the request thread: {error}"
            )))));
        }
        Box::pin(Head {
            shared: Some(shared),
        })
    }
}

struct Head {
    shared: Option<Arc<Shared>>,
}

impl Future for Head {
    type Output = Result<HttpResponse, HttpError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let shared = self.shared.clone().expect("polled after completion");
        let head = {
            let mut state = shared.lock();
            match state.head.take() {
                Some(head) => head,
                None => {
                    state.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };
        self.shared = None;
        Poll::Ready(head.map(|(status, headers)| HttpResponse {
            status,
            headers,
            body: Box::pin(Body { shared }),
        }))
    }
}

impl Drop for Head {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.lock().dropped = true;
            shared.space.notify_all();
        }
    }
}

struct Body {
    shared: Arc<Shared>,
}

impl Stream for Body {
    type Item = Result<Vec<u8>, HttpError>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self.shared.lock();
        if let Some(chunk) = state.chunks.pop_front() {
            drop(state);
            self.shared.space.notify_all();
            return Poll::Ready(Some(chunk));
        }
        if state.finished {
            return Poll::Ready(None);
        }
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        self.shared.lock().dropped = true;
        self.shared.space.notify_all();
    }
}
