//! Background removal with the BiRefNet model from the bg-removal crate.
//! The model file sits next to the program; it is loaded once on first use.

use bg_removal::{BgRemover, Options};
use image::DynamicImage;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const MODEL_FILE: &str = "birefnet-lite.safetensors";

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
