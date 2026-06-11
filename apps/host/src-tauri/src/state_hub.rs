use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path as AxumPath, State,
    },
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{App, AppHandle, Emitter, Manager};
use tokio::{
    net::TcpListener,
    sync::{broadcast, watch, RwLock},
    time,
};
use tower_http::cors::CorsLayer;

use crate::capture_publisher::{CapturePublisher, CAPTURE_INTERVAL_SECONDS, MAX_FRAME_AGE_SECONDS};

const STATE_SERVER_PORT: u16 = 17891;
const STATE_FILE_RELATIVE_PATH: &str = "agent_vision/state.json";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentState {
    Idle,
    Thinking,
    Capturing,
    Error,
    Offline,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStateSnapshot {
    pub state: AgentState,
    pub sequence: u64,
    pub updated_at: String,
    pub source: StateSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateSource {
    Startup,
    LocalApi,
    FileHeartbeat,
}

#[derive(Clone)]
pub struct StateHub {
    tx: watch::Sender<AgentStateSnapshot>,
    sequence: Arc<AtomicU64>,
    heartbeat_file_path: Arc<PathBuf>,
    video_tx: broadcast::Sender<VideoFrame>,
    latest_video_frame: Arc<RwLock<Option<VideoFrame>>>,
}

#[derive(Clone)]
struct HttpState {
    hub: StateHub,
    capture_publisher: CapturePublisher,
    phone_dist_dir: Arc<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub bytes: Vec<u8>,
    pub sequence: u64,
    pub received_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateUpdateRequest {
    state: AgentState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeartbeatFile {
    state: AgentState,
}

impl StateHub {
    pub fn new(heartbeat_file_path: PathBuf) -> Self {
        let initial = AgentStateSnapshot {
            state: AgentState::Idle,
            sequence: 0,
            updated_at: Utc::now().to_rfc3339(),
            source: StateSource::Startup,
        };
        let (tx, _rx) = watch::channel(initial);
        let (video_tx, _video_rx) = broadcast::channel(4);

        Self {
            tx,
            sequence: Arc::new(AtomicU64::new(0)),
            heartbeat_file_path: Arc::new(heartbeat_file_path),
            video_tx,
            latest_video_frame: Arc::new(RwLock::new(None)),
        }
    }

    pub fn snapshot(&self) -> AgentStateSnapshot {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<AgentStateSnapshot> {
        self.tx.subscribe()
    }

    pub fn heartbeat_file_path(&self) -> &PathBuf {
        &self.heartbeat_file_path
    }

    pub fn set_state(&self, state: AgentState, source: StateSource) -> AgentStateSnapshot {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let snapshot = AgentStateSnapshot {
            state,
            sequence,
            updated_at: Utc::now().to_rfc3339(),
            source,
        };

        let _ = self.tx.send(snapshot.clone());
        snapshot
    }

    pub fn publish_video_frame(&self, bytes: Vec<u8>) {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = VideoFrame {
            bytes,
            sequence,
            received_at: Utc::now().to_rfc3339(),
        };

        if let Ok(mut latest) = self.latest_video_frame.try_write() {
            *latest = Some(frame.clone());
        }
        let _ = self.video_tx.send(frame);
    }

    fn subscribe_video(&self) -> broadcast::Receiver<VideoFrame> {
        self.video_tx.subscribe()
    }

    pub async fn latest_video_frame(&self) -> Option<VideoFrame> {
        self.latest_video_frame.read().await.clone()
    }
}

/// 启动 AgentEye MVP 状态中心。
///
/// 通信约定:
/// - 外部 Agent / 脚本通过 `POST /api/state` 写入 thinking/idle。
/// - 手机端 PWA 和 Host 前端通过 `ws://<host-ip>:17891/ws` 订阅状态。
/// - Phone PWA 通过 `ws://<host-ip>:17891/video/push` 推送 JPEG binary。
/// - Host 悬浮窗通过 `ws://127.0.0.1:17891/video/feed` 订阅 JPEG binary。
/// - File Heartbeat 模式轮询项目目录下的 `agent_vision/state.json`。
pub fn setup_state_hub(app: &mut App) -> tauri::Result<()> {
    let app_handle = app.handle().clone();
    let state_file_path = resolve_state_file_path()?;
    ensure_state_directory(&state_file_path)?;
    let hub = StateHub::new(state_file_path.clone());
    app.manage(hub.clone());

    let capture_publisher = app
        .try_state::<CapturePublisher>()
        .ok_or_else(|| tauri::Error::Anyhow(anyhow::anyhow!("CapturePublisher 尚未初始化")))?
        .inner()
        .clone();
    let phone_dist_dir = resolve_phone_dist_dir()?;

    tauri::async_runtime::spawn(start_http_server(
        hub.clone(),
        capture_publisher,
        phone_dist_dir,
    ));
    tauri::async_runtime::spawn(watch_state_file(hub.clone(), state_file_path));
    tauri::async_runtime::spawn(forward_state_to_tauri(app_handle, hub));

    Ok(())
}

#[tauri::command]
pub fn get_agent_state(hub: tauri::State<'_, StateHub>) -> AgentStateSnapshot {
    hub.snapshot()
}

#[tauri::command]
pub fn get_state_hub_config(hub: tauri::State<'_, StateHub>) -> serde_json::Value {
    serde_json::json!({
      "port": STATE_SERVER_PORT,
      "heartbeatFilePath": hub.heartbeat_file_path().display().to_string()
    })
}

async fn start_http_server(
    hub: StateHub,
    capture_publisher: CapturePublisher,
    phone_dist_dir: PathBuf,
) {
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, STATE_SERVER_PORT));
    let app_state = HttpState {
        hub,
        capture_publisher,
        phone_dist_dir: Arc::new(phone_dist_dir),
    };
    let router = Router::new()
        .route("/health", get(health))
        .route("/phone", get(phone_index))
        .route("/phone/", get(phone_index))
        .route("/phone/assets/{*asset_path}", get(phone_asset))
        .route("/api/state", get(get_state).post(post_state))
        .route("/api/capture", get(get_capture_config).post(post_capture))
        .route("/ws", get(ws_handler))
        .route("/video/push", get(video_push_handler))
        .route("/video/feed", get(video_feed_handler))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    match TcpListener::bind(addr).await {
        Ok(listener) => {
            tracing::info!("AgentEye state server listening on {addr}");
            if let Err(error) = axum::serve(listener, router).await {
                tracing::error!("AgentEye state server stopped: {error}");
            }
        }
        Err(error) => {
            tracing::error!("AgentEye state server failed to bind {addr}: {error}");
        }
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
      "ok": true,
      "service": "agenteye-state-hub",
      "port": STATE_SERVER_PORT
    }))
}

async fn phone_index(State(state): State<HttpState>) -> Response {
    let index_path = state.phone_dist_dir.join("index.html");
    match tokio::fs::read_to_string(&index_path).await {
        Ok(content) => {
            let patched = content
                .replace("src=\"/assets/", "src=\"/phone/assets/")
                .replace("href=\"/assets/", "href=\"/phone/assets/");
            response_with_body(
                StatusCode::OK,
                "text/html; charset=utf-8",
                patched.into_bytes(),
            )
        }
        Err(error) => response_with_body(
            StatusCode::SERVICE_UNAVAILABLE,
            "text/plain; charset=utf-8",
            format!("AgentEye Phone PWA dist not found: {error}").into_bytes(),
        ),
    }
}

async fn phone_asset(
    State(state): State<HttpState>,
    AxumPath(asset_path): AxumPath<String>,
) -> Response {
    if asset_path.contains("..") || asset_path.contains('\\') {
        return response_with_body(
            StatusCode::BAD_REQUEST,
            "text/plain; charset=utf-8",
            b"invalid asset path".to_vec(),
        );
    }

    let path = state.phone_dist_dir.join("assets").join(&asset_path);
    match tokio::fs::read(&path).await {
        Ok(bytes) => response_with_body(StatusCode::OK, content_type_for(&asset_path), bytes),
        Err(_) => response_with_body(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            b"asset not found".to_vec(),
        ),
    }
}

fn response_with_body(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "text/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}

async fn get_state(State(state): State<HttpState>) -> impl IntoResponse {
    Json(state.hub.snapshot())
}

async fn post_state(
    State(state): State<HttpState>,
    Json(payload): Json<StateUpdateRequest>,
) -> impl IntoResponse {
    let snapshot = state.hub.set_state(payload.state, StateSource::LocalApi);
    (StatusCode::OK, Json(snapshot))
}

async fn get_capture_config(State(state): State<HttpState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "outputDir": state.capture_publisher.output_dir().display().to_string(),
        "latestPng": state.capture_publisher.output_dir().join("latest.png").display().to_string(),
        "latestJson": state.capture_publisher.output_dir().join("latest.json").display().to_string(),
        "intervalSeconds": CAPTURE_INTERVAL_SECONDS,
        "maxFrameAgeSeconds": MAX_FRAME_AGE_SECONDS
    }))
}

