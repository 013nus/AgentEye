use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{
    App, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

const FLOATING_WINDOW_LABEL: &str = "floating";
const CONTROLS_WINDOW_LABEL: &str = "controls";
const DEFAULT_LOGICAL_WIDTH: f64 = 360.0;
const DEFAULT_LOGICAL_HEIGHT: f64 = 640.0;
const MIN_LOGICAL_WIDTH: f64 = 300.0;
const MIN_LOGICAL_HEIGHT: f64 = 480.0;
const CONTROLS_LOGICAL_WIDTH: f64 = 154.0;
const CONTROLS_LOGICAL_HEIGHT: f64 = 42.0;
const CONTROLS_MARGIN: f64 = 10.0;
static PASSTHROUGH_REQUESTED: AtomicBool = AtomicBool::new(false);
static PASSTHROUGH_PREVIEW_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 初始化 AgentEye 的悬浮窗。
///
/// 设计目标:
/// - 无边框: 由前端绘制精密玻璃质感，不使用系统标题栏。
/// - 永远置顶: 确保开发板画面始终处于 Agent 可见区域。
/// - 高 DPI 友好: Tauri 配置使用 logical size，这里读取 scale factor 并校准最小物理尺寸。
/// - Windows 优先: 启用 Acrylic/Mica 这类系统材质时，只在对应平台尝试，不影响跨平台启动。
pub fn setup_floating_window(app: &mut App) -> tauri::Result<()> {
    let window = app
        .get_webview_window(FLOATING_WINDOW_LABEL)
        .expect("floating window must be declared in tauri.conf.json");
    let controls = app
        .get_webview_window(CONTROLS_WINDOW_LABEL)
        .expect("controls window must be declared in tauri.conf.json");

    configure_floating_window(&window)?;
    configure_controls_window(&controls)?;
    apply_platform_window_material(&window);
    sync_controls_to_floating(&window, &controls)?;
    bind_controls_to_floating(&window, &controls);
    start_passthrough_hover_monitor(app.handle().clone());

    // 等前端资源载入后再显示，避免用户看到一帧白屏或未完成布局。
    window.show()?;
    controls.show()?;
    window.set_focus()?;

    Ok(())
}

fn configure_floating_window(window: &WebviewWindow) -> tauri::Result<()> {
    window.set_decorations(false)?;
    window.set_always_on_top(true)?;
    window.set_resizable(true)?;
    window.set_shadow(true)?;

    // 默认不启用鼠标穿透。AgentEye 需要可拖拽、可调节，
    // 只有用户主动切换观察锁定时才穿透。
    window.set_ignore_cursor_events(false)?;

    let scale = window.scale_factor().unwrap_or(1.0);
    let default_physical_size = PhysicalSize::new(
        (DEFAULT_LOGICAL_WIDTH * scale).round() as u32,
        (DEFAULT_LOGICAL_HEIGHT * scale).round() as u32,
    );
    let min_physical_size = PhysicalSize::new(
        (MIN_LOGICAL_WIDTH * scale).round() as u32,
        (MIN_LOGICAL_HEIGHT * scale).round() as u32,
    );
    window.set_min_size(Some(Size::Physical(min_physical_size)))?;

    if should_apply_phone_camera_default_size(window, default_physical_size) {
        window.set_size(Size::Physical(default_physical_size))?;
    }

    // Windows 多显示器和高 DPI 环境下，配置文件里的 center 有时不是最理想位置。
    // 使用 monitor work_area 而不是 monitor size，避免默认窗口盖住任务栏/状态栏。
    if let Some(monitor) = window.current_monitor()? {
        let work_area = monitor.work_area();
        let logical_width = (DEFAULT_LOGICAL_WIDTH * scale).round() as i32;
        let logical_height = (DEFAULT_LOGICAL_HEIGHT * scale).round() as i32;
        let margin_x = (18.0 * scale).round() as i32;
        let margin_y = (6.0 * scale).round() as i32;

        let x = work_area.position.x + work_area.size.width as i32 - logical_width - margin_x;
        let y = work_area.position.y + work_area.size.height as i32 - logical_height - margin_y;

        window.set_position(Position::Physical(PhysicalPosition::new(
            x.max(work_area.position.x),
            y.max(work_area.position.y),
        )))?;
    }

    Ok(())
}

fn should_apply_phone_camera_default_size(
    window: &WebviewWindow,
    default_size: PhysicalSize<u32>,
) -> bool {
    let Ok(current_size) = window.outer_size() else {
        return true;
    };

    // 早期版本默认是横向小窗。若检测到当前窗口仍明显横向，
    // 启动时自动迁移到手机竖屏观察尺寸；用户之后仍可手动缩放。
    if current_size.width > current_size.height {
        return true;
    }

    current_size.width < default_size.width.saturating_sub(32)
        || current_size.height < default_size.height.saturating_sub(64)
        || current_size.width > default_size.width.saturating_add(80)
        || current_size.height > default_size.height.saturating_add(120)
}

fn configure_controls_window(window: &WebviewWindow) -> tauri::Result<()> {
    window.set_decorations(false)?;
    window.set_always_on_top(true)?;
    window.set_resizable(false)?;
    window.set_shadow(false)?;
    window.set_ignore_cursor_events(false)?;
    window.set_skip_taskbar(true)?;
    Ok(())
}

fn bind_controls_to_floating(floating: &WebviewWindow, controls: &WebviewWindow) {
    let floating_for_event = floating.clone();
    let controls_for_event = controls.clone();
    floating.on_window_event(move |event| match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            let _ = sync_controls_to_floating(&floating_for_event, &controls_for_event);
        }
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
            let _ = controls_for_event.close();
        }
        _ => {}
    });
}

