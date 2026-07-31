# Scissor

基于 **Rust + Tauri 2** 的轻量截图工具，支持全屏截图、选区截图、滚动长图、选区标注，以及全局快捷键调用。跨平台：**Linux** 与 **Windows 11**。

**License：** [MIT](./LICENSE)

## 功能

| 功能 | 说明 | 默认快捷键 |
| --- | --- | --- |
| 快捷启动 | 唤起主窗口 | `Ctrl+Shift+Q` |
| 选区截图 | 多屏各自全屏遮罩，任意屏幕拖拽选区；可移动/缩放选框 | `Ctrl+Shift+S` |
| 标注 | 选区后支持箭头、曲线、文本 | 工具栏 |
| 全屏截图 | 捕获主显示器画面 | `Ctrl+Shift+A` |
| 滚动截图 | 先选区，再滚动拼接长图 | 主窗口按钮 |

截图完成后**默认直接复制到剪贴板**；可在主窗口用「另存为」保存本地（Windows：`图片\Scissor`，Linux：`Pictures/Scissor`）。

## 技术结构

```
scissor/
├── src/                      # 前端（Vite + TypeScript）
├── src-tauri/
│   ├── src/                  # 截屏 / 滚动 / 窗口 / 快捷键
│   ├── tauri.conf.json
│   └── tauri.windows.conf.json
├── scripts/
│   ├── build-windows.ps1     # Win11 一键打包
│   ├── build-windows.cmd
│   ├── install-local-bin.mjs # 编译后同步本地命令（跨平台）
│   └── install.sh            # Linux 用户级安装
└── package.json
```

- 截屏：`xcap`
- 图像处理：`image`
- 剪贴板：`arboard`
- 自动滚动：Linux `xdotool` / Windows `enigo`；失败则手动逐帧
- 全局快捷键：`tauri-plugin-global-shortcut`
- Windows 安装包：Tauri NSIS（`*-setup.exe`）

## 开发环境

### 通用依赖

- Rust stable（[rustup](https://rustup.rs/)）
- Node.js 20+ LTS

### Linux 额外包（Debian/Ubuntu）

```bash
sudo apt install \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf \
  libxcb-xfixes0-dev libxcb-shape0-dev \
  xdotool
```

### Windows 11 额外依赖

| 组件 | 说明 |
| --- | --- |
| MSVC 工具链 | [Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) → 勾选「使用 C++ 的桌面开发」；`rustup` 默认 `x86_64-pc-windows-msvc` |
| WebView2 Runtime | Win11 通常自带；缺失时安装 [WebView2 Evergreen](https://developer.microsoft.com/microsoft-edge/webview2/) |
| NSIS | 由 Tauri CLI 在打包时自动处理，一般无需手装 |

### 运行

```bash
npm install
npm run tauri dev
```

Windows PowerShell：

```powershell
npm install
npm run tauri dev
```

## 打包

### Linux（deb / rpm）

```bash
npm run build:linux
```

成功后自动把二进制同步到 `~/.local/bin/scissor`。完整桌面入口：`npm run install`。

### Windows 11（推荐脚本）

在 **Windows** 本机执行（Linux 无法交叉产出正式 NSIS 安装包）：

```powershell
# 仓库根目录
powershell -ExecutionPolicy Bypass -File .\scripts\build-windows.ps1

# 或双击
.\scripts\build-windows.cmd

# 或仅调用 tauri（需已装好依赖）
npm run build:windows
```

脚本会检查 Node / Rust / WebView2，并生成：

```
src-tauri\target\release\bundle\nsis\Scissor_*_x64-setup.exe
```

**安装：** 双击 `*-setup.exe`（默认当前用户，无需管理员）→ 开始菜单 **Scissor**。若缺 WebView2，安装程序会引导下载。

**CI：** GitHub Actions 工作流 `Windows Build`（`workflow_dispatch` 或推送 `v*` 标签），产物 Artifact `scissor-windows-setup`；打标签时还会挂到 Release。

## 使用说明

1. 启动后可点击主界面按钮，或使用全局快捷键。
2. **选区**：拖拽矩形；框内拖动可微调，锚点可缩放；工具栏可画箭头 / 曲线 / 文本；`Enter` 完成，`Esc` 取消。
3. **滚动截图**：确认选区后优先自动滚动拼接；自动滚动不可用时进入手动模式——滚动页面后点「下一帧」，再点「结束拼接」。
4. 请保证目标窗口位于选区下且可滚动。
5. Win11 多屏（含副屏在主屏左侧）均可选区。

## 后续可扩展

- 自定义快捷键与保存路径
- 系统托盘常驻
- 代码签名（Authenticode）后的智能屏幕提示更友好
- Wayland 下基于 `xdg-desktop-portal` 的截屏/输入

## License

本项目采用 [MIT License](./LICENSE)。
