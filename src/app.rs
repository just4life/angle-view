//! Application state, commands and the folder/session model.

use crate::decode::Decoded;
use crate::formats::{self, Kind};
use crate::loader::{Loader, Msg};
use crate::meta::Metadata;
use crate::platform;
use crate::settings::{ClockStyle, Session, Settings, SortKey, Strip, ZoomMode};
use eframe::egui::{self, Context, Pos2, Rect, TextureHandle, TextureId, TextureOptions, Vec2};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
    pub size: u64,
    pub kind: Kind,
    pub flagged: bool,
}

impl Item {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let kind = formats::kind_of(&path)?;
        let md = std::fs::metadata(&path).ok()?;
        if !md.is_file() {
            return None;
        }
        Some(Self {
            name: path.file_name()?.to_string_lossy().into_owned(),
            modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            size: md.len(),
            kind,
            flagged: false,
            path,
        })
    }
}

pub struct FullImage {
    pub textures: Vec<TextureHandle>,
    pub delays: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub vector: bool,
    pub histogram: Box<[[u32; 256]; 4]>,
}

pub enum FullState {
    Loading,
    Ready(Rc<FullImage>),
    Failed(String),
}

#[derive(Default)]
pub struct ThumbEntry {
    pub tex: Option<TextureHandle>,
    pub size: u32,
    pub pending: bool,
    pub failed: bool,
    pub last_used: u64,
}

#[derive(Clone, Copy)]
pub struct View {
    pub mode: ZoomMode,
    /// Screen points per image pixel.
    pub scale: f32,
    pub pan: Vec2,
    /// Quarter turns clockwise.
    pub rot: u8,
    pub flip: bool,
}

/// What was painted for the main image in the last frame; the source of transitions.
#[derive(Clone)]
pub struct Drawn {
    /// Keeps the textures alive while a transition still paints them.
    #[allow(dead_code)]
    pub keep: Option<Rc<FullImage>>,
    pub tex: TextureId,
    pub rect: Rect,
    pub rot: u8,
    pub flip: bool,
}

pub struct TransitionAnim {
    pub from: Drawn,
    pub start: f64,
    pub dir: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Menu {
    None,
    Context(Pos2),
    Tools(Pos2),
    Zoom(Pos2),
    Folders(Pos2),
    Sort(Pos2),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    None,
    Info,
    Settings,
    Meta,
    Export,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    First,
    Prev,
    Next,
    Last,
    ToggleSlideshow,
    ToggleFullscreen,
    ToggleGrid,
    SetGrid(bool),
    SetStrip(Strip),
    Zoom(ZoomMode),
    ZoomIn,
    ZoomOut,
    RotateCw,
    RotateCcw,
    FlipH,
    Info,
    AllMeta,
    SelectAll,
    ClearSelection,
    CopyPrompt,
    Export,
    RemoveBgExport,
    PickBgModel,
    Settings,
    Delete,
    CopyPath,
    CopyImage,
    OpenWith,
    ShowInExplorer,
    OpenExternal,
    SetWallpaper,
    Flag,
    Unflag(PathBuf),
    ShowFlagged,
    ClearFlags,
    CopyFlagged,
    MoveFlagged,
    Sort(SortKey),
    SortDesc,
    AddQuickFolder,
    OpenPath(PathBuf),
    RemoveQuick(PathBuf),
    OpenFolder,
    OpenFiles,
    ToggleToolbar,
    PhotoFrame,
    Reload,
    ToggleCaption,
    CycleClock,
    IncludeSubfolders,
    AnimToggle,
    AnimStep(i32),
    OpenMenu(Menu),
    Exit,
}

pub struct App {
    pub s: Settings,
    pub loader: Loader,
    /// Every file in the session.
    pub items: Vec<Item>,
    /// Indices into `items` that are currently shown (after the flagged-only filter).
    pub visible: Vec<usize>,
    pub index: usize,
    pub folder: Option<PathBuf>,
    pub recursive: bool,
    pub flagged_only: bool,
    pub grid: bool,

    pub thumbs: HashMap<PathBuf, ThumbEntry>,
    pub thumb_px: u32,
    pub full: HashMap<PathBuf, FullState>,
    pub full_order: VecDeque<PathBuf>,
    pub meta: Option<(PathBuf, Metadata)>,
    pub meta_filter: String,
    /// Long metadata value opened in the text viewer: (title, text).
    pub text_view: Option<(String, String)>,
    pub export: crate::export_ui::ExportUi,

    pub view: View,
    pub shown: Option<PathBuf>,
    pub last_drawn: Option<Drawn>,
    pub pending_transition: Option<f32>,
    pub transition: Option<TransitionAnim>,
    pub nav_dir: i32,

    pub anim_frame: usize,
    pub anim_playing: bool,
    pub anim_next: f64,
    pub bottom_bar_until: f64,

    pub slideshow: bool,
    pub slide_deadline: Option<f64>,
    pub fullscreen: bool,
    pub frame_mode: bool,
    pub last_pointer_move: f64,
    pub wheel_acc: f32,
    pub pending_zoom: Option<(f32, Option<Pos2>)>,

