import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { save } from "@tauri-apps/plugin-dialog";
import { initOverlay } from "./overlay";

type CaptureResult = {
  width: number;
  height: number;
  png_base64: string;
  saved_path?: string | null;
  /** 后端是否已成功写入剪贴板 */
  copied?: boolean;
};

type HotkeyEvent = {
  action: "launch" | "region" | "fullscreen" | "scroll" | string;
};

let lastCapture: CaptureResult | null = null;
let busy = false;

function dataUrl(pngBase64: string) {
  return `data:image/png;base64,${pngBase64}`;
}

function setStatus(text: string, kind: "" | "ok" | "error" = "") {
  const el = document.querySelector<HTMLElement>("#status");
  if (!el) return;
  el.textContent = text;
  el.className = `status ${kind}`.trim();
}

async function withBusy<T>(fn: () => Promise<T>): Promise<T | undefined> {
  if (busy) return;
  busy = true;
  try {
    return await fn();
  } catch (err) {
    setStatus(String(err), "error");
    throw err;
  } finally {
    busy = false;
  }
}

function renderPreview() {
  const box = document.querySelector("#preview");
  const meta = document.querySelector("#meta");
  const copyBtn = document.querySelector<HTMLButtonElement>("#copy-btn");
  const saveBtn = document.querySelector<HTMLButtonElement>("#save-btn");
  if (!box || !meta || !copyBtn || !saveBtn) return;

  if (!lastCapture) {
    box.innerHTML = `<div class="empty">截图预览会显示在这里<br />也可使用全局快捷键快速截图</div>`;
    meta.textContent = "";
    copyBtn.disabled = true;
    saveBtn.disabled = true;
    return;
  }

  box.innerHTML = `<img alt="截图预览" src="${dataUrl(lastCapture.png_base64)}" />`;
  const path = lastCapture.saved_path ? `已保存：${lastCapture.saved_path}` : "未自动保存";
  meta.textContent = `${lastCapture.width} × ${lastCapture.height} · ${path}`;
  copyBtn.disabled = false;
  saveBtn.disabled = false;
}

function applyCapture(result: CaptureResult, label: string) {
  lastCapture = result;
  renderPreview();
  if (result.copied === false) {
    setStatus(
      `${label}完成，但复制到剪贴板失败，请点击「复制」重试${result.saved_path ? `（已保存）` : ""}`,
      "error",
    );
    return;
  }
  setStatus(
    `${label}完成，已复制到剪贴板${result.saved_path ? `，并已保存` : "（可用「另存为」保存本地）"}`,
    "ok",
  );
}

async function startFullscreen() {
  await withBusy(async () => {
    setStatus("正在全屏截图…");
    const result = await invoke<CaptureResult>("capture_fullscreen", {
      copy: true,
      save: false,
    });
    applyCapture(result, "全屏截图");
  });
}

async function startRegion() {
  await withBusy(async () => {
    setStatus("进入选区截图…");
    await invoke("begin_region_capture");
  });
}

async function startScroll() {
  await withBusy(async () => {
    setStatus("进入滚动截图选区…");
    await invoke("begin_scroll_capture");
  });
}

