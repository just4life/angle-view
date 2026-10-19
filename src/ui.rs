//! Per-frame drawing: toolbar, image viewer, thumbnail grid and overlays.

use crate::app::{Action, App, Drawn, FullState, Menu, Panel, TransitionAnim};
use crate::formats::{self, Kind};
use crate::icons::{self, Icon};
use crate::settings::{ClockStyle, Strip, Transition, WheelAction, ZoomMode};
use crate::style::*;
use eframe::egui::{
    self, epaint::Vertex, pos2, vec2, Align2, Color32, Context, CursorIcon, Event, FontId, Id, Key,
    Mesh, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
    TextureId, Ui, UiBuilder, Vec2,
};

pub const TOOLBAR_H: f32 = 44.0;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Applied when the slider is released, so the panel doesn't jump under the pointer.
        let scale = self.s.ui_scale.clamp(0.8, 2.0);
        if (ctx.zoom_factor() - scale).abs() > 0.001 && !ctx.input(|i| i.pointer.any_down()) {
            ctx.set_zoom_factor(scale);
        }
        self.tick();
        self.handle_dropped_files(&ctx);
        self.handle_keys(&ctx);

        let full = ui.max_rect();
        let now = ctx.input(|i| i.time);
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        if ctx.input(|i| i.pointer.delta() != Vec2::ZERO) {
            self.last_pointer_move = now;
        }
        let [r, g, b] = self.s.background;
        ui.painter().rect_filled(full, 0.0, Color32::from_rgb(r, g, b));

        let docked = self.s.toolbar_visible && !self.fullscreen;
        let near_top = pointer.is_some_and(|p| p.y < full.top() + TOOLBAR_H + 4.0)
            && (now - self.last_pointer_move < 3.0 || docked);
        let toolbar_shown = docked || near_top || self.menu != Menu::None;

        if self.count() == 0 {
            self.empty_ui(ui, full);
        } else if self.grid {
            let top = if docked { TOOLBAR_H } else { 0.0 };
            let rect = Rect::from_min_max(full.min + vec2(0.0, top), full.max);
            self.grid_ui(ui, rect);
        } else {
            let strip = if self.frame_mode { Strip::Off } else { self.s.strip };
            let size = self.s.strip_size.clamp(60.0, 300.0);
            let top = full.top() + if docked { TOOLBAR_H } else { 0.0 };
            let (view, strip_rect) = match strip {
                Strip::Off => (full, None),
                Strip::Bottom => (
                    Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - size)),
                    Some(Rect::from_min_max(pos2(full.left(), full.bottom() - size), full.max)),
                ),
                Strip::Side => (
                    Rect::from_min_max(pos2(full.left() + size, full.top()), full.max),
                    Some(Rect::from_min_max(pos2(full.left(), top), pos2(full.left() + size, full.bottom()))),
                ),
            };
            self.viewer_ui(ui, view, now);
            self.overlays_ui(ui, view, now, pointer);
            if let Some(r) = strip_rect {
                self.strip_ui(ui, r, strip == Strip::Bottom);
            }
        }

        if toolbar_shown {
            let rect = Rect::from_min_size(full.min, vec2(full.width(), TOOLBAR_H));
            self.toolbar_ui(ui, rect);
        }
        self.panels_ui(&ctx, full);
        self.menus_ui(&ctx);
        self.confirm_ui(&ctx);
        self.subfolders_ui(&ctx);
        self.model_ui(&ctx);
        self.text_view_ui(&ctx);
        self.poll_export(now);
        self.task_overlay(ui, full, now);
        self.toast_ui(ui, full, now);

        if self.fullscreen && now - self.last_pointer_move > 2.5 && self.menu == Menu::None && self.panel == Panel::None {
            ctx.set_cursor_icon(CursorIcon::None);
        } else if self.fullscreen {
            ctx.request_repaint_after_secs(2.6);
        }
        if self.s.clock != ClockStyle::Off && !self.grid {
            ctx.request_repaint_after_secs(1.0);
        }
        self.menu_just_opened = false;
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.on_close();
    }
}

impl App {
    fn handle_dropped_files(&mut self, ctx: &Context) {
        let files: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !files.is_empty() {
            self.open_files(files);
        }
    }

