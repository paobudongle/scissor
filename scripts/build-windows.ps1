#Requires -Version 5.1
<#
.SYNOPSIS
  在 Windows 11 本机打包 Scissor NSIS 安装包（*-setup.exe）

.DESCRIPTION
  检查 Node / Rust / MSVC 工具链后执行 tauri build --bundles nsis。
  Win11 通常已带 WebView2；若缺失，安装包会引导下载。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\scripts\build-windows.ps1
  powershell -ExecutionPolicy Bypass -File .\scripts\build-windows.ps1 -SkipNpmInstall
#>
[CmdletBinding()]
param(
  [switch]$SkipNpmInstall,
  [switch]$Ci
)

$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $Root

function Write-Step([string]$Message) {
  Write-Host "==> $Message" -ForegroundColor Cyan
}

function Assert-Command([string]$Name, [string]$Hint) {
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    Write-Host "缺少依赖: $Name" -ForegroundColor Red
    Write-Host "  $Hint" -ForegroundColor Yellow
    exit 1
  }
}

Write-Step "检查构建环境 (Windows)"
Assert-Command "node" "安装 Node.js 20+ LTS: https://nodejs.org/"
Assert-Command "npm"  "随 Node.js 安装"
Assert-Command "rustc" "安装 Rust: https://rustup.rs/ （选默认 MSVC 工具链）"
Assert-Command "cargo" "安装 Rust 后确保 cargo 在 PATH 中"

$nodeVer = (node -v).Trim()
$rustVer = (rustc --version).Trim()
Write-Host "    Node  $nodeVer"
Write-Host "    Rust  $rustVer"

# 粗检 MSVC 链接器（Tauri Windows 需要）
$hasLink = Get-Command "link.exe" -ErrorAction SilentlyContinue
if (-not $hasLink) {
  Write-Host "警告: 未在 PATH 中找到 link.exe（MSVC 链接器）。" -ForegroundColor Yellow
  Write-Host "  若编译失败，请安装「使用 C++ 的桌面开发」工作负载:" -ForegroundColor Yellow
  Write-Host "  https://visualstudio.microsoft.com/visual-cpp-build-tools/" -ForegroundColor Yellow
}

# WebView2：Win11 通常自带；开发时缺失会导致窗口白屏
$wv2 = Get-ItemProperty -Path "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue
if (-not $wv2) {
  $wv2 = Get-ItemProperty -Path "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue
}
if (-not $wv2) {
  Write-Host "提示: 未检测到 WebView2 Runtime。Win11 一般自带；若运行白屏请安装:" -ForegroundColor Yellow
  Write-Host "  https://developer.microsoft.com/microsoft-edge/webview2/" -ForegroundColor Yellow
} else {
  Write-Host "    WebView2 Runtime 已检测到"
}

Write-Step "确保应用图标"
if (-not (Test-Path "src-tauri\icons\icon.ico")) {
  if (-not (Test-Path "app-icon.png")) {
    Write-Host "缺少 app-icon.png 与 icon.ico，无法生成安装包图标" -ForegroundColor Red
    exit 1
  }
  npm run tauri -- icon app-icon.png
}

if (-not $SkipNpmInstall) {
  if ($Ci -or (Test-Path "package-lock.json")) {
    Write-Step "npm ci"
    npm ci
  } else {
    Write-Step "npm install"
    npm install
  }
} else {
  Write-Step "跳过 npm install（-SkipNpmInstall）"
}

# beforeBundleCommand 使用 node 脚本，Windows 下会自动 no-op
$env:SCISSOR_SKIP_LOCAL_INSTALL = "1"

Write-Step "打包 NSIS 安装程序 (Win11 x64)"
npm run tauri -- build --bundles nsis

$nsisDir = Join-Path $Root "src-tauri\target\release\bundle\nsis"
$setup = Get-ChildItem -Path $nsisDir -Filter "*-setup.exe" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1

if (-not $setup) {
  Write-Host "未找到 setup.exe，请检查上方构建日志" -ForegroundColor Red
  Write-Host "目录: $nsisDir" -ForegroundColor Yellow
  exit 1
}

$exe = Join-Path $Root "src-tauri\target\release\scissor.exe"
Write-Host ""
Write-Host "==> 打包完成" -ForegroundColor Green
Write-Host "    安装包: $($setup.FullName)"
Write-Host "    大小:   $([math]::Round($setup.Length / 1MB, 2)) MB"
if (Test-Path $exe) {
  Write-Host "    可执行: $exe"
}
Write-Host ""
Write-Host "安装说明:"
Write-Host "  1. 双击 *-setup.exe（默认当前用户，无需管理员）"
Write-Host "  2. 开始菜单出现 Scissor；快捷键 Ctrl+Shift+S 选区截图"
Write-Host "  3. 若缺少 WebView2，安装程序会引导下载"
Write-Host "  4. 截图默认复制到剪贴板，另存为写入「图片\Scissor」"