function initMain() {
  document.body.classList.add("main-window");
  const app = document.querySelector("#app");
  if (!app) return;

  app.innerHTML = `
    <div class="panel">
      <header class="brand">
        <img class="brand-mark" src="/src/assets/scissor-icon.png" width="44" height="44" alt="Scissor" />
        <div>
          <h1>Scissor</h1>
          <p>轻量截图工具 · 全屏 / 选区 / 滚动</p>
        </div>
      </header>

      <section class="actions">
        <button class="action" id="btn-region" type="button">
          <div>
            <strong>选区截图</strong>
            <span>双屏均可选区 · 拖动微调 · 箭头/曲线/文本标注</span>
          </div>
          <div class="kbd">Ctrl+Shift+S</div>
        </button>
        <button class="action" id="btn-fullscreen" type="button">
          <div>
            <strong>全屏截图</strong>
            <span>捕获当前主显示器，默认复制</span>
          </div>
          <div class="kbd">Ctrl+Shift+A</div>
        </button>
        <button class="action" id="btn-scroll" type="button">
          <div>
            <strong>滚动截图</strong>
            <span>选区后滚动拼接长图（Win 自动滚轮 / Linux xdotool）</span>
          </div>
          <div class="kbd">面板</div>
        </button>
      </section>
      <p class="meta" style="margin: -8px 0 16px">快捷启动主窗口：<span class="kbd" style="display:inline-block;padding:2px 8px;border-radius:999px;background:var(--bg-soft);border:1px solid var(--line);color:var(--accent)">Ctrl+Shift+Q</span></p>

      <section class="card" id="scroll-card" hidden>
        <h2>滚动截图（手动）</h2>
        <p class="meta" id="scroll-msg">请滚动目标区域后捕获下一帧</p>
        <div class="toolbar" style="margin-top: 12px">
          <button id="scroll-next-btn" type="button">下一帧</button>
          <button id="scroll-finish-btn" class="primary" type="button">结束拼接</button>
          <button id="scroll-cancel-btn" type="button">取消</button>
        </div>
      </section>

      <section class="card">
        <h2>最近截图</h2>
        <div class="preview-box" id="preview"></div>
        <div class="meta" id="meta"></div>
        <div class="toolbar" style="margin-top: 12px">
          <button id="copy-btn" type="button" disabled>复制</button>
          <button id="save-btn" class="primary" type="button" disabled>另存为</button>
        </div>
      </section>

      <div class="status" id="status">就绪。截图默认复制到剪贴板，可再另存为本地。</div>
    </div>
  `;

  renderPreview();

  document.querySelector("#btn-region")?.addEventListener("click", () => {
    void startRegion();
  });
  document.querySelector("#btn-fullscreen")?.addEventListener("click", () => {
    void startFullscreen();
  });
  document.querySelector("#btn-scroll")?.addEventListener("click", () => {
    void startScroll();
  });
  document.querySelector("#copy-btn")?.addEventListener("click", () => {
    void withBusy(async () => {
      await invoke("copy_last_capture");
      setStatus("已复制到剪贴板", "ok");
    });
  });
  document.querySelector("#save-btn")?.addEventListener("click", () => {
    void withBusy(async () => {
      const path = await save({
        defaultPath: `scissor_${Date.now()}.png`,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      if (!path) return;
      const saved = await invoke<string>("save_last_capture", { path });
      setStatus(`已保存到 ${saved}`, "ok");
      if (lastCapture) {
        lastCapture = { ...lastCapture, saved_path: saved };
        renderPreview();
      }
    });
  });

  const scrollCard = document.querySelector<HTMLElement>("#scroll-card");
  const scrollMsg = document.querySelector<HTMLElement>("#scroll-msg");

  function setScrollUi(visible: boolean, message?: string) {
    if (scrollCard) scrollCard.hidden = !visible;
    if (message && scrollMsg) scrollMsg.textContent = message;
  }

  document.querySelector("#scroll-next-btn")?.addEventListener("click", () => {
    void withBusy(async () => {
      setStatus("正在捕获下一帧…");
      const status = await invoke<{ frames: number; message: string }>(
        "scroll_capture_next_frame",
      );
      setScrollUi(true, status.message);
      setStatus(status.message, "ok");
    });
  });
  document.querySelector("#scroll-finish-btn")?.addEventListener("click", () => {
    void withBusy(async () => {
      setStatus("正在拼接长图…");
      const result = await invoke<CaptureResult>("scroll_capture_finish", {
        copy: true,
        save: false,
      });
      setScrollUi(false);
      applyCapture(result, "滚动截图");
    });
  });
  document.querySelector("#scroll-cancel-btn")?.addEventListener("click", () => {
    void withBusy(async () => {
      await invoke("scroll_capture_cancel");
      setScrollUi(false);
      setStatus("已取消滚动截图");
    });
  });

  void listen<HotkeyEvent>("hotkey", (event) => {
    const action = event.payload.action;
    if (action === "launch") {
      setStatus("已唤起主窗口");
      return;
    }
    if (action === "region") void startRegion();
    if (action === "fullscreen") void startFullscreen();
    if (action === "scroll") void startScroll();
  });

  void listen<CaptureResult>("capture-done", (event) => {
    setScrollUi(false);
    applyCapture(event.payload, "截图");
  });

  void listen<{ frames: number; message: string }>("scroll-manual", (event) => {
    setScrollUi(true, event.payload.message);
    setStatus(event.payload.message);
  });
}

window.addEventListener("DOMContentLoaded", () => {
  const label = getCurrentWindow().label;
  const params = new URLSearchParams(window.location.search);
  const isOverlay =
    label === "overlay" ||
    label.startsWith("overlay-") ||
    params.get("view") === "overlay";

  if (isOverlay) {
    initOverlay();
  } else {
    initMain();
  }
});