fn sync_controls_to_floating(
    floating: &WebviewWindow,
    controls: &WebviewWindow,
) -> tauri::Result<()> {
    let position = floating.outer_position()?;
    let size = floating.outer_size()?;
    let scale = floating.scale_factor().unwrap_or(1.0);
    let width = (CONTROLS_LOGICAL_WIDTH * scale).round() as i32;
    let margin = (CONTROLS_MARGIN * scale).round() as i32;
    let y_margin = (8.0 * scale).round() as i32;
    let x = position.x + size.width as i32 - width - margin;
    let y = position.y + y_margin;

    controls.set_position(Position::Physical(PhysicalPosition::new(x, y)))?;
    let controls_size = PhysicalSize::new(
        (CONTROLS_LOGICAL_WIDTH * scale).round() as u32,
        (CONTROLS_LOGICAL_HEIGHT * scale).round() as u32,
    );
    controls.set_size(Size::Physical(controls_size))?;
    Ok(())
}

/// 安装与悬浮窗相关的全局快捷键。
///
/// 当前注册两个全局快捷键:
/// - Ctrl+Alt+V: 未来触发 Capture Publisher，并把图片写入剪贴板。
/// - Ctrl+Alt+L: 强制关闭鼠标穿透，避免锁定后无法再点击悬浮窗。
///
/// 这里使用插件层注册，而不是前端监听键盘事件，因为前端只能捕获窗口获得焦点时的键盘输入。
pub fn install_floating_window_commands(app: &mut App) -> tauri::Result<()> {
    let capture_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyV);
    let unlock_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyL);

    app.global_shortcut()
        .register_multiple([capture_shortcut, unlock_shortcut])
        .map_err(|error| tauri::Error::Anyhow(anyhow::anyhow!("注册全局快捷键失败: {error}")))?;
    Ok(())
}

#[tauri::command]
pub fn start_window_drag(window: WebviewWindow) -> Result<(), String> {
    window
        .start_dragging()
        .map_err(|error| format!("启动窗口拖拽失败: {error}"))
}

#[tauri::command]
pub fn set_mouse_passthrough(app: AppHandle, enabled: bool) -> Result<(), String> {
    set_mouse_passthrough_for_app(&app, enabled, "ui")
}

#[tauri::command]
pub fn set_floating_window_always_on_top(app: AppHandle, enabled: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(FLOATING_WINDOW_LABEL) {
        window
            .set_always_on_top(enabled)
            .map_err(|error| format!("切换主窗口置顶失败: {error}"))?;
    }

    if let Some(window) = app.get_webview_window(CONTROLS_WINDOW_LABEL) {
        window
            .set_always_on_top(enabled)
            .map_err(|error| format!("切换控制窗置顶失败: {error}"))?;
    }

    Ok(())
}

#[tauri::command]
pub fn show_floating_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(FLOATING_WINDOW_LABEL)
        .ok_or_else(|| "找不到 AgentEye 悬浮窗".to_string())?;

    window
        .show()
        .map_err(|error| format!("显示悬浮窗失败: {error}"))?;
    window
        .set_focus()
        .map_err(|error| format!("聚焦悬浮窗失败: {error}"))?;

    Ok(())
}

#[tauri::command]
pub fn close_agenteye(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(FLOATING_WINDOW_LABEL) {
        let _ = window.close();
    }

    if let Some(window) = app.get_webview_window(CONTROLS_WINDOW_LABEL) {
        window
            .close()
            .map_err(|error| format!("关闭 AgentEye 控制窗失败: {error}"))?;
    }

    Ok(())
}

#[tauri::command]
pub fn request_pairing_panel(app: AppHandle) -> Result<(), String> {
    let Some(window) = app.get_webview_window(FLOATING_WINDOW_LABEL) else {
        return Err("找不到 AgentEye 主窗口".to_string());
    };

    window
        .emit(
            "agenteye://open-pairing",
            serde_json::json!({ "source": "controls" }),
        )
        .map_err(|error| format!("打开配对面板失败: {error}"))
}

