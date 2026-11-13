//! Background removal with the BiRefNet model from the bg-removal crate.
//! The model file sits next to the program; it is loaded once on first use.

use bg_removal::{BgRemover, Options};
use image::{DynamicImage, RgbaImage};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub const MODEL_FILE: &str = "birefnet-lite.safetensors";

/// Cut-out of the last whole file (as decoded, before rotation), shared by the preview
/// and the export so the network does not run twice for the same photo.
static LAST: Mutex<Option<(PathBuf, Option<SystemTime>, Arc<RgbaImage>)>> = Mutex::new(None);
static REMOVER: Mutex<Option<Arc<BgRemover>>> = Mutex::new(None);
/// Model file chosen by the user in a dialog, tried before the usual places.
static CUSTOM: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_custom(path: Option<PathBuf>) {
    *CUSTOM.lock().unwrap() = path;
}

/// Where the model file is: the user's choice, next to the exe, or in the settings folder.
pub fn model_path() -> Option<PathBuf> {
    let mut places: Vec<PathBuf> = CUSTOM.lock().unwrap().iter().cloned().collect();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        places.push(dir.join(MODEL_FILE));
        places.push(dir.join("models").join(MODEL_FILE));
    }
    places.push(crate::settings::config_dir().join(MODEL_FILE));
    if cfg!(debug_assertions) {
        places.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("models").join(MODEL_FILE));
    }
    places.into_iter().find(|p| p.is_file())
}

pub fn available() -> bool {
    REMOVER.lock().unwrap().is_some() || model_path().is_some()
}

fn remover() -> Result<Arc<BgRemover>, String> {
    let mut slot = REMOVER.lock().unwrap();
    if let Some(r) = slot.as_ref() {
        return Ok(r.clone());
    }
    let path = model_path().ok_or_else(|| format!("нет файла нейросети {MODEL_FILE}"))?;
    let r = Arc::new(BgRemover::from_file(&path).map_err(|e| format!("не удалось загрузить нейросеть: {}", e.0))?);
    *slot = Some(r.clone());
    Ok(r)
}

/// The main subject on a transparent background, at the image's own size.
pub fn remove(img: &DynamicImage) -> Result<DynamicImage, String> {
    let r = remover()?;
    Ok(DynamicImage::ImageRgba8(r.remove_background(&img.to_rgba8(), &Options::default())))
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The stored cut-out of `path`, if the file has not changed since.
pub fn cached(path: &Path) -> Option<Arc<RgbaImage>> {
    let last = LAST.lock().unwrap();
    let (p, t, img) = last.as_ref()?;
    (p == path && *t == mtime(path)).then(|| img.clone())
}

/// Removes the background of the whole file and keeps the result for later calls.
pub fn remove_file(path: &Path) -> Result<Arc<RgbaImage>, String> {
    if let Some(c) = cached(path) {
        return Ok(c);
    }
    let t = mtime(path);
    let img = crate::decode::load_dynamic(path)?;
    let r = remover()?;
    let out = Arc::new(r.remove_background(&img.to_rgba8(), &Options::default()));
    *LAST.lock().unwrap() = Some((path.to_path_buf(), t, out.clone()));
    Ok(out)
}
