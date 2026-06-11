import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Bot,
  Check,
  Eye,
  Lock,
  MousePointer2,
  Pin,
  QrCode,
  RefreshCw,
  ScanLine,
  Sparkles,
  Unlock,
  X,
  Zap,
} from "lucide-react";
import QRCode from "qrcode";
import type { AgentEyeAgentState, AgentEyeStateSnapshot } from "@agenteye/protocol";
import "./styles.css";

type CameraState = "connected" | "pairing" | "error";
type CaptureResult = {
  ok: boolean;
  imagePath: string;
  metadataPath: string;
  capturedAt: string;
  sourceSequence: number;
  width: number;
  height: number;
};
type CaptureConfig = {
  outputDir: string;
  latestPng: string;
  latestJson: string;
  intervalSeconds: number;
};
type PairingConfig = {
  hostIp: string;
  phoneUrl: string;
  phoneHttpsUrl: string;
  phoneHttpUrl: string;
  phoneDevPort: number;
  statePort: number;
  phoneServerReady: boolean;
  detectedAt: string;
};
type GuideStep = {
  id: number;
  title: string;
  instruction: string;
  check: string;
  hint: string;
};
type GuideMessage = {
  id: number;
  role: "assistant" | "user";
  text: string;
};
type SharedControlsProps = {
  isPinned: boolean;
  isPassthrough: boolean;
  isGuideOpen?: boolean;
  onTogglePin: () => Promise<void> | void;
  onTogglePassthrough: () => Promise<void> | void;
  onToggleGuide?: () => Promise<void> | void;
  onPair: () => Promise<void> | void;
  onCapture: () => Promise<void> | void;
  onClose: () => Promise<void> | void;
};

const STATE_PORT = 17891;
const VIDEO_FEED_URL = `ws://127.0.0.1:${STATE_PORT}/video/feed`;
const isControlsView = new URLSearchParams(window.location.search).get("view") === "controls";

const GUIDE_STEPS: GuideStep[] = [
  {
    id: 1,
    title: "固定手机视角",
    instruction: "把手机摄像头对准开发板全貌，确保 USB 口、主控芯片、LED 区域都在画面里。",
    check: "画面稳定后点“我已完成”。",
    hint: "先别追求完美构图，能稳定看到板子最重要。",
  },
  {
    id: 2,
    title: "连接供电线",
    instruction: "插入开发板供电线，观察电源指示灯是否常亮。",
    check: "灯亮且不闪烁异常后点“我已完成”。",
    hint: "如果灯不亮，先检查线材和电脑 USB 口供电。",
  },
  {
    id: 3,
    title: "确认下载/调试线",
    instruction: "连接下载器或串口模块，优先确认 GND 已共地，再连接 TX/RX 或 SWDIO/SWCLK。",
    check: "连接完成后点“我已完成”。",
    hint: "杜邦线多的时候，先共地，后信号线，最后再上电复查。",
  },
  {
    id: 4,
    title: "发布观察快照",
    instruction: "点击截图按钮发布 latest.png，让外部自动化脚本读取当前硬件画面。",
    check: "看到 latest.png updated 后点“我已完成”。",
    hint: "这一步相当于把物理现场交给本地脚本读取。",
  },
  {
    id: 5,
    title: "等待下一条硬件指令",
    instruction: "保持手机画面稳定，等待自动化脚本或人工输入下一条接线/验证目标。",
    check: "如果已经完成当前任务，可以重新开始一轮。",
    hint: "后续可接入 OpenAI/Codex API，把这里升级成真实多轮推理。",
  },
];

function getAgentLabel(state: AgentEyeAgentState) {
  switch (state) {
    case "thinking":
      return "脚本思考中";
    case "capturing":
      return "正在发布快照";
    case "error":
      return "状态异常";
    case "offline":
      return "脚本离线";
    default:
      return "脚本空闲";
  }
}

function getCameraLabel(state: CameraState) {
  switch (state) {
    case "connected":
      return "摄像头已连接";
    case "error":
      return "摄像头异常";
    default:
      return "等待手机连接";
  }
}

