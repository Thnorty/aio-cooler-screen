//! Preparing user media for the 480x480 screen: images become PNG, videos become
//! H.264 MP4 at 20 fps without audio (the format of the official app's samples).

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

pub const SCREEN_SIZE: u32 = 480;
const VIDEO_FPS: u32 = 20;
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp"];
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "gif"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fit {
    Cover,
    Contain,
    Stretch,
}

pub fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// File name in the official app's style: YYYY-MM-DD_HH-MM-SS-mmm (UTC).
pub fn timestamp_name(ext: &str) -> String {
    let ms = now_ms() as i64;
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s, milli) = (rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000);
    format!("{y:04}-{m:02}-{d:02}_{h:02}-{mi:02}-{s:02}-{milli:03}.{ext}")
}

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(bytes)
}

/// Scale an image for the screen. Transparent parts become black: the screen has nothing to
/// show through them.
pub fn fit_image(img: &DynamicImage, fit: Fit) -> RgbaImage {
    let scaled = match fit {
        Fit::Cover => img.resize_to_fill(SCREEN_SIZE, SCREEN_SIZE, FilterType::Lanczos3).to_rgba8(),
        Fit::Stretch => img.resize_exact(SCREEN_SIZE, SCREEN_SIZE, FilterType::Lanczos3).to_rgba8(),
        Fit::Contain => img.resize(SCREEN_SIZE, SCREEN_SIZE, FilterType::Lanczos3).to_rgba8(),
    };
    let mut canvas = RgbaImage::from_pixel(SCREEN_SIZE, SCREEN_SIZE, Rgba([0, 0, 0, 255]));
    let x = (SCREEN_SIZE - scaled.width()) / 2;
    let y = (SCREEN_SIZE - scaled.height()) / 2;
    image::imageops::overlay(&mut canvas, &scaled, x.into(), y.into());
    canvas
}

pub fn prepare_image(path: &Path, fit: Fit) -> Result<RgbaImage, String> {
    let img = image::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(fit_image(&img, fit))
}

fn find_ffmpeg() -> Result<PathBuf, String> {
    crate::ffmpeg::find().ok_or_else(|| "Video backgrounds need ffmpeg. Download it from the Device page.".into())
}

fn ffmpeg_command(path: &Path) -> Command {
    crate::ffmpeg::command(path)
}

fn video_filter(fit: Fit, extra: &str) -> String {
    let s = SCREEN_SIZE;
    let scale = match fit {
        Fit::Cover => format!("scale={s}:{s}:force_original_aspect_ratio=increase,crop={s}:{s}"),
        Fit::Contain => {
            format!("scale={s}:{s}:force_original_aspect_ratio=decrease,pad={s}:{s}:(ow-iw)/2:(oh-ih)/2:black")
        }
        Fit::Stretch => format!("scale={s}:{s}"),
    };
    // Premultiplying makes transparent areas (GIFs, WebM with alpha) black instead of showing
    // whatever colour they hide once the alpha is dropped.
    format!("format=rgba,premultiply=inplace=1,{scale},setsar=1{extra}")
}

/// Convert any video to the format the screen plays and write it to `out`.
pub fn prepare_video(path: &Path, fit: Fit, out: &Path) -> Result<(), String> {
    let ffmpeg = find_ffmpeg()?;
    let output = ffmpeg_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(path)
        .args(["-vf", &video_filter(fit, &format!(",fps={VIDEO_FPS}"))])
        .args(["-c:v", "libx264", "-profile:v", "high", "-pix_fmt", "yuv420p"])
        .args(["-b:v", "1000k", "-maxrate", "1500k", "-bufsize", "2000k"])
        .args(["-an", "-movflags", "+faststart"])
        .arg(out)
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(format!("ffmpeg failed to convert {}: {}", path.display(), err.trim()))
    }
}

/// First frame of a video, fitted like the conversion, for the GUI preview.
pub fn video_poster(path: &Path, fit: Fit) -> Result<RgbaImage, String> {
    let ffmpeg = find_ffmpeg()?;
    let output = ffmpeg_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vf", &video_filter(fit, ""), "-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()
        .map_err(|e| format!("failed to run ffmpeg: {e}"))?;
    image::load_from_memory(&output.stdout)
        .map(|i| i.to_rgba8())
        .map_err(|e| format!("cannot read video frame: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_parts_become_black() {
        // A see-through image hiding red, with an opaque white square in the middle.
        let img = RgbaImage::from_fn(100, 100, |x, y| {
            if (40..60).contains(&x) && (40..60).contains(&y) { Rgba([255, 255, 255, 255]) } else { Rgba([255, 0, 0, 0]) }
        });
        for fit in [Fit::Cover, Fit::Contain, Fit::Stretch] {
            let out = fit_image(&DynamicImage::ImageRgba8(img.clone()), fit);
            assert_eq!(*out.get_pixel(5, 5), Rgba([0, 0, 0, 255]), "{fit:?}");
            assert_eq!(*out.get_pixel(240, 240), Rgba([255, 255, 255, 255]), "{fit:?}");
        }
    }
}
