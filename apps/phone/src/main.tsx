import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Camera, Radio, Video, VideoOff } from "lucide-react";
import type { AgentEyeAgentState, AgentEyeStateSnapshot } from "@agenteye/protocol";
import "./styles.css";

const HOST_PORT = 17891;
const MAX_FRAME_LONG_EDGE = 1920;
const TARGET_FPS = 8;
const JPEG_QUALITY = 0.74;

function getHostAddress() {
  return new URLSearchParams(window.location.search).get("host") || window.location.hostname || "127.0.0.1";
}

function buildHttpsPhoneUrl(host: string) {
  return `https://${host}:1421/?host=${host}`;
}

function getSecureContextMessage() {
  const mediaDevices = navigator.mediaDevices as MediaDevices | undefined;
  if (window.isSecureContext && typeof mediaDevices?.getUserMedia === "function") {
    return undefined;
  }

  const host = getHostAddress();
  return [
    "当前浏览器要求 HTTPS 才能使用摄像头。",
    `请扫描电脑端 HTTPS 二维码，或打开 ${buildHttpsPhoneUrl(host)}`,
  ].join(" ");
}

function buildWebSocketUrl(path: "/ws" | "/video/push") {
  const host = getHostAddress();

  if (window.location.protocol === "https:") {
    const proxyPath = path === "/ws" ? "/agenteye/ws" : "/agenteye/video/push";
    return `wss://${host}:1421${proxyPath}`;
  }

  return `ws://${host}:${HOST_PORT}${path}`;
}

function App() {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const pushSocketRef = useRef<WebSocket | null>(null);

  const [agentState, setAgentState] = useState<AgentEyeAgentState>("offline");
  const [isStateConnected, setIsStateConnected] = useState(false);
  const [isVideoConnected, setIsVideoConnected] = useState(false);
  const [cameraError, setCameraError] = useState<string | undefined>();
  const [sequence, setSequence] = useState(0);
  const [sentFrames, setSentFrames] = useState(0);

  useEffect(() => {
    if (window.location.protocol !== "http:") {
      return;
    }

    const host = getHostAddress();
    const httpsUrl = buildHttpsPhoneUrl(host);
    const redirectTimer = window.setTimeout(() => {
      window.location.replace(httpsUrl);
    }, 900);

    setCameraError(`正在切换到 HTTPS 摄像头页面：${httpsUrl}`);
    return () => window.clearTimeout(redirectTimer);
  }, []);

  useEffect(() => {
    let socket: WebSocket | undefined;
    let reconnectTimer: number | undefined;
    let closedByEffect = false;

    const connect = () => {
      socket = new WebSocket(buildWebSocketUrl("/ws"));

      socket.onopen = () => {
        setIsStateConnected(true);
      };

      socket.onmessage = (event) => {
        const snapshot = JSON.parse(event.data) as AgentEyeStateSnapshot;
        setAgentState(snapshot.state);
        setSequence(snapshot.sequence);
      };

      socket.onclose = () => {
        setIsStateConnected(false);
        setAgentState("offline");

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
    };
  }, []);

  useEffect(() => {
    let captureTimer: number | undefined;
    let reconnectTimer: number | undefined;
    let closedByEffect = false;

    const connectVideoSocket = () => {
      const socket = new WebSocket(buildWebSocketUrl("/video/push"));
      pushSocketRef.current = socket;

      socket.onopen = () => {
        setIsVideoConnected(true);
      };

      socket.onclose = () => {
        setIsVideoConnected(false);
        if (!closedByEffect) {
          reconnectTimer = window.setTimeout(connectVideoSocket, 1200);
        }
      };

      socket.onerror = () => {
        socket.close();
      };
    };

    const startCamera = async () => {
      try {
        const secureContextMessage = getSecureContextMessage();
        if (secureContextMessage) {
          setCameraError(secureContextMessage);
          return;
        }

        const stream = await navigator.mediaDevices.getUserMedia({
          video: {
            facingMode: { ideal: "environment" },
            width: { ideal: 1920 },
            height: { ideal: 1080 },
            frameRate: { ideal: 30, max: 30 },
          },
          audio: false,
        });

        streamRef.current = stream;
        if (videoRef.current) {
          videoRef.current.srcObject = stream;
          videoRef.current.muted = true;
          videoRef.current.playsInline = true;
          await videoRef.current.play().catch((error) => {
            if (error instanceof DOMException && error.name === "AbortError") {
              return;
            }
            throw error;
          });
          setCameraError(undefined);
        }
      } catch (error) {
        setCameraError(error instanceof Error ? error.message : "摄像头授权失败");
      }
    };

    const captureFrame = () => {
      const video = videoRef.current;
      const canvas = canvasRef.current;
      const socket = pushSocketRef.current;

      if (!video || !canvas || !socket || socket.readyState !== WebSocket.OPEN || video.videoWidth === 0) {
        return;
      }

      const longEdge = Math.max(video.videoWidth, video.videoHeight);
      const scale = Math.min(1, MAX_FRAME_LONG_EDGE / longEdge);
      canvas.width = Math.round(video.videoWidth * scale);
      canvas.height = Math.round(video.videoHeight * scale);

      const context = canvas.getContext("2d");
      if (!context) {
        return;
      }

      context.drawImage(video, 0, 0, canvas.width, canvas.height);
      canvas.toBlob(
        (blob) => {
          if (!blob || socket.readyState !== WebSocket.OPEN) {
            return;
          }
          socket.send(blob);
          setSentFrames((value) => value + 1);
        },
        "image/jpeg",
        JPEG_QUALITY,
      );
    };

    connectVideoSocket();
    void startCamera();
    captureTimer = window.setInterval(captureFrame, Math.round(1000 / TARGET_FPS));

    return () => {
      closedByEffect = true;
      if (captureTimer) {
        window.clearInterval(captureTimer);
      }
      if (reconnectTimer) {
        window.clearTimeout(reconnectTimer);
      }
      pushSocketRef.current?.close();
      streamRef.current?.getTracks().forEach((track) => track.stop());
    };
  }, []);

  const stateLabel = useMemo(() => {
    if (!isStateConnected) {
      return "电脑端离线";
    }

    switch (agentState) {
      case "thinking":
        return "脚本思考中";
      case "capturing":
        return "正在发布快照";
      case "error":
        return "信号异常";
      case "idle":
        return "脚本空闲";
      default:
        return "电脑端离线";
    }
  }, [agentState, isStateConnected]);

  const lightState = isStateConnected ? agentState : "offline";

  return (
    <main className="phone-shell">
      <section className="camera-preview" aria-label="摄像头预览">
        <video ref={videoRef} className="camera-video" playsInline muted />
        {cameraError && (
          <div className="camera-error">
            <Camera size={34} strokeWidth={1.6} />
            <span>{cameraError}</span>
          </div>
        )}
        <div className="camera-overlay">
          {isVideoConnected ? <Video size={15} /> : <VideoOff size={15} />}
          <span>{isVideoConnected ? "正在传回电脑" : "视频链路离线"}</span>
          <small>#{sentFrames}</small>
        </div>
        <canvas ref={canvasRef} className="capture-canvas" aria-hidden="true" />
      </section>
      <section className={`status-light status-light--${lightState}`} aria-label="脚本状态灯">
        <div className="state-badge">
          <Radio size={15} />
          <span>{stateLabel}</span>
          <small>#{sequence}</small>
        </div>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
