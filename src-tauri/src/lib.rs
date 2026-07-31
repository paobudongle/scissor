mod capture;
mod scroll;

use capture::{CaptureResult, Region};
use image::RgbaImage;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl,
    WebviewWindowBuilder,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[derive(Clone, serde::Serialize)]
struct HotkeyEvent {
    action: String,
}

#[derive(Clone, serde::Serialize)]
struct OverlaySession {
    mode: String,
    /// 当前覆盖层对应的显示器 id
    monitor_id: u32,
    monitor_name: String,
    width: u32,
    height: u32,
    /// 该显示器在虚拟桌面中的原点（物理像素）
    monitor_x: i32,
    monitor_y: i32,
    is_primary: bool,
    /// 底图本地路径（避免超大 base64 导致前端黑屏）
    png_path: String,
}

struct ScrollSession {
    region: Region,
    frames: Vec<RgbaImage>,
}

struct AppState {
    /// 多屏选区会话（按 monitor_id）；底图落盘，确认时再读入裁剪
    overlay_sessions: Mutex<HashMap<u32, OverlaySession>>,
    /// 最近一次截图结果
    last_result: Mutex<Option<RgbaImage>>,
    /// 手动滚动截图会话
    scroll_session: Mutex<Option<ScrollSession>>,
    /// 进程级剪贴板句柄。Linux 上必须长期持有，否则 set_image 后立刻 drop
    /// 会导致粘贴无内容（X11/Wayland 剪贴板所有权模型）。
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            overlay_sessions: Mutex::new(HashMap::new()),
            last_result: Mutex::new(None),
            scroll_session: Mutex::new(None),
            clipboard: Mutex::new(None),
        }
    }
}

impl AppState {
    fn copy_image_to_clipboard(&self, image: &RgbaImage) -> Result<(), String> {
        capture::copy_image_to_clipboard(&mut self.clipboard.lock(), image)
    }
}

fn hide_app_windows(app: &AppHandle) {
    for (_, window) in app.webview_windows() {
        let _ = window.hide();
    }
}

fn show_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        // 异常残留时窗口可能缩成 10x10 或跑出屏幕，统一恢复
        let _ = main.set_size(tauri::Size::Logical(tauri::LogicalSize::new(440.0, 680.0)));
        let _ = main.center();
        let _ = main.unminimize();
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

fn destroy_overlay(app: &AppHandle) {
    let labels: Vec<String> = app
        .webview_windows()
        .into_keys()
        .filter(|label| label == "overlay" || label.starts_with("overlay-"))
        .collect();
    for label in labels {
        if let Some(win) = app.get_webview_window(&label) {
            let _ = win.hide();
            let _ = win.destroy();
        }
    }
}

/// 用 Tauri 显示器几何铺满覆盖层，避免 xcap 与 WM 坐标系偏差导致副屏点不中。
fn overlay_geometry_for_capture(
    app: &AppHandle,
    phys_x: i32,
    phys_y: i32,
    phys_w: u32,
    phys_h: u32,
) -> Result<(f64, f64, f64, f64), String> {
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let cx = phys_x as f64 + phys_w as f64 / 2.0;
    let cy = phys_y as f64 + phys_h as f64 / 2.0;

    let mut best: Option<(f64, f64, f64, f64, f64)> = None; // score, lx, ly, lw, lh
    for m in &monitors {
        let scale = m.scale_factor().max(0.1);
        let pos = m.position();
        let size = m.size();
        let mx = pos.x as f64;
        let my = pos.y as f64;
        let mw = size.width as f64;
        let mh = size.height as f64;

        let x0 = (phys_x as f64).max(mx);
        let y0 = (phys_y as f64).max(my);
        let x1 = ((phys_x + phys_w as i32) as f64).min(mx + mw);
        let y1 = ((phys_y + phys_h as i32) as f64).min(my + mh);
        let overlap = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
        let contains = cx >= mx && cx < mx + mw && cy >= my && cy < my + mh;
        let score = overlap + if contains { 1e12 } else { 0.0 };

        if score <= 0.0 {
            continue;
        }
        let geom = (score, mx / scale, my / scale, mw / scale, mh / scale);
        if best.map(|b| geom.0 > b.0).unwrap_or(true) {
            best = Some(geom);
        }
    }

    if let Some((_, lx, ly, lw, lh)) = best {
        return Ok((lx, ly, lw, lh));
    }

    // 回退：按 1x scale 使用 xcap 物理矩形
    Ok((
        phys_x as f64,
        phys_y as f64,
        phys_w as f64,
        phys_h as f64,
    ))
}

