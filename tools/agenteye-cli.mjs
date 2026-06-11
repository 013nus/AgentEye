#!/usr/bin/env node

import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const VALID_STATES = new Set(["idle", "thinking", "capturing", "error", "offline"]);
const DEFAULT_BASE_URL = process.env.AGENTEYE_HOST_URL ?? "http://127.0.0.1:17891";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");
const visionDir = path.join(repoRoot, "agent_vision");
const heartbeatPath = path.join(visionDir, "state.json");

const [, , command, value, ...rest] = process.argv;
const options = parseOptions(rest);
const baseUrl = options.host ?? DEFAULT_BASE_URL;

try {
  await run(command, value);
} catch (error) {
  console.error(`[AgentEye] ${error.message}`);
  process.exitCode = 1;
}

async function run(commandName, valueName) {
  switch (commandName) {
    case "state":
      await setState(requiredState(valueName));
      break;
    case "thinking":
    case "idle":
    case "capturing":
    case "error":
    case "offline":
      await setState(commandName);
      break;
    case "capture":
      await captureNow();
      break;
    case "status":
      await printStatus();
      break;
    case "paths":
      printPaths();
      break;
    case "help":
    case undefined:
      printHelp();
      break;
    default:
      throw new Error(`未知命令: ${commandName}`);
  }
}

async function setState(state) {
  try {
    const snapshot = await postJson("/api/state", { state });
    console.log(`[AgentEye] Local API 状态已切换: ${snapshot.state} #${snapshot.sequence}`);
  } catch (error) {
    // Host 未启动时仍然写入 File Heartbeat；Host 启动后会在 500ms 轮询周期内读到它。
    await writeHeartbeat(state);
    console.log(`[AgentEye] Local API 不可用，已写入 File Heartbeat: ${state}`);
    console.log(`[AgentEye] ${heartbeatPath}`);
  }
}

async function captureNow() {
  const result = await postJson("/api/capture", {});
  console.log(`[AgentEye] latest.png 已发布: ${result.imagePath}`);
  console.log(`[AgentEye] metadata: ${result.metadataPath}`);
  console.log(`[AgentEye] source frame: #${result.sourceSequence}, ${result.width}x${result.height}`);
}

async function printStatus() {
  const response = await fetch(`${baseUrl}/api/state`);
  if (!response.ok) {
    throw new Error(`读取状态失败: HTTP ${response.status}`);
  }
  const snapshot = await response.json();
  console.log(JSON.stringify(snapshot, null, 2));
}

function printPaths() {
  console.log(JSON.stringify(
    {
      repoRoot,
      visionDir,
      heartbeatPath,
      latestPng: path.join(visionDir, "latest.png"),
      latestJson: path.join(visionDir, "latest.json"),
    },
    null,
    2,
  ));
}

async function postJson(route, body) {
  const response = await fetch(`${baseUrl}${route}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  const payload = text ? JSON.parse(text) : {};

  if (!response.ok) {
    throw new Error(payload.message ?? `HTTP ${response.status}`);
  }
  return payload;
}

async function writeHeartbeat(state) {
  await mkdir(visionDir, { recursive: true });
  await writeFile(heartbeatPath, `${JSON.stringify({ state }, null, 2)}\n`, "utf8");
}

function requiredState(state) {
  if (!VALID_STATES.has(state)) {
    throw new Error(`state 必须是: ${Array.from(VALID_STATES).join(", ")}`);
  }
  return state;
}

function parseOptions(args) {
  const parsed = {};
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--host") {
      parsed.host = args[index + 1];
      index += 1;
    }
  }
  return parsed;
}

function printHelp() {
  console.log(`AgentEye CLI

Usage:
  npm run agenteye -- thinking
  npm run agenteye -- idle
  npm run agenteye -- state thinking
  npm run agenteye -- capture
  npm run agenteye -- status
  npm run agenteye -- paths

Options:
  --host http://127.0.0.1:17891

Notes:
  state 命令优先调用 Local API；Host 不在线时自动写入 agent_vision/state.json。
  capture 命令需要 Host 在线，且已经接收到至少一帧手机摄像头画面。
`);
}
