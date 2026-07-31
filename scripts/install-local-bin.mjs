#!/usr/bin/env node
/**
 * 编译后同步本地可执行文件。
 * - Linux/macOS → ~/.local/bin/scissor
 * - Windows → 跳过（请用 NSIS 安装包；开发调试直接跑 target/release/scissor.exe）
 *
 * 由 tauri.conf.json beforeBundleCommand 调用；也可：npm run install:local
 * 跳过：SCISSOR_SKIP_LOCAL_INSTALL=1
 */
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

if (process.env.SCISSOR_SKIP_LOCAL_INSTALL === "1") {
  console.log("==> 跳过本地安装（SCISSOR_SKIP_LOCAL_INSTALL=1）");
  process.exit(0);
}

if (process.platform === "win32") {
  console.log("==> Windows 构建：跳过用户目录同步（请安装 NSIS 生成的 *-setup.exe）");
  const exe = path.join(root, "src-tauri", "target", "release", "scissor.exe");
  if (fs.existsSync(exe)) {
    console.log(`    调试可直接运行: ${exe}`);
  }
  process.exit(0);
}

const binSrc = path.join(root, "src-tauri", "target", "release", "scissor");
if (!fs.existsSync(binSrc)) {
  console.log(`未找到 ${binSrc}，跳过本地安装`);
  process.exit(0);
}

const destDir = path.join(os.homedir(), ".local", "bin");
const dest = path.join(destDir, "scissor");

fs.mkdirSync(destDir, { recursive: true });

// 避免覆盖正在运行的进程
spawnSync("pkill", ["-x", "scissor"], { stdio: "ignore" });
try {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 200);
} catch {
  /* ignore */
}

fs.copyFileSync(binSrc, dest);
fs.chmodSync(dest, 0o755);
console.log(`==> 已更新本地命令: ${dest}`);
