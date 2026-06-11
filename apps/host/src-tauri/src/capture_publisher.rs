use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use chrono::Utc;
use image::ImageFormat;
use serde::Serialize;
use tauri::{App, AppHandle, Emitter, Manager};
use tokio::time;

use crate::state_hub::{AgentState, StateHub, StateSource, VideoFrame};

pub const CAPTURE_INTERVAL_SECONDS: u64 = 5;
pub const MAX_FRAME_AGE_SECONDS: i64 = 10;
pub const HISTORY_MAX_FILES: usize = 100;
pub const HISTORY_PRUNE_COUNT: usize = 50;

#[derive(Clone)]
pub struct CapturePublisher {
    output_dir: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub ok: bool,
    pub image_path: String,
    pub metadata_path: String,
    pub captured_at: String,
    pub source_sequence: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObservationMetadata {
    version: &'static str,
    captured_at: String,
    image_path: String,
    width: u32,
    height: u32,
    source: &'static str,
    mode: &'static str,
    agent_state: AgentState,
    camera_state: &'static str,
    sequence: u64,
    source_frame: SourceFrameMetadata,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceFrameMetadata {
    sequence: u64,
    received_at: String,
}

impl CapturePublisher {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    pub fn output_dir(&self) -> &PathBuf {
        &self.output_dir
    }

    pub async fn publish_latest_frame(&self, hub: &StateHub) -> anyhow::Result<CaptureResult> {
        let Some(frame) = hub.latest_video_frame().await else {
            self.clear_latest_outputs_best_effort().await;
            anyhow::bail!("还没有可发布的手机画面，请确认手机端已经连接并正在推流");
        };

        if let Err(error) = ensure_frame_is_fresh(&frame) {
            self.clear_latest_outputs_best_effort().await;
            return Err(error);
        }

        let captured_at = Utc::now().to_rfc3339();
        let image = match image::load_from_memory_with_format(&frame.bytes, ImageFormat::Jpeg)
            .or_else(|_| image::load_from_memory(&frame.bytes))
        {
            Ok(image) => image,
            Err(error) => {
                self.clear_latest_outputs_best_effort().await;
                anyhow::bail!("无法解码手机视频帧，已拒绝发布无效画面: {error}");
            }
        };
        let rgba = image.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();

        let latest_png = self.output_dir.join("latest.png");
        let latest_json = self.output_dir.join("latest.json");
        let history_dir = self.output_dir.join("history");
        tokio::fs::create_dir_all(&history_dir).await?;

        let history_name = format!("{}.png", sanitize_timestamp(&captured_at));
        let history_png = history_dir.join(history_name);

        let mut png_bytes = Vec::new();
        {
            let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
            image::ImageEncoder::write_image(
                encoder,
                rgba.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )?;
        }

        atomic_write(&latest_png, &png_bytes).await?;
        atomic_write(&history_png, &png_bytes).await?;
        prune_history_if_needed(&history_dir).await;

        let state = hub.snapshot();
        let metadata = ObservationMetadata {
            version: "0.1",
            captured_at: captured_at.clone(),
            image_path: latest_png.display().to_string(),
            width,
            height,
            source: "phone-mjpeg",
            mode: "board-only",
            agent_state: state.state,
            camera_state: "connected",
            sequence: frame.sequence,
            source_frame: SourceFrameMetadata {
                sequence: frame.sequence,
                received_at: frame.received_at,
            },
        };
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
        atomic_write(&latest_json, &metadata_bytes).await?;

        Ok(CaptureResult {
            ok: true,
            image_path: latest_png.display().to_string(),
            metadata_path: latest_json.display().to_string(),
            captured_at,
            source_sequence: frame.sequence,
            width,
            height,
        })
    }

    async fn clear_latest_outputs_best_effort(&self) {
        if let Err(error) = clear_latest_outputs(&self.output_dir).await {
            tracing::warn!("failed to clear stale latest capture outputs: {error}");
        }
    }
}

pub fn setup_capture_publisher(app: &mut App) -> tauri::Result<()> {
    let output_dir = resolve_output_dir()?;
    std::fs::create_dir_all(&output_dir)?;
    if let Err(error) = clear_latest_outputs_sync(&output_dir) {
        tracing::warn!("failed to clear stale latest capture outputs on startup: {error}");
    }

    let publisher = CapturePublisher::new(output_dir);
    app.manage(publisher.clone());

    let app_handle = app.handle().clone();
    tauri::async_runtime::spawn(run_periodic_capture(app_handle, publisher));

    Ok(())
}

#[tauri::command]
pub async fn capture_latest_frame(
    hub: tauri::State<'_, StateHub>,
    publisher: tauri::State<'_, CapturePublisher>,
) -> Result<CaptureResult, String> {
    publish_and_emit(None, &hub, &publisher).await
}

#[tauri::command]
pub fn get_capture_config(publisher: tauri::State<'_, CapturePublisher>) -> serde_json::Value {
    serde_json::json!({
        "outputDir": publisher.output_dir().display().to_string(),
        "latestPng": publisher.output_dir().join("latest.png").display().to_string(),
        "latestJson": publisher.output_dir().join("latest.json").display().to_string(),
        "intervalSeconds": CAPTURE_INTERVAL_SECONDS,
        "maxFrameAgeSeconds": MAX_FRAME_AGE_SECONDS,
        "historyMaxFiles": HISTORY_MAX_FILES,
        "historyPruneCount": HISTORY_PRUNE_COUNT
    })
}

pub async fn publish_from_shortcut(app: AppHandle) {
    let Some(hub) = app.try_state::<StateHub>() else {
        return;
    };
    let Some(publisher) = app.try_state::<CapturePublisher>() else {
        return;
    };

    let _ = publish_and_emit(Some(&app), &hub, &publisher).await;
}

async fn run_periodic_capture(app: AppHandle, publisher: CapturePublisher) {
    let mut interval = time::interval(Duration::from_secs(CAPTURE_INTERVAL_SECONDS));

    loop {
        interval.tick().await;
        let Some(hub) = app.try_state::<StateHub>() else {
            continue;
        };
        let _ = publish_and_emit(Some(&app), &hub, &publisher).await;
    }
}

async fn publish_and_emit(
    app: Option<&AppHandle>,
    hub: &StateHub,
    publisher: &CapturePublisher,
) -> Result<CaptureResult, String> {
    let previous_state = hub.snapshot().state;
    hub.set_state(AgentState::Capturing, StateSource::LocalApi);
    let result = publisher
        .publish_latest_frame(hub)
        .await
        .map_err(|error| format!("发布最新画面失败: {error}"));

    if let Some(app) = app {
        if let Some(window) = app.get_webview_window("floating") {
            match &result {
                Ok(payload) => {
                    let _ = window.emit("agenteye://capture-result", payload);
                }
                Err(error) => {
                    let _ = window.emit(
                        "agenteye://capture-error",
                        serde_json::json!({ "message": error }),
                    );
                }
            }
        }
    }

    let restore_state = if previous_state == AgentState::Capturing {
        AgentState::Idle
    } else {
        previous_state
    };
    hub.set_state(restore_state, StateSource::LocalApi);
    result
}

fn resolve_output_dir() -> tauri::Result<PathBuf> {
    let current = std::env::current_dir()?;
    Ok(resolve_workspace_root(&current).join("agent_vision"))
}

fn resolve_workspace_root(current: &Path) -> PathBuf {
    let mut cursor = Some(current);

    while let Some(base) = cursor {
        if base.join("package.json").exists()
            && base.join("apps").join("host").exists()
            && base.join("apps").join("phone").exists()
        {
            return base.to_path_buf();
        }

        cursor = base.parent();
    }

    current.to_path_buf()
}

fn ensure_frame_is_fresh(frame: &VideoFrame) -> anyhow::Result<()> {
    let received_at = chrono::DateTime::parse_from_rfc3339(&frame.received_at)
        .map_err(|error| anyhow::anyhow!("视频帧时间戳无法解析，已拒绝发布异常画面: {error}"))?
        .with_timezone(&Utc);
    let age = Utc::now().signed_duration_since(received_at);
    let age_seconds = age.num_seconds();

    if age_seconds > MAX_FRAME_AGE_SECONDS {
        anyhow::bail!("最近一帧已经过期 {age_seconds} 秒，已拒绝发布旧画面，请确认手机端仍在推流");
    }

    if age_seconds < -5 {
        anyhow::bail!("视频帧时间戳来自未来，已拒绝发布异常画面");
    }

    Ok(())
}

async fn clear_latest_outputs(output_dir: &Path) -> anyhow::Result<()> {
    remove_file_if_exists(output_dir.join("latest.png")).await?;
    remove_file_if_exists(output_dir.join("latest.json")).await?;
    Ok(())
}

async fn prune_history_if_needed(history_dir: &Path) {
    if let Err(error) = prune_history_if_needed_inner(history_dir).await {
        tracing::warn!("failed to prune history captures: {error}");
    }
}

async fn prune_history_if_needed_inner(history_dir: &Path) -> anyhow::Result<()> {
    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(history_dir).await?;

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("png") {
            continue;
        }

        let metadata = entry.metadata().await?;
        if !metadata.is_file() {
            continue;
        }

        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push((modified, path));
    }

    if entries.len() <= HISTORY_MAX_FILES {
        return Ok(());
    }

    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    for (_, path) in entries.into_iter().take(HISTORY_PRUNE_COUNT) {
        remove_file_if_exists(path).await?;
    }

    Ok(())
}

async fn remove_file_if_exists(path: PathBuf) -> anyhow::Result<()> {
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn clear_latest_outputs_sync(output_dir: &Path) -> anyhow::Result<()> {
    remove_file_sync_if_exists(output_dir.join("latest.png"))?;
    remove_file_sync_if_exists(output_dir.join("latest.json"))?;
    Ok(())
}

fn remove_file_sync_if_exists(path: PathBuf) -> anyhow::Result<()> {
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let tmp_path = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("tmp")
    ));
    tokio::fs::write(&tmp_path, bytes).await?;
    replace_file(&tmp_path, path).await?;
    Ok(())
}

async fn replace_file(tmp_path: &Path, path: &Path) -> anyhow::Result<()> {
    let tmp_path = tmp_path.to_path_buf();
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || replace_file_sync(&tmp_path, &path)).await?
}

#[cfg(target_os = "windows")]
fn replace_file_sync(tmp_path: &Path, path: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };

    let source: Vec<u16> = tmp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
    };

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file_sync(tmp_path: &Path, path: &Path) -> anyhow::Result<()> {
    std::fs::rename(tmp_path, path)?;
    Ok(())
}

fn sanitize_timestamp(timestamp: &str) -> String {
    timestamp
        .chars()
        .map(|character| match character {
            ':' | '.' => '-',
            other => other,
        })
        .collect()
}
