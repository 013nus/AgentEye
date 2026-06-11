# Phase 2 工程初始化说明

**日期**: 2026-06-09  
**阶段目标**: 初始化 AgentEye monorepo，并实现 Tauri Host 的悬浮窗基础能力。

## 1. 工程结构

```text
AgentEye/
  package.json
  apps/
    host/
      package.json
      index.html
      vite.config.ts
      src/
        main.tsx
        styles.css
        vite-env.d.ts
      src-tauri/
        Cargo.toml
        tauri.conf.json
        build.rs
        capabilities/default.json
        src/
          main.rs
          lib.rs
          floating_window.rs
    phone/
      package.json
      index.html
      vite.config.ts
      src/
        main.tsx
        styles.css
        vite-env.d.ts
  packages/
    protocol/
      package.json
      src/index.ts
  tools/
    agenteye-cli.mjs
  agent_vision/
    state.json
    latest.png
    latest.json
    history/
```

## 2. 已加入的核心依赖

Host 前端:

- React 19
- TypeScript 6
- Vite 7
- lucide-react
- @tauri-apps/api

Host Tauri/Rust:

- tauri v2
- tauri-plugin-global-shortcut
- tauri-plugin-window-state
- tauri-plugin-positioner
- tauri-plugin-clipboard-manager
- tauri-plugin-fs
- tauri-plugin-opener
- window-vibrancy
- axum/tokio/tower-http, 为后续 Local API 和 WebSocket 预留

Phone PWA:

- React 19
- TypeScript 6
- Vite 7
- lucide-react

## 3. 已实现的 Host 悬浮窗能力

Rust 侧:

- 从 `tauri.conf.json` 声明 `floating` 窗口。
- 无边框 `decorations: false`。
- 透明背景 `transparent: true`。
- 永远置顶 `alwaysOnTop: true`，并在 Rust 启动时再次强制设置。
- 跳过任务栏 `skipTaskbar: true`。
- 支持窗口阴影。
- 高 DPI 下校准最小尺寸。
- Windows 下尝试 Mica，失败后降级 Acrylic，再失败则使用透明 WebView 背景。
- 提供 `start_window_drag` 命令，前端可触发系统级拖拽。
- 提供 `set_mouse_passthrough` 命令，支持鼠标穿透开关。
- 提供 `set_floating_window_always_on_top` 命令，支持置顶开关。
- 注册全局快捷键 `Ctrl+Alt+V`，先广播事件，后续接 Capture Publisher。
- 悬浮窗提供左上角加宽专用拖拽把手，悬停显示 grab 光标。
- 拖拽从 Tauri `start_dragging()` 切换为非阻塞自绘拖拽: 前端读取全局鼠标物理坐标和窗口物理坐标，用 `setPosition` 更新窗口位置，避免 Windows 系统拖拽循环造成指针残影、截图延迟和 WebView 重绘阻塞。

## 3.1 已实现的 Host/Phone 状态通信能力

Host Rust 状态中心:

- 启动 `0.0.0.0:17891` HTTP/WebSocket 服务。
- `GET /health`: 健康检查。
- `GET /api/state`: 读取当前 Agent 状态。
- `POST /api/state`: Local API 模式写入状态。
- `GET /ws`: WebSocket 状态广播，给手机端 PWA 和未来观察客户端订阅。
- 轮询 `AgentEye/agent_vision/state.json`: File Heartbeat 模式。
- 通过 Tauri event `agenteye://agent-state` 同步状态到 Host 悬浮窗前端。

Phone PWA:

- 自动连接 `ws://<当前页面hostname>:17891/ws`。
- `thinking` 显示黄灯。
- `idle` 显示绿灯。
- `capturing` 显示青灯。
- `error` 显示红灯。
- `offline` 显示灰灯。

## 3.2 已实现的 Phone -> Host 视频 fallback

Host Rust:

- `GET /video/push`: 接收 Phone PWA 推送的 JPEG binary frame。
- `GET /video/feed`: 转发最新 JPEG binary frame 给 Host 悬浮窗。
- 视频帧不在 Rust 侧重编码，降低延迟和实现复杂度。

Phone PWA:

- 使用 `navigator.mediaDevices.getUserMedia()` 获取后置摄像头。
- 使用隐藏 canvas 压缩 JPEG。
- 默认 `640px` 宽、`8fps`、`0.66` JPEG quality。
- 通过 WebSocket 推送到 Host。

Host Floating Window:

- 订阅 `ws://127.0.0.1:17891/video/feed`。
- 接收 JPEG binary 后生成 Object URL 并实时显示。
- 显示 frame sequence 和摄像头连接状态。
- 提供 Pair 面板，自动显示当前电脑局域网 IP、手机 HTTPS 访问地址和二维码。
- Pair 面板显示 Phone PWA 服务是否在线，帮助定位 `1421` 未启动导致的扫码打不开问题。
- Host Hub 增加 `/phone` 和 `/phone/assets/*` 路由，在 `1421` 未启动时提供 HTTP 兜底页面。
- Phone PWA 在非 HTTPS 安全上下文中显示明确提示，避免暴露 `getUserMedia` 底层异常。

