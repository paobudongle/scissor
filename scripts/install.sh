#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_SRC="$ROOT/src-tauri/target/release/scissor"
ICONS_DIR="$ROOT/src-tauri/icons"
ICON_NAME="com.scissor.desktop"
# 与 deb 包同名，用户目录会覆盖 /usr/share/applications/Scissor.desktop，避免双图标
DESKTOP_ID="Scissor.desktop"

if [[ ! -x "$BIN_SRC" ]]; then
  echo "未找到 release 二进制，先执行: npm run tauri build"
  echo "或: (cd src-tauri && cargo build --release)"
  exit 1
fi
if [[ ! -f "$ICONS_DIR/icon.png" ]]; then
  echo "未找到图标: $ICONS_DIR/icon.png"
  echo "可先执行: npm run tauri icon app-icon.png"
  exit 1
fi

echo "==> 安装可执行文件"
node "$ROOT/scripts/install-local-bin.mjs"

if [[ -x /usr/bin/scissor ]]; then
  echo "==> 检测到系统包 /usr/bin/scissor"
  echo "    PATH 会优先使用 \$HOME/.local/bin/scissor（本次已更新为新版）"
  echo "    桌面项将写入同名 $DESKTOP_ID 以覆盖系统入口，只显示一个图标"
  echo "    若只想保留 apt 包，可改跑: ./scripts/uninstall.sh && 使用 /usr/bin/scissor"
fi

echo "==> 安装多尺寸图标 ($ICON_NAME)"
# 确保用户 hicolor 主题可被 GTK 索引
mkdir -p "$HOME/.local/share/icons/hicolor"
if [[ ! -f "$HOME/.local/share/icons/hicolor/index.theme" ]]; then
  cat > "$HOME/.local/share/icons/hicolor/index.theme" <<'THEME'
[Icon Theme]
Name=Hicolor
Comment=Fallback icon theme
Directories=16x16/apps,24x24/apps,32x32/apps,48x48/apps,64x64/apps,128x128/apps,256x256/apps,512x512/apps

[16x16/apps]
Size=16
Context=Applications
Type=Fixed

[24x24/apps]
Size=24
Context=Applications
Type=Fixed

[32x32/apps]
Size=32
Context=Applications
Type=Fixed

[48x48/apps]
Size=48
Context=Applications
Type=Fixed

[64x64/apps]
Size=64
Context=Applications
Type=Fixed

[128x128/apps]
Size=128
Context=Applications
Type=Fixed

[256x256/apps]
Size=256
Context=Applications
Type=Fixed

[512x512/apps]
Size=512
Context=Applications
Type=Fixed
THEME
fi

install_sized_icon() {
  local size="$1"
  local preferred="$2"
  local dir="$HOME/.local/share/icons/hicolor/${size}x${size}/apps"
  local src="$ICONS_DIR/icon.png"
  mkdir -p "$dir"
  if [[ -n "$preferred" && -f "$preferred" ]]; then
    src="$preferred"
  fi
  # 精确缩放到目标尺寸（直接拷贝会导致 16/24 目录里放错尺寸）
  if command -v convert >/dev/null 2>&1; then
    convert "$src" -resize "${size}x${size}" "$dir/${ICON_NAME}.png"
  elif [[ "$src" == *"/${size}x${size}.png" ]]; then
    install -m 644 "$src" "$dir/${ICON_NAME}.png"
  else
    echo "需要 ImageMagick(convert) 以生成 ${size}x${size} 图标" >&2
    exit 1
  fi
  # 兼容旧桌面项 / WM_CLASS / deb Icon=scissor 查找
  cp -f "$dir/${ICON_NAME}.png" "$dir/scissor.png"
  cp -f "$dir/${ICON_NAME}.png" "$dir/Scissor.png"
}

install_sized_icon 16  "$ICONS_DIR/32x32.png"
install_sized_icon 24  "$ICONS_DIR/32x32.png"
install_sized_icon 32  "$ICONS_DIR/32x32.png"
install_sized_icon 48  "$ICONS_DIR/64x64.png"
install_sized_icon 64  "$ICONS_DIR/64x64.png"
install_sized_icon 128 "$ICONS_DIR/128x128.png"
install_sized_icon 256 "$ICONS_DIR/128x128@2x.png"
install_sized_icon 512 "$ICONS_DIR/icon.png"

# pixmaps 兜底（部分桌面环境只扫这里）
mkdir -p "$HOME/.local/share/pixmaps"
install -m 644 "$ICONS_DIR/128x128.png" "$HOME/.local/share/pixmaps/${ICON_NAME}.png"
cp -f "$HOME/.local/share/pixmaps/${ICON_NAME}.png" "$HOME/.local/share/pixmaps/scissor.png"
cp -f "$HOME/.local/share/pixmaps/${ICON_NAME}.png" "$HOME/.local/share/pixmaps/Scissor.png"

ICON_FILE="$HOME/.local/share/icons/hicolor/128x128/apps/${ICON_NAME}.png"

echo "==> 写入桌面入口（仅一份，覆盖系统同名项）"
mkdir -p "$HOME/.local/share/applications"
# 清理历史重复入口（旧 APP_ID 命名会产生第二个图标）
rm -f "$HOME/.local/share/applications/scissor.desktop" \
      "$HOME/.local/share/applications/com.scissor.desktop.desktop" \
      "$HOME/.local/share/applications/com.scissor.desktop.desktop.desktop"

# 绝对路径 Exec，避免 PATH 里旧二进制；文件名与 deb 一致以覆盖双图标
cat > "$HOME/.local/share/applications/$DESKTOP_ID" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Scissor
GenericName=Screenshot Tool
Comment=轻量截图工具 · 全屏 / 选区 / 滚动
Exec=$HOME/.local/bin/scissor
Icon=${ICON_FILE}
Terminal=false
Categories=Graphics;
Keywords=screenshot;capture;截图;scissors;
StartupNotify=true
StartupWMClass=scissor
EOF
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi
# 清掉常见图标/缩略图缓存，强制桌面重新读设计稿
rm -rf "$HOME/.cache/thumbnails"/* \
       "$HOME/.cache/icon-cache.kcache" \
       "$HOME/.cache/plasma_theme_cache" 2>/dev/null || true
touch "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "==> 安装完成"
echo "    命令: $HOME/.local/bin/scissor"
echo "    which scissor -> $(command -v scissor 2>/dev/null || echo '?')"
echo "    桌面项: $HOME/.local/share/applications/$DESKTOP_ID"
echo "    若菜单仍显示两个图标：注销重登，或 sudo apt remove scissor 后仅保留用户安装"
