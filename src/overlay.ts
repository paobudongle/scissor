import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type OverlaySession = {
  mode: "region" | "scroll" | string;
  monitor_id: number;
  monitor_name: string;
  width: number;
  height: number;
  monitor_x: number;
  monitor_y: number;
  is_primary: boolean;
  png_path: string;
};

type Mode = "region" | "scroll";
type Tool = "select" | "arrow" | "pen" | "text";
type Handle = "tl" | "t" | "tr" | "l" | "r" | "bl" | "b" | "br";

type DragKind =
  | { kind: "create"; ox: number; oy: number }
  | { kind: "move"; ox: number; oy: number; sx: number; sy: number }
  | { kind: "resize"; handle: Handle; ox: number; oy: number; start: Rect }
  | { kind: "arrow"; x1: number; y1: number }
  | { kind: "pen"; points: Point[] }
  | null;

type Point = { x: number; y: number };
type Rect = { x: number; y: number; width: number; height: number };

type Annotation =
  | { type: "arrow"; x1: number; y1: number; x2: number; y2: number; color: string }
  | { type: "pen"; points: Point[]; color: string; width: number }
  | { type: "text"; x: number; y: number; text: string; color: string; fontSize: number };

declare global {
  interface Window {
    __SCISSOR_SESSION__?: OverlaySession;
  }
}

const ANNO_COLOR = "#ff3b30";
const PEN_WIDTH = 3;
const MIN_SEL = 3;

function toFileUrl(fsPath: string): string {
  // Windows: C:\Users\... -> file:///C:/Users/...
  if (/^[a-zA-Z]:[\\/]/.test(fsPath)) {
    return `file:///${fsPath.replace(/\\/g, "/")}`;
  }
  return `file://${fsPath}`;
}

function parseMonitorId(label: string, search: string): number | null {
  const fromLabel = /^overlay-(\d+)$/.exec(label);
  if (fromLabel) return Number(fromLabel[1]);
  const fromQuery = new URLSearchParams(search).get("monitorId");
  if (fromQuery != null && fromQuery !== "") return Number(fromQuery);
  return null;
}

function clamp(n: number, min: number, max: number) {
  return Math.max(min, Math.min(max, n));
}

function normalizeRect(x1: number, y1: number, x2: number, y2: number): Rect {
  const left = Math.min(x1, x2);
  const top = Math.min(y1, y2);
  const right = Math.max(x1, x2);
  const bottom = Math.max(y1, y2);
  return {
    x: Math.max(0, left),
    y: Math.max(0, top),
    width: Math.min(window.innerWidth, right) - Math.max(0, left),
    height: Math.min(window.innerHeight, bottom) - Math.max(0, top),
  };
}

function hitHandle(rect: Rect, x: number, y: number): Handle | null {
  const pads: { h: Handle; hx: number; hy: number }[] = [
    { h: "tl", hx: rect.x, hy: rect.y },
    { h: "t", hx: rect.x + rect.width / 2, hy: rect.y },
    { h: "tr", hx: rect.x + rect.width, hy: rect.y },
    { h: "l", hx: rect.x, hy: rect.y + rect.height / 2 },
    { h: "r", hx: rect.x + rect.width, hy: rect.y + rect.height / 2 },
    { h: "bl", hx: rect.x, hy: rect.y + rect.height },
    { h: "b", hx: rect.x + rect.width / 2, hy: rect.y + rect.height },
    { h: "br", hx: rect.x + rect.width, hy: rect.y + rect.height },
  ];
  const tol = 10;
  for (const p of pads) {
    if (Math.abs(x - p.hx) <= tol && Math.abs(y - p.hy) <= tol) return p.h;
  }
  return null;
}

function insideRect(rect: Rect, x: number, y: number) {
  return x >= rect.x && y >= rect.y && x <= rect.x + rect.width && y <= rect.y + rect.height;
}

