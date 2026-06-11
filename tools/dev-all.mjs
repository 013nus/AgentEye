#!/usr/bin/env node

import { spawn } from "node:child_process";
import process from "node:process";

const commands = [
  {
    name: "phone",
    command: "npm",
    args: ["run", "dev:https", "-w", "@agenteye/phone"],
  },
  {
    name: "host",
    command: "npm",
    args: ["run", "dev:host"],
  },
];

const children = commands.map(({ name, command, args }) => {
  const child = spawn(command, args, {
    cwd: process.cwd(),
    shell: true,
    stdio: ["ignore", "pipe", "pipe"],
  });

  child.stdout.on("data", (chunk) => writeLines(name, chunk));
  child.stderr.on("data", (chunk) => writeLines(name, chunk));
  child.on("exit", (code) => {
    console.log(`[${name}] exited with code ${code}`);
    shutdown();
  });

  return child;
});

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);

function writeLines(name, chunk) {
  String(chunk)
    .split(/\r?\n/)
    .filter(Boolean)
    .forEach((line) => console.log(`[${name}] ${line}`));
}

function shutdown() {
  for (const child of children) {
    if (!child.killed) {
      child.kill();
    }
  }
}
