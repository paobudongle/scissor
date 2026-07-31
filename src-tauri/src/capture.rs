use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageFormat, Rgba, RgbaImage};
use serde::Serialize;
use std::io::Cursor;
use std::path::Path;
use xcap::Monitor;

#[derive(Debug, Clone, Serialize)]
pub struct MonitorInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureResult {
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
    pub saved_path: Option<String>,
    /// 是否已成功写入剪贴板（自动复制失败时仍返回截图预览）
    pub copied: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Region {
    /// 相对单屏底图时 ≥0；滚动截图绝对坐标时可为负（副屏在主屏左侧）
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// 单屏截取结果（用于多屏选区）
#[derive(Debug, Clone)]
pub struct MonitorCapture {
    pub info: MonitorInfo,
    pub image: RgbaImage,
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
    let monitors = Monitor::all().map_err(|e| format!("枚举显示器失败: {e}"))?;
    let mut list: Vec<MonitorInfo> = monitors
        .into_iter()
        .map(|m| MonitorInfo {
            id: m.id(),
            name: m.name().to_string(),
            width: m.width(),
            height: m.height(),
            x: m.x(),
            y: m.y(),
            is_primary: m.is_primary(),
        })
        .collect();
    // 主屏优先，其余按坐标排序，保证覆盖层创建顺序稳定
    list.sort_by(|a, b| {
        b.is_primary
            .cmp(&a.is_primary)
            .then(a.x.cmp(&b.x))
            .then(a.y.cmp(&b.y))
    });
    Ok(list)
}

pub fn capture_monitor_by_id(id: Option<u32>) -> Result<RgbaImage, String> {
    let monitors = Monitor::all().map_err(|e| format!("枚举显示器失败: {e}"))?;
    let monitor = match id {
        Some(id) => monitors
            .into_iter()
            .find(|m| m.id() == id)
            .ok_or_else(|| format!("未找到显示器 id={id}"))?,
        None => monitors
            .into_iter()
            .find(|m| m.is_primary())
            .or_else(|| Monitor::all().ok().and_then(|m| m.into_iter().next()))
            .ok_or_else(|| "未找到可用显示器".to_string())?,
    };

    monitor
        .capture_image()
        .map_err(|e| format!("截图失败: {e}"))
}

/// 捕获所有显示器画面，供多屏选区使用。
pub fn capture_all_monitors() -> Result<Vec<MonitorCapture>, String> {
    let monitors = Monitor::all().map_err(|e| format!("枚举显示器失败: {e}"))?;
    if monitors.is_empty() {
        return Err("未找到可用显示器".into());
    }

    let mut captures = Vec::with_capacity(monitors.len());
    for m in monitors {
        let info = MonitorInfo {
            id: m.id(),
            name: m.name().to_string(),
            width: m.width(),
            height: m.height(),
            x: m.x(),
            y: m.y(),
            is_primary: m.is_primary(),
        };
        let image = m
            .capture_image()
            .map_err(|e| format!("截取显示器 {} 失败: {e}", info.name))?;
        captures.push(MonitorCapture { info, image });
    }

    captures.sort_by(|a, b| {
        b.info
            .is_primary
            .cmp(&a.info.is_primary)
            .then(a.info.x.cmp(&b.info.x))
            .then(a.info.y.cmp(&b.info.y))
    });
    Ok(captures)
}

/// 将多屏截图拼成虚拟桌面大图（可用于全屏截图）。
pub fn stitch_virtual_desktop(captures: &[MonitorCapture]) -> Result<RgbaImage, String> {
    if captures.is_empty() {
        return Err("没有可拼接的显示器截图".into());
    }

    let min_x = captures.iter().map(|c| c.info.x).min().unwrap_or(0);
    let min_y = captures.iter().map(|c| c.info.y).min().unwrap_or(0);
    let max_x = captures
        .iter()
        .map(|c| c.info.x + c.info.width as i32)
        .max()
        .unwrap_or(0);
    let max_y = captures
        .iter()
        .map(|c| c.info.y + c.info.height as i32)
        .max()
        .unwrap_or(0);

    let width = (max_x - min_x).max(1) as u32;
    let height = (max_y - min_y).max(1) as u32;
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    for cap in captures {
        let dx = (cap.info.x - min_x).max(0) as i64;
        let dy = (cap.info.y - min_y).max(0) as i64;
        image::imageops::overlay(&mut canvas, &cap.image, dx, dy);
    }
    Ok(canvas)
}

pub fn crop_image(image: &RgbaImage, region: &Region) -> Result<RgbaImage, String> {
    if region.width == 0 || region.height == 0 {
        return Err("截图区域无效".into());
    }
    if region.x < 0 || region.y < 0 {
        return Err("截图区域坐标无效".into());
    }

    let max_x = image.width().saturating_sub(1);
    let max_y = image.height().saturating_sub(1);
    let x = (region.x as u32).min(max_x);
    let y = (region.y as u32).min(max_y);
    let width = region.width.min(image.width().saturating_sub(x));
    let height = region.height.min(image.height().saturating_sub(y));

    if width == 0 || height == 0 {
        return Err("截图区域超出屏幕范围".into());
    }

    Ok(image::imageops::crop_imm(image, x, y, width, height).to_image())
}

pub fn encode_png_base64(image: &RgbaImage) -> Result<String, String> {
    let mut buffer = Cursor::new(Vec::new());
    image
        .write_to(&mut buffer, ImageFormat::Png)
        .map_err(|e| format!("编码 PNG 失败: {e}"))?;
    Ok(STANDARD.encode(buffer.into_inner()))
}

pub fn to_capture_result(
    image: &RgbaImage,
    saved_path: Option<String>,
    copied: bool,
) -> Result<CaptureResult, String> {
    Ok(CaptureResult {
        width: image.width(),
        height: image.height(),
        png_base64: encode_png_base64(image)?,
        saved_path,
        copied,
    })
}

pub fn default_save_path(prefix: &str) -> Result<std::path::PathBuf, String> {
    let pictures = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "无法定位图片目录".to_string())?;
    let dir = pictures.join("Scissor");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建保存目录失败: {e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    Ok(dir.join(format!("{prefix}_{stamp}.png")))
}

pub fn save_png(image: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    image
        .save_with_format(path, ImageFormat::Png)
        .map_err(|e| format!("保存图片失败: {e}"))
}

pub fn load_png(path: &Path) -> Result<RgbaImage, String> {
    let img = image::open(path)
        .map_err(|e| format!("读取图片失败 {}: {e}", path.display()))?;
    Ok(img.to_rgba8())
}

/// 将图片写入剪贴板。
///
/// Linux（X11/Wayland）上剪贴板由「所有权」提供数据：写入后必须继续持有
/// [`arboard::Clipboard`]，不能立刻 drop，否则粘贴端会拿到空内容。
/// 调用方应把 `clipboard` 放在进程级长期状态里复用。
pub fn copy_image_to_clipboard(
    clipboard: &mut Option<arboard::Clipboard>,
    image: &RgbaImage,
) -> Result<(), String> {
    if clipboard.is_none() {
        *clipboard = Some(
            arboard::Clipboard::new().map_err(|e| format!("打开剪贴板失败: {e}"))?,
        );
    }
    let cb = clipboard
        .as_mut()
        .expect("clipboard just initialized");
    let (width, height) = (image.width() as usize, image.height() as usize);
    let bytes = image.as_raw().clone();
    let data = arboard::ImageData {
        width,
        height,
        bytes: bytes.into(),
    };
    match cb.set_image(data.clone()) {
        Ok(()) => Ok(()),
        Err(e) => {
            // 连接失效或被占用时重建一次再写
            *clipboard = Some(
                arboard::Clipboard::new().map_err(|re| {
                    format!("写入剪贴板失败: {e}；重建剪贴板也失败: {re}")
                })?,
            );
            clipboard
                .as_mut()
                .expect("clipboard just recreated")
                .set_image(data)
                .map_err(|re| format!("写入剪贴板失败: {re}"))
        }
    }
}

/// 比较两张同尺寸图的平均绝对差，用于判断滚动是否到底。
pub fn mean_abs_diff(a: &RgbaImage, b: &RgbaImage) -> f64 {
    if a.width() != b.width() || a.height() != b.height() {
        return f64::MAX;
    }
    let mut total = 0u64;
    let pixels = (a.width() * a.height()) as u64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        total += pa.0[0].abs_diff(pb.0[0]) as u64;
        total += pa.0[1].abs_diff(pb.0[1]) as u64;
        total += pa.0[2].abs_diff(pb.0[2]) as u64;
    }
    total as f64 / (pixels as f64 * 3.0)
}