    pub menu: Menu,
    pub menu_just_opened: bool,
    pub click_closes_menu: bool,
    pub panel: Panel,
    pub confirm_delete: Option<Vec<PathBuf>>,
    /// Files selected in the grid (Ctrl / Shift + click, Ctrl+A).
    pub selected: HashSet<PathBuf>,
    pub sel_anchor: usize,
    /// A picked folder that has subfolders: asking whether to include them.
    /// The background-removal model was not found: ask for its file.
    pub ask_model: bool,
    pub ask_subfolders: Option<PathBuf>,
    pub toast: Option<(String, f64)>,

    pub grid_cols: usize,
    pub grid_offset: f32,
    /// Where a wheel scroll of the grid is heading; animated towards over a few frames.
    pub grid_target: Option<f32>,
    pub strip_offset: f32,
    pub strip_target: Option<f32>,
    /// Index the strip last scrolled to, so it follows navigation.
    pub strip_index: usize,
    pub grid_scroll_to_current: bool,
    pub frame_no: u64,

    watcher: Option<notify::RecommendedWatcher>,
    fs_tx: crossbeam_channel::Sender<notify::Result<notify::Event>>,
    fs_rx: crossbeam_channel::Receiver<notify::Result<notify::Event>>,
    pub title: String,
    pub ctx: Context,
}

const TEX_OPTIONS: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: Some(egui::TextureFilter::Linear),
};

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, arg: Option<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        crate::style::setup(&ctx);
        let s = Settings::load();
        crate::cutout::set_custom(s.bg_model.clone());
        let (fs_tx, fs_rx) = crossbeam_channel::unbounded();
        let mut app = Self {
            view: View { mode: s.default_zoom, scale: 1.0, pan: Vec2::ZERO, rot: 0, flip: false },
            s,
            loader: Loader::new(&ctx),
            items: Vec::new(),
            visible: Vec::new(),
            index: 0,
            folder: None,
            recursive: false,
            flagged_only: false,
            grid: true,
            thumbs: HashMap::new(),
            thumb_px: 0,
            full: HashMap::new(),
            full_order: VecDeque::new(),
            meta: None,
            meta_filter: String::new(),
            text_view: None,
            export: Default::default(),
            shown: None,
            last_drawn: None,
            pending_transition: None,
            transition: None,
            nav_dir: 1,
            anim_frame: 0,
            anim_playing: true,
            anim_next: 0.0,
            bottom_bar_until: 0.0,
            slideshow: false,
            slide_deadline: None,
            fullscreen: false,
            frame_mode: false,
            last_pointer_move: 0.0,
            wheel_acc: 0.0,
            pending_zoom: None,
            menu: Menu::None,
            menu_just_opened: false,
            click_closes_menu: false,
            panel: Panel::None,
            confirm_delete: None,
            selected: HashSet::new(),
            sel_anchor: 0,
            ask_model: false,
            ask_subfolders: None,
            toast: None,
            grid_cols: 1,
            grid_offset: 0.0,
            grid_target: None,
            strip_offset: 0.0,
            strip_target: None,
            strip_index: usize::MAX,
            grid_scroll_to_current: false,
            frame_no: 0,
            watcher: None,
            fs_tx,
            fs_rx,
            title: String::new(),
            ctx,
        };
        let frame_mode = std::env::args().any(|a| a == "--frame" || a == "/frame");
        match arg {
            Some(p) => app.open_path(&p),
            None => {
                if app.s.restore_session {
                    if let Some(sess) = Session::load_from(&Session::autosave_path()) {
                        app.apply_session(sess);
                    }
                }
            }
        }
        if frame_mode {
            app.act(Action::PhotoFrame);
        }
        app
    }

    // ---------- model ----------

    pub fn count(&self) -> usize {
        self.visible.len()
    }

    pub fn item(&self, vi: usize) -> Option<&Item> {
        self.visible.get(vi).and_then(|&i| self.items.get(i))
    }

    pub fn current(&self) -> Option<&Item> {
        self.item(self.index)
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.current().map(|i| i.path.clone())
    }

    fn rebuild_visible(&mut self, keep: Option<PathBuf>) {
        self.visible = (0..self.items.len())
            .filter(|&i| !self.flagged_only || self.items[i].flagged)
            .collect();
        self.index = keep
            .and_then(|p| self.visible.iter().position(|&i| self.items[i].path == p))
            .unwrap_or(0)
            .min(self.visible.len().saturating_sub(1));
        self.grid_scroll_to_current = true;
    }

    fn sort_items(&mut self) {
        let key = self.s.sort;
        let desc = self.s.sort_desc;
        if key == SortKey::Random {
            fastrand::shuffle(&mut self.items);
            return;
        }
        self.items.sort_by(|a, b| {
            let o = match key {
                SortKey::Name => natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy()),
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Size => a.size.cmp(&b.size),
                SortKey::Type => formats::ext_of(&a.path)
                    .cmp(&formats::ext_of(&b.path))
                    .then_with(|| natural_cmp(&a.name, &b.name)),
                SortKey::Random => std::cmp::Ordering::Equal,
            };
            if desc { o.reverse() } else { o }
        });
    }

    pub fn resort(&mut self) {
        let keep = self.current_path();
        self.sort_items();
        self.rebuild_visible(keep);
    }

    pub fn open_path(&mut self, path: &Path) {
        if formats::is_session(path) {
            if let Some(sess) = Session::load_from(path) {
                self.apply_session(sess);
            }
            return;
        }
        if path.is_dir() {
            self.load_folder(path.to_path_buf(), None, self.s.include_subfolders);
            self.set_grid(true);
        } else if path.is_file() {
            let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
            self.load_folder(dir, Some(path.to_path_buf()), false);
            self.set_grid(false);
        }
    }

    /// Marks the loaded items that are in the saved list of flagged files.
    fn sync_flags(&mut self) {
        let flagged: HashSet<&PathBuf> = self.s.flagged.iter().collect();
        for it in &mut self.items {
            it.flagged = flagged.contains(&it.path);
        }
    }

    /// Flagged files that still exist, newest first.
    pub fn flagged_files(&self) -> Vec<PathBuf> {
        self.s.flagged.iter().filter(|p| p.is_file()).cloned().collect()
    }

    /// Puts text on the system clipboard.
    pub fn copy_text(&self, text: String) {
        #[cfg(windows)]
        if let Ok(mut cb) = arboard::Clipboard::new() {
            if cb.set_text(text.clone()).is_ok() {
                return;
            }
        }
        self.ctx.copy_text(text);
    }

    pub fn open_folder(&mut self, dir: PathBuf, recursive: bool) {
        self.ask_subfolders = None;
        self.s.include_subfolders = recursive;
        self.load_folder(dir, None, recursive);
        self.set_grid(true);
    }

    pub fn load_folder(&mut self, dir: PathBuf, select: Option<PathBuf>, recursive: bool) {
        let mut items = Vec::new();
        scan_dir(&dir, recursive, &mut items, 0);
        self.items = items;
        self.sync_flags();
        self.folder = Some(dir.clone());
        self.recursive = recursive;
        self.flagged_only = false;
        self.sort_items();
        self.rebuild_visible(select);
        self.reset_caches();
        self.s.push_recent(dir.clone());
        self.watch(Some(dir));
        self.on_index_changed(0);
    }

    pub fn open_files(&mut self, files: Vec<PathBuf>) {
        if files.len() == 1 {
            self.open_path(&files[0]);
            return;
        }
        self.items = files.into_iter().filter_map(Item::from_path).collect();
        self.sync_flags();
        self.folder = None;
        self.recursive = false;
        self.flagged_only = false;
        self.sort_items();
        self.rebuild_visible(None);
        self.reset_caches();
        self.watch(None);
        self.set_grid(self.items.len() > 1);
        self.on_index_changed(0);
    }

    fn reset_caches(&mut self) {
        self.loader.clear_thumb_queue();
        self.thumbs.clear();
        self.full.clear();
        self.full_order.clear();
        self.shown = None;
        self.last_drawn = None;
        self.transition = None;
    }

    pub fn session(&self) -> Session {
        Session {
            folder: self.folder.clone(),
            recursive: self.recursive,
            files: if self.folder.is_none() { self.items.iter().map(|i| i.path.clone()).collect() } else { Vec::new() },
            current: self.current_path(),
            flagged: self.items.iter().filter(|i| i.flagged).map(|i| i.path.clone()).collect(),
            grid: self.grid,
        }
    }

    pub fn apply_session(&mut self, sess: Session) {
        if let Some(dir) = sess.folder.clone().filter(|d| d.is_dir()) {
            self.load_folder(dir, sess.current.clone(), sess.recursive);
        } else if !sess.files.is_empty() {
            self.open_files(sess.files.clone());
            let keep = sess.current.clone();
            self.rebuild_visible(keep);
        } else {
            return;
        }
        for p in sess.flagged {
            if !self.s.flagged.contains(&p) {
                self.s.flagged.push(p);
            }
        }
        self.sync_flags();
        self.set_grid(sess.grid);
        self.on_index_changed(0);
    }

    fn watch(&mut self, dir: Option<PathBuf>) {
        use notify::Watcher;
        self.watcher = None;
        let Some(dir) = dir else { return };
        if !self.s.watch_folder {
            return;
        }
        let tx = self.fs_tx.clone();
        let ctx = self.ctx.clone();
        let w = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        if let Ok(mut w) = w {
            let mode = if self.recursive { notify::RecursiveMode::Recursive } else { notify::RecursiveMode::NonRecursive };
            if w.watch(&dir, mode).is_ok() {
                self.watcher = Some(w);
            }
        }
    }

    pub fn rewatch(&mut self) {
        self.watch(self.folder.clone());
    }

    fn process_fs_events(&mut self) {
        use notify::EventKind;
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        while let Ok(res) = self.fs_rx.try_recv() {
            let Ok(ev) = res else { continue };
            for p in ev.paths {
                if formats::kind_of(&p).is_none() {
                    continue;
                }
                match ev.kind {
                    EventKind::Create(_) => added.push(p),
                    EventKind::Remove(_) => removed.push(p),
                    EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                        if p.exists() { added.push(p) } else { removed.push(p) }
                    }
                    EventKind::Modify(_) => changed.push(p),
                    _ => {}
                }
            }
        }
        if added.is_empty() && removed.is_empty() && changed.is_empty() {
            return;
        }
        let keep = self.current_path();
        let mut new_path = None;
        let removed: HashSet<PathBuf> = removed.into_iter().collect();
        if !removed.is_empty() {
            self.items.retain(|i| !removed.contains(&i.path));
            for p in &removed {
                self.invalidate(p);
            }
        }
        for p in added {
            if self.items.iter().any(|i| i.path == p) {
                changed.push(p);
                continue;
            }
            if let Some(it) = Item::from_path(p.clone()) {
                self.items.push(it);
                new_path = Some(p);
            }
        }
        for p in &changed {
            if let Some(it) = self.items.iter_mut().find(|i| &i.path == p) {
                if let Ok(md) = std::fs::metadata(p) {
                    it.size = md.len();
                    it.modified = md.modified().unwrap_or(it.modified);
                }
            }
            self.invalidate(p);
        }
        if self.s.sort != SortKey::Random {
            self.sort_items();
        }
        let jump = new_path.filter(|_| self.s.jump_to_new);
        let target = jump.clone().or(keep.clone().filter(|k| !removed.contains(k)));
        self.rebuild_visible(target);
        if jump.is_some() || keep.as_ref().is_some_and(|k| removed.contains(k)) {
            self.on_index_changed(1);
        }
    }

    pub fn invalidate(&mut self, p: &Path) {
        self.thumbs.remove(p);
        self.full.remove(p);
        self.full_order.retain(|x| x != p);
        if self.shown.as_deref() == Some(p) {
            self.shown = None;
        }
    }

    // ---------- navigation ----------

    pub fn go_to(&mut self, i: usize, dir: i32) {
        if self.count() == 0 {
            return;
        }
        let i = i.min(self.count() - 1);
        if i == self.index && self.shown.is_some() {
            return;
        }
        self.index = i;
        self.on_index_changed(dir);
    }

    pub fn on_index_changed(&mut self, dir: i32) {
        self.nav_dir = if dir < 0 { -1 } else { 1 };
        self.anim_frame = 0;
        self.anim_playing = self.s.autoplay_animations;
        self.slide_deadline = None;
        self.grid_scroll_to_current = true;
        if !self.grid && (self.slideshow || self.s.transition_on_manual) && dir != 0 {
            self.pending_transition = Some(self.nav_dir as f32);
        } else {
            self.pending_transition = None;
        }
        if !self.s.remember_zoom {
            self.view.mode = self.s.default_zoom;
        }
        self.view.rot = 0;
        self.view.flip = false;
        self.view.pan = Vec2::ZERO;
        if matches!(self.panel, Panel::Info | Panel::Meta) {
            self.request_meta();
        }
    }

    pub fn step(&mut self, delta: i32) {
        let n = self.count() as i32;
        if n == 0 {
            return;
        }
        let mut i = self.index as i32 + delta;
        if i < 0 || i >= n {
            if self.slideshow && self.s.slideshow_loop || self.frame_mode {
                i = i.rem_euclid(n);
            } else {
                i = i.clamp(0, n - 1);
            }
        }
        self.go_to(i as usize, delta.signum());
    }

    fn slideshow_advance(&mut self) {
        let n = self.count();
        if n == 0 {
            return;
        }
        if self.s.slideshow_random && n > 1 {
            let mut i = fastrand::usize(..n);
            if i == self.index {
                i = (i + 1) % n;
            }
            self.go_to(i, 1);
        } else if self.index + 1 >= n && !self.s.slideshow_loop && !self.frame_mode {
            self.set_slideshow(false);
        } else {
            self.index = (self.index + 1) % n;
            self.on_index_changed(1);
        }
    }

    pub fn set_slideshow(&mut self, on: bool) {
        self.slideshow = on;
        self.slide_deadline = None;
        if on {
            if self.grid {
                self.set_grid(false);
            }
            if self.s.slideshow_fullscreen && !self.fullscreen {
                self.set_fullscreen(true);
            }
        }
        platform::keep_display_on(on || self.frame_mode);
    }

    pub fn set_grid(&mut self, grid: bool) {
        self.grid = grid;
        self.grid_scroll_to_current = true;
        if grid {
            self.slideshow = false;
            platform::keep_display_on(self.frame_mode);
        }
        self.pending_transition = None;
    }

    pub fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        if !on && self.frame_mode {
            self.frame_mode = false;
            self.set_slideshow(false);
        }
    }

    // ---------- background results ----------

    fn process_messages(&mut self) {
        while let Ok(msg) = self.loader.rx.try_recv() {
            match msg {
                Msg::Full(path, result) => {
                    let state = match result {
                        Ok(d) => FullState::Ready(Rc::new(self.upload(&path, d))),
                        Err(e) => FullState::Failed(e),
                    };
                    self.full.insert(path.clone(), state);
                    self.full_order.retain(|p| p != &path);
                    self.full_order.push_back(path);
                }
                Msg::Thumb(path, size, result) => {
                    let e = self.thumbs.entry(path.clone()).or_default();
                    e.pending = false;
                    match result {
                        Ok(img) => {
                            e.tex = Some(self.ctx.load_texture(format!("t:{}", path.display()), img, TEX_OPTIONS));
                            e.size = size;
                        }
                        Err(_) => e.failed = true,
                    }
                }
                Msg::Meta(path, m) => {
                    if self.current().is_some_and(|c| c.path == path) {
                        self.meta = Some((path, m));
                    }
                }
            }
        }
        self.trim_caches();
    }

    fn upload(&self, path: &Path, d: Decoded) -> FullImage {
        let name = path.display().to_string();
        let mut textures = Vec::with_capacity(d.frames.len());
        let mut delays = Vec::with_capacity(d.frames.len());
        for (i, f) in d.frames.into_iter().enumerate() {
            textures.push(self.ctx.load_texture(format!("f{i}:{name}"), f.image, TEX_OPTIONS));
            delays.push(f.delay_ms);
        }
        FullImage { textures, delays, width: d.width, height: d.height, format: d.format, vector: d.vector, histogram: d.histogram }
    }

    fn trim_caches(&mut self) {
        let wanted = self.wanted_paths();
        while self.full_order.len() > 6 {
            let Some(pos) = self.full_order.iter().position(|p| !wanted.contains(p)) else { break };
            let p = self.full_order.remove(pos).unwrap();
            self.full.remove(&p);
        }
        if self.thumbs.len() > 3000 {
            let mut ages: Vec<u64> = self.thumbs.values().map(|t| t.last_used).collect();
            ages.sort_unstable();
            let cutoff = ages[ages.len() - 2000];
            self.thumbs.retain(|_, t| t.last_used >= cutoff || t.pending);
        }
    }

    fn wanted_paths(&self) -> Vec<PathBuf> {
        let n = self.count() as i32;
        if n == 0 || self.grid {
            return Vec::new();
        }
        let i = self.index as i32;
        let d = self.nav_dir;
        // Order matters: the loader is LIFO, so the current image goes last.
        let mut out = Vec::new();
        for k in [i - d, i + 2 * d, i + d, i] {
            let k = if self.slideshow || self.frame_mode { k.rem_euclid(n) } else { k };
            if (0..n).contains(&k) {
                if let Some(it) = self.item(k as usize) {
                    if it.kind == Kind::Image && !out.contains(&it.path) {
                        out.push(it.path.clone());
                    }
                }
            }
        }
        out
    }

    fn ensure_loading(&mut self) {
        let wanted = self.wanted_paths();
        let side = self.ctx.input(|i| i.max_texture_side) as u32;
        self.loader.set_max_side(side);
        self.loader.set_wanted(wanted.iter().cloned());
        self.full.retain(|p, s| !matches!(s, FullState::Loading) || wanted.contains(p));
        for p in wanted {
            if !self.full.contains_key(&p) {
                self.full.insert(p.clone(), FullState::Loading);
                self.loader.request_full(p);
            }
        }
    }

    pub fn request_meta(&mut self) {
        self.meta = None;
        if let Some(p) = self.current_path() {
            self.loader.request_meta(p);
        }
    }

    pub fn thumb_for(&mut self, path: &Path) -> Option<&ThumbEntry> {
        let want = self.thumb_px;
        let frame = self.frame_no;
        let e = self.thumbs.entry(path.to_path_buf()).or_default();
        e.last_used = frame;
        let kind = formats::kind_of(path);
        if kind == Some(Kind::Image) && !e.pending && !e.failed && (e.tex.is_none() || e.size + 16 < want) {
            e.pending = true;
            self.loader.request_thumb(path.to_path_buf(), want);
        }
        self.thumbs.get(path)
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        let now = self.ctx.input(|i| i.time);
        self.toast = Some((text.into(), now + 1.6));
    }

    // ---------- commands ----------

    pub fn act(&mut self, a: Action) {
        if !matches!(a, Action::OpenMenu(_)) {
            self.menu = Menu::None;
        }
        match a {
            Action::First => self.go_to(0, -1),
            Action::Last => self.go_to(self.count().saturating_sub(1), 1),
            Action::Prev => self.step(-1),
            Action::Next => self.step(1),
            Action::ToggleSlideshow => self.set_slideshow(!self.slideshow),
            Action::ToggleFullscreen => self.set_fullscreen(!self.fullscreen),
            Action::ToggleGrid => self.set_grid(!self.grid),
            Action::SetGrid(g) => self.set_grid(g),
            Action::SetStrip(s) => {
                self.s.strip = s;
                self.strip_target = None;
                self.strip_index = usize::MAX;
                self.set_grid(false);
            }
            Action::Zoom(m) => {
                self.view.mode = m;
                self.view.pan = Vec2::ZERO;
                if self.grid {
                    self.set_grid(false);
                }
            }
            Action::ZoomIn => self.zoom_by(1.25, None),
            Action::ZoomOut => self.zoom_by(0.8, None),
            Action::RotateCw => self.view.rot = (self.view.rot + 1) % 4,
            Action::RotateCcw => self.view.rot = (self.view.rot + 3) % 4,
            Action::FlipH => self.view.flip = !self.view.flip,
            Action::Info => self.toggle_panel(Panel::Info),
            Action::AllMeta => self.toggle_panel(Panel::Meta),
            Action::Export => {
                if self.multi_selected() {
                    self.s.export.apply = crate::export::ApplyTo::Selected;
                } else if self.s.export.apply == crate::export::ApplyTo::Selected {
                    self.s.export.apply = crate::export::ApplyTo::Current;
                }
                self.toggle_panel(Panel::Export);
            }
            Action::RemoveBgExport => {
                self.s.export.remove_bg = true;
                if self.panel != Panel::Export {
                    self.act(Action::Export);
                }
                if !crate::cutout::available() {
                    self.ask_model = true;
                }
            }
            Action::PickBgModel => {
                self.ask_model = false;
                if let Some(f) = rfd::FileDialog::new().set_title("Файл нейросети для удаления фона").add_filter("Модель", &["safetensors"]).pick_file() {
                    crate::cutout::set_custom(Some(f.clone()));
                    self.s.bg_model = Some(f);
                    self.toast("Нейросеть подключена");
                }
            }
            Action::CopyPrompt => {
                let cached = self.meta.as_ref().filter(|(p, _)| Some(p) == self.current_path().as_ref()).map(|(_, m)| m.prompt.clone());
                if let Some(p) = self.current_path() {
                    match cached.unwrap_or_else(|| crate::meta_full::extract_prompt(&p)) {
                        Some(text) => {
                            self.copy_text(text);
                            self.toast("Промпт скопирован");
                        }
                        None => self.toast("В этом файле нет промпта"),
                    }
                }
            }
            Action::Settings => self.toggle_panel(Panel::Settings),
            Action::Delete => {
                let paths = self.targets();
                if !paths.is_empty() {
                    if self.s.confirm_delete {
                        self.confirm_delete = Some(paths);
                    } else {
                        self.delete_now(paths);
                    }
                }
            }
            Action::CopyPath => {
                let paths = self.targets();
                if !paths.is_empty() {
                    let text: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                    self.copy_text(text.join("\r\n"));
                    self.toast(if paths.len() > 1 { format!("Скопировано путей: {}", paths.len()) } else { "Путь скопирован".into() });
                }
            }
            Action::SelectAll => {
                self.selected = self.visible.iter().map(|&i| self.items[i].path.clone()).collect();
                self.toast(format!("Выделено: {}", self.selected.len()));
            }
            Action::ClearSelection => self.selected.clear(),
            Action::CopyImage => {
                if let Some(p) = self.current_path() {
                    match platform::copy_image(&p) {
                        Ok(()) => self.toast("Изображение скопировано"),
                        Err(e) => self.toast(format!("Не удалось скопировать: {e}")),
                    }
                }
            }
            Action::OpenWith => {
                if let Some(p) = self.current_path() {
                    platform::open_with(&p);
                }
            }
            Action::ShowInExplorer => {
                if let Some(p) = self.current_path() {
                    platform::show_in_explorer(&p);
                }
            }
            Action::OpenExternal => {
                if let Some(p) = self.current_path() {
                    platform::open_default(&p);
                }
            }
            Action::SetWallpaper => {
                if let Some(p) = self.current_path() {
                    match platform::set_wallpaper(&p) {
                        Ok(()) => self.toast("Установлено как фон рабочего стола"),
                        Err(e) => self.toast(format!("Не удалось: {e}")),
                    }
                }
            }
            Action::Flag => {
                let paths: HashSet<PathBuf> = self.targets().into_iter().collect();
                let f = !self.current().is_some_and(|c| c.flagged && paths.contains(&c.path));
                let mut n = 0;
                for it in self.items.iter_mut().filter(|i| paths.contains(&i.path)) {
                    it.flagged = f;
                    n += 1;
                }
                self.s.flagged.retain(|p| !paths.contains(p));
                if f {
                    let mut add: Vec<PathBuf> = paths.into_iter().collect();
                    add.sort();
                    add.extend(self.s.flagged.drain(..));
                    self.s.flagged = add;
                }
                self.s.save();
                if n > 0 {
                    self.toast(match (f, n) {
                        (true, 1) => "Добавлено в избранное (список в «Недавних»)".to_string(),
                        (false, 1) => "Убрано из избранного".to_string(),
                        (true, n) => format!("Добавлено в избранное: {n}"),
                        (false, n) => format!("Убрано из избранного: {n}"),
                    });
                }
            }
            Action::Unflag(p) => {
                self.s.flagged.retain(|x| x != &p);
                self.s.save();
                self.sync_flags();
            }
            Action::ShowFlagged => {
                let files = self.flagged_files();
                if files.is_empty() {
                    self.toast("В избранном пусто");
                } else {
                    self.open_files(files);
                }
            }
            Action::ClearFlags => {
                self.s.flagged.clear();
                self.s.save();
                self.sync_flags();
                self.toast("Избранное очищено");
            }
            Action::CopyFlagged | Action::MoveFlagged => self.transfer(a == Action::MoveFlagged),
            Action::Sort(k) => {
                self.s.sort = k;
                self.resort();
            }
            Action::SortDesc => {
                self.s.sort_desc = !self.s.sort_desc;
                self.resort();
            }
            Action::AddQuickFolder => {
                if let Some(f) = self.folder.clone().or_else(|| self.current_path().and_then(|p| p.parent().map(|x| x.to_path_buf()))) {
                    if !self.s.quick_access.contains(&f) {
                        self.s.quick_access.push(f);
                    }
                    self.toast("Папка добавлена в быстрый доступ");
                }
            }
            Action::OpenPath(p) => self.open_path(&p),
            Action::RemoveQuick(p) => self.s.quick_access.retain(|x| x != &p),
            Action::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Открыть папку").pick_folder() {
                    let has_subdirs = std::fs::read_dir(&dir).is_ok_and(|rd| {
                        rd.flatten().any(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().to_string_lossy().starts_with('.'))
                    });
                    if has_subdirs {
                        self.ask_subfolders = Some(dir);
                    } else {
                        self.open_folder(dir, false);
                    }
                }
            }
            Action::OpenFiles => {
                let mut exts: Vec<&str> = formats::IMAGE_EXT.to_vec();
                exts.extend_from_slice(formats::VIDEO_EXT);
                exts.extend_from_slice(formats::PENDING_EXT);
                if let Some(files) = rfd::FileDialog::new()
                    .set_title("Открыть файлы")
                    .add_filter("Изображения и видео", &exts)
                    .add_filter("Сеанс", &["ivsession"])
                    .add_filter("Все файлы", &["*"])
                    .pick_files()
                {
                    self.open_files(files);
                }
            }
            Action::ToggleToolbar => self.s.toolbar_visible = !self.s.toolbar_visible,
            Action::PhotoFrame => {
                self.frame_mode = !self.frame_mode;
                if self.frame_mode {
                    if self.s.clock == ClockStyle::Off {
                        self.s.clock = ClockStyle::Digital;
                    }
                    self.set_fullscreen(true);
                    self.set_slideshow(true);
                } else {
                    self.set_slideshow(false);
                    self.set_fullscreen(false);
                }
            }
            Action::Reload => {
                if let Some(dir) = self.folder.clone() {
                    let cur = self.current_path();
                    self.load_folder(dir, cur, self.recursive);
                } else if let Some(p) = self.current_path() {
                    self.invalidate(&p);
                }
            }
            Action::ToggleCaption => self.s.show_caption = !self.s.show_caption,
            Action::CycleClock => {
                self.s.clock = match self.s.clock {
                    ClockStyle::Off => ClockStyle::Digital,
                    ClockStyle::Digital => ClockStyle::Analog,
                    ClockStyle::Analog => ClockStyle::Off,
                }
            }
            Action::IncludeSubfolders => {
                self.s.include_subfolders = !self.s.include_subfolders;
                if let Some(dir) = self.folder.clone() {
                    let cur = self.current_path();
                    self.load_folder(dir, cur, self.s.include_subfolders);
                }
            }
            Action::AnimToggle => {
                self.anim_playing = !self.anim_playing;
                self.anim_next = 0.0;
            }
            Action::AnimStep(d) => {
                self.anim_playing = false;
                if let Some(img) = self.current_full() {
                    let n = img.textures.len() as i32;
                    if n > 1 {
                        self.anim_frame = (self.anim_frame as i32 + d).rem_euclid(n) as usize;
                    }
                }
            }
            Action::OpenMenu(m) => {
                self.menu = if self.menu == m { Menu::None } else { m };
                self.menu_just_opened = true;
            }
            Action::Exit => self.ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn toggle_panel(&mut self, p: Panel) {
        self.panel = if self.panel == p { Panel::None } else { p };
        if matches!(self.panel, Panel::Info | Panel::Meta) {
            self.request_meta();
        }
    }

    pub fn current_full(&self) -> Option<Rc<FullImage>> {
        let p = self.current_path()?;
        match self.full.get(&p) {
            Some(FullState::Ready(img)) => Some(img.clone()),
            _ => None,
        }
    }

    /// Files a command acts on: the grid selection when several files are selected,
    /// otherwise the current file.
    pub fn targets(&self) -> Vec<PathBuf> {
        if self.multi_selected() {
            self.visible.iter().map(|&i| &self.items[i].path).filter(|p| self.selected.contains(*p)).cloned().collect()
        } else {
            self.current_path().into_iter().collect()
        }
    }

    pub fn multi_selected(&self) -> bool {
        self.grid && self.selected.len() > 1
    }

    pub fn delete_now(&mut self, paths: Vec<PathBuf>) {
        let gone: HashSet<PathBuf> = paths.iter().cloned().collect();
        // Land on the first file after the deleted ones that survives.
        let next = (self.index..self.count())
            .chain((0..self.index).rev())
            .filter_map(|vi| self.item(vi))
            .find(|it| !gone.contains(&it.path))
            .map(|x| x.path.clone());
        let mut ok = Vec::new();
        let mut err = None;
        for p in &paths {
            match trash::delete(p) {
                Ok(()) => ok.push(p.clone()),
                Err(e) => err = Some(e.to_string()),
            }
        }
        let removed: HashSet<PathBuf> = ok.iter().cloned().collect();
        self.items.retain(|i| !removed.contains(&i.path));
        self.selected.retain(|p| !removed.contains(p));
        for p in &ok {
            self.invalidate(p);
        }
        self.rebuild_visible(next);
        self.on_index_changed(1);
        match (ok.len(), err) {
            (_, Some(e)) => self.toast(format!("Не удалось удалить: {e}")),
            (1, None) => self.toast("Перемещено в корзину"),
            (n, None) => self.toast(format!("В корзину: {n}")),
        }
    }

    fn transfer(&mut self, mv: bool) {
        let use_sel = self.multi_selected();
        let flagged: Vec<PathBuf> = if use_sel { self.targets() } else { self.items.iter().filter(|i| i.flagged).map(|i| i.path.clone()).collect() };
        if flagged.is_empty() {
            self.toast("В избранном пусто (звёздочка у имени файла или P)");
            return;
        }
        let what = if use_sel { "выделенные" } else { "избранные" };
        let title = if mv { format!("Переместить {what} в папку") } else { format!("Копировать {what} в папку") };
        let Some(dest) = rfd::FileDialog::new().set_title(title).pick_folder() else { return };
        let mut ok = 0;
        for src in &flagged {
            let Some(name) = src.file_name() else { continue };
            let target = unique_path(&dest.join(name));
            let res = if mv {
                std::fs::rename(src, &target).or_else(|_| std::fs::copy(src, &target).and_then(|_| std::fs::remove_file(src)))
            } else {
                std::fs::copy(src, &target).map(|_| ())
            };
            if res.is_ok() {
                ok += 1;
            }
        }
        if mv {
            let moved: HashSet<PathBuf> = flagged.into_iter().collect();
            let keep = self.current_path().filter(|p| !moved.contains(p));
            self.items.retain(|i| !moved.contains(&i.path));
            self.selected.retain(|p| !moved.contains(p));
            self.rebuild_visible(keep);
            self.on_index_changed(0);
        }
        self.toast(format!("{} файлов: {ok}", if mv { "Перемещено" } else { "Скопировано" }));
    }

    pub fn zoom_by(&mut self, factor: f32, around: Option<Pos2>) {
        self.pending_zoom = Some((factor, around));
    }
}

