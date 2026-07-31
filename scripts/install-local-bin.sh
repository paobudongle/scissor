#!/usr/bin/env bash
# 兼容旧入口：转调跨平台 Node 脚本
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec node "$ROOT/scripts/install-local-bin.mjs" "$@"