function useHostBridge() {
  const [isPassthrough, setIsPassthrough] = useState(false);
  const [isPinned, setIsPinned] = useState(true);
  const [agentState, setAgentState] = useState<AgentEyeAgentState>("idle");
  const [stateSequence, setStateSequence] = useState(0);
  const [captureConfig, setCaptureConfig] = useState<CaptureConfig | undefined>();
  const [lastCapture, setLastCapture] = useState<CaptureResult | undefined>();
  const [captureError, setCaptureError] = useState<string | undefined>();
  const [pairingConfig, setPairingConfig] = useState<PairingConfig | undefined>();
  const [pairingQrUrl, setPairingQrUrl] = useState<string | undefined>();
  const [isPairingOpen, setIsPairingOpen] = useState(false);
  const [isPassthroughPreview, setIsPassthroughPreview] = useState(false);

  useEffect(() => {
    invoke<AgentEyeStateSnapshot>("get_agent_state")
      .then((snapshot) => {
        setAgentState(snapshot.state);
        setStateSequence(snapshot.sequence);
      })
      .catch((error) => console.warn("读取 AgentEye 状态失败", error));

    invoke<CaptureConfig>("get_capture_config")
      .then(setCaptureConfig)
      .catch((error) => console.warn("读取快照配置失败", error));

    const unlistenPromise = listen<AgentEyeStateSnapshot>("agenteye://agent-state", (event) => {
      setAgentState(event.payload.state);
      setStateSequence(event.payload.sequence);
    });
    const captureResultPromise = listen<CaptureResult>("agenteye://capture-result", (event) => {
      setLastCapture(event.payload);
      setCaptureError(undefined);
    });
    const captureErrorPromise = listen<{ message: string }>("agenteye://capture-error", (event) => {
      setCaptureError(event.payload.message);
    });
    const passthroughPromise = listen<{ enabled: boolean }>("agenteye://mouse-passthrough", (event) => {
      setIsPassthrough(event.payload.enabled);
      if (!event.payload.enabled) {
        setIsPassthroughPreview(false);
      }
    });
    const passthroughPreviewPromise = listen<{ active: boolean }>("agenteye://mouse-passthrough-preview", (event) => {
      setIsPassthroughPreview(event.payload.active);
    });

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
      void captureResultPromise.then((unlisten) => unlisten());
      void captureErrorPromise.then((unlisten) => unlisten());
      void passthroughPromise.then((unlisten) => unlisten());
      void passthroughPreviewPromise.then((unlisten) => unlisten());
    };
  }, []);

  const refreshPairingConfig = useCallback(async () => {
    const config = await invoke<PairingConfig>("prepare_pairing_config");
    setPairingConfig(config);
    const qrUrl = await QRCode.toDataURL(config.phoneUrl, {
      errorCorrectionLevel: "M",
      margin: 1,
      width: 188,
      color: {
        dark: "#101418",
        light: "#f4f7fa",
      },
    });
    setPairingQrUrl(qrUrl);
  }, []);

  useEffect(() => {
    void refreshPairingConfig().catch((error) => console.warn("读取配对配置失败", error));
  }, [refreshPairingConfig]);

  useEffect(() => {
    const pairingPanelPromise = listen("agenteye://open-pairing", () => {
      setIsPairingOpen(true);
      void refreshPairingConfig().catch((error) => console.warn("刷新配对二维码失败", error));
    });

    return () => {
      void pairingPanelPromise.then((unlisten) => unlisten());
    };
  }, [refreshPairingConfig]);

  const togglePassthrough = useCallback(async () => {
    const nextValue = !isPassthrough;
    await invoke("set_mouse_passthrough", { enabled: nextValue });
    setIsPassthrough(nextValue);
  }, [isPassthrough]);

  const togglePin = useCallback(async () => {
    const nextValue = !isPinned;
    await invoke("set_floating_window_always_on_top", { enabled: nextValue });
    setIsPinned(nextValue);
  }, [isPinned]);

  const captureNow = useCallback(async () => {
    try {
      const result = await invoke<CaptureResult>("capture_latest_frame");
      setLastCapture(result);
      setCaptureError(undefined);
    } catch (error) {
      setCaptureError(error instanceof Error ? error.message : String(error));
    }
  }, []);

  const openPairing = useCallback(async () => {
    setIsPairingOpen(true);
    await refreshPairingConfig();
    await invoke("request_pairing_panel").catch((error) => console.warn("请求主窗口打开配对面板失败", error));
  }, [refreshPairingConfig]);

  const closeAgentEye = useCallback(async () => {
    await invoke("close_agenteye");
  }, []);

  return {
    isPassthrough,
    isPassthroughPreview,
    isPinned,
    agentState,
    stateSequence,
    captureConfig,
    lastCapture,
    captureError,
    pairingConfig,
    pairingQrUrl,
    isPairingOpen,
    setIsPairingOpen,
    refreshPairingConfig,
    togglePassthrough,
    togglePin,
    captureNow,
    openPairing,
    closeAgentEye,
  };
}