fn create_monitor_overlay(
    app: &AppHandle,
    monitor_id: u32,
    width: f64,
    height: f64,
    x: f64,
    y: f64,
) -> Result<tauri::WebviewWindow, String> {
    let label = format!("overlay-{monitor_id}");
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.destroy();
        thread::sleep(Duration::from_millis(20));
    }

    // URL 带 view=overlay，前端即使 label 异常也会进入选区模式（修复误进主界面导致黑屏）
    let url = format!("index.html?view=overlay&monitorId={monitor_id}");
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(format!("Scissor Overlay {monitor_id}"))
        .decorations(false)
        .transparent(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .closable(false)
        .focused(true)
        .visible(false)
        .inner_size(width, height)
        .position(x, y)
        .build()
        .map_err(|e| format!("创建覆盖层窗口失败({label}): {e}"))?;

    let _ = window.set_size(tauri::Size::Logical(LogicalSize::new(width, height)));
    let _ = window.set_position(tauri::Position::Logical(LogicalPosition::new(x, y)));
    Ok(window)
}

fn quit_app(app: &AppHandle) {
    destroy_overlay(app);
    let _ = app.global_shortcut().unregister_all();
    tauri_plugin_single_instance::destroy(app);
    app.exit(0);
}

async fn prepare_region_session(
    app: &AppHandle,
    state: &AppState,
    mode: &str,
) -> Result<CaptureResult, String> {
    hide_app_windows(app);
    // 等待窗口真正隐藏，避免把自己截进去
    thread::sleep(Duration::from_millis(180));

    destroy_overlay(app);
    thread::sleep(Duration::from_millis(40));

    // 捕获所有屏幕，并为每块屏幕创建独立全屏选区层（类微信多屏）
    let captures = capture::capture_all_monitors()?;
    if captures.is_empty() {
        return Err("未捕获到任何屏幕".into());
    }

    let overlay_dir = std::env::temp_dir().join("scissor-overlays");
    let _ = std::fs::remove_dir_all(&overlay_dir);
    std::fs::create_dir_all(&overlay_dir)
        .map_err(|e| format!("创建临时目录失败: {e}"))?;

    let mut sessions = HashMap::new();
    let mut created = Vec::new();

    for cap in &captures {
        let (lx, ly, lw, lh) = overlay_geometry_for_capture(
            app,
            cap.info.x,
            cap.info.y,
            cap.info.width,
            cap.info.height,
        )?;
        let win = create_monitor_overlay(app, cap.info.id, lw, lh, lx, ly)?;

        let png_path = overlay_dir.join(format!("mon-{}.png", cap.info.id));
        capture::save_png(&cap.image, &png_path)?;

        let session = OverlaySession {
            mode: mode.to_string(),
            monitor_id: cap.info.id,
            monitor_name: cap.info.name.clone(),
            width: cap.image.width(),
            height: cap.image.height(),
            monitor_x: cap.info.x,
            monitor_y: cap.info.y,
            is_primary: cap.info.is_primary,
            png_path: png_path.to_string_lossy().to_string(),
        };
        sessions.insert(cap.info.id, session);
        created.push((cap.info.id, win, cap.info.is_primary));
    }

    *state.overlay_sessions.lock() = sessions.clone();

    // 每块屏都要可点：全部 show + always_on_top，并轮流 focus 一次确保 WM 接纳输入
    for (_id, win, _is_primary) in &created {
        win.show().map_err(|e| e.to_string())?;
        let _ = win.unminimize();
        let _ = win.set_always_on_top(true);
        let _ = win.set_ignore_cursor_events(false);
        let _ = win.set_focus();
    }

    // 只注入会话元数据；前端用 asset 协议加载本地 PNG，避免超大 base64
    let app_clone = app.clone();
    let sessions_clone = sessions.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(120));
        for (id, session) in &sessions_clone {
            let label = format!("overlay-{id}");
            let Some(win) = app_clone.get_webview_window(&label) else {
                continue;
            };
            if let Ok(json) = serde_json::to_string(session) {
                let _ = win.eval(&format!(
                    "window.__SCISSOR_SESSION__ = {json}; window.dispatchEvent(new Event('scissor-session'));"
                ));
            }
            let _ = win.emit("overlay-session", session);
            let _ = win.set_always_on_top(true);
            let _ = win.set_ignore_cursor_events(false);
        }
        // 主屏最后抢焦点；副屏在 mouseenter 时由前端再 setFocus
        if let Some((id, _)) = sessions_clone.iter().find(|(_, s)| s.is_primary) {
            if let Some(win) = app_clone.get_webview_window(&format!("overlay-{id}")) {
                let _ = win.set_focus();
            }
        }
    });

    let primary = captures
        .iter()
        .find(|c| c.info.is_primary)
        .unwrap_or(&captures[0]);
    capture::to_capture_result(&primary.image, None, false)
}

