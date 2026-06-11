mod capture_publisher;
mod floating_window;
mod pairing;
mod phone_dev_server;
mod state_hub;

use capture_publisher::{publish_from_shortcut, setup_capture_publisher};
use phone_dev_server::setup_phone_dev_server;
use floating_window::{
    force_disable_mouse_passthrough, install_floating_window_commands, setup_floating_window,
};
use state_hub::setup_state_hub;
use tauri::{Emitter, Manager, RunEvent};

/// AgentEye Host 的 Rust 入口。
///
/// 当前阶段只搭建桌面外壳、状态中心和窗口能力。视频流、截图发布、剪贴板发布
/// 会作为独立模块继续挂载，避免系统级窗口逻辑和业务逻辑耦合在一起。
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        return;
                    }

                    let shortcut_text = format!("{shortcut:?}");
                    if shortcut_text.contains("KeyL") {
                        force_disable_mouse_passthrough(app);
                        return;
                    }

                    // MVP 阶段先把截图快捷键事件广播到前端，同时触发 Rust 侧 Capture Publisher。
                    let payload = serde_json::json!({
                      "shortcut": shortcut_text,
                      "action": "capture_to_clipboard"
                    });

                    if let Some(window) = app.get_webview_window("floating") {
                        let _ = window.emit("agenteye://global-shortcut", payload);
                    }

                    let app_handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        publish_from_shortcut(app_handle).await;
                    });
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            floating_window::start_window_drag,
            floating_window::set_mouse_passthrough,
            floating_window::set_floating_window_always_on_top,
            floating_window::show_floating_window,
            floating_window::close_agenteye,
            floating_window::request_pairing_panel,
            state_hub::get_agent_state,
            state_hub::get_state_hub_config,
            capture_publisher::capture_latest_frame,
            capture_publisher::get_capture_config,
            pairing::get_pairing_config,
            pairing::prepare_pairing_config
        ])
        .setup(|app| {
            setup_capture_publisher(app)?;
            setup_state_hub(app)?;
            setup_phone_dev_server(app)?;
            setup_floating_window(app)?;
            install_floating_window_commands(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build AgentEye Host")
        .run(|app_handle, event| {
            if matches!(event, RunEvent::Exit) {
                phone_dev_server::shutdown_phone_dev_server(app_handle);
            }
        });
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .try_init();
}