function useVideoFeed() {
  const [cameraState, setCameraState] = useState<CameraState>("pairing");
  const [frameUrl, setFrameUrl] = useState<string | undefined>();
  const [frameSequence, setFrameSequence] = useState(0);

  useEffect(() => {
    let socket: WebSocket | undefined;
    let reconnectTimer: number | undefined;
    let lastFrameUrl: string | undefined;
    let closedByEffect = false;

    const connect = () => {
      socket = new WebSocket(VIDEO_FEED_URL);
      socket.binaryType = "blob";

      socket.onopen = () => {
        setCameraState("pairing");
      };

      socket.onmessage = (event) => {
        if (typeof event.data === "string") {
          try {
            const metadata = JSON.parse(event.data) as { sequence?: number };
            if (typeof metadata.sequence === "number") {
              setFrameSequence(metadata.sequence);
            }
          } catch {
            // 视频 metadata 只用于诊断，解析失败不影响画面。
          }
          return;
        }

        const nextUrl = URL.createObjectURL(event.data as Blob);
        setFrameUrl(nextUrl);
        setCameraState("connected");

        if (lastFrameUrl) {
          URL.revokeObjectURL(lastFrameUrl);
        }
        lastFrameUrl = nextUrl;
      };

      socket.onclose = () => {
        setCameraState("error");
        if (!closedByEffect) {
          reconnectTimer = window.setTimeout(connect, 1200);
        }
      };

      socket.onerror = () => {
        socket?.close();
      };
    };

    connect();

    return () => {
      closedByEffect = true;
      if (reconnectTimer) {
        window.clearTimeout(reconnectTimer);
      }
      socket?.close();
      if (lastFrameUrl) {
        URL.revokeObjectURL(lastFrameUrl);
      }
    };
  }, []);

  return { cameraState, frameUrl, frameSequence };
}

function useFloatingDrag(isDragLocked: boolean) {
  const beginWindowDrag = useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (isDragLocked || event.button !== 0) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      void invoke("start_window_drag").catch((error) => console.warn("启动 AgentEye 拖拽失败", error));
    },
    [isDragLocked],
  );

  return { beginWindowDrag };
}