#[tauri::command]
fn list_monitors() -> Result<Vec<capture::MonitorInfo>, String> {
    capture::list_monitors()
}

#[tauri::command]
async fn capture_fullscreen(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    copy: Option<bool>,
    save: Option<bool>,
) -> Result<CaptureResult, String> {
    hide_app_windows(&app);
    thread::sleep(Duration::from_millis(180));

    let image = capture::capture_monitor_by_id(None)?;
    let result = finalize_capture(
        &app,
        &state,
        image,
        "fullscreen",
        copy.unwrap_or(true),
        save.unwrap_or(false),
    )?;
    show_main(&app);
    let _ = app.emit("capture-done", &result);
    Ok(result)
}

#[tauri::command]
async fn begin_region_capture(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<CaptureResult, String> {
    prepare_region_session(&app, &state, "region").await
}

#[tauri::command]
async fn begin_scroll_capture(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<CaptureResult, String> {
    // 滚动截图同样先进入选区模式，前端确认区域后再调用 confirm_scroll_capture
    prepare_region_session(&app, &state, "scroll").await
}

#[tauri::command]
async fn confirm_region_capture(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    region: Region,
    monitor_id: u32,
    copy: Option<bool>,
    save: Option<bool>,
) -> Result<CaptureResult, String> {
    let png_path = state
        .overlay_sessions
        .lock()
        .get(&monitor_id)
        .map(|s| s.png_path.clone())
        .ok_or_else(|| format!("没有显示器 {monitor_id} 的截图会话"))?;
    destroy_overlay(&app);

    let screen = capture::load_png(Path::new(&png_path))?;
    let cropped = capture::crop_image(&screen, &region)?;
    let result = finalize_capture(
        &app,
        &state,
        cropped,
        "region",
        copy.unwrap_or(true),
        save.unwrap_or(false),
    )?;
    show_main(&app);
    let _ = app.emit("capture-done", &result);
    Ok(result)
}

/// 前端完成标注后提交最终 PNG（base64，无 data: 前缀）。
#[tauri::command]
async fn confirm_annotated_capture(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    png_base64: String,
    copy: Option<bool>,
    save: Option<bool>,
) -> Result<CaptureResult, String> {
    destroy_overlay(&app);

    let raw = png_base64
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(png_base64.as_str())
        .trim();
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, raw)
        .map_err(|e| format!("解码标注图失败: {e}"))?;
    let image = image::load_from_memory(&bytes)
        .map_err(|e| format!("解析标注图失败: {e}"))?
        .to_rgba8();

    let result = finalize_capture(
        &app,
        &state,
        image,
        "region",
        copy.unwrap_or(true),
        save.unwrap_or(false),
    )?;
    show_main(&app);
    let _ = app.emit("capture-done", &result);
    Ok(result)
}

#[derive(Clone, serde::Serialize)]
struct ScrollStatus {
    mode: String,
    frames: usize,
    message: String,
}

#[tauri::command]
async fn confirm_scroll_capture(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    region: Region,
    monitor_id: u32,
    max_frames: Option<u32>,
    copy: Option<bool>,
    save: Option<bool>,
) -> Result<serde_json::Value, String> {
    // 将选区从单屏相对坐标转为虚拟桌面绝对坐标
    let (ox, oy) = state
        .overlay_sessions
        .lock()
        .get(&monitor_id)
        .map(|s| (s.monitor_x, s.monitor_y))
        .ok_or_else(|| format!("未知显示器 {monitor_id}"))?;
    let abs_region = Region {
        x: ox + region.x,
        y: oy + region.y,
        width: region.width,
        height: region.height,
    };

    destroy_overlay(&app);
    hide_app_windows(&app);
    thread::sleep(Duration::from_millis(160));

    // 优先自动滚动（Linux: xdotool / Windows: enigo）；失败则手动逐帧
    match scroll::capture_scrolling_auto(&abs_region, max_frames.unwrap_or(12)) {
        Ok(image) => {
            let result = finalize_capture(
                &app,
                &state,
                image,
                "scroll",
                copy.unwrap_or(true),
                save.unwrap_or(false),
            )?;
            show_main(&app);
            let _ = app.emit("capture-done", &result);
            Ok(serde_json::json!({
                "mode": "auto",
                "result": result,
            }))
        }
        Err(auto_err) => {
            let first = scroll::capture_region_live(&abs_region)?;
            *state.scroll_session.lock() = Some(ScrollSession {
                region: abs_region,
                frames: vec![first],
            });
            show_main(&app);
            let status = ScrollStatus {
                mode: "manual".into(),
                frames: 1,
                message: format!(
                    "自动滚动不可用（{auto_err}）。请滚动目标区域后点「下一帧」，完成时点「结束拼接」。"
                ),
            };
            let _ = app.emit("scroll-manual", &status);
            Ok(serde_json::json!({
                "mode": "manual",
                "status": status,
            }))
        }
    }
}

#[tauri::command]
async fn scroll_capture_next_frame(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<ScrollStatus, String> {
    let region = {
        let guard = state.scroll_session.lock();
        guard
            .as_ref()
            .map(|s| s.region.clone())
            .ok_or_else(|| "当前没有进行中的滚动截图".to_string())?
    };

    hide_app_windows(&app);
    thread::sleep(Duration::from_millis(140));
    let next = scroll::capture_region_live(&region)?;

    let (frames, changed) = {
        let mut guard = state.scroll_session.lock();
        let session = guard
            .as_mut()
            .ok_or_else(|| "当前没有进行中的滚动截图".to_string())?;
        let changed = session
            .frames
            .last()
            .map(|prev| capture::mean_abs_diff(prev, &next) >= 1.5)
            .unwrap_or(true);
        if changed {
            session.frames.push(next);
        }
        (session.frames.len(), changed)
    };
    show_main(&app);

    Ok(ScrollStatus {
        mode: "manual".into(),
        frames,
        message: if changed {
            format!("已采集第 {frames} 帧，继续滚动后可再捕获")
        } else {
            "内容几乎未变化，可能已到底部".into()
        },
    })
}

#[tauri::command]
async fn scroll_capture_finish(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    copy: Option<bool>,
    save: Option<bool>,
) -> Result<CaptureResult, String> {
    let session = state
        .scroll_session
        .lock()
        .take()
        .ok_or_else(|| "当前没有进行中的滚动截图".to_string())?;
    let image = scroll::stitch_vertical(&session.frames)?;
    let result = finalize_capture(
        &app,
        &state,
        image,
        "scroll",
        copy.unwrap_or(true),
        save.unwrap_or(false),
    )?;
    show_main(&app);
    let _ = app.emit("capture-done", &result);
    Ok(result)
}

#[tauri::command]
async fn scroll_capture_cancel(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    *state.scroll_session.lock() = None;
    show_main(&app);
    Ok(())
}

#[tauri::command]
async fn cancel_capture(app: AppHandle) -> Result<(), String> {
    destroy_overlay(&app);
    show_main(&app);
    Ok(())
}

#[tauri::command]
fn copy_last_capture(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let guard = state.last_result.lock();
    let image = guard
        .as_ref()
        .ok_or_else(|| "暂无截图可复制".to_string())?;
    // 锁顺序：last_result → clipboard（finalize 只锁 clipboard，不会死锁）
    state.copy_image_to_clipboard(image)
}

#[tauri::command]
fn save_last_capture(state: State<'_, Arc<AppState>>, path: String) -> Result<String, String> {
    let guard = state.last_result.lock();
    let image = guard
        .as_ref()
        .ok_or_else(|| "暂无截图可保存".to_string())?;
    let path = std::path::PathBuf::from(path);
    capture::save_png(image, &path)?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn get_hotkeys() -> serde_json::Value {
    serde_json::json!({
        "launch": "Ctrl+Shift+Q",
        "region": "Ctrl+Shift+S",
        "fullscreen": "Ctrl+Shift+A",
        "cancel": "Esc（选区中）"
    })
}

#[tauri::command]
fn get_overlay_session(
    state: State<'_, Arc<AppState>>,
    monitor_id: u32,
) -> Result<OverlaySession, String> {
    state
        .overlay_sessions
        .lock()
        .get(&monitor_id)
        .cloned()
        .ok_or_else(|| format!("当前没有显示器 {monitor_id} 的选区会话"))
}

fn finalize_capture(
    _app: &AppHandle,
    state: &AppState,
    image: RgbaImage,
    prefix: &str,
    copy: bool,
    save: bool,
) -> Result<CaptureResult, String> {
    let mut saved_path = None;
    if save {
        let path = capture::default_save_path(prefix)?;
        capture::save_png(&image, &path)?;
        saved_path = Some(path.display().to_string());
    }
    let copied = if copy {
        match state.copy_image_to_clipboard(&image) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("自动复制到剪贴板失败: {e}");
                false
            }
        }
    } else {
        false
    };
    let result = capture::to_capture_result(&image, saved_path, copied)?;
    *state.last_result.lock() = Some(image);
    Ok(result)
}

fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
    let launch = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyQ);
    let region = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS);
    let fullscreen = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyA);

    // 清理残留快捷键，避免上次异常退出后无法再次注册
    let _ = app.global_shortcut().unregister_all();
    let _ = app.global_shortcut().unregister(launch);
    let _ = app.global_shortcut().unregister(region);
    let _ = app.global_shortcut().unregister(fullscreen);

    let mut errors = Vec::new();

    if let Err(e) = app.global_shortcut().on_shortcut(launch, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            show_main(app);
            let _ = app.emit("hotkey", HotkeyEvent { action: "launch".into() });
        }
    }) {
        errors.push(format!("Ctrl+Shift+Q: {e}"));
    }

    if let Err(e) = app.global_shortcut().on_shortcut(region, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            let _ = app.emit("hotkey", HotkeyEvent { action: "region".into() });
        }
    }) {
        errors.push(format!("Ctrl+Shift+S: {e}"));
    }

    if let Err(e) = app.global_shortcut().on_shortcut(fullscreen, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            let _ = app.emit("hotkey", HotkeyEvent {
                action: "fullscreen".into(),
            });
        }
    }) {
        errors.push(format!("Ctrl+Shift+A: {e}"));
    }

    if !errors.is_empty() {
        eprintln!("部分全局快捷键注册失败: {}", errors.join("; "));
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = Arc::new(AppState::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 已有实例：优先唤起主窗；若主窗已毁则彻底退出，便于用户重新启动
            if app.get_webview_window("main").is_some() {
                show_main(app);
            } else {
                quit_app(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            list_monitors,
            capture_fullscreen,
            begin_region_capture,
            begin_scroll_capture,
            confirm_region_capture,
            confirm_annotated_capture,
            confirm_scroll_capture,
            scroll_capture_next_frame,
            scroll_capture_finish,
            scroll_capture_cancel,
            cancel_capture,
            copy_last_capture,
            save_last_capture,
            get_hotkeys,
            get_overlay_session,
        ])
        .setup(|app| {
            // 设置窗口标题栏图标为设计稿
            if let Some(main) = app.get_webview_window("main") {
                if let Ok(dyn_img) =
                    image::load_from_memory(include_bytes!("../icons/128x128.png"))
                {
                    let rgba = dyn_img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    let icon = tauri::image::Image::new_owned(rgba.into_raw(), w, h);
                    if let Err(e) = main.set_icon(icon) {
                        eprintln!("设置窗口图标失败: {e}");
                    }
                }
            }
            show_main(app.handle());
            let _ = register_hotkeys(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        // 关闭主窗口 = 退出整个应用，释放单实例锁与快捷键
                        api.prevent_close();
                        quit_app(window.app_handle());
                    } else if window.label().starts_with("overlay") {
                        api.prevent_close();
                        destroy_overlay(window.app_handle());
                        show_main(window.app_handle());
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    if window.label() == "main" {
                        quit_app(window.app_handle());
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
