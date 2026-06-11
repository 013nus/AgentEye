#!/usr/bin/env node

import { existsSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { spawn } from "node:child_process";
import path from "node:path";
import process from "node:process";

const cargoBin = path.join(homedir(), ".cargo", "bin");
const vcvars64 = "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat";

const phoneChild = spawn("npm", ["run", "dev:https", "-w", "@agenteye/phone"], {
  cwd: process.cwd(),
  shell: true,
  stdio: ["ignore", "pipe", "pipe"],
});

phoneChild.stdout.on("data", (chunk) => writeLines("phone", chunk));
phoneChild.stderr.on("data", (chunk) => writeLines("phone", chunk));

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
phoneChild.on("exit", (code) => {
  console.log(`[phone] exited with code ${code}`);
  shutdown();
});

if (process.platform === "win32" && existsSync(vcvars64)) {
  const scriptPath = path.join(tmpdir(), "agenteye-dev-host.cmd");
  writeFileSync(
    scriptPath,
    [
      "@echo off",
      `set "PATH=${cargoBin};%PATH%"`,
      `call "${vcvars64}"`,
      "npm run dev -w @agenteye/host",
      "",
    ].join("\r\n"),
    "utf8",
  );

  run("cmd.exe", ["/d", "/c", scriptPath]);
} else {
  run("npm", ["run", "dev", "-w", "@agenteye/host"], {
    PATH: `${cargoBin}${path.delimiter}${process.env.PATH ?? ""}`,
  });
}

function writeLines(name, chunk) {
  String(chunk)
    .split(/\r?\n/)
    .filter(Boolean)
    .forEach((line) => console.log(`[${name}] ${line}`));
}

function run(command, args, extraEnv = {}) {
  const child = spawn(command, args, {
    cwd: process.cwd(),
    env: {
      ...process.env,
      ...extraEnv,
    },
    stdio: "inherit",
    shell: false,
  });

  child.on("exit", (code) => {
    process.exitCode = code ?? 0;
    shutdown();
  });
}

function shutdown() {
  if (!phoneChild.killed) {
    phoneChild.kill();
  }
}