function useGuide(
  enabled: boolean,
  cameraState: CameraState,
  agentState: AgentEyeAgentState,
) {
  const [currentStepIndex, setCurrentStepIndex] = useState(0);
  const [isThinking, setIsThinking] = useState(false);
  const timerRef = useRef<number | undefined>(undefined);
  const [messages, setMessages] = useState<GuideMessage[]>([
    {
      id: 1,
      role: "assistant",
      text: "我会只给下一步，不刷长篇。你完成一步就点按钮，我先快速确认，再立刻给下一步。",
    },
  ]);

  const currentStep = GUIDE_STEPS[currentStepIndex];

  useEffect(() => {
    if (enabled) {
      return;
    }

    if (timerRef.current) {
      window.clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
    setIsThinking(false);
  }, [enabled]);

  const completeStep = useCallback(() => {
    if (!enabled) {
      return;
    }

    const nextIndex = Math.min(currentStepIndex + 1, GUIDE_STEPS.length - 1);
    const nextStep = GUIDE_STEPS[nextIndex];

    setMessages((items) => [
      ...items,
      { id: Date.now(), role: "user" as const, text: "我已完成。" },
      { id: Date.now() + 1, role: "assistant" as const, text: "看到你弄好了，我先给下一步。" },
    ].slice(-5));
    setIsThinking(true);

    timerRef.current = window.setTimeout(() => {
      setCurrentStepIndex(nextIndex);
      setMessages((items) => [
        ...items,
        {
          id: Date.now() + 2,
          role: "assistant" as const,
          text: `下一步：${nextStep.instruction}`,
        },
      ].slice(-5));
      setIsThinking(false);
      timerRef.current = undefined;
    }, 360);
  }, [currentStepIndex, enabled]);

  const resetGuide = useCallback(() => {
    setCurrentStepIndex(0);
    setIsThinking(false);
    setMessages([
      {
        id: Date.now(),
        role: "assistant",
        text: "流程已重置。我们从固定手机视角开始。",
      },
    ]);
  }, []);

  const quickContext = useMemo(() => {
    if (agentState === "thinking") {
      return "脚本正在处理，先别急着换线。";
    }
    if (cameraState !== "connected") {
      return "先让手机画面连上，我再带你做下一步。";
    }
    return "你做完就点按钮，我会立刻接下一步。";
  }, [agentState, cameraState]);

  return { currentStep, currentStepIndex, isThinking, messages, quickContext, completeStep, resetGuide };
}

function ControlButtons({
  isPinned,
  isPassthrough,
  isGuideOpen,
  onTogglePin,
  onTogglePassthrough,
  onToggleGuide,
  onPair,
  onCapture,
  onClose,
}: SharedControlsProps) {
  return (
    <div className="control-buttons" aria-label="AgentEye 控制按钮">
      <button type="button" onClick={onTogglePin} title={isPinned ? "取消置顶" : "置顶窗口"}>
        <Pin size={15} />
      </button>
      <button type="button" onClick={onPair} title="手机配对">
        <QrCode size={15} />
      </button>
      {onToggleGuide && (
        <button
          type="button"
          className={isGuideOpen ? "control-button--active" : undefined}
          onClick={onToggleGuide}
          title={isGuideOpen ? "关闭现场引导" : "开启现场引导"}
        >
          <Bot size={15} />
        </button>
      )}
      <button
        type="button"
        className={isPassthrough ? "control-button--active" : undefined}
        onClick={onTogglePassthrough}
        title={isPassthrough ? "关闭鼠标穿透" : "开启鼠标穿透"}
      >
        {isPassthrough ? <Unlock size={15} /> : <Lock size={15} />}
      </button>
      <button type="button" onClick={onCapture} title="发布 latest.png">
        <ScanLine size={15} />
      </button>
      <button type="button" className="control-button--danger" onClick={onClose} title="关闭 AgentEye">
        <X size={15} />
      </button>
    </div>
  );
}

function ControlsApp() {
  const bridge = useHostBridge();

  return (
    <main className="controls-shell">
      <ControlButtons
        isPinned={bridge.isPinned}
        isPassthrough={bridge.isPassthrough}
        onTogglePin={bridge.togglePin}
        onTogglePassthrough={bridge.togglePassthrough}
        onPair={bridge.openPairing}
        onCapture={bridge.captureNow}
        onClose={bridge.closeAgentEye}
      />
    </main>
  );
}

function HostApp() {
  const bridge = useHostBridge();
  const video = useVideoFeed();
  const drag = useFloatingDrag(bridge.isPassthrough && !bridge.isPassthroughPreview);
  const [isGuideOpen, setIsGuideOpen] = useState(
    () => window.localStorage.getItem("agenteye.guide.manualEnabled.v2") === "true",
  );
  const guide = useGuide(isGuideOpen, video.cameraState, bridge.agentState);
  const agentLabel = getAgentLabel(bridge.agentState);
  const cameraLabel = getCameraLabel(video.cameraState);
  const toggleGuide = useCallback(() => {
    setIsGuideOpen((current) => {
      const nextValue = !current;
      window.localStorage.setItem("agenteye.guide.manualEnabled.v2", String(nextValue));
      return nextValue;
    });
  }, []);

  return (
    <main
      className={`floating-shell ${isGuideOpen ? "floating-shell--guide-open" : "floating-shell--guide-closed"} ${
        bridge.isPassthroughPreview ? "floating-shell--passthrough-preview" : ""
      }`}
      aria-label="AgentEye 悬浮窗"
    >
      {isGuideOpen ? (
        <section className="guide-panel" aria-label="现场工序引导">
          <div className="guide-header">
            <div className="guide-mark">
              <Bot size={16} />
            </div>
            <div>
              <strong>现场引导</strong>
              <span>{guide.isThinking ? "准备下一步" : "手动模式"}</span>
            </div>
            <button
              type="button"
              className="guide-close"
              onClick={toggleGuide}
              title="关闭现场引导，停止本地引导逻辑"
            >
              <X size={14} />
            </button>
          </div>

          <div className="guide-step">
            <div className="guide-step-index">{String(guide.currentStepIndex + 1).padStart(2, "0")}</div>
            <div>
              <span>当前步骤</span>
              <strong>{guide.currentStep.title}</strong>
            </div>
          </div>

          <p className="guide-instruction">{guide.currentStep.instruction}</p>
          <p className="guide-check">{guide.currentStep.check}</p>

          <div className="guide-actions">
            <button type="button" className="guide-primary" onClick={guide.completeStep}>
              <Check size={15} />
              <span>我已完成</span>
            </button>
            <button type="button" className="guide-secondary" onClick={guide.resetGuide} title="重置流程">
              <RefreshCw size={14} />
            </button>
          </div>

          <div className="guide-context">
            <Sparkles size={14} />
            <span>{guide.quickContext}</span>
          </div>

          <div className="guide-log" aria-label="引导消息">
            {guide.messages.map((message) => (
              <div key={message.id} className={`guide-message guide-message--${message.role}`}>
                {message.text}
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <section className="visual-panel">
        <div className="host-controls">
          <ControlButtons
            isPinned={bridge.isPinned}
            isPassthrough={bridge.isPassthrough}
            isGuideOpen={isGuideOpen}
            onTogglePin={bridge.togglePin}
            onTogglePassthrough={bridge.togglePassthrough}
            onToggleGuide={toggleGuide}
            onPair={bridge.openPairing}
            onCapture={bridge.captureNow}
            onClose={bridge.closeAgentEye}
          />
        </div>

        <section className="video-plane">
          {video.frameUrl ? (
            <img className="live-frame" src={video.frameUrl} alt="硬件实时画面" draggable={false} />
          ) : (
            <>
              <div className="scan-grid" />
              <div className="empty-state">
                <Eye size={28} strokeWidth={1.7} />
                <div>
                  <strong>AgentEye</strong>
                  <span>{cameraLabel}</span>
                </div>
              </div>
            </>
          )}
          {video.frameSequence > 0 && (
            <div className="frame-counter">
              <span>帧</span>
              <strong>#{video.frameSequence}</strong>
            </div>
          )}
        </section>

        <div className={`agent-ribbon agent-ribbon--${bridge.agentState}`} aria-label={agentLabel}>
          <Zap size={13} />
          <span>{agentLabel}</span>
          <small>#{bridge.stateSequence}</small>
        </div>

        <div className="capture-ribbon" title={bridge.captureConfig?.latestPng}>
          <span>{bridge.lastCapture ? "latest.png 已更新" : "等待发布快照"}</span>
          <small>
            {bridge.captureError ?? (bridge.lastCapture ? `${bridge.lastCapture.width}x${bridge.lastCapture.height}` : "自动 5s")}
          </small>
        </div>

        {bridge.isPassthrough && (
          <div className="passthrough-badge">
            <MousePointer2 size={13} />
            <span>画面穿透中，右上角控制条仍可点击</span>
          </div>
        )}

        {bridge.isPairingOpen && (
          <aside className="pairing-panel" aria-label="手机配对面板">
            <div className="pairing-header">
              <div>
                <strong>配对手机</strong>
                <span>
                  {bridge.pairingConfig
                    ? `${bridge.pairingConfig.hostIp} | ${bridge.pairingConfig.phoneServerReady ? "HTTPS 已就绪" : "HTTPS 未启动"}`
                    : "正在检测"}
                </span>
              </div>
              <button type="button" onClick={() => bridge.setIsPairingOpen(false)} title="关闭配对">
                <X size={14} />
              </button>
            </div>
            <div className="qr-surface">
              {bridge.pairingQrUrl ? <img src={bridge.pairingQrUrl} alt="手机配对二维码" /> : <QrCode size={42} />}
            </div>
            <div className="pairing-url" title={bridge.pairingConfig?.phoneUrl}>
              {bridge.pairingConfig?.phoneUrl ?? "正在生成手机入口"}
            </div>
            <div className="pairing-actions">
              <button type="button" onClick={bridge.refreshPairingConfig} title="刷新局域网地址">
                <RefreshCw size={14} />
              </button>
              <span>
                {bridge.pairingConfig?.phoneServerReady ? "HTTPS 服务就绪" : "正在启动手机 HTTPS 服务..."} | Hub{" "}
                {bridge.pairingConfig?.statePort ?? 17891}
              </span>
            </div>
          </aside>
        )}

        <div
          className="drag-strip"
          data-tauri-drag-region
          onPointerDown={drag.beginWindowDrag}
          aria-hidden="true"
        />

        <button
          type="button"
          className="drag-handle"
          data-tauri-drag-region
          onPointerDown={drag.beginWindowDrag}
          title="拖动窗口"
        >
          <MousePointer2 size={14} />
          <span>拖动</span>
        </button>

        <div className={`pixel-status pixel-status--${video.cameraState}`} title={cameraLabel} />
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isControlsView ? <ControlsApp /> : <HostApp />}</React.StrictMode>,
);
