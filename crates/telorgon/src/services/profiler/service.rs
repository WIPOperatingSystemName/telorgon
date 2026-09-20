use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::runtime::instrumentation::{Collector, Session, SessionConfig};
use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE, LOCATION,
    ORIGIN, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use tokio::sync::{broadcast, oneshot, watch};
use tokio::time::MissedTickBehavior;

use crate::services::profiler::ServerConfig;
use crate::services::profiler::protocol::encode_viewer_gap;
use crate::services::profiler::store::{LiveBatch, SessionStore};

const INDEX_HTML: &str = include_str!("assets/index.html");
const PROFILER_CSS: &str = include_str!("assets/profiler.css");
const PROFILER_JS: &str = include_str!("assets/profiler.js");
const COOKIE_PREFIX: &str = "crate::runtime::instrumentation";
const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";

#[derive(Debug)]
pub struct ServerError(String);

impl ServerError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ServerError {}

#[derive(Clone)]
struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    token: String,
    cookie_name: String,
    origin: String,
    store: Mutex<SessionStore>,
    live: broadcast::Sender<LiveBatch>,
    stop: watch::Receiver<bool>,
    input_recording_viewers: Mutex<HashMap<ViewerInputSource, usize>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ViewerCommand {
    SetInputRecording {
        source: ViewerInputSource,
        enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum ViewerInputSource {
    PointerMotion,
    PointerButton,
    Scroll,
    Keyboard,
    TouchMotion,
    TouchContact,
    DeviceChange,
}

impl ViewerInputSource {
    const fn profiler_source(self) -> crate::runtime::instrumentation::InputRecordingSource {
        match self {
            Self::PointerMotion => crate::runtime::instrumentation::InputRecordingSource::PointerMotion,
            Self::PointerButton => crate::runtime::instrumentation::InputRecordingSource::PointerButton,
            Self::Scroll => crate::runtime::instrumentation::InputRecordingSource::Scroll,
            Self::Keyboard => crate::runtime::instrumentation::InputRecordingSource::Keyboard,
            Self::TouchMotion => crate::runtime::instrumentation::InputRecordingSource::TouchMotion,
            Self::TouchContact => crate::runtime::instrumentation::InputRecordingSource::TouchContact,
            Self::DeviceChange => crate::runtime::instrumentation::InputRecordingSource::DeviceChange,
        }
    }
}

/// Application-owned service whose drop path synchronously stops and joins its one thread.
pub struct ProfilerServer {
    url: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    session: Option<Session>,
}

impl ProfilerServer {
    /// Starts the same bounded profiler lifecycle for any managed host when activation was
    /// requested. Hosts select their entrypoint only through [`ServerConfig`] metadata.
    pub fn start_if_requested(
        request: crate::services::profiler::ProfilerRequest,
        config: ServerConfig,
    ) -> Result<Option<Self>, ServerError> {
        if request == crate::services::profiler::ProfilerRequest::Disabled {
            return Ok(None);
        }
        Self::start(config).map(Some)
    }

    pub fn start(config: ServerConfig) -> Result<Self, ServerError> {
        let listener = StdTcpListener::bind(("127.0.0.1", config.port)).map_err(|error| {
            ServerError::new(format!(
                "failed to bind stable profiler loopback port {}: {error}; stop the other profiler or set TELORGON_PROFILER_PORT",
                config.port
            ))
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            ServerError::new(format!("failed to configure profiler loopback: {error}"))
        })?;
        let address = listener.local_addr().map_err(|error| {
            ServerError::new(format!("failed to inspect profiler loopback: {error}"))
        })?;
        let token = random_token();
        let origin = format!("http://127.0.0.1:{}", address.port());
        let cookie_name = format!("{COOKIE_PREFIX}_{}", address.port());
        let url = format!("{origin}/?token={token}");
        let (session, collector) = Session::start(SessionConfig::default()).map_err(|error| {
            ServerError::new(format!("failed to start profiler session: {error}"))
        })?;
        let (live, _) = broadcast::channel(32);
        let (stop_tx, stop_rx) = watch::channel(false);
        let state = AppState {
            inner: Arc::new(AppStateInner {
                token,
                cookie_name,
                origin,
                store: Mutex::new(SessionStore::new(config.metadata)),
                live,
                stop: stop_rx,
                input_recording_viewers: Mutex::new(HashMap::new()),
            }),
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (startup_tx, startup_rx) = mpsc::sync_channel(1);
        let service_state = state.clone();
        let thread = thread::Builder::new()
            .name("telorgon-profiler".to_owned())
            .spawn(move || {
                service_thread(
                    listener,
                    service_state,
                    collector,
                    shutdown_rx,
                    stop_tx,
                    startup_tx,
                );
            })
            .map_err(|error| {
                ServerError::new(format!("failed to create profiler service thread: {error}"))
            })?;
        match startup_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                let _ = thread.join();
                return Err(ServerError::new(message));
            }
            Err(_) => {
                let _ = thread.join();
                return Err(ServerError::new(
                    "profiler service thread exited during startup",
                ));
            }
        }
        println!("Telorgon Profiler: {url}");
        crate::runtime::instrumentation::record_instant("profiler.session.started");
        Ok(Self {
            url,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
            session: Some(session),
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for ProfilerServer {
    fn drop(&mut self) {
        crate::runtime::instrumentation::record_instant("profiler.session.ended");
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("telorgon-profiler: service thread panicked during shutdown");
        }
        self.session.take();
    }
}

fn service_thread(
    listener: StdTcpListener,
    state: AppState,
    collector: Collector,
    shutdown_rx: oneshot::Receiver<()>,
    stop_tx: watch::Sender<bool>,
    startup_tx: mpsc::SyncSender<Result<(), String>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = startup_tx.send(Err(format!("failed to create profiler runtime: {error}")));
            return;
        }
    };
    runtime.block_on(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = startup_tx.send(Err(format!("failed to adopt profiler listener: {error}")));
                return;
            }
        };
        let app = router(state.clone());
        let mut collector_stop = state.inner.stop.clone();
        let collector_state = state.clone();
        let collector_task = tokio::spawn(async move {
            collect_events(collector, collector_state, &mut collector_stop).await;
        });
        let _ = startup_tx.send(Ok(()));
        let graceful_stop = stop_tx.clone();
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
                let _ = graceful_stop.send(true);
            })
            .await;
        let _ = stop_tx.send(true);
        let _ = collector_task.await;
        if let Err(error) = result {
            eprintln!("telorgon-profiler: service stopped after an HTTP error: {error}");
        }
    });
}