    fn handle_keys(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() || self.text_view.is_some() {
            return;
        }
        let keys: Vec<(Key, Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (key, m) in keys {
            if let Some(a) = self.key_action(key, m) {
                self.act(a);
            }
        }
    }

    fn key_action(&mut self, key: Key, m: Modifiers) -> Option<Action> {
        if let Some(dir) = self.ask_subfolders.clone() {
            match key {
                Key::Enter => self.open_folder(dir, true),
                Key::Escape => self.ask_subfolders = None,
                _ => {}
            }
            return None;
        }
        if self.confirm_delete.is_some() {
            match key {
                Key::Enter => {
                    if let Some(p) = self.confirm_delete.take() {
                        self.delete_now(p);
                    }
                }
                Key::Escape => self.confirm_delete = None,
                _ => {}
            }
            return None;
        }
        let ctrl = m.ctrl || m.command;
        if self.grid {
            let cols = self.grid_cols.max(1) as i32;
            let nav = |app: &mut App, d: i32| {
                let n = app.count() as i32;
                let i = (app.index as i32 + d).clamp(0, (n - 1).max(0));
                app.index = i as usize;
                app.grid_scroll_to_current = true;
            };
            match key {
                Key::ArrowLeft => return { nav(self, -1); None },
                Key::ArrowRight => return { nav(self, 1); None },
                Key::ArrowUp => return { nav(self, -cols); None },
                Key::ArrowDown => return { nav(self, cols); None },
                Key::PageUp => return { nav(self, -cols * 4); None },
                Key::PageDown => return { nav(self, cols * 4); None },
                Key::Home => return { nav(self, -(self.count() as i32)); None },
                Key::End => return { nav(self, self.count() as i32); None },
                Key::Enter => {
                    return Some(if self.current().is_some_and(|c| c.kind == Kind::Video) {
                        Action::OpenExternal
                    } else {
                        Action::SetGrid(false)
                    })
                }
                _ => {}
            }
        }
        Some(match key {
            Key::Escape => {
                if self.menu != Menu::None {
                    self.menu = Menu::None;
                } else if self.panel != Panel::None {
                    self.panel = Panel::None;
                } else if self.multi_selected() {
                    self.selected.clear();
                } else if self.frame_mode {
                    return Some(Action::PhotoFrame);
                } else if self.fullscreen {
                    return Some(Action::ToggleFullscreen);
                } else if self.slideshow {
                    return Some(Action::ToggleSlideshow);
                } else if !self.grid {
                    return Some(Action::SetGrid(true));
                }
                return None;
            }
            Key::ArrowLeft | Key::ArrowUp | Key::PageUp | Key::Backspace => Action::Prev,
            Key::ArrowRight | Key::ArrowDown | Key::PageDown => Action::Next,
            Key::Home => Action::First,
            Key::End => Action::Last,
            Key::Enter => {
                if self.current().is_some_and(|c| c.kind == Kind::Video) {
                    Action::OpenExternal
                } else {
                    Action::SetGrid(true)
                }
            }
            Key::Space => Action::ToggleSlideshow,
            Key::F11 | Key::F => Action::ToggleFullscreen,
            Key::G => Action::ToggleGrid,
            Key::Plus | Key::Equals => Action::ZoomIn,
            Key::Minus => Action::ZoomOut,
            Key::Num0 => Action::Zoom(ZoomMode::Fit),
            Key::Num1 => Action::Zoom(ZoomMode::Actual),
            Key::Num2 => Action::Zoom(ZoomMode::Fill),
            Key::Num3 => Action::Zoom(ZoomMode::FitWidth),
            Key::R if m.shift => Action::RotateCcw,
            Key::R => Action::RotateCw,
            Key::L => Action::RotateCcw,
            Key::H => Action::FlipH,
            Key::I => Action::Info,
            Key::M => Action::AllMeta,
            Key::T => Action::ToggleToolbar,
            Key::P => Action::Flag,
            Key::B => Action::RemoveBgExport,
            Key::Delete => Action::Delete,
            Key::C if ctrl && m.shift => Action::CopyPath,
            Key::C if ctrl => Action::CopyImage,
            Key::O if ctrl && m.shift => Action::OpenFolder,
            Key::O if ctrl => Action::OpenFiles,
            Key::S if ctrl => Action::Export,
            Key::S => Action::Settings,
            Key::W if ctrl => Action::Exit,
            Key::F5 => Action::Reload,
            Key::Comma => Action::AnimStep(-1),
            Key::Period => Action::AnimStep(1),
            Key::A if ctrl && self.grid => Action::SelectAll,
            Key::A => Action::AnimToggle,
            Key::K => Action::CycleClock,
            Key::N => Action::ToggleCaption,
            Key::E => Action::ShowInExplorer,
            _ => return None,
        })
    }

    // ---------- toolbar ----------

    fn tb_button(&self, ui: &Ui, x: &mut f32, rect: Rect, icon: Icon, label: &str, right: bool) -> egui::Response {
        let font = FontId::proportional(10.0);
        let w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), TEXT).size().x).max(26.0) + 12.0;
        let r = if right {
            *x -= w;
            Rect::from_min_size(pos2(*x, rect.top() + 2.0), vec2(w, rect.height() - 4.0))
        } else {
            let r = Rect::from_min_size(pos2(*x, rect.top() + 2.0), vec2(w, rect.height() - 4.0));
            *x += w;
            r
        };
        let resp = ui.interact(r, Id::NULL.with(("tb", label)), Sense::click());
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(r, 3.0, HOVER);
        }
        let icon_rect = Rect::from_center_size(pos2(r.center().x, r.top() + 12.0), vec2(19.0, 19.0));
        icons::paint(p, icon_rect, icon, TEXT);
        p.text(pos2(r.center().x, r.bottom() - 2.0), Align2::CENTER_BOTTOM, label, font, TEXT_DIM);
        resp
    }

    fn toolbar_ui(&mut self, ui: &mut Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, TOOLBAR_BG);
        // Swallow clicks on the bar background so they don't reach the image below.
        let _ = ui.interact(rect, Id::NULL.with("toolbar_bg"), Sense::click_and_drag());
        let mut actions = Vec::new();

        let chev = Rect::from_min_size(pos2(rect.left() + 2.0, rect.top() + 2.0), vec2(16.0, 18.0));
        let cr = ui.interact(chev, Id::NULL.with("tb_collapse"), Sense::click());
        if cr.hovered() {
            ui.painter().rect_filled(chev, 2.0, HOVER);
        }
        icons::paint(ui.painter(), chev.shrink(2.0), Icon::ChevronUp, TEXT_DIM);
        if cr.on_hover_text(if self.s.toolbar_visible { "Скрыть панель (T)" } else { "Закрепить панель (T)" }).clicked() {
            actions.push(Action::ToggleToolbar);
        }

        let below = |r: &egui::Response| pos2(r.rect.left(), rect.bottom() + 2.0);
        let mut x = rect.left() + 20.0;
        let r = self.tb_button(ui, &mut x, rect, Icon::More, "Ещё", false);
        if r.clicked() {
            actions.push(Action::OpenMenu(Menu::Context(below(&r))));
        }
        if self.tb_button(ui, &mut x, rect, Icon::Settings, "Параметры", false).clicked() {
            actions.push(Action::Settings);
        }
        if self.tb_button(ui, &mut x, rect, Icon::Folder, "Папка", false).clicked() {
            actions.push(Action::OpenFolder);
        }
        let r = self.tb_button(ui, &mut x, rect, Icon::Star, "Недавние", false);
        if r.clicked() {
            actions.push(Action::OpenMenu(Menu::Folders(below(&r))));
        }
        if self.tb_button(ui, &mut x, rect, Icon::File, "Файлы", false).clicked() {
            actions.push(Action::OpenFiles);
        }
        let (vi, vl) = if self.grid { (Icon::Single, "Один") } else { (Icon::Grid, "Сетка") };
        if self.tb_button(ui, &mut x, rect, vi, vl, false).clicked() {
            actions.push(Action::ToggleGrid);
        }
        if self.grid {
            let r = self.tb_button(ui, &mut x, rect, Icon::Sort, "Сортировка", false);
            if r.clicked() {
                actions.push(Action::OpenMenu(Menu::Sort(below(&r))));
            }
        }

        // Right side, laid out from the right edge.
        let mut rx = rect.right() - 4.0;
        let (fi, fl) = if self.fullscreen { (Icon::ExitFullscreen, "Окно") } else { (Icon::Fullscreen, "Весь экран") };
        if self.tb_button(ui, &mut rx, rect, fi, fl, true).clicked() {
            actions.push(Action::ToggleFullscreen);
        }
        let r = self.tb_button(ui, &mut rx, rect, Icon::Tools, "Сервис", true);
        if r.clicked() {
            actions.push(Action::OpenMenu(Menu::Tools(pos2(r.rect.right() - 382.0, rect.bottom() + 2.0))));
        }
        if self.tb_button(ui, &mut rx, rect, Icon::Last, "Последний", true).clicked() {
            actions.push(Action::Last);
        }
        if self.tb_button(ui, &mut rx, rect, Icon::Next, "Вперёд", true).clicked() {
            actions.push(Action::Next);
        }
        let (pi, pl) = if self.slideshow { (Icon::Pause, "Пауза") } else { (Icon::Play, "Пуск") };
        if self.tb_button(ui, &mut rx, rect, pi, pl, true).clicked() {
            actions.push(Action::ToggleSlideshow);
        }
        if self.tb_button(ui, &mut rx, rect, Icon::Prev, "Назад", true).clicked() {
            actions.push(Action::Prev);
        }
        if self.tb_button(ui, &mut rx, rect, Icon::First, "Первый", true).clicked() {
            actions.push(Action::First);
        }
        let r = self.tb_button(ui, &mut rx, rect, Icon::Zoom, "Масштаб", true);
        if r.clicked() {
            actions.push(Action::OpenMenu(Menu::Zoom(pos2(r.rect.left(), rect.bottom() + 2.0))));
        }

        // File name and position, clipped to the free space between the groups.
        if let Some(it) = self.current() {
            let flagged = it.flagged;
            let counter = format!("{} из {}", self.index + 1, self.count());
            let p = ui.painter().with_clip_rect(Rect::from_min_max(pos2(x + 6.0, rect.top()), pos2(rx - 6.0, rect.bottom())));
            let mut tx = x + 8.0;
            // Star toggle: adds the file to favourites, listed under "Недавние".
            let sr = Rect::from_center_size(pos2(tx + 9.0, rect.center().y), vec2(26.0, 26.0));
            if sr.right() < rx - 6.0 {
                let resp = ui.interact(sr, Id::NULL.with("fav_star"), Sense::click());
                if resp.hovered() {
                    p.rect_filled(sr, 4.0, HOVER);
                }
                if flagged {
                    icons::star_filled(&p, sr.shrink(5.0), ACCENT);
                } else {
                    icons::paint(&p, sr.shrink(4.0), Icon::Star, TEXT_DIM);
                }
                let tip = if flagged { "Убрать из избранного (P)" } else { "В избранное (P)" };
                if resp.on_hover_text(tip).clicked() {
                    actions.push(Action::Flag);
                }
                tx += 26.0;
            }
            let name_g = p.layout_no_wrap(it.name.clone(), FontId::proportional(13.0), TEXT);
            let nw = name_g.size().x;
            p.galley(pos2(tx, rect.center().y - name_g.size().y / 2.0), name_g, TEXT);
            p.text(pos2(tx + 12.0 + nw, rect.center().y), Align2::LEFT_CENTER, counter, FontId::proportional(12.0), TEXT_DIM);
        }
        for a in actions {
            self.act(a);
        }
    }

    // ---------- empty state ----------

    fn empty_ui(&mut self, ui: &mut Ui, full: Rect) {
        let c = full.center();
        let p = ui.painter();
        icons::paint(p, Rect::from_center_size(c - vec2(0.0, 70.0), vec2(56.0, 56.0)), Icon::Picture, TEXT_DIM);
        p.text(c - vec2(0.0, 20.0), Align2::CENTER_CENTER, "Перетащите сюда папку или файлы", FontId::proportional(18.0), TEXT);
        let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(c + vec2(0.0, 30.0), vec2(360.0, 40.0))));
        child.horizontal_centered(|ui| {
            if ui.button("Открыть папку…").clicked() {
                self.act(Action::OpenFolder);
            }
            if ui.button("Открыть файлы…").clicked() {
                self.act(Action::OpenFiles);
            }
        });
    }

    // ---------- thumbnail grid ----------

    fn grid_ui(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let cell = self.s.thumb_size.round();
        // Ctrl + wheel changes the thumbnail size.
        let (wheel, ctrl) = wheel_input(&ctx);
        let hovering = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if ctrl && wheel != 0.0 && hovering && self.menu == Menu::None {
            self.s.thumb_size = (self.s.thumb_size * 1.12f32.powf(wheel)).clamp(64.0, 480.0);
        }

        let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
        child.spacing_mut().item_spacing = Vec2::ZERO;
        let cols = (((rect.width() - 12.0) / cell).floor() as usize).max(1);
        self.grid_cols = cols;
        let n = self.count();
        let rows = n.div_ceil(cols);
        let row_h = cell;

        // The wheel scrolls whole rows of thumbnails (egui's own wheel step is a few pixels).
        let mut area = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("grid")
            .scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: false, ..Default::default() });
        let max_off = (rows as f32 * row_h - rect.height()).max(0.0);
        if !ctrl && wheel != 0.0 && hovering && self.menu == Menu::None {
            let from = self.grid_target.unwrap_or(self.grid_offset);
            self.grid_target = Some((from - wheel * row_h * self.s.grid_wheel_rows).clamp(0.0, max_off));
        }
        if let Some(t) = self.grid_target {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let off = self.grid_offset;
            let next = if (t - off).abs() < 1.0 { t } else { off + (t - off) * (dt * 16.0).min(1.0) };
            area = area.vertical_scroll_offset(next);
            if next == t {
                self.grid_target = None;
            } else {
                ctx.request_repaint();
            }
        }
        if self.grid_scroll_to_current {
            self.grid_target = None;
            self.grid_scroll_to_current = false;
            let row = (self.index / cols) as f32;
            let off = self.grid_offset;
            let view_h = rect.height();
            let top = row * row_h;
            if top < off {
                area = area.vertical_scroll_offset(top);
            } else if top + row_h > off + view_h {
                area = area.vertical_scroll_offset(top + row_h - view_h);
            }
        }
        let mut clicked: Option<(usize, u8)> = None;
        let out = area.show_rows(&mut child, row_h, rows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    for c in 0..cols {
                        let vi = r * cols + c;
                        if vi >= n {
                            break;
                        }
                        if let Some(k) = self.grid_cell(ui, vi, cell) {
                            clicked = Some((vi, k));
                        }
                    }
                });
            }
        });
        self.grid_offset = out.state.clamped_offset().y;
        if let Some((vi, k)) = clicked {
            let m = ctx.input(|i| i.modifiers);
            let path = self.item(vi).map(|it| it.path.clone());
            if let Some(path) = path {
                if k == 1 && (m.ctrl || m.command) {
                    // Ctrl + click adds or removes one file, keeping the rest.
                    if self.selected.is_empty() {
                        if let Some(cur) = self.current_path() {
                            self.selected.insert(cur);
                        }
                    }
                    if !self.selected.remove(&path) {
                        self.selected.insert(path);
                    }
                    self.sel_anchor = vi;
                } else if k == 1 && m.shift {
                    let (a, b) = (self.sel_anchor.min(vi), self.sel_anchor.max(vi));
                    if !(m.ctrl || m.command) {
                        self.selected.clear();
                    }
                    for i in a..=b {
                        if let Some(it) = self.item(i) {
                            self.selected.insert(it.path.clone());
                        }
                    }
                } else if k == 3 && self.selected.contains(&path) {
                    // Right click inside the selection keeps it for the menu.
                } else {
                    self.selected.clear();
                    self.selected.insert(path);
                    self.sel_anchor = vi;
                }
            }
            self.index = vi;
            self.on_index_changed(0);
            match k {
                2 => {
                    if self.current().is_some_and(|c| c.kind == Kind::Video) {
                        self.act(Action::OpenExternal);
                    } else {
                        self.set_grid(false);
                    }
                }
                3 => {
                    let pos = ctx.input(|i| i.pointer.hover_pos()).unwrap_or_default();
                    self.act(Action::OpenMenu(Menu::Context(pos)));
                }
                _ => {}
            }
        }
    }

    /// Returns 1 on click, 2 on double click, 3 on right click.
    fn grid_cell(&mut self, ui: &mut Ui, vi: usize, cell: f32) -> Option<u8> {
        let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell), Sense::click());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        let (path, kind, flagged, name) = {
            let it = self.item(vi)?;
            (it.path.clone(), it.kind, it.flagged, it.name.clone())
        };
        let selected = vi == self.index;
        let picked = self.selected.len() > 1 && self.selected.contains(&path);
        let thumb = self.thumb_for(&path).and_then(|t| t.tex.as_ref().map(|t| (t.id(), t.size_vec2())));
        let p = ui.painter();
        if picked {
            p.rect_filled(rect.shrink(1.0), 4.0, ACCENT.gamma_multiply(0.28));
        } else if resp.hovered() {
            p.rect_filled(rect.shrink(1.0), 3.0, Color32::from_rgb(30, 30, 30));
        }
        let inner = rect.shrink(4.0);
        let shown = match thumb {
            Some((id, size)) => {
                let s = (inner.width() / size.x).min(inner.height() / size.y);
                let r = Rect::from_center_size(inner.center(), size * s);
                p.image(id, r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                r
            }
            None => {
                let r = inner.shrink(inner.width() * 0.12);
                p.rect_filled(r, 2.0, Color32::from_rgb(22, 22, 22));
                let ext = formats::ext_of(&path).to_uppercase();
                p.text(r.center() + vec2(0.0, 18.0), Align2::CENTER_CENTER, ext, FontId::proportional(11.0), TEXT_DIM);
                r
            }
        };
        if kind == Kind::Video {
            let c = shown.center();
            p.circle_filled(c, 15.0, Color32::from_black_alpha(150));
            icons::paint(p, Rect::from_center_size(c + vec2(1.5, 0.0), vec2(16.0, 16.0)), Icon::Play, Color32::WHITE);
        }
        if flagged {
            let fr = Rect::from_min_size(pos2(shown.right() - 20.0, shown.top() + 3.0), vec2(16.0, 16.0));
            p.circle_filled(fr.center(), 10.0, Color32::from_black_alpha(160));
            icons::star_filled(p, fr.shrink(1.0), ACCENT);
        }
        if picked {
            p.rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(2.0, ACCENT), StrokeKind::Inside);
            let c = pos2(rect.left() + 14.0, rect.top() + 14.0);
            p.circle_filled(c, 9.0, ACCENT);
            icons::paint(p, Rect::from_center_size(c, vec2(12.0, 12.0)), Icon::Check, Color32::WHITE);
        }
        if selected {
            p.rect_stroke(shown.expand(1.0), 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
        }
        let resp = resp.on_hover_text_at_pointer(name);
        if resp.double_clicked() {
            Some(2)
        } else if resp.secondary_clicked() {
            Some(3)
        } else if resp.clicked() {
            Some(1)
        } else {
            None
        }
    }

    // ---------- thumbnail strip ----------

    /// A row (bottom) or column (side) of thumbnails that follows the current image.
    fn strip_ui(&mut self, ui: &mut Ui, rect: Rect, horizontal: bool) {
        let ctx = ui.ctx().clone();
        ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(12, 12, 12));
        let cell = if horizontal { rect.height() } else { rect.width() };
        let len = if horizontal { rect.width() } else { rect.height() };
        let n = self.count();
        let max_off = (n as f32 * cell - len).max(0.0);
        // Keep the current image in view when it changes.
        if self.strip_index != self.index {
            let center = (self.index as f32 + 0.5) * cell - len / 2.0;
            let first = self.strip_index == usize::MAX;
            self.strip_index = self.index;
            let lo = self.index as f32 * cell;
            if first {
                self.strip_offset = center.clamp(0.0, max_off);
            } else if lo < self.strip_offset + cell * 0.5 || lo + cell > self.strip_offset + len - cell * 0.5 {
                self.strip_target = Some(center.clamp(0.0, max_off));
            }
        }
        let (wheel, _) = wheel_input(&ctx);
        let hovering = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
        if wheel != 0.0 && hovering && self.menu == Menu::None {
            let from = self.strip_target.unwrap_or(self.strip_offset);
            self.strip_target = Some((from - wheel * cell * 2.0).clamp(0.0, max_off));
        }
        if let Some(t) = self.strip_target {
            let dt = ctx.input(|i| i.stable_dt).min(0.05);
            let o = self.strip_offset;
            self.strip_offset = if (t - o).abs() < 1.0 { t } else { o + (t - o) * (dt * 14.0).min(1.0) };
            if self.strip_offset == t {
                self.strip_target = None;
            } else {
                ctx.request_repaint();
            }
        }
        self.strip_offset = self.strip_offset.clamp(0.0, max_off);
        let first = (self.strip_offset / cell).floor() as usize;
        let last = (((self.strip_offset + len) / cell).ceil() as usize).min(n);
        let p = ui.painter_at(rect);
        let mut go = None;
        for vi in first..last {
            let along = vi as f32 * cell - self.strip_offset;
            let r = if horizontal {
                Rect::from_min_size(pos2(rect.left() + along, rect.top()), vec2(cell, cell))
            } else {
                Rect::from_min_size(pos2(rect.left(), rect.top() + along), vec2(cell, cell))
            };
            let Some((path, name)) = self.item(vi).map(|it| (it.path.clone(), it.name.clone())) else { continue };
            let resp = ui.interact(r.intersect(rect), Id::NULL.with(("strip", vi)), Sense::click());
            let thumb = self.thumb_for(&path).and_then(|t| t.tex.as_ref().map(|t| (t.id(), t.size_vec2())));
            let inner = r.shrink(4.0);
            let shown = match thumb {
                Some((id, size)) => {
                    let s = (inner.width() / size.x).min(inner.height() / size.y);
                    let ir = Rect::from_center_size(inner.center(), size * s);
                    p.image(id, ir, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                    ir
                }
                None => {
                    let ir = inner.shrink(inner.width() * 0.15);
                    p.rect_filled(ir, 2.0, Color32::from_rgb(26, 26, 26));
                    ir
                }
            };
            if vi == self.index {
                p.rect_stroke(shown.expand(1.0), 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
            } else if resp.hovered() {
                p.rect_filled(shown, 0.0, Color32::from_white_alpha(18));
            }
            let resp = resp.on_hover_text_at_pointer(name);
            if resp.clicked() {
                go = Some(vi);
            }
            if resp.secondary_clicked() {
                go = Some(vi);
                let pos = resp.interact_pointer_pos().unwrap_or(r.center());
                self.act(Action::OpenMenu(Menu::Context(pos)));
            }
        }
        if let Some(vi) = go {
            if vi != self.index {
                let dir = if vi > self.index { 1 } else { -1 };
                self.go_to(vi, dir);
            }
        }
    }

    // ---------- single image view ----------

    fn viewer_ui(&mut self, ui: &mut Ui, rect: Rect, now: f64) {
        let ctx = ui.ctx().clone();
        let resp = ui.interact(rect, Id::NULL.with("viewer"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let ppp = ctx.pixels_per_point();
        let Some(path) = self.current_path() else { return };
        let kind = self.current().map(|c| c.kind).unwrap_or(Kind::Image);

        // Mouse: wheel navigates or zooms, drag pans, double click toggles fullscreen.
        let (wheel, ctrl) = wheel_input(&ctx);
        if wheel != 0.0 && resp.hovered() {
            let zoom = (self.s.wheel == WheelAction::Zoom) != ctrl;
            if zoom {
                self.zoom_by(1.2f32.powf(wheel), resp.hover_pos());
            } else {
                self.wheel_acc += wheel;
                while self.wheel_acc >= 1.0 {
                    self.wheel_acc -= 1.0;
                    self.step(-1);
                }
                while self.wheel_acc <= -1.0 {
                    self.wheel_acc += 1.0;
                    self.step(1);
                }
            }
        }
        if resp.dragged_by(PointerButton::Primary) && !self.crop_selecting() {
            self.view.pan += resp.drag_delta();
        }
        // A press that only closes an open menu must not toggle the panel.
        if ctx.input(|i| i.pointer.any_pressed()) {
            self.click_closes_menu = self.menu != Menu::None;
        }
        // Narrow strips along the left and right edges switch to the previous and next file.
        let edge = (rect.width() * 0.1).min(80.0);
        let edge_dir = |p: Pos2| {
            if p.x < rect.left() + edge {
                Some(-1)
            } else if p.x > rect.right() - edge {
                Some(1)
            } else {
                None
            }
        };
        let edge_click = resp.interact_pointer_pos().and_then(edge_dir).filter(|_| self.count() > 1);
        if let (true, Some(d)) = (resp.clicked() && !self.crop_selecting() && !self.click_closes_menu, edge_click) {
            // Every click counts, so fast clicking flips through files instead of going fullscreen.
            self.step(d);
        } else if resp.double_clicked() {
            // The first click of the pair already toggled the panel; undo that.
            self.s.toolbar_visible = !self.s.toolbar_visible;
            self.act(Action::ToggleFullscreen);
        } else if resp.clicked() && !self.crop_selecting() && !self.click_closes_menu {
            // A click on the photo hides or shows the top panel.
            self.s.toolbar_visible = !self.s.toolbar_visible;
        }
        if resp.secondary_clicked() {
            let pos = resp.interact_pointer_pos().unwrap_or(rect.center());
            self.act(Action::OpenMenu(Menu::Context(pos)));
        }
        if resp.middle_clicked() {
            let m = if self.view.mode == ZoomMode::Actual { self.s.default_zoom } else { ZoomMode::Actual };
            self.act(Action::Zoom(m));
        }
        if ctx.input(|i| i.pointer.button_clicked(PointerButton::Extra1)) {
            self.step(-1);
        }
        if ctx.input(|i| i.pointer.button_clicked(PointerButton::Extra2)) {
            self.step(1);
        }
        // The click handlers above may have navigated; re-read the current file.
        let Some(path_now) = self.current_path() else { return };
        if path_now != path {
            ctx.request_repaint();
        }
        let path = path_now;

        let state = match self.full.get(&path) {
            Some(FullState::Ready(img)) => Ok(img.clone()),
            Some(FullState::Failed(e)) => Err(Some(e.clone())),
            _ => Err(None),
        };
        match state {
            Ok(img) => {
                if self.shown.as_ref() != Some(&path) {
                    if let (Some(dir), Some(from)) = (self.pending_transition.take(), self.last_drawn.clone()) {
                        if self.s.transition != Transition::None {
                            self.transition = Some(TransitionAnim { from, start: now, dir });
                        }
                    }
                    self.shown = Some(path.clone());
                    self.anim_next = 0.0;
                }
                // Animation frames.
                let frames = img.textures.len();
                if self.anim_frame >= frames {
                    self.anim_frame = 0;
                }
                if frames > 1 && self.anim_playing {
                    if self.anim_next == 0.0 {
                        self.anim_next = now + img.delays[self.anim_frame] as f64 / 1000.0;
                    } else if now >= self.anim_next {
                        self.anim_frame = (self.anim_frame + 1) % frames;
                        self.anim_next = now + img.delays[self.anim_frame] as f64 / 1000.0;
                    }
                    ctx.request_repaint_after_secs((self.anim_next - now).max(0.0) as f32);
                }
                let (w, h) = if self.view.rot % 2 == 1 { (img.height, img.width) } else { (img.width, img.height) };
                let actual = if img.vector { 1.0 } else { 1.0 / ppp };
                self.layout_view(rect, vec2(w as f32, h as f32), actual);
                let size = vec2(w as f32, h as f32) * self.view.scale;
                let img_rect = Rect::from_center_size(rect.center() + self.view.pan, size);
                let tex = img.textures[self.anim_frame].id();
                let drawn = Drawn { keep: Some(img.clone()), tex, rect: img_rect, rot: self.view.rot, flip: self.view.flip };
                self.paint_with_transition(&painter, &drawn, rect, now);
                self.last_drawn = Some(drawn);
                self.export_overlay(ui, &resp, img_rect, rect, (w, h));
                if resp.dragged() && size.x <= rect.width() + 0.5 && size.y <= rect.height() + 0.5 {
                    ctx.set_cursor_icon(CursorIcon::Default);
                } else if resp.dragged() {
                    ctx.set_cursor_icon(CursorIcon::Grabbing);
                } else if size.x > rect.width() + 0.5 || size.y > rect.height() + 0.5 {
                    if resp.hovered() {
                        ctx.set_cursor_icon(CursorIcon::Grab);
                    }
                }
            }
            Err(err) => {
                // While decoding, show the thumbnail (or the previous image) in place.
                let thumb = self.thumbs.get(&path).and_then(|t| t.tex.as_ref()).map(|t| (t.id(), t.size_vec2()));
                if let (Some((id, size)), None) = (thumb, &err) {
                    let fit = (rect.width() / size.x).min(rect.height() / size.y);
                    let r = Rect::from_center_size(rect.center(), size * fit);
                    paint_texture(&painter, id, r, 0, false, Color32::WHITE);
                } else if let (Some(d), None) = (&self.last_drawn, &err) {
                    paint_texture(&painter, d.tex, d.rect, d.rot, d.flip, Color32::WHITE);
                }
                if err.is_none() && kind == Kind::Image {
                    ctx.request_repaint_after_secs(0.1);
                }
                if let Some(e) = err.or_else(|| {
                    (kind != Kind::Image).then(|| "Этот формат пока открывается только во внешней программе.".to_string())
                }) {
                    let name = self.current().map(|c| c.name.clone()).unwrap_or_default();
                    let c = rect.center();
                    painter.text(c - vec2(0.0, 26.0), Align2::CENTER_CENTER, name, FontId::proportional(16.0), TEXT);
                    painter.text(c, Align2::CENTER_CENTER, e, FontId::proportional(13.0), TEXT_DIM);
                    let mut child = ui.new_child(UiBuilder::new().max_rect(Rect::from_center_size(c + vec2(0.0, 36.0), vec2(260.0, 30.0))));
                    child.vertical_centered(|ui| {
                        if ui.button("Открыть в программе по умолчанию").clicked() {
                            self.act(Action::OpenExternal);
                        }
                    });
                    self.last_drawn = None;
                    if self.shown.as_ref() != Some(&path) {
                        self.shown = Some(path.clone());
                    }
                }
            }
        }
        if let Some(d) = resp.hover_pos().and_then(edge_dir) {
            self.edge_hint(ui, &resp, rect, edge, d);
        }
    }

    /// Hint for the edge strips: a faint shade and a chevron under the pointer.
    fn edge_hint(&self, ui: &Ui, resp: &egui::Response, rect: Rect, edge: f32, dir: i32) {
        if self.count() < 2 || self.crop_selecting() || self.menu != Menu::None || resp.dragged() {
            return;
        }
        let zone = if dir < 0 {
            Rect::from_min_max(rect.left_top(), pos2(rect.left() + edge, rect.bottom()))
        } else {
            Rect::from_min_max(pos2(rect.right() - edge, rect.top()), rect.right_bottom())
        };
        let p = ui.painter_at(rect);
        p.rect_filled(zone, 0.0, Color32::from_white_alpha(10));
        let icon = Rect::from_center_size(zone.center(), vec2(28.0, 28.0));
        icons::paint(&p, icon, if dir < 0 { Icon::ChevronLeft } else { Icon::ChevronRight }, TEXT);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    /// Resolves the zoom mode into a scale and keeps the pan inside sensible bounds.
    fn layout_view(&mut self, area: Rect, size: Vec2, actual: f32) {
        let fit = (area.width() / size.x).min(area.height() / size.y);
        let fill = (area.width() / size.x).max(area.height() / size.y);
        let old = self.view.scale;
        let scale = match self.view.mode {
            ZoomMode::Fit => fit,
            ZoomMode::FitLarger => fit.min(actual),
            ZoomMode::Fill => fill,
            ZoomMode::Actual => actual,
            ZoomMode::FitWidth => area.width() / size.x,
            ZoomMode::FitHeight => area.height() / size.y,
            ZoomMode::Custom => old,
        };
        self.view.scale = scale.max(1e-4);
        if let Some((factor, at)) = self.pending_zoom.take() {
            let min = fit.min(actual) * 0.1;
            let new = (self.view.scale * factor).clamp(min, actual * 64.0);
            let at = at.unwrap_or(area.center());
            let c = area.center() + self.view.pan;
            let c2 = at + (c - at) * (new / self.view.scale);
            self.view.pan = c2 - area.center();
            self.view.scale = new;
            self.view.mode = ZoomMode::Custom;
            self.toast(format!("{:.0}%", new / actual * 100.0));
        }
        let d = size * self.view.scale;
        let max_x = ((d.x - area.width()) / 2.0).max(0.0);
        let max_y = ((d.y - area.height()) / 2.0).max(0.0);
        self.view.pan.x = self.view.pan.x.clamp(-max_x, max_x);
        self.view.pan.y = self.view.pan.y.clamp(-max_y, max_y);
    }

    fn paint_with_transition(&mut self, painter: &egui::Painter, cur: &Drawn, area: Rect, now: f64) {
        let Some(t) = &self.transition else {
            paint_texture(painter, cur.tex, cur.rect, cur.rot, cur.flip, Color32::WHITE);
            return;
        };
        let dur = self.s.transition_ms.max(1) as f64 / 1000.0;
        let p = ((now - t.start) / dur).clamp(0.0, 1.0) as f32;
        let e = p * p * (3.0 - 2.0 * p);
        let from = &t.from;
        match self.s.transition {
            Transition::Slide => {
                let off = area.width() * t.dir;
                paint_texture(painter, from.tex, from.rect.translate(vec2(-off * e, 0.0)), from.rot, from.flip, Color32::WHITE);
                paint_texture(painter, cur.tex, cur.rect.translate(vec2(off * (1.0 - e), 0.0)), cur.rot, cur.flip, Color32::WHITE);
            }
            Transition::Zoom => {
                paint_texture(painter, from.tex, from.rect.scale_from_center(1.0 + 0.06 * e), from.rot, from.flip, Color32::WHITE.gamma_multiply(1.0 - e));
                paint_texture(painter, cur.tex, cur.rect.scale_from_center(0.92 + 0.08 * e), cur.rot, cur.flip, Color32::WHITE.gamma_multiply(e));
            }
            _ => {
                paint_texture(painter, from.tex, from.rect, from.rot, from.flip, Color32::WHITE.gamma_multiply(1.0 - e));
                paint_texture(painter, cur.tex, cur.rect, cur.rot, cur.flip, Color32::WHITE.gamma_multiply(e));
            }
        }
        if p >= 1.0 {
            self.transition = None;
        } else {
            painter.ctx().request_repaint();
        }
    }

    // ---------- overlays ----------

    fn overlays_ui(&mut self, ui: &mut Ui, full: Rect, now: f64, pointer: Option<Pos2>) {
        let p = ui.painter().clone();
        if self.s.show_caption {
            if let Some(it) = self.current() {
                let text = format!("{}   {} / {}", it.name, self.index + 1, self.count());
                shadow_text(&p, full.left_bottom() + vec2(16.0, -16.0), Align2::LEFT_BOTTOM, &text, 15.0);
            }
        }
        if self.s.clock != ClockStyle::Off {
            crate::clock::paint(&p, full, self.s.clock, self.s.show_calendar);
        }
        // Frame controls for animations, revealed by moving the mouse near the bottom edge.
        let frames = self.current_full().map(|i| i.textures.len()).unwrap_or(0);
        if frames > 1 {
            if pointer.is_some_and(|p| p.y > full.bottom() - 110.0) && now - self.last_pointer_move < 0.1 {
                self.bottom_bar_until = now + 2.5;
            }
            if now < self.bottom_bar_until || !self.anim_playing {
                self.anim_bar(ui, full, frames);
                ctx_repaint_at(ui.ctx(), self.bottom_bar_until - now);
            }
        }
    }

    fn anim_bar(&mut self, ui: &mut Ui, full: Rect, frames: usize) {
        let bar = Rect::from_center_size(pos2(full.center().x, full.bottom() - 40.0), vec2(230.0, 40.0));
        let p = ui.painter();
        p.rect_filled(bar, 6.0, Color32::from_black_alpha(190));
        let mut act = None;
        let items = [
            (Icon::StepBack, Action::AnimStep(-1)),
            (if self.anim_playing { Icon::Pause } else { Icon::Play }, Action::AnimToggle),
            (Icon::StepForward, Action::AnimStep(1)),
        ];
        for (k, (icon, a)) in items.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(bar.left() + 8.0 + k as f32 * 38.0, bar.top() + 4.0), vec2(34.0, 32.0));
            let resp = ui.interact(r, Id::NULL.with(("anim", k)), Sense::click());
            if resp.hovered() {
                p.rect_filled(r, 4.0, HOVER);
            }
            icons::paint(p, r.shrink(8.0), icon, TEXT);
            if resp.clicked() {
                act = Some(a);
            }
        }
        p.text(pos2(bar.right() - 14.0, bar.center().y), Align2::RIGHT_CENTER, format!("{} / {frames}", self.anim_frame + 1), FontId::proportional(13.0), TEXT);
        if let Some(a) = act {
            self.act(a);
        }
    }

    fn toast_ui(&mut self, ui: &mut Ui, full: Rect, now: f64) {
        let Some((text, until)) = &self.toast else { return };
        if now > *until {
            self.toast = None;
            return;
        }
        let p = ui.painter();
        let g = p.layout_no_wrap(text.clone(), FontId::proportional(15.0), TEXT);
        let r = Rect::from_center_size(pos2(full.center().x, full.bottom() - 100.0), g.size() + vec2(32.0, 18.0));
        p.rect_filled(r, 8.0, Color32::from_rgba_premultiplied(30, 26, 40, 235));
        p.rect_stroke(r, 8.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.5)), StrokeKind::Inside);
        p.galley(r.center() - g.size() / 2.0, g, TEXT);
        ui.ctx().request_repaint_after_secs((*until - now) as f32);
    }
}