pub fn force_disable_mouse_passthrough(app: &AppHandle) {
    if let Err(error) = set_mouse_passthrough_for_app(app, false, "shortcut") {
        tracing::warn!("failed to force disable mouse passthrough: {error}");
    }
}

fn set_mouse_passthrough_for_app(
    app: &AppHandle,
    enabled: bool,
    source: &'static str,
) -> Result<(), String> {
    let window = app
        .get_webview_window(FLOATING_WINDOW_LABEL)
        .ok_or_else(|| "找不到 AgentEye 主窗口".to_string())?;

    PASSTHROUGH_REQUESTED.store(enabled, Ordering::Relaxed);
    PASSTHROUGH_PREVIEW_ACTIVE.store(false, Ordering::Relaxed);

    match set_mouse_passthrough_for_window(&window, enabled) {
        Ok(()) => {
            broadcast_mouse_passthrough(app, enabled, source);
            broadcast_mouse_passthrough_preview(app, false);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn broadcast_mouse_passthrough(app: &AppHandle, enabled: bool, source: &'static str) {
    let payload = serde_json::json!({ "enabled": enabled, "source": source });

    if let Some(window) = app.get_webview_window(FLOATING_WINDOW_LABEL) {
        let _ = window.emit("agenteye://mouse-passthrough", &payload);
    }

    if let Some(window) = app.get_webview_window(CONTROLS_WINDOW_LABEL) {
        let _ = window.emit("agenteye://mouse-passthrough", &payload);
    }
}

fn broadcast_mouse_passthrough_preview(app: &AppHandle, active: bool) {
    let payload = serde_json::json!({ "active": active });

    if let Some(window) = app.get_webview_window(FLOATING_WINDOW_LABEL) {
        let _ = window.emit("agenteye://mouse-passthrough-preview", &payload);
    }

    if let Some(window) = app.get_webview_window(CONTROLS_WINDOW_LABEL) {
        let _ = window.emit("agenteye://mouse-passthrough-preview", &payload);
    }
}

fn set_mouse_passthrough_for_window(window: &WebviewWindow, enabled: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| format!("切换鼠标穿透失败: {error}"))
}

fn start_passthrough_hover_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(90));

        loop {
            interval.tick().await;

            if !PASSTHROUGH_REQUESTED.load(Ordering::Relaxed) {
                continue;
            }

            let Some(window) = app.get_webview_window(FLOATING_WINDOW_LABEL) else {
                continue;
            };
            let Some(cursor) = current_cursor_position() else {
                continue;
            };

            let inside_window = match (window.outer_position(), window.outer_size()) {
                (Ok(position), Ok(size)) => {
                    let right = position.x + size.width as i32;
                    let bottom = position.y + size.height as i32;

                    cursor.x >= position.x
                        && cursor.x <= right
                        && cursor.y >= position.y
                        && cursor.y <= bottom
                }
                _ => false,
            };

            let was_active = PASSTHROUGH_PREVIEW_ACTIVE.swap(inside_window, Ordering::Relaxed);
            if was_active == inside_window {
                continue;
            }

            // 用户开启穿透后，鼠标移到视频窗口上时临时恢复可交互和 100% 可见；
            // 鼠标移走后再恢复穿透，避免持续挡住桌面下方内容。
            if let Err(error) = set_mouse_passthrough_for_window(&window, !inside_window) {
                tracing::warn!("failed to update passthrough hover preview: {error}");
            }
            broadcast_mouse_passthrough_preview(&app, inside_window);
        }
    });
}

#[derive(Clone, Copy)]
struct CursorPosition {
    x: i32,
    y: i32,
}

#[cfg(target_os = "windows")]
fn current_cursor_position() -> Option<CursorPosition> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).ok()?;
    }

    Some(CursorPosition {
        x: point.x,
        y: point.y,
    })
}

#[cfg(not(target_os = "windows"))]
fn current_cursor_position() -> Option<CursorPosition> {
    None
}

#[cfg(target_os = "windows")]
fn apply_platform_window_material(window: &WebviewWindow) {
    // Windows 11 上优先使用 Mica，Windows 10/不支持时降级到 Acrylic。
    // 失败时只记录日志，不阻塞应用启动。视觉材质是增强项，不能成为核心功能单点失败。
    if let Err(mica_error) = window_vibrancy::apply_mica(window, Some(true)) {
        tracing::debug!("Mica 不可用，尝试 Acrylic: {mica_error}");

        if let Err(acrylic_error) = window_vibrancy::apply_acrylic(window, Some((18, 20, 24, 180)))
        {
            tracing::debug!("Acrylic 不可用，使用透明 WebView 背景: {acrylic_error}");
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_platform_window_material(_window: &WebviewWindow) {
    // macOS/Linux 的材质后续分别接入 vibrancy 或 compositor 能力。
}
