#!/usr/bin/env bash
set -euo pipefail

echo "==> 停止 Scissor"
pkill -x scissor 2>/dev/null || true
sleep 0.4

echo "==> 移除可执行文件"
rm -f "$HOME/.local/bin/scissor" "$HOME/.local/bin/Scissor.AppImage"

echo "==> 移除桌面入口"
rm -f "$HOME/.local/share/applications/scissor.desktop" \
      "$HOME/.local/share/applications/Scissor.desktop" \
      "$HOME/.local/share/applications/com.scissor.desktop.desktop" \
      "$HOME/.local/share/applications/com.scissor.desktop.desktop.desktop"

echo "==> 移除图标"
find "$HOME/.local/share/icons" -name 'scissor.png' -delete 2>/dev/null || true
find "$HOME/.local/share/icons" -name 'Scissor.png' -delete 2>/dev/null || true
find "$HOME/.local/share/icons" -name 'com.scissor.desktop.png' -delete 2>/dev/null || true
rm -f "$HOME/.local/share/pixmaps/scissor.png" \
      "$HOME/.local/share/pixmaps/Scissor.png" \
      "$HOME/.local/share/pixmaps/com.scissor.desktop.png" 2>/dev/null || true

echo "==> 清理运行缓存"
rm -rf "$HOME/.local/share/com.scissor.desktop" \
       /tmp/scissor-overlays \
       /tmp/scissor.log /tmp/scissor2.log /tmp/scissor3.log 2>/dev/null || true
rm -rf "$HOME/.cache/thumbnails"/* 2>/dev/null || true

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

if [[ -f /usr/share/applications/Scissor.desktop ]] || [[ -x /usr/bin/scissor ]]; then
  echo "==> 仍检测到系统包 Scissor（apt/dpkg）"
  echo "    若要一并卸掉: sudo apt remove scissor"
fi

echo "==> 用户级卸载完成"
