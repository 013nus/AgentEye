# AgentEye 通信协议 MVP

**日期**: 2026-06-09  
**阶段**: Phase 2, State + Video + Capture

## 1. 当前通信方式

AgentEye MVP 采用 **电脑端 Host 作为局域网状态与视频中心** 的模式。

```text
External Agent / Script
  -> HTTP POST /api/state 或 agent_vision/state.json
  -> Host State Hub
  -> WebSocket /ws
  -> Phone PWA 状态灯

Phone PWA camera
  -> JPEG binary over WebSocket
  -> Host /video/push
  -> Host /video/feed
  -> Floating Window live frame
  -> Capture Publisher
  -> agent_vision/latest.png + latest.json
```

Host 监听地址:

```text
http://0.0.0.0:17891
ws://0.0.0.0:17891/ws
ws://0.0.0.0:17891/video/push
ws://0.0.0.0:17891/video/feed
```

## 2. Local API 模式

写入状态:

```powershell
Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:17891/api/state `
  -ContentType 'application/json' `
  -Body '{"state":"thinking"}'
```

切回空闲:

```powershell
Invoke-RestMethod `
  -Method Post `
  -Uri http://127.0.0.1:17891/api/state `
  -ContentType 'application/json' `
  -Body '{"state":"idle"}'
```

读取当前状态:

```powershell
Invoke-RestMethod http://127.0.0.1:17891/api/state
```

健康检查:

```powershell
Invoke-RestMethod http://127.0.0.1:17891/health
```

主动发布当前最新画面:

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:17891/api/capture
```

读取 Capture 配置:

```powershell
Invoke-RestMethod http://127.0.0.1:17891/api/capture
```

## 3. File Heartbeat 模式

Host 会轮询项目根目录下:

```text
AgentEye/agent_vision/state.json
```

文件内容:

```json
{
  "state": "thinking"
}
```

支持状态:

```text
idle
thinking
capturing
error
offline
```

写入示例:

```powershell
New-Item -ItemType Directory -Force .\agent_vision | Out-Null
'{"state":"thinking"}' | Set-Content -Encoding UTF8 .\agent_vision\state.json
```

## 4. WebSocket 状态广播

订阅地址:

```text
ws://<host-ip>:17891/ws
```

消息格式:

```json
{
  "state": "thinking",
  "sequence": 12,
  "updatedAt": "2026-06-09T01:10:05.140Z",
  "source": "local-api"
}
```

`source` 可能值:

```text
startup
local-api
file-heartbeat
```

## 5. 视频流 MVP

当前实现 MJPEG/WebSocket fallback。

Phone PWA 推送地址:

```text
ws://<电脑IP>:17891/video/push
```

传输内容:

```text
JPEG binary frame
```

默认参数:

```text
width: 640px
fps: 8
jpeg quality: 0.66
```

Host 悬浮窗订阅地址:

```text
ws://127.0.0.1:17891/video/feed
```

Host 会先发送一条 JSON metadata，再发送 JPEG binary:

```json
{
  "type": "video-frame",
  "sequence": 42,
  "receivedAt": "2026-06-09T01:10:05.140Z",
  "bytes": 32512
}
```

## 6. Agent 可读输出

Capture Publisher 输出:

```text
AgentEye/agent_vision/latest.png
AgentEye/agent_vision/latest.json
AgentEye/agent_vision/history/*.png
```

输出原则:

- `latest.png` 是真实 PNG，不是伪装扩展名的 JPEG。
- 写入使用临时文件 + rename，避免 Agent 读到半张图片。
- 默认每 5 秒自动发布一次。
- Host 工具栏截图按钮、`Ctrl+Alt+V`、`POST /api/capture` 都可以触发立即发布。
- `npm run agenteye -- capture` 会调用 `POST /api/capture`，适合 CLI Agent 或脚本触发。
- Windows 写入路径使用临时文件 + `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)` 替换，连续发布不会因为目标文件已存在而失败。

`latest.json` 示例:

```json
{
  "version": "0.1",
  "capturedAt": "2026-06-09T01:10:05.140Z",
  "imagePath": "C:\\Users\\Administrator\\Desktop\\毕业设计\\软件研发\\AgentEye\\agent_vision\\latest.png",
  "width": 640,
  "height": 360,
  "source": "phone-mjpeg",
  "mode": "board-only",
  "agentState": "idle",
  "cameraState": "connected",
  "sequence": 42,
  "sourceFrame": {
    "sequence": 42,
    "receivedAt": "2026-06-09T01:10:04.980Z"
  }
}
```

## 7. 手机端与电脑端对齐

当前对齐方式:

- Host 启动 `17891` 状态、视频、capture 服务。
- Host 悬浮窗 Pair 面板通过 `get_pairing_config` 读取当前电脑局域网 IP。
- Pair 面板优先生成 `https://<电脑IP>:1421/?host=<电脑IP>` 二维码。
- 如果 `1421` Phone PWA 未启动，Pair 面板退到 `http://<电脑IP>:17891/phone?host=<电脑IP>`，由 Host Hub 自己托管 Phone 页面，避免直接出现连接拒绝。
- Phone PWA 优先使用 URL query 里的 `host`，没有 `host` 时再通过 `window.location.hostname` 推导电脑 IP。
- Phone 连接 `ws://<电脑IP>:17891/ws` 接收状态灯。
- Phone 连接 `ws://<电脑IP>:17891/video/push` 推送摄像头画面。
- Host 悬浮窗连接 `ws://127.0.0.1:17891/video/feed` 显示画面。

手机状态灯:

- `thinking`: 黄灯
- `idle`: 绿灯
- `capturing`: 青灯
- `error`: 红灯
- `offline`: 灰灯

## 8. HTTPS 真机访问

手机浏览器调用摄像头需要 secure context。真机访问局域网地址时，建议使用 Phone PWA 的 HTTPS dev 模式:

```powershell
npm run dev:https -w @agenteye/phone
```

首次访问自签名证书页面时，浏览器可能要求手动信任。

HTTPS 模式下，手机页面不会直接连接 `ws://<电脑IP>:17891`，否则会被浏览器混合内容策略拦截。Phone dev server 已配置 WebSocket 代理:

```text
wss://<电脑IP>:1421/agenteye/ws
  -> ws://127.0.0.1:17891/ws

wss://<电脑IP>:1421/agenteye/video/push
  -> ws://127.0.0.1:17891/video/push
```

扫码 URL 示例:

```text
https://172.23.219.68:1421/?host=172.23.219.68
```

如果电脑连接的是手机热点，IP 可能在重连热点后变化。此时只需要刷新 Host Pair 面板并重新扫码，不需要手动查 IP。

注意: `17891/phone` 是连通性兜底入口。部分手机浏览器只允许 HTTPS 页面调用摄像头，因此实际摄像头推流仍推荐使用 `npm run dev:all` 启动 `1421` HTTPS Phone PWA。Phone PWA 会在非安全上下文中提示用户打开 HTTPS Pair 地址。

## 9. 后续视频流对齐

状态链路和 Capture 输出保持稳定，视频链路后续分两步:

1. MVP fallback: Phone 用 camera + canvas 编码 JPEG，经 WebSocket 推给 Host。
2. 正式主线: Phone 和 Host 用同一个配对 session 建立 WebRTC，视频走 WebRTC track，状态可继续走 `/ws`，也可迁移到 WebRTC DataChannel。

设计原则:

- 状态链路轻、稳、可被 Agent/脚本直接驱动。
- 视频链路后续单独演进，不影响黄/绿灯和 `latest.png` 输出。
- Host 始终是局域网通信中心，手机端只需知道电脑 IP 和端口。
