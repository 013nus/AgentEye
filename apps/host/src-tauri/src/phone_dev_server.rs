use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::{App, Manager};

const PHONE_DEV_PORT: u16 = 1421;
const STARTUP_WAIT: Duration = Duration::from_secs(20);
const POLL_INTERVAL: Duration = Duration::from_millis(400);

pub struct PhoneDevServerState {
    child: Mutex<Option<Child>>,
}

impl PhoneDevServerState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }
}

pub fn setup_phone_dev_server(app: &mut App) -> tauri::Result<()> {
    let state = PhoneDevServerState::new();
    app.manage(state);

    let app_handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = ensure_phone_dev_server(&app_handle).await {
            tracing::warn!("failed to ensure phone HTTPS dev server: {error}");
        }
    });

    Ok(())
}

pub async fn ensure_phone_dev_server(app: &tauri::AppHandle) -> tauri::Result<()> {
    if is_port_ready(PHONE_DEV_PORT) {
        tracing::info!("phone HTTPS dev server already listening on {PHONE_DEV_PORT}");
        return Ok(());
    }

    let Some(repo_root) = resolve_monorepo_root_from_cwd() else {
        tracing::info!("monorepo root not found; skip auto-starting phone HTTPS dev server");
        return Ok(());
    };

    if !repo_root.join("apps").join("phone").join("package.json").exists() {
        tracing::info!("phone workspace not found; skip auto-starting phone HTTPS dev server");
        return Ok(());
    }

    spawn_phone_dev_server(app, &repo_root)?;
    wait_for_port(PHONE_DEV_PORT, STARTUP_WAIT).await?;
    Ok(())
}

fn spawn_phone_dev_server(app: &tauri::AppHandle, repo_root: &Path) -> tauri::Result<()> {
    let state = app.state::<PhoneDevServerState>();
    let mut guard = state
        .child
        .lock()
        .map_err(|_| tauri::Error::Anyhow(anyhow::anyhow!("phone dev server lock poisoned")))?;

    if guard.is_some() {
        return Ok(());
    }

    tracing::info!(
        "starting phone HTTPS dev server on port {PHONE_DEV_PORT} from {}",
        repo_root.display()
    );

    let child = spawn_npm_phone_dev(repo_root).map_err(|error| {
        tauri::Error::Anyhow(anyhow::anyhow!(
            "无法自动启动手机 HTTPS 服务，请确认已安装 Node.js/npm: {error}"
        ))
    })?;

    *guard = Some(child);
    Ok(())
}

fn spawn_npm_phone_dev(repo_root: &Path) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        Command::new("cmd")
            .args(["/C", "npm run dev:https -w @agenteye/phone"])
            .current_dir(repo_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    }

    #[cfg(not(windows))]
    {
        Command::new("npm")
            .args(["run", "dev:https", "-w", "@agenteye/phone"])
            .current_dir(repo_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
}

async fn wait_for_port(port: u16, timeout: Duration) -> tauri::Result<()> {
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if is_port_ready(port) {
            tracing::info!("phone HTTPS dev server is ready on {port}");
            return Ok(());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    Err(tauri::Error::Anyhow(anyhow::anyhow!(
        "手机 HTTPS 服务在 {} 秒内未就绪，请检查 Node.js 环境或手动运行 npm run dev:all",
        timeout.as_secs()
    )))
}

pub fn is_port_ready(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(180)).is_ok()
}

fn resolve_monorepo_root_from_cwd() -> Option<PathBuf> {
    let current = std::env::current_dir().ok()?;
    let mut cursor = Some(current.as_path());

    while let Some(base) = cursor {
        if base.join("package.json").exists()
            && base.join("apps").join("host").exists()
            && base.join("apps").join("phone").exists()
        {
            return Some(base.to_path_buf());
        }
        cursor = base.parent();
    }

    None
}

pub fn shutdown_phone_dev_server(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<PhoneDevServerState>() else {
        return;
    };

    let Ok(mut guard) = state.child.lock() else {
        return;
    };

    if let Some(mut child) = guard.take() {
        tracing::info!("stopping auto-started phone HTTPS dev server");
        let _ = child.kill();
        let _ = child.wait();
    }
}