## 3.3 已实现的 Agent Capture Publisher

Rust Capture Publisher:

- 缓存 Host 最新收到的一帧手机 JPEG。
- 默认每 5 秒把最新帧发布为 `AgentEye/agent_vision/latest.png`。
- 同步写入 `AgentEye/agent_vision/latest.json`，记录时间、尺寸、Agent 状态和源帧 sequence。
- 保留历史帧到 `AgentEye/agent_vision/history/*.png`。
- 写入使用临时文件 + Windows `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` 替换，避免 Agent 读到半写入文件，也避免 Windows 重复覆盖失败。
- 捕获期间短暂切换 `capturing`，完成后恢复原先的 `thinking/idle/offline/error` 状态。

触发入口:

- Host 工具栏扫描按钮。
- Host 工具栏 Pair 按钮用于手机扫码接入。
- 全局快捷键 `Ctrl+Alt+V`。
- `POST http://127.0.0.1:17891/api/capture`。
- `npm run agenteye -- capture`。

CLI Agent 对齐工具:

```powershell
npm run agenteye -- thinking
npm run agenteye -- idle
npm run agenteye -- capture
npm run agenteye -- status
npm run agenteye -- paths
```

`thinking/idle` 会优先调用 Local API。Host 未启动时，CLI 自动写入 `agent_vision/state.json`，等待 Host 后续通过 File Heartbeat 读取。

真机注意:

- 手机浏览器摄像头需要 HTTPS。
- 推荐使用 `npm run dev:all` 同时启动 Host 和 Phone PWA。
- 也可以单独使用 `npm run dev:https -w @agenteye/phone` 启动手机端。
- HTTPS 模式下 Phone 会连接 Vite 代理路径 `wss://<电脑IP>:1421/agenteye/video/push`，由 dev server 转发到 Host。

通信协议文档:

```text
docs/AgentEye_通信协议_MVP.md
```

前端侧:

- Apple Industrial 风格的悬浮窗壳。
- Hover 工具条。
- 右下角微型像素状态灯。
- 拖拽区域使用 Rust/Tauri 原生拖拽，不手写坐标移动。
- 鼠标穿透和置顶按钮已接入 Rust command。

## 4. 验证结果

已通过:

```powershell
npm install
npm run setup:network
npm run dev:all
npm run typecheck
npm run vite:build -w @agenteye/host
npm run build:phone
```

HTTP / File Heartbeat smoke test 已通过:

```text
HealthOk         : True
ApiPostedState   : thinking
ApiCurrentState  : thinking
FileCurrentState : idle
FileSource       : file-heartbeat
```

Video WebSocket smoke test 已通过:

```text
{"ok":true,"gotText":true,"gotBinary":true}
```

Capture Publisher smoke test 已通过:

```text
firstCaptureOk   : True
secondCaptureOk  : True
latestPng        : AgentEye/agent_vision/latest.png
latestPngBytes   : 1558
latestJson       : AgentEye/agent_vision/latest.json
metadataWidth    : 32
metadataHeight   : 24
metadataAgentState: idle
```

Dev all smoke test 已通过:

```text
phonePwa1421 : True
hostHub17891 : True
url          : https://172.23.219.68:1421/?host=172.23.219.68
```

Rust/Cargo、WebView2 Runtime、Visual Studio 2022 Build Tools 均已安装并验证。

已通过完整 Host Tauri 构建:

```powershell
cmd.exe /d /s /c "set ""PATH=%USERPROFILE%\.cargo\bin;%PATH%"" && call ""C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"" && npm run build:host -- -- --no-bundle"
```

生成产物:

```text
C:\Users\Administrator\Desktop\毕业设计\软件研发\AgentEye\apps\host\src-tauri\target\release\agenteye-host.exe
```

环境细节:

- Rust: `rustc 1.96.0`, stable `x86_64-pc-windows-msvc`
- Cargo: `cargo 1.96.0`
- WebView2 Runtime: 已安装
- Visual Studio Build Tools: 2022, VCTools workload 已安装

注意:

当前系统 PATH 中存在 `C:\Git\usr\bin\link.exe`。它不是 MSVC linker。如果没有先加载 `vcvars64.bat`，Rust 可能误用 Git 的 `link.exe`，导致链接阶段失败。开发/构建 Tauri Host 时应使用上面的命令，或在 Visual Studio Developer Command Prompt 中执行。

## 5. 下一步建议

1. 增加真实窗口截图路径，支持只截 Host 悬浮窗或全屏含悬浮窗。
2. 增加 Phone/Host 配对码和局域网 IP 自动展示。
3. 接入 WebRTC 主链路，保留当前 MJPEG/WebSocket 为 fallback。
4. 增加托盘菜单、设置页和开机自启。
5. 为 CLI 增加 `agenteye watch <command>`，自动包裹 Agent 命令生命周期。
