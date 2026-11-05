//! The dragged file's picture in the wheel's hub on Windows and Linux, as macOS shows
//! Quick Look's thumbnail there (wheel_window.rs). Windows: the shell's own thumbnail,
//! or the file type's icon where it has none. Linux: the picture itself for images,
//! else the thumbnail a file manager already made (the freedesktop thumbnail cache).
//! Worked out on a thread of its own and sent as a PNG data address when ready
//! ("wheel-picture"), so the wheel never waits for it.

#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::path::Path;

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Pixels on each side: twice the 40 px it is drawn at, for high-density screens.
const SIZE: i32 = 80;

#[derive(Clone, Serialize)]
struct Picture {
    path: String,
    url: String,
}

/// Sends the picture of `path` to the wheel, if this system has one for it.
pub fn send(app: &AppHandle, path: &str) {
    if cfg!(target_os = "macos") {
        return;
    }
    let app = app.clone();
    let path = path.to_string();
    std::thread::spawn(move || {
        if let Some(png) = png_for(Path::new(&path)) {
            let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png));
            let _ = app.emit("wheel-picture", Picture { path, url });
        }
    });
}

/// The shell's thumbnail (or icon) as straight RGBA, then PNG.
#[cfg(windows)]
fn png_for(path: &Path) -> Option<Vec<u8>> {
    use std::ffi::c_void;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::SIZE as Size;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT};

    unsafe {
        // A thread of its own, so its own apartment.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let bitmap = factory.GetImage(Size { cx: SIZE, cy: SIZE }, SIIGBF_RESIZETOFIT).ok()?;
        let mut info = BITMAP::default();
        let read = GetObjectW(bitmap.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut info as *mut BITMAP as *mut c_void));
        let (w, h) = (info.bmWidth, info.bmHeight);
        let mut pixels = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top row first
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = CreateCompatibleDC(None);
        let rows = if read == 0 || pixels.is_empty() {
            0
        } else {
            GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr() as *mut c_void), &mut header, DIB_RGB_COLORS)
        };
        let _ = DeleteDC(dc);
        let _ = DeleteObject(bitmap.into());
        if rows == 0 {
            return None;
        }
        straight_rgba(&mut pixels);
        encode(w as u32, h as u32, &pixels)
    }
}

/// The shell gives BGRA with the colour already multiplied by alpha, and a photo's
/// thumbnail may carry no alpha at all (every pixel 0): then it is opaque.
#[cfg_attr(not(windows), allow(dead_code))]
fn straight_rgba(pixels: &mut [u8]) {
    let opaque = pixels.chunks_exact(4).all(|p| p[3] == 0);
    for p in pixels.chunks_exact_mut(4) {
        p.swap(0, 2);
        if opaque {
            p[3] = 255;
        } else if p[3] > 0 && p[3] < 255 {
            let a = u32::from(p[3]);
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

#[cfg(windows)]
fn encode(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

/// Images by themselves, everything else from the thumbnail cache. gdk-pixbuf may be
/// used off the main thread; the icon theme may not, so there is no icon fallback.
#[cfg(target_os = "linux")]
fn png_for(path: &Path) -> Option<Vec<u8>> {
    use gtk::gdk_pixbuf::Pixbuf;
    let scaled = |p: &Path| Pixbuf::from_file_at_scale(p, SIZE, SIZE, true).ok();
    let picture = scaled(path).or_else(|| {
        let uri = glib::filename_to_uri(path, None).ok()?;
        let digest = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri.as_str())?;
        let cache = glib::user_cache_dir().join("thumbnails");
        ["large", "normal"].iter().find_map(|size| scaled(&cache.join(size).join(format!("{digest}.png"))))
    })?;
    picture.save_to_bufferv("png", &[]).ok()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn png_for(_path: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_pixels_turn_into_straight_rgba() {
        // Half-transparent blue (premultiplied 0,0,64 at alpha 128) and opaque red, as BGRA.
        let mut px = vec![64, 0, 0, 128, 0, 0, 255, 255];
        straight_rgba(&mut px);
        assert_eq!(px, vec![0, 0, 128, 128, 255, 0, 0, 255]);
        // No alpha anywhere: a photo, opaque.
        let mut photo = vec![10, 20, 30, 0, 40, 50, 60, 0];
        straight_rgba(&mut photo);
        assert_eq!(photo, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }
}