async fn collect_events(
    mut collector: Collector,
    state: AppState,
    stop: &mut watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(16));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut events = Vec::with_capacity(1_024);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                events.clear();
                collector.drain_into(&mut events);
                let lanes = collector.lanes();
                let views = collector.views();
                let batch = state
                    .inner
                    .store
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .append(&events, lanes, views);
                if let Some(batch) = batch {
                    let _ = state.inner.live.send(batch);
                }
            }
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    events.clear();
                    collector.drain_into(&mut events);
                    let lanes = collector.lanes();
                    let views = collector.views();
                    let _ = state
                        .inner
                        .store
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .append(&events, lanes, views);
                    break;
                }
            }
        }
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/profiler.css", get(stylesheet))
        .route("/assets/profiler.js", get(script))
        .route("/metadata", get(metadata))
        .route("/capture", get(capture))
        .route("/reconnect", post(reconnect))
        .route("/live", get(live))
        .with_state(state)
}

async fn index(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response<Body> {
    if query
        .get("token")
        .is_some_and(|candidate| secure_eq(candidate, &state.inner.token))
    {
        let mut response = response(StatusCode::SEE_OTHER, "text/plain; charset=utf-8", "");
        response
            .headers_mut()
            .insert(LOCATION, HeaderValue::from_static("/"));
        set_session_cookie(&mut response, &state);
        return response;
    }
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    response(StatusCode::OK, "text/html; charset=utf-8", INDEX_HTML)
}

async fn stylesheet(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    response(StatusCode::OK, "text/css; charset=utf-8", PROFILER_CSS)
}

async fn script(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    response(
        StatusCode::OK,
        "text/javascript; charset=utf-8",
        PROFILER_JS,
    )
}

async fn metadata(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    let body = state
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .metadata_json();
    response(StatusCode::OK, "application/json", body)
}

async fn capture(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    if !authorized(&headers, &state) {
        return unauthorized();
    }
    let bytes = state
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .capture();
    let mut response = response(StatusCode::OK, "application/vnd.telorgon.profiler", bytes);
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=profile.telorgon-trace"),
    );
    response
}