async fn post_capture(State(state): State<HttpState>) -> impl IntoResponse {
    match state
        .capture_publisher
        .publish_latest_frame(&state.hub)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))).into_response(),
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "ok": false,
                "message": error.to_string()
            })),
        )
            .into_response(),
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<HttpState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state.hub))
}

async fn video_push_handler(
    ws: WebSocketUpgrade,
    State(state): State<HttpState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_video_push(socket, state.hub))
}

async fn video_feed_handler(
    ws: WebSocketUpgrade,
    State(state): State<HttpState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_video_feed(socket, state.hub))
}

async fn handle_ws(socket: WebSocket, hub: StateHub) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = hub.subscribe();

    if let Ok(initial) = serde_json::to_string(&hub.snapshot()) {
        if sender.send(Message::Text(initial.into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
          changed = rx.changed() => {
            if changed.is_err() {
              break;
            }

            let snapshot = rx.borrow().clone();
            match serde_json::to_string(&snapshot) {
              Ok(payload) => {
                if sender.send(Message::Text(payload.into())).await.is_err() {
                  break;
                }
              }
              Err(error) => tracing::error!("failed to serialize state snapshot: {error}"),
            }
          }
          incoming = receiver.next() => {
            match incoming {
              Some(Ok(Message::Close(_))) | None => break,
              Some(Ok(_)) => {}
              Some(Err(error)) => {
                tracing::debug!("state websocket closed with error: {error}");
                break;
              }
            }
          }
        }
    }
}

