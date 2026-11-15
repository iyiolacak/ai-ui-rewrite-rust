//! A browser client over `fetch`. Dropping the response body cancels the
//! reader and aborts the request, so no task outlives the run.

use ai_ui_core::AbortSignal;
use ai_ui_data_stream::{HttpClient, HttpError, HttpRequest, HttpResponse, ResponseFuture};
use futures_core::Stream;
use js_sys::{Reflect, Uint8Array};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, Headers, ReadableStreamDefaultReader, RequestCredentials, RequestInit,
    Response,
};

fn error(value: JsValue) -> HttpError {
    HttpError(
        value
            .as_string()
            .or_else(|| js_sys::Error::from(value).message().as_string())
            .unwrap_or_else(|| "fetch failed".into()),
    )
}

/// `fetch` with `credentials: same-origin` by default, as upstream.
#[derive(Clone, Copy, Debug, Default)]
pub struct FetchClient {
    /// Send cookies and credentials cross-origin too (`credentials: include`).
    pub include_credentials: bool,
}

impl HttpClient for FetchClient {
    fn post(&self, request: HttpRequest, signal: AbortSignal) -> ResponseFuture {
        let include = self.include_credentials;
        Box::pin(async move {
            let window = web_sys::window().ok_or_else(|| HttpError("no window".into()))?;
            let controller = AbortController::new().map_err(error)?;
            let headers = Headers::new().map_err(error)?;
            for (name, value) in &request.headers {
                headers.append(name, value).map_err(error)?;
            }
            let init = RequestInit::new();
            init.set_method("POST");
            init.set_headers(&headers);
            init.set_body(&JsValue::from_str(&String::from_utf8_lossy(&request.body)));
            init.set_signal(Some(&controller.signal()));
            init.set_credentials(if include {
                RequestCredentials::Include
            } else {
                RequestCredentials::SameOrigin
            });
            let response: Response =
                JsFuture::from(window.fetch_with_str_and_init(&request.url, &init))
                    .await
                    .map_err(error)?
                    .dyn_into()
                    .map_err(error)?;
            if signal.is_aborted() {
                controller.abort();
                return Err(HttpError("aborted".into()));
            }
            let mut pairs = Vec::new();
            if let Ok(Some(iterator)) = js_sys::try_iter(&response.headers()) {
                for entry in iterator.flatten() {
                    let entry: js_sys::Array = entry.unchecked_into();
                    if let (Some(name), Some(value)) =
                        (entry.get(0).as_string(), entry.get(1).as_string())
                    {
                        pairs.push((name, value));
                    }
                }
            }
            let reader = response
                .body()
                .ok_or_else(|| HttpError("Response body is null".into()))?
                .get_reader()
                .unchecked_into::<ReadableStreamDefaultReader>();
            Ok(HttpResponse {
                status: response.status(),
                headers: pairs,
                body: Box::pin(Body {
                    reader,
                    controller,
                    pending: None,
                    done: false,
                }),
            })
        })
    }
}

struct Body {
    reader: ReadableStreamDefaultReader,
    controller: AbortController,
    pending: Option<JsFuture>,
    done: bool,
}

impl Stream for Body {
    type Item = Result<Vec<u8>, HttpError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        let reader = self.reader.clone();
        let pending = self
            .pending
            .get_or_insert_with(|| JsFuture::from(reader.read()));
        let result = match Pin::new(pending).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };
        self.pending = None;
        let chunk = match result {
            Ok(chunk) => chunk,
            Err(value) => {
                self.done = true;
                return Poll::Ready(Some(Err(error(value))));
            }
        };
        if Reflect::get(&chunk, &JsValue::from_str("done"))
            .ok()
            .and_then(|d| d.as_bool())
            == Some(true)
        {
            self.done = true;
            return Poll::Ready(None);
        }
        let value = Reflect::get(&chunk, &JsValue::from_str("value")).unwrap_or(JsValue::UNDEFINED);
        Poll::Ready(Some(Ok(Uint8Array::new(&value).to_vec())))
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.reader.cancel();
            self.controller.abort();
        }
    }
}
