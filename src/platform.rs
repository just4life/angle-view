//! Small OS integrations: Explorer, "Open with", wallpaper, clipboard, keeping the display on.

use std::path::Path;
use std::process::Command;

pub fn show_in_explorer(path: &Path) {
    if cfg!(windows) {
        let _ = Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
    } else if let Some(dir) = path.parent() {
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

pub fn open_with(path: &Path) {
    if cfg!(windows) {
        let _ = Command::new("rundll32.exe").arg(format!("shell32.dll,OpenAs_RunDLL {}", path.display())).spawn();
    } else {
        open_default(path);
    }
}

pub fn open_default(path: &Path) {
    if cfg!(windows) {
        let _ = Command::new("explorer.exe").arg(path).spawn();
    } else {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

pub fn copy_image(path: &Path) -> Result<(), String> {
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: std::borrow::Cow::Owned(img.into_raw()),
    })
    .map_err(|e| e.to_string())
}

/// Windows accepts JPEG, PNG and BMP wallpapers; anything else is converted to PNG first.
pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    let ext = crate::formats::ext_of(path);
    let file = if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "bmp") {
        path.to_path_buf()
    } else {
        let d = crate::decode::decode_full(path, 16384)?;
        let f = d.frames.into_iter().next().ok_or("нет кадров")?;
        let [w, h] = f.image.size;
        let rgba: Vec<u8> = f.image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let out = crate::settings::config_dir().join("wallpaper.png");
        image::save_buffer(&out, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
        out
    };
    set_wallpaper_file(&file)
}

#[cfg(windows)]
fn set_wallpaper_file(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
    };
    let abs = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let s = abs.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    let wide: Vec<u16> = std::ffi::OsStr::new(&s).encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, wide.as_ptr() as *mut _, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)
    };
    if ok != 0 { Ok(()) } else { Err("система отклонила файл".into()) }
}

#[cfg(not(windows))]
fn set_wallpaper_file(_path: &Path) -> Result<(), String> {
    Err("доступно только в Windows".into())
}

#[cfg(windows)]
pub fn keep_display_on(on: bool) {
    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
    };
    unsafe {
        if on {
            SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
        } else {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

#[cfg(not(windows))]
pub fn keep_display_on(_on: bool) {}
