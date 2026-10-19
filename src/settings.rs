use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ZoomMode {
    Fit,
    FitLarger,
    Fill,
    Actual,
    FitWidth,
    FitHeight,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Transition {
    None,
    Fade,
    Slide,
    Zoom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SortKey {
    Name,
    Modified,
    Size,
    Type,
    Random,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WheelAction {
    Navigate,
    Zoom,
}

/// Thumbnail strip next to the single image view.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Strip {
    Off,
    Side,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ClockStyle {
    Off,
    Digital,
    Analog,
}

const SETTINGS_VERSION: u32 = 3;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Format version of the settings file, used to migrate changed defaults.
    #[serde(default)]
    pub version: u32,
    pub slideshow_interval: f32,
    pub slideshow_random: bool,
    pub slideshow_loop: bool,
    pub slideshow_fullscreen: bool,
    pub transition: Transition,
    pub transition_ms: u32,
    pub transition_on_manual: bool,
    pub background: [u8; 3],
    pub default_zoom: ZoomMode,
    pub remember_zoom: bool,
    pub wheel: WheelAction,
    pub include_subfolders: bool,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub thumb_size: f32,
    /// Thumbnail rows scrolled per wheel notch in the grid.
    pub grid_wheel_rows: f32,
    /// Size of text and controls, 1.0 = 100 %.
    pub ui_scale: f32,
    pub strip: Strip,
    pub strip_size: f32,
    pub watch_folder: bool,
    pub jump_to_new: bool,
    pub show_caption: bool,
    pub clock: ClockStyle,
    pub show_calendar: bool,
    pub confirm_delete: bool,
    pub toolbar_visible: bool,
    pub restore_session: bool,
    pub autoplay_animations: bool,
    pub quick_access: Vec<PathBuf>,
    pub recent: Vec<PathBuf>,
    /// Files added to favourites ("Избранное"), across all folders, newest first.
    pub flagged: Vec<PathBuf>,
    /// Model file for background removal picked by the user, when it is not next to the program.
    pub bg_model: Option<PathBuf>,
    pub export: crate::export::ExportSettings,
    pub export_profiles: Vec<crate::export::ExportProfile>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            slideshow_interval: 5.0,
            slideshow_random: false,
            slideshow_loop: true,
            slideshow_fullscreen: true,
            transition: Transition::Fade,
            transition_ms: 450,
            transition_on_manual: false,
            background: [0, 0, 0],
            default_zoom: ZoomMode::FitLarger,
            remember_zoom: false,
            wheel: WheelAction::Zoom,
            include_subfolders: false,
            sort: SortKey::Name,
            sort_desc: false,
            grid_wheel_rows: 1.0,
            ui_scale: 1.0,
            strip: Strip::Off,
            strip_size: 110.0,
            thumb_size: 150.0,
            watch_folder: true,
            jump_to_new: false,
            show_caption: false,
            clock: ClockStyle::Off,
            show_calendar: false,
            confirm_delete: true,
            toolbar_visible: true,
            restore_session: true,
            autoplay_animations: true,
            quick_access: Vec::new(),
            recent: Vec::new(),
            flagged: Vec::new(),
            bg_model: None,
            export: Default::default(),
            export_profiles: Vec::new(),
        }
    }
}

/// A saved working set: a folder (optionally recursive) or an explicit list of files.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub folder: Option<PathBuf>,
    pub recursive: bool,
    pub files: Vec<PathBuf>,
    pub current: Option<PathBuf>,
    pub flagged: Vec<PathBuf>,
    pub grid: bool,
}

pub fn config_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("AngleView");
    // Settings from the version that was still called "Image Viewer" move over once.
    let old = base.join("ImageViewer");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(entries) = std::fs::read_dir(&old) {
            for e in entries.flatten() {
                let _ = std::fs::copy(e.path(), dir.join(e.file_name()));
            }
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_json<T: Serialize>(path: &std::path::Path, value: &T) {
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(path, text);
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Self = load_json(&config_dir().join("settings.json")).unwrap_or_default();
        if s.version < 2 {
            // Version 2: the mouse wheel zooms by default instead of switching images.
            s.wheel = WheelAction::Zoom;
        }
        if s.version < 3 {
            // Version 3: the crop ratio presets changed and 3:4 became the default.
            s.export.ratio = (3, 4);
        }
        s.version = SETTINGS_VERSION;
        s
    }
    pub fn save(&self) {
        save_json(&config_dir().join("settings.json"), self);
    }
    pub fn push_recent(&mut self, p: PathBuf) {
        self.recent.retain(|x| x != &p);
        self.recent.insert(0, p);
        self.recent.truncate(10);
    }
}

impl Session {
    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        load_json(path)
    }
    pub fn save_to(&self, path: &std::path::Path) {
        save_json(path, self);
    }
    pub fn autosave_path() -> PathBuf {
        config_dir().join("last_session.json")
    }
}