function resizeFromHandle(start: Rect, handle: Handle, x: number, y: number): Rect {
  let { x: left, y: top, width, height } = start;
  let right = left + width;
  let bottom = top + height;

  if (handle.includes("l")) left = x;
  if (handle.includes("r")) right = x;
  if (handle.includes("t")) top = y;
  if (handle.includes("b")) bottom = y;

  left = clamp(left, 0, window.innerWidth);
  right = clamp(right, 0, window.innerWidth);
  top = clamp(top, 0, window.innerHeight);
  bottom = clamp(bottom, 0, window.innerHeight);

  return normalizeRect(left, top, right, bottom);
}

function drawArrow(
  ctx: CanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  color: string,
  lineWidth: number,
) {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len = Math.hypot(dx, dy) || 1;
  const ux = dx / len;
  const uy = dy / len;
  const head = Math.max(10, lineWidth * 4);
  const hx = x2 - ux * head;
  const hy = y2 - uy * head;
  const px = -uy;
  const py = ux;

  ctx.save();
  ctx.strokeStyle = color;
  ctx.fillStyle = color;
  ctx.lineWidth = lineWidth;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(x1, y1);
  ctx.lineTo(hx, hy);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(x2, y2);
  ctx.lineTo(hx + px * head * 0.45, hy + py * head * 0.45);
  ctx.lineTo(hx - px * head * 0.45, hy - py * head * 0.45);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function drawPen(
  ctx: CanvasRenderingContext2D,
  points: Point[],
  color: string,
  width: number,
) {
  if (points.length < 2) return;
  ctx.save();
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(points[0].x, points[0].y);
  for (let i = 1; i < points.length; i++) {
    ctx.lineTo(points[i].x, points[i].y);
  }
  ctx.stroke();
  ctx.restore();
}

function drawTextAnno(
  ctx: CanvasRenderingContext2D,
  anno: Extract<Annotation, { type: "text" }>,
) {
  ctx.save();
  ctx.fillStyle = anno.color;
  ctx.font = `600 ${anno.fontSize}px "Segoe UI", "PingFang SC", "Noto Sans SC", sans-serif`;
  ctx.textBaseline = "top";
  const lines = anno.text.split("\n");
  lines.forEach((line, i) => {
    ctx.fillText(line, anno.x, anno.y + i * (anno.fontSize * 1.35));
  });
  ctx.restore();
}

export function initOverlay() {
  document.body.classList.add("overlay-window");
  const app = document.querySelector("#app");
  if (!app) return;

  const currentWin = getCurrentWindow();
  const winLabel = currentWin.label;
  const monitorIdFromLabel = parseMonitorId(winLabel, window.location.search);

  let mode: Mode = "region";
  let monitorId = monitorIdFromLabel ?? 0;
  let imageW = 0;
  let imageH = 0;
  let imageReady = false;
  let current: Rect = { x: 0, y: 0, width: 0, height: 0 };
  let tool: Tool = "select";
  let drag: DragKind = null;
  let annotations: Annotation[] = [];
  let draft: Annotation | null = null;
  let textEditor: HTMLTextAreaElement | null = null;
  let appliedKey = "";

  app.innerHTML = `
    <div class="shot-root" id="shot-root">
      <img class="shot-screen" id="screen" alt="" draggable="false" />
      <div class="shot-dim" id="dim" aria-hidden="true"></div>
      <div class="shot-selection" id="selection" hidden>
        <canvas class="shot-anno-canvas" id="anno-canvas"></canvas>
        <i class="handle tl" data-handle="tl"></i>
        <i class="handle t" data-handle="t"></i>
        <i class="handle tr" data-handle="tr"></i>
        <i class="handle l" data-handle="l"></i>
        <i class="handle r" data-handle="r"></i>
        <i class="handle bl" data-handle="bl"></i>
        <i class="handle b" data-handle="b"></i>
        <i class="handle br" data-handle="br"></i>
        <div class="shot-size" id="size-tag">0 × 0</div>
      </div>
      <div class="shot-tip" id="hint">正在加载屏幕画面…</div>
      <div class="shot-toolbar" id="toolbar" hidden>
        <div class="tool-group" id="anno-tools">
          <button type="button" class="tool-icon active" data-tool="select" title="调整选区">✥</button>
          <button type="button" class="tool-icon" data-tool="arrow" title="箭头">➤</button>
          <button type="button" class="tool-icon" data-tool="pen" title="曲线">∿</button>
          <button type="button" class="tool-icon" data-tool="text" title="文本">T</button>
        </div>
        <span class="tool-sep" id="anno-sep"></span>
        <button type="button" class="tool-btn" id="cancel-btn" title="取消 (Esc)">
          <span class="icon">✕</span><span>取消</span>
        </button>
        <button type="button" class="tool-btn" id="recapture-btn" title="重新选取">
          <span class="icon">↺</span><span>重选</span>
        </button>
        <button type="button" class="tool-btn primary" id="confirm-btn" title="完成 (Enter)" disabled>
          <span class="icon">✓</span><span>完成</span>
        </button>
      </div>
    </div>
  `;

  const root = document.querySelector<HTMLElement>("#shot-root")!;
  const screenEl = document.querySelector<HTMLImageElement>("#screen")!;
  const dimEl = document.querySelector<HTMLElement>("#dim")!;
  const selectionEl = document.querySelector<HTMLElement>("#selection")!;
  const annoCanvas = document.querySelector<HTMLCanvasElement>("#anno-canvas")!;
  const sizeTag = document.querySelector<HTMLElement>("#size-tag")!;
  const hint = document.querySelector<HTMLElement>("#hint")!;
  const toolbar = document.querySelector<HTMLElement>("#toolbar")!;
  const annoTools = document.querySelector<HTMLElement>("#anno-tools")!;
  const annoSep = document.querySelector<HTMLElement>("#anno-sep")!;
  const confirmBtn = document.querySelector<HTMLButtonElement>("#confirm-btn")!;
  const cancelBtn = document.querySelector<HTMLButtonElement>("#cancel-btn")!;
  const recaptureBtn = document.querySelector<HTMLButtonElement>("#recapture-btn")!;

  dimEl.style.opacity = "0";

  function focusThisOverlay() {
    // 拖拽中不要抢焦点，Linux/WebKitGTK 上 setFocus 会打断 mousemove/mouseup
    if (drag) return;
    void currentWin.isFocused().then((focused) => {
      if (!focused && !drag) {
        return currentWin.setFocus();
      }
    }).catch(() => undefined);
  }

  function setScreenSrc(path: string) {
    imageReady = false;
    dimEl.style.opacity = "0";
    hint.hidden = false;
    hint.textContent = "正在加载屏幕画面…";

    const candidates = [
      convertFileSrc(path),
      convertFileSrc(path, "asset"),
      toFileUrl(path),
    ];

    let idx = 0;
    const tryNext = () => {
      if (idx >= candidates.length) {
        hint.textContent = "屏幕画面加载失败，请按 Esc 取消后重试";
        return;
      }
      const url = candidates[idx++];
      screenEl.onload = () => {
        imageReady = true;
        dimEl.style.opacity = "";
        updateHint();
      };
      screenEl.onerror = () => tryNext();
      screenEl.src = url;
    };
    tryNext();
  }

  function updateHint() {
    if (!imageReady) return;
    const screenHint = `屏幕 ${monitorId}`;
    if (mode === "scroll") {
      hint.textContent = `【${screenHint}】拖拽选择滚动区域 · Esc 取消`;
    } else if (!hasSelection()) {
      hint.textContent = `【${screenHint}】拖拽选取 · 可在任意屏幕操作 · Esc 取消`;
    } else if (tool === "arrow") {
      hint.textContent = "拖拽绘制箭头";
    } else if (tool === "pen") {
      hint.textContent = "按住绘制曲线";
    } else if (tool === "text") {
      hint.textContent = "点击选区内添加文本";
    } else {
      hint.textContent = "拖动选区微调 · 拖锚点缩放 · 或使用标注工具";
    }
  }

  function hasSelection() {
    return current.width >= MIN_SEL && current.height >= MIN_SEL;
  }

  function setTool(next: Tool) {
    tool = next;
    annoTools.querySelectorAll<HTMLButtonElement>("[data-tool]").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.tool === next);
    });
    root.classList.toggle("tool-arrow", next === "arrow");
    root.classList.toggle("tool-pen", next === "pen");
    root.classList.toggle("tool-text", next === "text");
    root.classList.toggle("tool-select", next === "select");
    commitTextEditor();
    updateHint();
    syncSelection();
  }

  function applySession(session: OverlaySession) {
    if (monitorIdFromLabel != null && session.monitor_id !== monitorIdFromLabel) {
      return;
    }
    mode = session.mode === "scroll" ? "scroll" : "region";
    monitorId = session.monitor_id;
    imageW = session.width;
    imageH = session.height;
    confirmBtn.querySelector("span:last-child")!.textContent =
      mode === "scroll" ? "开始" : "完成";
    const showAnno = mode === "region";
    annoTools.hidden = !showAnno;
    annoSep.hidden = !showAnno;

    if (!session.png_path) {
      hint.textContent = "未收到屏幕截图路径";
      return;
    }

    // 同一会话重复注入时不要 reset，否则会清掉已选区和标注
    const key = `${session.monitor_id}|${session.png_path}|${session.mode}`;
    if (key === appliedKey) {
      return;
    }
    appliedKey = key;
    setScreenSrc(session.png_path);
    resetSelection();
  }

  function resetSelection() {
    commitTextEditor();
    current = { x: 0, y: 0, width: 0, height: 0 };
    drag = null;
    draft = null;
    annotations = [];
    setTool("select");
    syncSelection();
  }

  function scaleFactors() {
    return {
      scaleX: imageW > 0 ? imageW / window.innerWidth : window.devicePixelRatio || 1,
      scaleY: imageH > 0 ? imageH / window.innerHeight : window.devicePixelRatio || 1,
    };
  }

  function paintAnnotations() {
    const ctx = annoCanvas.getContext("2d");
    if (!ctx || !hasSelection()) return;
    const dpr = window.devicePixelRatio || 1;
    const w = current.width;
    const h = current.height;
    annoCanvas.width = Math.max(1, Math.round(w * dpr));
    annoCanvas.height = Math.max(1, Math.round(h * dpr));
    annoCanvas.style.width = `${w}px`;
    annoCanvas.style.height = `${h}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const all = draft ? [...annotations, draft] : annotations;
    for (const anno of all) {
      if (anno.type === "arrow") {
        drawArrow(ctx, anno.x1, anno.y1, anno.x2, anno.y2, anno.color, 3);
      } else if (anno.type === "pen") {
        drawPen(ctx, anno.points, anno.color, anno.width);
      } else if (anno.type === "text") {
        drawTextAnno(ctx, anno);
      }
    }
  }

  function syncSelection() {
    const valid = hasSelection();
    const interacting = drag != null;
    selectionEl.hidden = !valid;
    toolbar.hidden = !valid || interacting;
    confirmBtn.disabled = !valid;
    if (imageReady) {
      hint.hidden = valid && !interacting;
    }
    dimEl.classList.toggle("faded", valid);

    if (!valid) {
      paintAnnotations();
      return;
    }

    const { x, y, width, height } = current;
    selectionEl.style.left = `${x}px`;
    selectionEl.style.top = `${y}px`;
    selectionEl.style.width = `${width}px`;
    selectionEl.style.height = `${height}px`;

    const { scaleX, scaleY } = scaleFactors();
    sizeTag.textContent = `${Math.round(width * scaleX)} × ${Math.round(height * scaleY)}`;

    paintAnnotations();

    const barW = toolbar.offsetWidth || 360;
    const barH = toolbar.offsetHeight || 52;
    let bx = x + width - barW;
    let by = y + height + 12;
    if (bx < 12) bx = 12;
    if (bx + barW > window.innerWidth - 12) bx = window.innerWidth - barW - 12;
    if (by + barH > window.innerHeight - 12) by = y - barH - 12;
    if (by < 12) by = 12;
    toolbar.style.left = `${bx}px`;
    toolbar.style.top = `${by}px`;
  }

  function scaleRegion() {
    const { scaleX, scaleY } = scaleFactors();
    return {
      x: Math.max(0, Math.round(current.x * scaleX)),
      y: Math.max(0, Math.round(current.y * scaleY)),
      width: Math.max(1, Math.round(current.width * scaleX)),
      height: Math.max(1, Math.round(current.height * scaleY)),
    };
  }

  function toLocal(clientX: number, clientY: number): Point {
    return { x: clientX - current.x, y: clientY - current.y };
  }

  function commitTextEditor() {
    if (!textEditor) return;
    const value = textEditor.value.replace(/\s+$/g, "");
    const x = Number(textEditor.dataset.x || 0);
    const y = Number(textEditor.dataset.y || 0);
    textEditor.remove();
    textEditor = null;
    if (value) {
      annotations.push({
        type: "text",
        x,
        y,
        text: value,
        color: ANNO_COLOR,
        fontSize: 18,
      });
      paintAnnotations();
    }
  }

  function openTextEditor(localX: number, localY: number) {
    commitTextEditor();
    const ta = document.createElement("textarea");
    ta.className = "shot-text-input";
    ta.dataset.x = String(localX);
    ta.dataset.y = String(localY);
    ta.style.left = `${localX}px`;
    ta.style.top = `${localY}px`;
    ta.placeholder = "输入文字…";
    ta.rows = 2;
    selectionEl.appendChild(ta);
    textEditor = ta;
    ta.focus();
    ta.addEventListener("mousedown", (e) => e.stopPropagation());
    ta.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") {
        e.preventDefault();
        ta.value = "";
        commitTextEditor();
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        commitTextEditor();
      }
    });
    ta.addEventListener("blur", () => commitTextEditor());
  }

  async function pullSession() {
    if (monitorIdFromLabel == null) return;
    try {
      const session = await invoke<OverlaySession>("get_overlay_session", {
        monitorId: monitorIdFromLabel,
      });
      applySession(session);
    } catch {
      /* waiting */
    }
  }

  function consumeInjectedSession() {
    if (window.__SCISSOR_SESSION__) {
      applySession(window.__SCISSOR_SESSION__);
    }
  }

  void listen<OverlaySession>("overlay-session", (event) => {
    applySession(event.payload);
  });
  window.addEventListener("scissor-session", () => consumeInjectedSession());
  consumeInjectedSession();
  void pullSession();

  let tries = 0;
  const timer = window.setInterval(() => {
    tries += 1;
    consumeInjectedSession();
    void pullSession();
    if (tries >= 20 || imageReady) {
      window.clearInterval(timer);
    }
  }, 100);

  // 副屏可交互：进入覆盖层时再抢焦点（拖拽中跳过）
  root.addEventListener("mouseenter", () => focusThisOverlay());

  toolbar.addEventListener("mousedown", (e) => e.stopPropagation());
  toolbar.addEventListener("click", (e) => e.stopPropagation());
  toolbar.addEventListener("pointerdown", (e) => e.stopPropagation());

  annoTools.addEventListener("click", (e) => {
    const btn = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-tool]");
    if (!btn?.dataset.tool) return;
    e.preventDefault();
    e.stopPropagation();
    setTool(btn.dataset.tool as Tool);
  });

  root.addEventListener("mousedown", (e) => {
    if ((e.target as HTMLElement).closest(".shot-toolbar")) return;
    if ((e.target as HTMLElement).closest(".shot-text-input")) return;
    if (!imageReady) return;
    if (e.button !== 0) return;
    e.preventDefault();

    const x = e.clientX;
    const y = e.clientY;

    // 锚点缩放优先（标注工具下也可微调选区）
    if (hasSelection()) {
      const handle = hitHandle(current, x, y);
      if (handle) {
        drag = { kind: "resize", handle, ox: x, oy: y, start: { ...current } };
        toolbar.hidden = true;
        return;
      }
    }

    if (hasSelection() && mode === "region" && tool !== "select") {
      if (!insideRect(current, x, y)) {
        // 点在选区外：重新框选
        commitTextEditor();
        annotations = [];
        draft = null;
        drag = { kind: "create", ox: x, oy: y };
        current = { x, y, width: 0, height: 0 };
        setTool("select");
        syncSelection();
        return;
      }

      const local = toLocal(x, y);
      if (tool === "arrow") {
        drag = { kind: "arrow", x1: local.x, y1: local.y };
        draft = {
          type: "arrow",
          x1: local.x,
          y1: local.y,
          x2: local.x,
          y2: local.y,
          color: ANNO_COLOR,
        };
        syncSelection();
        return;
      }
      if (tool === "pen") {
        drag = { kind: "pen", points: [local] };
        draft = { type: "pen", points: [local], color: ANNO_COLOR, width: PEN_WIDTH };
        syncSelection();
        return;
      }
      if (tool === "text") {
        openTextEditor(local.x, local.y);
        return;
      }
    }

    if (hasSelection() && insideRect(current, x, y) && (tool === "select" || mode === "scroll")) {
      drag = {
        kind: "move",
        ox: x,
        oy: y,
        sx: current.x,
        sy: current.y,
      };
      toolbar.hidden = true;
      return;
    }

    // 新建选区
    commitTextEditor();
    annotations = [];
    draft = null;
    drag = { kind: "create", ox: x, oy: y };
    current = { x, y, width: 0, height: 0 };
    toolbar.hidden = true;
    syncSelection();
  });

  window.addEventListener("mousemove", (e) => {
    if (!drag) {
      if (hasSelection() && tool === "select") {
        const handle = hitHandle(current, e.clientX, e.clientY);
        if (handle) {
          const cursors: Record<Handle, string> = {
            tl: "nwse-resize",
            br: "nwse-resize",
            tr: "nesw-resize",
            bl: "nesw-resize",
            t: "ns-resize",
            b: "ns-resize",
            l: "ew-resize",
            r: "ew-resize",
          };
          root.style.cursor = cursors[handle];
        } else if (insideRect(current, e.clientX, e.clientY)) {
          root.style.cursor = "move";
        } else {
          root.style.cursor = "";
        }
      }
      return;
    }

    if (drag.kind === "create") {
      current = normalizeRect(drag.ox, drag.oy, e.clientX, e.clientY);
      syncSelection();
      return;
    }

    if (drag.kind === "move") {
      const dx = e.clientX - drag.ox;
      const dy = e.clientY - drag.oy;
      const nx = clamp(drag.sx + dx, 0, window.innerWidth - current.width);
      const ny = clamp(drag.sy + dy, 0, window.innerHeight - current.height);
      current = { ...current, x: nx, y: ny };
      syncSelection();
      return;
    }

    if (drag.kind === "resize") {
      current = resizeFromHandle(drag.start, drag.handle, e.clientX, e.clientY);
      syncSelection();
      return;
    }

    if (drag.kind === "arrow" && draft?.type === "arrow") {
      const local = toLocal(e.clientX, e.clientY);
      draft = { ...draft, x2: local.x, y2: local.y };
      paintAnnotations();
      return;
    }

    if (drag.kind === "pen" && draft?.type === "pen") {
      const local = toLocal(e.clientX, e.clientY);
      const pts = drag.points;
      const last = pts[pts.length - 1];
      if (!last || Math.hypot(local.x - last.x, local.y - last.y) >= 1.5) {
        pts.push(local);
        draft = { ...draft, points: [...pts] };
        paintAnnotations();
      }
    }
  });

  window.addEventListener("mouseup", () => {
    if (!drag) return;
    if (drag.kind === "arrow" && draft?.type === "arrow") {
      if (Math.hypot(draft.x2 - draft.x1, draft.y2 - draft.y1) >= 6) {
        annotations.push(draft);
      }
      draft = null;
    } else if (drag.kind === "pen" && draft?.type === "pen") {
      if (draft.points.length >= 2) annotations.push(draft);
      draft = null;
    }
    drag = null;
    syncSelection();
    updateHint();
  });

  async function buildAnnotatedPng(): Promise<string> {
    commitTextEditor();
    const region = scaleRegion();
    const { scaleX, scaleY } = scaleFactors();
    const canvas = document.createElement("canvas");
    canvas.width = region.width;
    canvas.height = region.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("无法创建画布");

    // 从已加载底图裁剪
    const sx = region.x;
    const sy = region.y;
    ctx.drawImage(
      screenEl,
      sx,
      sy,
      region.width,
      region.height,
      0,
      0,
      region.width,
      region.height,
    );

    // 标注：选区内 CSS 坐标 → 图像像素
    for (const anno of annotations) {
      if (anno.type === "arrow") {
        drawArrow(
          ctx,
          anno.x1 * scaleX,
          anno.y1 * scaleY,
          anno.x2 * scaleX,
          anno.y2 * scaleY,
          anno.color,
          Math.max(2, 3 * ((scaleX + scaleY) / 2)),
        );
      } else if (anno.type === "pen") {
        drawPen(
          ctx,
          anno.points.map((p) => ({ x: p.x * scaleX, y: p.y * scaleY })),
          anno.color,
          Math.max(1, anno.width * ((scaleX + scaleY) / 2)),
        );
      } else if (anno.type === "text") {
        drawTextAnno(ctx, {
          ...anno,
          x: anno.x * scaleX,
          y: anno.y * scaleY,
          fontSize: Math.max(12, Math.round(anno.fontSize * ((scaleX + scaleY) / 2))),
        });
      }
    }

    const dataUrl = canvas.toDataURL("image/png");
    return dataUrl.replace(/^data:image\/png;base64,/, "");
  }

  async function confirm() {
    if (confirmBtn.disabled) return;
    commitTextEditor();
    confirmBtn.disabled = true;
    hint.hidden = false;
    hint.textContent = mode === "scroll" ? "正在启动滚动截图…" : "正在生成截图…";
    toolbar.hidden = true;
    try {
      if (mode === "scroll") {
        const region = scaleRegion();
        await invoke("confirm_scroll_capture", {
          region,
          monitorId,
          maxFrames: 15,
          copy: true,
          save: false,
        });
      } else if (annotations.length > 0) {
        const pngBase64 = await buildAnnotatedPng();
        await invoke("confirm_annotated_capture", {
          pngBase64,
          copy: true,
          save: false,
        });
      } else {
        const region = scaleRegion();
        await invoke("confirm_region_capture", {
          region,
          monitorId,
          copy: true,
          save: false,
        });
      }
    } catch (err) {
      confirmBtn.disabled = false;
      toolbar.hidden = false;
      hint.textContent = `截图失败：${String(err)}`;
    }
  }

  async function cancel() {
    try {
      await invoke("cancel_capture");
    } catch (err) {
      hint.hidden = false;
      hint.textContent = `取消失败：${String(err)}`;
    }
  }

  confirmBtn.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    void confirm();
  });
  cancelBtn.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    void cancel();
  });
  recaptureBtn.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    resetSelection();
    if (imageReady) {
      hint.hidden = false;
      updateHint();
    }
  });

  window.addEventListener("keydown", (e) => {
    if (textEditor && document.activeElement === textEditor) return;
    if (e.key === "Escape") {
      e.preventDefault();
      void cancel();
    }
    if (e.key === "Enter") {
      e.preventDefault();
      void confirm();
    }
  });

  window.addEventListener("resize", () => syncSelection());
  setTool("select");
}
