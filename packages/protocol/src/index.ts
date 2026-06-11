export type AgentEyeAgentState = "idle" | "thinking" | "capturing" | "error" | "offline";

export type AgentEyeCameraState = "connected" | "pairing" | "error" | "offline";

export interface AgentEyeStateEvent {
  type: "agent-state";
  state: AgentEyeAgentState;
  sequence: number;
  updatedAt: string;
}

export interface AgentEyeStateSnapshot {
  state: AgentEyeAgentState;
  sequence: number;
  updatedAt: string;
  source: "startup" | "local-api" | "file-heartbeat";
}

export interface AgentEyeObservationMetadata {
  version: "0.1";
  capturedAt: string;
  imagePath: string;
  width: number;
  height: number;
  source: "phone-webrtc" | "phone-mjpeg" | "usb-camera" | "unknown";
  mode: "board-only" | "window" | "fullscreen";
  agentState: AgentEyeAgentState;
  cameraState: AgentEyeCameraState;
  sequence: number;
}

export interface AgentEyeVideoFrameMetadata {
  type: "video-frame";
  sequence: number;
  receivedAt: string;
  bytes: number;
}

export interface AgentEyeCaptureResult {
  ok: boolean;
  imagePath: string;
  metadataPath: string;
  capturedAt: string;
  sourceSequence: number;
  width: number;
  height: number;
}