fn ctx_repaint_at(ctx: &Context, secs: f64) {
    if secs > 0.0 {
        ctx.request_repaint_after_secs(secs as f32);
    }
}

/// Sum of wheel notches this frame (positive = up) and whether Ctrl was held.
pub fn wheel_input(ctx: &Context) -> (f32, bool) {
    ctx.input(|i| {
        let mut dy = 0.0;
        let mut ctrl = false;
        for e in &i.events {
            if let Event::MouseWheel { unit, delta, modifiers, .. } = e {
                dy += match unit {
                    MouseWheelUnit::Line => delta.y,
                    MouseWheelUnit::Point => delta.y / 50.0,
                    MouseWheelUnit::Page => delta.y * 3.0,
                };
                ctrl |= modifiers.ctrl || modifiers.command;
            }
        }
        (dy, ctrl || i.modifiers.ctrl)
    })
}

/// Paints a texture into `rect`, rotated by quarter turns and optionally mirrored.
pub fn paint_texture(p: &egui::Painter, tex: TextureId, rect: Rect, rot: u8, flip: bool, tint: Color32) {
    let uvs = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let mut uv: [Pos2; 4] = std::array::from_fn(|i| uvs[(i + 4 - rot as usize % 4) % 4]);
    if flip {
        uv = [uv[1], uv[0], uv[3], uv[2]];
    }
    let mut mesh = Mesh::with_texture(tex);
    for i in 0..4 {
        mesh.vertices.push(Vertex { pos: corners[i], uv: uv[i], color: tint });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    p.add(Shape::mesh(mesh));
}

pub fn shadow_text(p: &egui::Painter, pos: Pos2, align: Align2, text: &str, size: f32) {
    let font = FontId::proportional(size);
    p.text(pos + vec2(1.0, 1.0), align, text, font.clone(), Color32::from_black_alpha(220));
    p.text(pos, align, text, font, Color32::WHITE);
}