impl App {
    pub fn tick(&mut self) {
        self.frame_no += 1;
        self.process_messages();
        self.process_fs_events();
        self.ensure_loading();
        let ppp = self.ctx.pixels_per_point();
        let want = ((self.s.thumb_size * ppp) as u32).div_ceil(32) * 32;
        if want > self.thumb_px {
            self.thumb_px = want;
        }
        let title = match self.current() {
            Some(it) => format!("{} — Angle View", it.name),
            None => "Angle View".to_string(),
        };
        if title != self.title {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        // Slideshow timer: starts counting once the current image is on screen.
        let now = self.ctx.input(|i| i.time);
        if self.slideshow && !self.grid {
            let ready = match self.current_path().and_then(|p| self.full.get(&p)) {
                Some(FullState::Loading) => false,
                _ => self.shown == self.current_path() || self.current().is_some_and(|c| c.kind != Kind::Image),
            };
            match self.slide_deadline {
                None if ready => self.slide_deadline = Some(now + self.s.slideshow_interval as f64),
                Some(t) if now >= t => self.slideshow_advance(),
                Some(t) => self.ctx.request_repaint_after_secs((t - now) as f32),
                None => {}
            }
        }
    }

    pub fn on_close(&mut self) {
        self.s.save();
        self.session().save_to(&Session::autosave_path());
    }
}

fn scan_dir(dir: &Path, recursive: bool, out: &mut Vec<Item>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_dir() {
            if recursive && depth < 32 {
                scan_dir(&path, true, out, depth + 1);
            }
        } else if let Some(kind) = formats::kind_of(&path) {
            let md = e.metadata().ok();
            out.push(Item {
                name: e.file_name().to_string_lossy().into_owned(),
                modified: md.as_ref().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH),
                size: md.map(|m| m.len()).unwrap_or(0),
                kind,
                flagged: false,
                path,
            });
        }
    }
}

fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = p.parent().unwrap_or(Path::new("."));
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).unwrap()
}

/// Compares strings so that "img2" sorts before "img10", ignoring case.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(|c| c.is_ascii_digit()) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}
