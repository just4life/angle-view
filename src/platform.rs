//! Small OS integrations: Explorer, "Open with", wallpaper, clipboard, keeping the display on.

use std::path::Path;
use std::process::Command;

/// Opens the file's folder in Explorer with the file selected.
#[cfg(windows)]
pub fn show_in_explorer(path: &Path) {
    let path = shell_path(path);
    // The shell call can block while Explorer starts, so it runs off the UI thread.
    std::thread::spawn(move || {
        if !win::select_in_folder(&path) {
            use std::os::windows::process::CommandExt;
            // The path must be quoted after the comma: unquoted spaces or commas make
            // Explorer ignore it and open Documents.
            let _ = Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
        }
    });
}

#[cfg(not(windows))]
pub fn show_in_explorer(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

/// The system "Open with" dialog for the file.
#[cfg(windows)]
pub fn open_with(path: &Path) {
    let path = shell_path(path);
    std::thread::spawn(move || {
        if !win::open_with_dialog(&path) {
            use std::os::windows::process::CommandExt;
            // rundll32 takes the rest of its command line as the file name, unquoted.
            let _ = Command::new("rundll32.exe").raw_arg(format!("shell32.dll,OpenAs_RunDLL {}", path.display())).spawn();
        }
    });
}

#[cfg(not(windows))]
pub fn open_with(path: &Path) {
    open_default(path);
}

/// Absolute path with backslashes and without the `\\?\` prefix, which shell functions reject.
#[cfg(windows)]
fn shell_path(path: &Path) -> std::path::PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy().replace('/', "\\");
    let s = match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s),
    };
    std::path::PathBuf::from(s)
}

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows_sys::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, SHOpenWithDialog, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO};

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// Runs `f` with COM initialized on this thread, as the shell functions require.
    fn with_com<T>(f: impl FnOnce() -> T) -> T {
        let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        let r = f();
        if hr >= 0 {
            unsafe { CoUninitialize() };
        }
        r
    }

    pub fn select_in_folder(path: &Path) -> bool {
        let w = wide(path);
        with_com(|| unsafe {
            let pidl = ILCreateFromPathW(w.as_ptr());
            if pidl.is_null() {
                return false;
            }
            let hr = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
            ILFree(pidl);
            hr >= 0
        })
    }

    pub fn open_with_dialog(path: &Path) -> bool {
        let w = wide(path);
        let info = OPENASINFO { pcszFile: w.as_ptr(), pcszClass: std::ptr::null(), oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC };
        // Closing the dialog without a choice returns "cancelled": that is not a failure.
        const CANCELLED: i32 = 0x800704C7u32 as i32;
        let hr = with_com(|| unsafe { SHOpenWithDialog(std::ptr::null_mut(), &info) });
        hr >= 0 || hr == CANCELLED
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
