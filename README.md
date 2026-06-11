# AgentEye

AgentEye is an open-source desktop/PWA system that turns physical hardware into readable visual context for AI coding agents.

## Phase 2 MVP

- Host: Tauri v2 + React + Rust, frameless always-on-top floating window.
- Phone: React PWA in the phone browser, back-camera capture over WebSocket.
- Agent bridge: Local API + File Heartbeat + `agent_vision/latest.png`.

## Run

```powershell
npm install
npm run setup:network
npm run dev
```

`npm run dev` 会同时启动 Host 和手机 HTTPS PWA（1421）。单独打开 Host 时也会自动尝试拉起 1421 服务。

开发编译会产生大量 Rust 缓存（`apps/host/src-tauri/target`，可达数 GB）。上传 GitHub 前不必提交该目录；本地瘦身可执行：

```powershell
npm run clean
```

Open the phone PWA from the phone browser:

```text
https://<computer-lan-ip>:1421
```

The Host floating window also has a Pair button. Click it to show the current LAN URL and QR code, then scan it from the phone browser/camera. If the HTTPS phone dev server is not running, Host falls back to:

```text
http://<computer-lan-ip>:17891/phone?host=<computer-lan-ip>
```

The fallback proves connectivity, but some mobile browsers only allow camera access on HTTPS, so `npm run dev:all` is still the recommended path.

For production-style Host build on Windows, use the MSVC environment:

```powershell
cmd.exe /d /s /c "set ""PATH=%USERPROFILE%\.cargo\bin;%PATH%"" && call ""C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"" && npm run build:host -- -- --no-bundle"
```

Generated Host exe:

```text
apps/host/src-tauri/target/release/agenteye-host.exe
```

## Agent Commands

```powershell
npm run agenteye -- thinking
npm run agenteye -- idle
npm run agenteye -- capture
npm run agenteye -- status
npm run agenteye -- paths
```

Agent-readable output:

```text
agent_vision/latest.png
agent_vision/latest.json
agent_vision/state.json
```