async fn handle_video_push(socket: WebSocket, hub: StateHub) {
    let (_sender, mut receiver) = socket.split();

    while let Some(message) = receiver.next().await {
        match message {
            Ok(Message::Binary(bytes)) => {
                if !bytes.is_empty() {
                    hub.publish_video_frame(bytes.to_vec());
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

async fn handle_video_feed(socket: WebSocket, hub: StateHub) {
    let (mut sender, mut receiver) = socket.split();
    let mut video_rx = hub.subscribe_video();

    loop {
        tokio::select! {
            frame = video_rx.recv() => {
                match frame {
                    Ok(frame) => {
                        // 先发送一个轻量 JSON metadata，再发送 JPEG binary。
                        // 前端当前只消费 binary；metadata 留给后续延迟/帧率诊断。
                        let metadata = serde_json::json!({
                            "type": "video-frame",
                            "sequence": frame.sequence,
                            "receivedAt": frame.received_at,
                            "bytes": frame.bytes.len()
                        });
                        if sender.send(Message::Text(metadata.to_string().into())).await.is_err() {
                            break;
                        }
                        if sender.send(Message::Binary(frame.bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::debug!("video feed lagged, skipped {skipped} frames");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::debug!("video feed websocket closed with error: {error}");
                        break;
                    }
                }
            }
        }
    }
}

async fn forward_state_to_tauri(app: AppHandle, hub: StateHub) {
    let mut rx = hub.subscribe();

    while rx.changed().await.is_ok() {
        let snapshot = rx.borrow().clone();
        if let Some(window) = app.get_webview_window("floating") {
            let _ = window.emit("agenteye://agent-state", &snapshot);
        }
    }
}

async fn watch_state_file(hub: StateHub, path: PathBuf) {
    let mut last_modified: Option<SystemTime> = None;
    let mut interval = time::interval(Duration::from_millis(500));

    loop {
        interval.tick().await;

        let metadata = match tokio::fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };

        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error) => {
                tracing::debug!("failed to read state file modified time: {error}");
                continue;
            }
        };

        if last_modified.is_some_and(|last| last >= modified) {
            continue;
        }
        last_modified = Some(modified);

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let normalized_content = content.trim_start_matches('\u{feff}');
                match serde_json::from_str::<HeartbeatFile>(normalized_content) {
                    Ok(file_state) => {
                        hub.set_state(file_state.state, StateSource::FileHeartbeat);
                    }
                    Err(error) => tracing::warn!("invalid AgentEye state heartbeat file: {error}"),
                }
            }
            Err(error) => tracing::debug!("failed to read AgentEye state heartbeat file: {error}"),
        }
    }
}

fn resolve_state_file_path() -> tauri::Result<PathBuf> {
    let mut base = std::env::current_dir()?;

    // Tauri dev/build 从 apps/host 启动时，current_dir 通常是 host app 目录。
    // 这里向上找到 monorepo 根，让 agent_vision 固定落在 AgentEye/agent_vision。
    if base.file_name().is_some_and(|name| name == "host") {
        base.pop();
        base.pop();
    }

    Ok(base.join(STATE_FILE_RELATIVE_PATH))
}

fn resolve_phone_dist_dir() -> tauri::Result<PathBuf> {
    let current = std::env::current_dir()?;
    let mut cursor = Some(current.as_path());

    while let Some(base) = cursor {
        let monorepo_candidate = base.join("apps").join("phone").join("dist");
        if monorepo_candidate.join("index.html").exists() {
            return Ok(monorepo_candidate);
        }

        let apps_candidate = base.join("phone").join("dist");
        if apps_candidate.join("index.html").exists() {
            return Ok(apps_candidate);
        }

        cursor = base.parent();
    }

    Ok(current.join("apps").join("phone").join("dist"))
}

fn ensure_state_directory(path: &PathBuf) -> tauri::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