async fn reconnect(State(state): State<AppState>, headers: HeaderMap) -> Response<Body> {
    if !reconnect_authorized(&headers, &state) {
        return unauthorized();
    }
    let mut response = response(StatusCode::NO_CONTENT, "text/plain; charset=utf-8", "");
    set_session_cookie(&mut response, &state);
    response
}

async fn live(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response<Body> {
    if !authorized(&headers, &state)
        || headers
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok())
            .is_none_or(|origin| !secure_eq(origin, &state.inner.origin))
    {
        return unauthorized();
    }
    upgrade
        .on_upgrade(move |socket| websocket_client(socket, state))
        .into_response()
}

async fn websocket_client(mut socket: WebSocket, state: AppState) {
    let mut live = state.inner.live.subscribe();
    let mut input_recording = HashSet::new();
    let snapshot = state
        .inner
        .store
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .snapshot();
    if socket
        .send(Message::Text(snapshot.metadata_json.into()))
        .await
        .is_err()
        || socket
            .send(Message::Binary(snapshot.labels.into()))
            .await
            .is_err()
        || socket
            .send(Message::Binary(snapshot.events.into()))
            .await
            .is_err()
    {
        return;
    }
    let mut stop = state.inner.stop.clone();
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ViewerCommand::SetInputRecording { source, enabled }) =
                        serde_json::from_str(text.as_str())
                    {
                        update_input_recording_preference(
                            &state,
                            &mut input_recording,
                            source,
                            enabled,
                        );
                    }
                }
                _ => {}
            },
            batch = live.recv() => match batch {
                Ok(batch) => {
                    if let Some(metadata) = batch.metadata_json
                        && socket
                            .send(Message::Text(metadata.to_string().into()))
                            .await
                            .is_err()
                    {
                        break;
                    }
                    if let Some(labels) = batch.labels
                        && socket.send(Message::Binary(labels.to_vec().into())).await.is_err()
                    {
                        break;
                    }
                    if socket.send(Message::Binary(batch.events.to_vec().into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(dropped)) => {
                    if socket
                        .send(Message::Binary(encode_viewer_gap(dropped).into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    }
    for source in input_recording.clone() {
        update_input_recording_preference(&state, &mut input_recording, source, false);
    }
}

fn update_input_recording_preference(
    state: &AppState,
    current: &mut HashSet<ViewerInputSource>,
    source: ViewerInputSource,
    enabled: bool,
) {
    if current.contains(&source) == enabled {
        return;
    }
    let mut viewers = state
        .inner
        .input_recording_viewers
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let count = viewers.entry(source).or_default();
    if enabled {
        *count = count.saturating_add(1);
        current.insert(source);
    } else {
        *count = count.saturating_sub(1);
        current.remove(&source);
    }
    crate::runtime::instrumentation::set_input_recording_enabled(source.profiler_source(), *count != 0);
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Body>,
) -> Response<Body> {
    let mut response = Response::new(body.into());
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    response
}

fn unauthorized() -> Response<Body> {
    response(
        StatusCode::UNAUTHORIZED,
        "text/plain; charset=utf-8",
        "unauthorized profiler request",
    )
}

fn authorized(headers: &HeaderMap, state: &AppState) -> bool {
    profiler_cookie(headers, &state.inner.cookie_name)
        .is_some_and(|candidate| secure_eq(candidate, &state.inner.token))
}

fn reconnect_authorized(headers: &HeaderMap, state: &AppState) -> bool {
    let same_origin = headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| secure_eq(origin, &state.inner.origin));
    same_origin
        && profiler_cookie(headers, &state.inner.cookie_name).is_some_and(|token| {
            token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn profiler_cookie<'a>(headers: &'a HeaderMap, cookie_name: &str) -> Option<&'a str> {
    for header in headers.get_all(COOKIE) {
        let Ok(cookies) = header.to_str() else {
            continue;
        };
        for cookie in cookies.split(';') {
            if let Some(value) = cookie
                .trim()
                .strip_prefix(cookie_name)
                .and_then(|value| value.strip_prefix('='))
            {
                return Some(value);
            }
        }
    }
    None
}

fn set_session_cookie(response: &mut Response<Body>, state: &AppState) {
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Strict; Path=/",
        state.inner.cookie_name, state.inner.token
    );
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(SET_COOKIE, cookie);
    }
}

fn secure_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn random_token() -> String {
    let bytes: [u8; 32] = rand::random();
    let mut token = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::profiler::SessionMetadata;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let (live, _) = broadcast::channel(1);
        let (_, stop) = watch::channel(false);
        AppState {
            inner: Arc::new(AppStateInner {
                token: "secret".to_owned(),
                cookie_name: "crate::profiler_1234".to_owned(),
                origin: "http://127.0.0.1:1234".to_owned(),
                store: Mutex::new(SessionStore::new(SessionMetadata::for_tests())),
                live,
                stop,
                input_recording_viewers: Mutex::new(HashMap::new()),
            }),
        }
    }

    #[test]
    fn cookie_authentication_requires_an_exact_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("other=x; crate::profiler_1234=secret"),
        );
        let state = test_state();
        assert!(authorized(&headers, &state));
        headers.insert(
            COOKIE,
            HeaderValue::from_static("crate::profiler_1234=secre"),
        );
        assert!(!authorized(&headers, &state));
    }

    #[test]
    fn reconnect_requires_same_origin_and_a_prior_profiler_cookie() {
        let state = test_state();
        let prior_token = "b".repeat(64);
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("http://127.0.0.1:1234"));
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("crate::profiler_1234={prior_token}")).unwrap(),
        );
        assert!(reconnect_authorized(&headers, &state));
        headers.insert(ORIGIN, HeaderValue::from_static("https://example.com"));
        assert!(!reconnect_authorized(&headers, &state));
    }

    #[test]
    fn embedded_assets_are_nonempty_and_external_resource_free() {
        assert!(INDEX_HTML.contains("/assets/profiler.css"));
        assert!(INDEX_HTML.contains("/assets/profiler.js"));
        assert!(INDEX_HTML.contains("Target not connected"));
        assert!(PROFILER_JS.contains("/reconnect"));
        assert!(PROFILER_JS.contains("seenSequences"));
        assert!(INDEX_HTML.contains("Range hot spots"));
        assert!(INDEX_HTML.contains("Raw frame flame chart"));
        assert!(INDEX_HTML.contains("data-workspace=\"performance\""));
        assert!(INDEX_HTML.contains("data-performance-view=\"responsiveness\""));
        assert!(INDEX_HTML.contains("data-performance-view=\"inputs\""));
        assert!(INDEX_HTML.contains("Presentation outcomes"));
        assert!(INDEX_HTML.contains("Host frame-work distribution"));
        assert!(INDEX_HTML.contains("view-filter"));
        assert!(PROFILER_JS.contains("AGGREGATION.CUMULATIVE"));
        assert!(PROFILER_JS.contains("analyzeFrame"));
        assert!(PROFILER_JS.contains("presentation.vulkan_wsi.zero_timeout_acquire_stall"));
        assert!(PROFILER_JS.contains("Likely GPU driver / Vulkan WSI stall"));
        assert!(PROFILER_JS.contains("vkAcquireNextImageKHR"));
        assert!(INDEX_HTML.contains("diagnostics-guidance"));
        assert!(PROFILER_CSS.contains(".diagnostic-callout"));
        assert!(PROFILER_JS.contains("analyzeResponsiveness"));
        assert!(PROFILER_JS.contains("responsiveness.resize.native_ended"));
        assert!(PROFILER_JS.contains("p95ResizeReleaseToPresent"));
        assert!(PROFILER_JS.contains("activeDuration"));
        assert!(PROFILER_JS.contains("ordinalPlotAxis"));
        assert!(PROFILER_JS.contains("formatSessionTimestamp"));
        assert!(PROFILER_JS.contains("eventAffectsActiveWorkspace"));
        assert!(!PROFILER_JS.contains("wallX("));
        assert!(!INDEX_HTML.contains("wall-clock time axis"));
        assert!(PROFILER_JS.contains("consecutiveGapsByView"));
        assert!(INDEX_HTML.contains("input-recording-options"));
        assert!(INDEX_HTML.contains("input-event-plot"));
        assert!(INDEX_HTML.contains("input-event-log-body"));
        assert!(INDEX_HTML.contains("pointer-pipeline-body"));
        assert!(INDEX_HTML.contains("pointer-path-body"));
        assert!(INDEX_HTML.contains("lucide-play-icon"));
        assert!(INDEX_HTML.contains("lucide-pause-icon"));
        assert!(INDEX_HTML.contains("aria-label=\"Pause profiling\""));
        assert!(PROFILER_JS.contains("input.non_pointer_events.received"));
        assert!(PROFILER_JS.contains("frame.trigger.pointer_move_only"));
        assert!(PROFILER_JS.contains("set_input_recording"));
        assert!(PROFILER_JS.contains("renderInputPerformance"));
        assert!(PROFILER_JS.contains("pointerPipelineSampleForEvent"));
        assert!(PROFILER_JS.contains("freshest_event_to_cursor_submit_ns"));
        assert!(PROFILER_JS.contains("input_recording_sources"));
        assert!(PROFILER_JS.contains("renderInputRecordingControls();"));
        assert!(!PROFILER_JS.contains("updatePointerMovesButton"));
        assert!(PROFILER_JS.contains("if (state.paused) return;"));
        assert!(PROFILER_JS.contains("followLiveSelection"));
        assert!(PROFILER_JS.contains("button.setAttribute(\"aria-label\", label)"));
        assert!(PROFILER_JS.contains("button.classList.toggle(\"is-paused\", state.paused)"));
        assert!(PROFILER_JS.contains("state.paused ? \"Paused\" : \"Live\""));
        assert!(PROFILER_CSS.contains(".pause-toggle.is-paused .lucide-play"));
        assert!(PROFILER_CSS.contains(".status-dot.paused"));
        assert!(!PROFILER_JS.contains("state.paused = true"));
        assert!(!PROFILER_JS.contains("event.code === \"Space\""));
        assert!(INDEX_HTML.contains("plot-point-details"));
        assert!(INDEX_HTML.contains("plot-view-back"));
        assert!(INDEX_HTML.contains("plot-view-reset"));
        assert!(INDEX_HTML.contains("plot-frame-limit"));
        assert!(INDEX_HTML.contains("clear-analysis-range"));
        assert!(!INDEX_HTML.contains("range-30"));
        assert!(!INDEX_HTML.contains("range-120"));
        assert!(!INDEX_HTML.contains("range-all"));
        assert!(INDEX_HTML.contains("inspector-resize-handle"));
        assert!(!INDEX_HTML.contains("frame-summary-button"));
        assert!(PROFILER_JS.contains("plotPointAt"));
        assert!(PROFILER_JS.contains("plotViewHistory"));
        assert!(PROFILER_JS.contains("plotFrameLimit"));
        assert!(PROFILER_JS.contains("plot._dragMode === \"zoom\""));
        assert!(PROFILER_JS.contains("setInspectorWidth"));
        assert!(!PROFILER_JS.contains("frame-summary-button"));
        assert!(PROFILER_JS.contains("plotFrameSelectionBounds"));
        assert!(PROFILER_JS.contains("Release to analyze range"));
        assert!(PROFILER_JS.contains("renderRangeInspector"));
        assert!(PROFILER_JS.contains("drawRangeBoundaryGuides"));
        assert!(PROFILER_JS.contains("clearManualRangeSelection"));
        assert!(PROFILER_JS.contains("selectManualRange"));
        assert!(PROFILER_JS.contains("hasManualRangeSelection"));
        assert!(PROFILER_JS.contains("return plotViewFrames();"));
        assert!(!PROFILER_JS.contains("rangeMode"));
        assert!(!PROFILER_JS.contains("rangeSize"));
        assert!(!PROFILER_JS.contains("rangeAnchorEndId"));
        assert!(PROFILER_JS.contains("enhanceSortableTable"));
        assert!(PROFILER_JS.contains("applyStoredTableSorts"));
        assert!(PROFILER_JS.contains("tableCellSortValue"));
        assert!(INDEX_HTML.contains("<th scope=\"col\">Category</th>"));
        assert!(PROFILER_JS.contains("INSPECTOR_MAX_WIDTH = 760"));
        assert!(PROFILER_JS.contains("inspectorMaximumWidth"));
        assert!(PROFILER_JS.contains("queueInspectorDragWidth"));
        assert!(PROFILER_JS.contains("inspectorDragActive"));
        assert!(!PROFILER_JS.contains("scheduleInspectorResizePlot"));
        assert!(PROFILER_JS.contains(
            "renderRangeAnalysis(rangeFrames, rangeResponsiveness);\n  renderInspector();"
        ));
        assert!(PROFILER_JS.contains("LIVE_RENDER_INTERVAL_MS"));
        assert!(!PROFILER_JS.contains("isGraphEvent"));
        assert!(PROFILER_CSS.contains("--category-presentation"));
        assert!(PROFILER_CSS.contains("--inspector-width"));
        assert!(PROFILER_CSS.contains("scrollbar-width: thin"));
        assert!(PROFILER_CSS.contains("::-webkit-scrollbar-thumb"));
        assert!(PROFILER_CSS.contains(".table-sort-button"));
        assert!(!PROFILER_JS.contains("/bytes|memory|upload"));
        assert!(!INDEX_HTML.contains("https://"));
        assert!(!PROFILER_JS.contains("eval("));
        assert!(!PROFILER_CSS.contains("linear-gradient"));
    }

    #[test]
    fn input_recording_viewer_command_is_explicit_and_bounded() {
        let enabled: ViewerCommand = serde_json::from_str(
            r#"{"type":"set_input_recording","source":"pointer_motion","enabled":true}"#,
        )
        .unwrap();
        assert_eq!(
            enabled,
            ViewerCommand::SetInputRecording {
                source: ViewerInputSource::PointerMotion,
                enabled: true,
            }
        );
        assert!(
            serde_json::from_str::<ViewerCommand>(
                r#"{"type":"set_input_recording","source":"clipboard","enabled":true}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<ViewerCommand>(r#"{"type":"unknown"}"#).is_err());
    }

    #[test]
    fn generated_tokens_are_full_width_hex() {
        let token = random_token();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn disabled_generic_host_start_does_not_create_a_session_or_listener() {
        let result = ProfilerServer::start_if_requested(
            crate::services::profiler::ProfilerRequest::Disabled,
            ServerConfig::for_target(crate::services::profiler::ProfileTarget::ShellEnvironment),
        )
        .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn router_serves_only_authenticated_fixed_assets() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let unauthorized = router(test_state())
                .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

            let stylesheet = router(test_state())
                .oneshot(
                    Request::builder()
                        .uri("/assets/profiler.css")
                        .header(COOKIE, "crate::profiler_1234=secret")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(stylesheet.status(), StatusCode::OK);

            let unknown = router(test_state())
                .oneshot(
                    Request::builder()
                        .uri("/filesystem/path")
                        .header(COOKIE, "crate::profiler_1234=secret")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

            let prior_token = "b".repeat(64);
            let renewed = router(test_state())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/reconnect")
                        .header(ORIGIN, "http://127.0.0.1:1234")
                        .header(COOKIE, format!("crate::profiler_1234={prior_token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(renewed.status(), StatusCode::NO_CONTENT);
            assert!(renewed.headers().contains_key(SET_COOKIE));
        });
    }
}
