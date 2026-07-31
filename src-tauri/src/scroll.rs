use crate::capture::{self, Region};
use image::{Rgba, RgbaImage};
use std::thread;
use std::time::Duration;

/// 尝试在选区中心向下滚动；失败时返回 Err，由调用方改走手动模式。
pub fn try_auto_scroll(region: &Region, steps: i32) -> Result<(), String> {
    let steps = steps.clamp(1, 20);
    platform_auto_scroll(region, steps)?;
    thread::sleep(Duration::from_millis(220));
    Ok(())
}

#[cfg(target_os = "linux")]
fn platform_auto_scroll(region: &Region, steps: i32) -> Result<(), String> {
    use std::process::Command;

    let cx = region.x + region.width as i32 / 2;
    let cy = region.y + region.height as i32 / 2;
    let status = Command::new("xdotool")
        .args([
            "mousemove",
            &cx.to_string(),
            &cy.to_string(),
            "click",
            "--repeat",
            &steps.to_string(),
            "--delay",
            "30",
            "5",
        ])
        .status()
        .map_err(|e| format!("未找到 xdotool（自动滚动不可用）: {e}"))?;

    if !status.success() {
        return Err("xdotool 滚动失败".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn platform_auto_scroll(region: &Region, steps: i32) -> Result<(), String> {
    use enigo::{
        Axis, Coordinate,
        Direction::Click,
        Enigo, Mouse, Settings,
    };

    let cx = region.x + region.width as i32 / 2;
    let cy = region.y + region.height as i32 / 2;
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("初始化输入模拟失败: {e}"))?;

    // 先点选区中心，确保滚轮作用于目标窗口
    enigo
        .move_mouse(cx, cy, Coordinate::Abs)
        .map_err(|e| format!("移动鼠标失败: {e}"))?;
    enigo
        .button(enigo::Button::Left, Click)
        .map_err(|e| format!("点击选区失败: {e}"))?;
    thread::sleep(Duration::from_millis(40));

    // Vertical 正值向下滚动
    enigo
        .scroll(steps, Axis::Vertical)
        .map_err(|e| format!("滚轮模拟失败: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_auto_scroll(_region: &Region, _steps: i32) -> Result<(), String> {
    Err("自动滚动暂不支持此平台".into())
}

/// `region` 使用虚拟桌面绝对坐标（与显示器 x/y 同一坐标系）。
pub fn capture_region_live(region: &Region) -> Result<RgbaImage, String> {
    let captures = capture::capture_all_monitors()?;
    let min_x = captures.iter().map(|c| c.info.x).min().unwrap_or(0);
    let min_y = captures.iter().map(|c| c.info.y).min().unwrap_or(0);
    let stitched = capture::stitch_virtual_desktop(&captures)?;
    let local = Region {
        x: region.x - min_x,
        y: region.y - min_y,
        width: region.width,
        height: region.height,
    };
    capture::crop_image(&stitched, &local)
}

/// 自动滚动采集多帧；若系统无法模拟滚轮则返回错误。
pub fn capture_scrolling_auto(
    region: &Region,
    max_frames: u32,
) -> Result<RgbaImage, String> {
    if region.width < 16 || region.height < 16 {
        return Err("滚动截图区域太小".into());
    }

    let max_frames = max_frames.clamp(2, 40);
    let mut frames = Vec::with_capacity(max_frames as usize);
    frames.push(capture_region_live(region)?);

    let steps = ((region.height as f32 / 120.0) * 0.55).ceil().max(1.0) as i32;

    // 先探测一次，避免采了很多帧才发现不能滚动
    try_auto_scroll(region, steps)?;

    for _ in 1..max_frames {
        try_auto_scroll(region, steps)?;
        let next = capture_region_live(region)?;
        let prev = frames.last().unwrap();
        if capture::mean_abs_diff(prev, &next) < 1.5 {
            break;
        }
        frames.push(next);
    }

    stitch_vertical(&frames)
}

/// 按垂直方向拼接多帧，通过模板匹配估计重叠高度。
pub fn stitch_vertical(frames: &[RgbaImage]) -> Result<RgbaImage, String> {
    if frames.is_empty() {
        return Err("没有可拼接的帧".into());
    }
    if frames.len() == 1 {
        return Ok(frames[0].clone());
    }

    let width = frames[0].width();
    for frame in frames {
        if frame.width() != width {
            return Err("帧宽度不一致，无法拼接".into());
        }
    }

    let mut offsets = Vec::with_capacity(frames.len());
    offsets.push(0u32);

    for i in 1..frames.len() {
        let overlap = estimate_overlap(&frames[i - 1], &frames[i]);
        let advance = frames[i - 1].height().saturating_sub(overlap);
        let next_y = offsets[i - 1] + advance;
        offsets.push(next_y);
    }

    let total_height = offsets
        .last()
        .copied()
        .unwrap_or(0)
        .saturating_add(frames.last().unwrap().height());

    let mut canvas = RgbaImage::from_pixel(width, total_height, Rgba([0, 0, 0, 255]));
    for (frame, y) in frames.iter().zip(offsets.into_iter()) {
        image::imageops::replace(&mut canvas, frame, 0, y as i64);
    }
    Ok(canvas)
}

fn estimate_overlap(prev: &RgbaImage, next: &RgbaImage) -> u32 {
    let h = prev.height().min(next.height());
    if h < 8 {
        return 0;
    }

    let strip_h = (h / 10).clamp(8, 48);
    let search_max = ((h as f32 * 0.85) as u32).max(strip_h).min(h);

    let strip_y = prev.height().saturating_sub(strip_h);
    let mut best_y = 0u32;
    let mut best_score = u64::MAX;

    for y in 0..=(search_max.saturating_sub(strip_h)) {
        let score = strip_diff(prev, strip_y, next, y, strip_h);
        if score < best_score {
            best_score = score;
            best_y = y;
        }
    }

    let overlap = best_y + strip_h;
    overlap.min(h.saturating_sub(4))
}

fn strip_diff(a: &RgbaImage, ay: u32, b: &RgbaImage, by: u32, strip_h: u32) -> u64 {
    let width = a.width().min(b.width());
    let mut total = 0u64;
    let step_x = (width / 64).max(1);
    let step_y = (strip_h / 16).max(1);

    let mut samples = 0u64;
    let mut y = 0u32;
    while y < strip_h {
        let mut x = 0u32;
        while x < width {
            let pa = a.get_pixel(x, ay + y).0;
            let pb = b.get_pixel(x, by + y).0;
            total += pa[0].abs_diff(pb[0]) as u64;
            total += pa[1].abs_diff(pb[1]) as u64;
            total += pa[2].abs_diff(pb[2]) as u64;
            samples += 1;
            x += step_x;
        }
        y += step_y;
    }

    if samples == 0 {
        u64::MAX
    } else {
        total / samples
    }
}
