//! Export panel: settings, profiles, crop selection on the image, preview and background export.

use crate::app::{Action, App};
use crate::export::{self, ApplyTo, CropMode, Dest, ExportMode, ExportProfile, ExportSettings, Interp, Job, KeepMeta, OutFormat, PngCompression, ResizeBy, ScaleMode};
use crate::formats::Kind;
use crate::style::*;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Response, RichText, Stroke, StrokeKind, Ui};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

const LEFT: u8 = 1;
const TOP: u8 = 2;
const RIGHT: u8 = 4;
const BOTTOM: u8 = 8;
const MOVE: u8 = 16;
const NEW: u8 = 32;

type PreviewResult = Result<(u32, u32, usize, usize), String>;

enum TaskKind {
    Files { in_place: bool },
    Clipboard,
}

struct Task {
    kind: TaskKind,
    /// Shown with the progress bar: "Экспорт" or "Удаление фона".
    label: &'static str,
    started: f64,
    done: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    total: usize,
    rx: Receiver<(Vec<PathBuf>, Vec<String>)>,
}

pub struct ExportUi {
    /// Selected crop in normalized coordinates of the image as shown (after rotation).
    pub crop_sel: Option<[f32; 4]>,
    /// Crop frame being dragged: which edges move (bit mask), frame at drag start, press point.
    crop_drag: Option<(u8, [f32; 4], Pos2)>,
    /// Viewer area and image rectangle from the last frame, for "crop to visible".
    pub view_area: Rect,
    pub img_rect: Rect,
    preview_key: String,
    preview_due: f64,
    preview_running: Option<(String, Receiver<PreviewResult>)>,
    preview: Option<(String, PreviewResult)>,
    task: Option<Task>,
    profile_name: String,
    confirm_until: f64,
}

impl Default for ExportUi {
    fn default() -> Self {
        Self {
            crop_sel: None,
            crop_drag: None,
            view_area: Rect::NOTHING,
            img_rect: Rect::NOTHING,
            preview_key: String::new(),
            preview_due: 0.0,
            preview_running: None,
            preview: None,
            task: None,
            profile_name: String::new(),
            confirm_until: 0.0,
        }
    }
}

impl App {
    fn export_crop_mode(&self) -> Option<CropMode> {
        (self.panel == crate::app::Panel::Export && self.s.export.crop_on).then_some(self.s.export.crop)
    }

    /// True while the left mouse button draws a crop frame instead of panning.
    pub fn crop_selecting(&self) -> bool {
        self.export_crop_mode() == Some(CropMode::Select)
    }

    fn visible_crop(&self) -> Option<[f32; 4]> {
        let (a, r) = (self.export.view_area, self.export.img_rect);
        if r.width() <= 0.0 || r.height() <= 0.0 {
            return None;
        }
        let v = a.intersect(r);
        let n = [(v.left() - r.left()) / r.width(), (v.top() - r.top()) / r.height(), (v.right() - r.left()) / r.width(), (v.bottom() - r.top()) / r.height()];
        (n != [0.0, 0.0, 1.0, 1.0]).then_some(n)
    }

    fn crop_for_jobs(&self) -> Option<[f32; 4]> {
        match self.s.export.crop {
            CropMode::Visible => self.visible_crop(),
            CropMode::Select => self.export.crop_sel,
            CropMode::Ratio => None,
        }
    }

    fn export_jobs(&self) -> Vec<Job> {
        let crop = self.crop_for_jobs();
        let cur = self.current_path();
        let pick = |flagged_only: bool| -> Vec<Job> {
            self.visible
                .iter()
                .map(|&i| &self.items[i])
                .filter(|it| it.kind == Kind::Image && (!flagged_only || it.flagged))
                .map(|it| {
                    let is_cur = Some(&it.path) == cur.as_ref();
                    Job { path: it.path.clone(), rot: if is_cur { self.view.rot } else { 0 }, flip: is_cur && self.view.flip, crop }
                })
                .collect()
        };
        match self.s.export.apply {
            ApplyTo::Current => match self.current() {
                Some(it) if it.kind == Kind::Image => vec![Job { path: it.path.clone(), rot: self.view.rot, flip: self.view.flip, crop }],
                _ => Vec::new(),
            },
            ApplyTo::All => pick(false),
            ApplyTo::Flagged => pick(true),
            ApplyTo::Selected => self.export_jobs_for(&self.targets(), crop),
        }
    }

    fn export_jobs_for(&self, paths: &[PathBuf], crop: Option<[f32; 4]>) -> Vec<Job> {
        let cur = self.current_path();
        paths
            .iter()
            .filter(|p| crate::formats::kind_of(p) == Some(Kind::Image))
            .map(|p| {
                let is_cur = Some(p) == cur.as_ref();
                Job { path: p.clone(), rot: if is_cur { self.view.rot } else { 0 }, flip: is_cur && self.view.flip, crop }
            })
            .collect()
    }

    // ---------- crop overlay in the viewer ----------

    /// Called by the viewer after the image is painted.
    pub fn export_overlay(&mut self, ui: &Ui, resp: &Response, img_rect: Rect, area: Rect, img_px: (u32, u32)) {
        self.export.view_area = area;
        self.export.img_rect = img_rect;
        let Some(mode) = self.export_crop_mode() else { return };
        let to_norm = |p: Pos2| [((p.x - img_rect.left()) / img_rect.width()).clamp(0.0, 1.0), ((p.y - img_rect.top()) / img_rect.height()).clamp(0.0, 1.0)];
        if mode == CropMode::Select {
            // Edges and corners of an existing frame resize it, its inside moves it,
            // a drag elsewhere draws a new frame.
            let screen = |n: [f32; 4]| {
                Rect::from_min_max(
                    pos2(img_rect.left() + n[0] * img_rect.width(), img_rect.top() + n[1] * img_rect.height()),
                    pos2(img_rect.left() + n[2] * img_rect.width(), img_rect.top() + n[3] * img_rect.height()),
                )
            };
            let current = self.export.crop_sel;
            let hit = |p: Pos2| -> u8 {
                let Some(sel) = current else { return NEW };
                let r = screen(sel);
                let g = 9.0;
                if !r.expand(g).contains(p) {
                    return NEW;
                }
                let mut m = 0;
                if (p.x - r.left()).abs() <= g {
                    m |= LEFT;
                } else if (p.x - r.right()).abs() <= g {
                    m |= RIGHT;
                }
                if (p.y - r.top()).abs() <= g {
                    m |= TOP;
                } else if (p.y - r.bottom()).abs() <= g {
                    m |= BOTTOM;
                }
                if m == 0 && r.contains(p) { MOVE } else if m == 0 { NEW } else { m }
            };
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(origin) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                    let sel = self.export.crop_sel.unwrap_or([0.0; 4]);
                    self.export.crop_drag = Some((hit(origin), sel, origin));
                }
            }
            if let (Some((mask, orig, origin)), Some(now)) = (self.export.crop_drag, resp.interact_pointer_pos()) {
                if resp.dragged_by(egui::PointerButton::Primary) {
                    let (a, b) = (to_norm(origin), to_norm(now));
                    let new = if mask == NEW {
                        [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
                    } else if mask == MOVE {
                        let (w, h) = (orig[2] - orig[0], orig[3] - orig[1]);
                        let x = (orig[0] + b[0] - a[0]).clamp(0.0, 1.0 - w);
                        let y = (orig[1] + b[1] - a[1]).clamp(0.0, 1.0 - h);
                        [x, y, x + w, y + h]
                    } else {
                        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                        let mut s = orig;
                        if mask & LEFT != 0 { s[0] = (s[0] + dx).clamp(0.0, 1.0); }
                        if mask & RIGHT != 0 { s[2] = (s[2] + dx).clamp(0.0, 1.0); }
                        if mask & TOP != 0 { s[1] = (s[1] + dy).clamp(0.0, 1.0); }
                        if mask & BOTTOM != 0 { s[3] = (s[3] + dy).clamp(0.0, 1.0); }
                        [s[0].min(s[2]), s[1].min(s[3]), s[0].max(s[2]), s[1].max(s[3])]
                    };
                    if new[2] - new[0] > 0.002 && new[3] - new[1] > 0.002 {
                        self.export.crop_sel = Some(new);
                    } else if mask == NEW {
                        self.export.crop_sel = None;
                    }
                }
            }
            if resp.drag_stopped() {
                self.export.crop_drag = None;
            }
            let mask = match self.export.crop_drag {
                Some((m, _, _)) => Some(m),
                None => resp.hover_pos().map(hit),
            };
            if let Some(m) = mask {
                use egui::CursorIcon as C;
                let icon = match m {
                    MOVE => if self.export.crop_drag.is_some() { C::Grabbing } else { C::Grab },
                    m if m == LEFT | TOP || m == RIGHT | BOTTOM => C::ResizeNwSe,
                    m if m == RIGHT | TOP || m == LEFT | BOTTOM => C::ResizeNeSw,
                    LEFT | RIGHT => C::ResizeHorizontal,
                    TOP | BOTTOM => C::ResizeVertical,
                    _ => C::Crosshair,
                };
                ui.ctx().set_cursor_icon(icon);
            }
        }
        let n = match mode {
            CropMode::Visible => return,
            CropMode::Select => match self.export.crop_sel {
                Some(s) => s,
                None => {
                    let p = ui.painter_at(area);
                    p.text(area.center_bottom() - vec2(0.0, 40.0), Align2::CENTER_CENTER, "Выделите область мышью", FontId::proportional(14.0), TEXT);
                    return;
                }
            },
            CropMode::Ratio => {
                let (x, y, w, h) = export::ratio_rect(img_px.0, img_px.1, self.s.export.ratio);
                let (fw, fh) = (img_px.0.max(1) as f32, img_px.1.max(1) as f32);
                [x as f32 / fw, y as f32 / fh, (x + w) as f32 / fw, (y + h) as f32 / fh]
            }
        };
        let sel = Rect::from_min_max(
            pos2(img_rect.left() + n[0] * img_rect.width(), img_rect.top() + n[1] * img_rect.height()),
            pos2(img_rect.left() + n[2] * img_rect.width(), img_rect.top() + n[3] * img_rect.height()),
        );
        let p = ui.painter_at(area);
        let shade = Color32::from_black_alpha(150);
        let r = img_rect.intersect(area);
        for part in [
            Rect::from_min_max(r.min, pos2(r.right(), sel.top())),
            Rect::from_min_max(pos2(r.left(), sel.bottom()), r.max),
            Rect::from_min_max(pos2(r.left(), sel.top()), pos2(sel.left(), sel.bottom())),
            Rect::from_min_max(pos2(sel.right(), sel.top()), pos2(r.right(), sel.bottom())),
        ] {
            if part.is_positive() {
                p.rect_filled(part, 0.0, shade);
            }
        }
        p.rect_stroke(sel, 0.0, Stroke::new(1.5, ACCENT), StrokeKind::Middle);
        if mode == CropMode::Select {
            // Thirds and handles show the frame can be adjusted.
            let thin = Stroke::new(1.0, Color32::from_white_alpha(60));
            for k in [1.0 / 3.0, 2.0 / 3.0] {
                let x = sel.left() + sel.width() * k;
                let y = sel.top() + sel.height() * k;
                p.line_segment([pos2(x, sel.top()), pos2(x, sel.bottom())], thin);
                p.line_segment([pos2(sel.left(), y), pos2(sel.right(), y)], thin);
            }
            for c in [sel.left_top(), sel.right_top(), sel.left_bottom(), sel.right_bottom(), sel.center_top(), sel.center_bottom(), sel.left_center(), sel.right_center()] {
                let h = Rect::from_center_size(c, vec2(9.0, 9.0));
                p.rect_filled(h, 1.0, Color32::WHITE);
                p.rect_stroke(h, 1.0, Stroke::new(1.0, ACCENT_DARK), StrokeKind::Outside);
            }
        }
        let (pw, ph) = (((n[2] - n[0]) * img_px.0 as f32).round(), ((n[3] - n[1]) * img_px.1 as f32).round());
        crate::ui::shadow_text(&p, sel.left_top() + vec2(4.0, -16.0), Align2::LEFT_CENTER, &format!("{pw:.0} × {ph:.0}"), 12.0);
    }

    // ---------- panel ----------

    pub fn export_panel(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        self.poll_export(now);
        let flagged = self.items.iter().filter(|i| i.flagged).count();
        let out_fmt = self.current_path().map(|p| self.s.export.out_format(&p));
        let jpeg_out = out_fmt == Some(OutFormat::Jpeg);
        let model_ok = crate::cutout::available();
        let mut pick_model = false;
        let total = self.visible.iter().filter(|&&i| self.items[i].kind == Kind::Image).count();
        let picked = if self.multi_selected() { self.selected.len() } else { 0 };
        if picked == 0 && self.s.export.apply == ApplyTo::Selected {
            self.s.export.apply = ApplyTo::Current;
        }
        heading(ui, "Профиль");

        // Profiles.
        ui.horizontal(|ui| {
            let mut load = None;
            let mut delete = None;
            egui::ComboBox::from_id_salt("exp_profile").selected_text("Выбрать…").width(150.0).show_ui(ui, |ui| {
                if self.s.export_profiles.is_empty() {
                    ui.label(RichText::new("Сохранённых профилей нет").color(TEXT_DIM));
                }
                for (k, p) in self.s.export_profiles.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(p.settings == self.s.export, &p.name).clicked() {
                            load = Some(k);
                        }
                        if ui.small_button("×").on_hover_text("Удалить профиль").clicked() {
                            delete = Some(k);
                        }
                    });
                }
            });
            if let Some(k) = load {
                let folder = self.s.export.folder.clone();
                self.s.export = self.s.export_profiles[k].settings.clone();
                self.s.export.folder = folder;
                self.export.profile_name = self.s.export_profiles[k].name.clone();
            }
            if let Some(k) = delete {
                self.s.export_profiles.remove(k);
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.export.profile_name).hint_text("имя профиля").desired_width(170.0));
            if ui.button("Сохранить профиль").clicked() {
                let name = self.export.profile_name.trim().to_string();
                if name.is_empty() {
                    self.toast("Введите имя профиля");
                } else {
                    let settings = self.s.export.clone();
                    match self.s.export_profiles.iter_mut().find(|p| p.name == name) {
                        Some(p) => p.settings = settings,
                        None => self.s.export_profiles.push(ExportProfile { name, settings }),
                    }
                    self.toast("Профиль сохранён");
                }
            }
        });

        let s = &mut self.s.export;
        heading(ui, "Действие");
        segmented(ui, &mut s.mode, &[(ExportMode::Modify, "Изменить", true), (ExportMode::Copy, "Копия", true), (ExportMode::Move, "Переместить", true)], 3);
        let hint = match s.mode {
            ExportMode::Modify => "Основное действие: перезаписать файл (старый уходит в корзину).",
            ExportMode::Copy => "Основное действие: сохранить копию, исходный файл не меняется.",
            ExportMode::Move => "Сохранить в новое место или под новым именем, исходный файл уходит в корзину.",
        };
        ui.label(RichText::new(hint).size(12.0).color(TEXT_DIM));

        heading(ui, "Файлы");
        let mut options = vec![(ApplyTo::Current, "Текущий".to_string(), true)];
        if picked > 0 {
            options.push((ApplyTo::Selected, format!("Выделенные ({picked})"), true));
        }
        options.push((ApplyTo::All, format!("Все ({total})"), true));
        options.push((ApplyTo::Flagged, format!("Избранные ({flagged})"), flagged > 0));
        let opts: Vec<(ApplyTo, &str, bool)> = options.iter().map(|(v, l, on)| (*v, l.as_str(), *on)).collect();
        segmented(ui, &mut s.apply, &opts, 2);

        heading(ui, "Обработка");
        ui.checkbox(&mut s.crop_on, RichText::new("Обрезка").strong());
        if s.crop_on {
            ui.indent("exp_crop", |ui| {
                segmented(ui, &mut s.crop, &[(CropMode::Visible, "Видимое", true), (CropMode::Select, "Выделить", true), (CropMode::Ratio, "Пропорции", true)], 3);
                match s.crop {
                    CropMode::Visible => {
                        ui.label(RichText::new("Сохраняется то, что сейчас видно в окне. Приблизьте и сдвиньте фото.").size(11.0).color(TEXT_DIM));
                    }
                    CropMode::Select => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Выделите область мышью на фото.").size(11.0).color(TEXT_DIM));
                            if self.export.crop_sel.is_some() && ui.small_button("Сбросить").clicked() {
                                self.export.crop_sel = None;
                            }
                        });
                    }
                    CropMode::Ratio => {
                        segmented(ui, &mut s.ratio, &[((1, 1), "1:1", true), ((4, 3), "4:3", true), ((3, 4), "3:4", true), ((16, 9), "16:9", true), ((9, 16), "9:16", true)], 5);
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut s.ratio.0).range(1..=100));
                            ui.label(":");
                            ui.add(egui::DragValue::new(&mut s.ratio.1).range(1..=100));
                            if ui.button("Поменять").clicked() {
                                s.ratio = (s.ratio.1, s.ratio.0);
                            }
                        });
                    }
                }
            });
        }

        ui.add_space(4.0);
        ui.checkbox(&mut s.resize_on, RichText::new("Изменить размер").strong());
        if s.resize_on {
            ui.indent("exp_resize", |ui| {
                segmented(ui, &mut s.resize_by, &[(ResizeBy::Pixels, "В пикселях", true), (ResizeBy::Scale, "В процентах", true)], 2);
                if s.resize_by == ResizeBy::Scale {
                    ui.add(egui::Slider::new(&mut s.scale_pct, 1.0..=400.0).suffix(" %").logarithmic(true));
                } else {
                    let mut remove = None;
                    let n = s.sizes.len();
                    for (k, (w, h)) in s.sizes.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Ширина").color(TEXT_DIM));
                            ui.add(size_value(w));
                            ui.label(RichText::new("Высота").color(TEXT_DIM));
                            ui.add(size_value(h));
                            if n > 1 && ui.small_button("×").clicked() {
                                remove = Some(k);
                            }
                        });
                    }
                    if let Some(k) = remove {
                        s.sizes.remove(k);
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("+ Добавить размер").clicked() {
                            s.sizes.push((0, 0));
                        }
                        ui.label(RichText::new("0 = авто").size(11.0).color(TEXT_DIM));
                    });
                }
                egui::Grid::new("exp_scale").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label(RichText::new("Режим").color(TEXT_DIM));
                    egui::ComboBox::from_id_salt("exp_mode").selected_text(s.scale_mode.label()).width(190.0).show_ui(ui, |ui| {
                        for m in ScaleMode::ALL {
                            ui.selectable_value(&mut s.scale_mode, m, m.label());
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new("Интерполяция").color(TEXT_DIM));
                    egui::ComboBox::from_id_salt("exp_interp").selected_text(s.interp.label()).width(190.0).show_ui(ui, |ui| {
                        for m in Interp::ALL {
                            ui.selectable_value(&mut s.interp, m, m.label());
                        }
                    });
                    ui.end_row();
                });
            });
        }

        ui.add_space(4.0);
        ui.checkbox(&mut s.remove_bg, RichText::new("Удалить фон").strong());
        if s.remove_bg {
            ui.indent("exp_nobg", |ui| {
                ui.label(RichText::new("Нейросеть находит главный объект и убирает фон, края и волосы уточняются по фото. Около 5–10 с на фото.").size(11.0).color(TEXT_DIM));
                const WHITE: [u8; 3] = [255, 255, 255];
                const BLACK: [u8; 3] = [0, 0, 0];
                let mut fill: u8 = match (s.bg_on, s.bg) {
                    (false, _) => 0,
                    (true, WHITE) => 1,
                    (true, BLACK) => 2,
                    _ => 3,
                };
                ui.label(RichText::new("Фон в результате").color(TEXT_DIM));
                if segmented(ui, &mut fill, &[(0, "Прозрачный", true), (1, "Белый", true), (2, "Чёрный", true), (3, "Свой цвет", true)], 2) {
                    s.bg_on = fill != 0;
                    match fill {
                        1 => s.bg = WHITE,
                        2 => s.bg = BLACK,
                        3 if s.bg == WHITE || s.bg == BLACK => s.bg = [0, 177, 64],
                        _ => {}
                    }
                }
                if fill == 3 {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Цвет фона").color(TEXT_DIM));
                        ui.color_edit_button_srgb(&mut s.bg);
                    });
                }
                if fill == 0 && matches!(out_fmt, Some(OutFormat::Jpeg | OutFormat::Bmp)) {
                    ui.label(RichText::new("Этот формат не хранит прозрачность, фон будет белым. Для прозрачного выберите PNG или WebP.").size(11.0).color(ACCENT));
                }
                if !model_ok {
                    ui.label(RichText::new(format!("Не найден файл нейросети {}.", crate::cutout::MODEL_FILE)).size(12.0).color(ACCENT));
                    if ui.button("Указать файл нейросети…").clicked() {
                        pick_model = true;
                    }
                }
            });
        }

        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new("Параметры кодирования").strong()).id_salt("exp_enc").show(ui, |ui| {
            egui::Grid::new("exp_enc_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new("Метаданные").color(TEXT_DIM));
                egui::ComboBox::from_id_salt("exp_meta").selected_text(s.keep_meta.label()).width(190.0).show_ui(ui, |ui| {
                    for m in KeepMeta::ALL {
                        ui.selectable_value(&mut s.keep_meta, m, m.label());
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Сжатие PNG").color(TEXT_DIM));
                egui::ComboBox::from_id_salt("exp_png").selected_text(s.png.label()).width(190.0).show_ui(ui, |ui| {
                    for m in PngCompression::ALL {
                        ui.selectable_value(&mut s.png, m, m.label());
                    }
                });
                ui.end_row();
            });
            ui.label(RichText::new("«Все» сохраняет EXIF, цветовой профиль и текстовые блоки PNG (prompt, workflow).").size(11.0).color(TEXT_DIM));
            ui.horizontal(|ui| {
                ui.checkbox(&mut s.bg_on, "Фон под прозрачные пиксели");
                if s.bg_on {
                    ui.color_edit_button_srgb(&mut s.bg);
                }
            });
            ui.checkbox(&mut s.prefer_alpha, "Предпочитать формат пикселей с прозрачностью");
            ui.checkbox(&mut s.dither, "Сглаживание (дизеринг) для GIF");
        });
        egui::CollapsingHeader::new(RichText::new("Дополнительно").strong()).id_salt("exp_adv").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Имя файла").color(TEXT_DIM));
                ui.add(egui::TextEdit::singleline(&mut s.name_template).desired_width(180.0));
            });
            ui.label(RichText::new("{name} имя исходника, {w} {h} размер, {date} дата").size(11.0).color(TEXT_DIM));
        });

        heading(ui, "Сохранение");
        ui.checkbox(&mut s.open_after, "Открыть после экспорта");
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.reuse_folder, "Сохранять в ту же папку без вопроса");
        });
        if s.reuse_folder {
            match s.folder.clone() {
                Some(f) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(f.display().to_string()).size(11.0).color(TEXT_DIM));
                        if ui.small_button("Сменить").clicked() {
                            s.folder = None;
                        }
                    });
                }
                None => {
                    ui.label(RichText::new("Папка будет выбрана при первом экспорте.").size(11.0).color(TEXT_DIM));
                }
            }
        }

        heading(ui, "Формат");
        egui::Grid::new("exp_fmt_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Формат файла").color(TEXT_DIM));
            egui::ComboBox::from_id_salt("exp_fmt").selected_text(s.format.label()).width(190.0).show_ui(ui, |ui| {
                for f in OutFormat::ALL {
                    ui.selectable_value(&mut s.format, f, f.label());
                }
            });
            ui.end_row();
            if jpeg_out || s.format == OutFormat::Jpeg {
                ui.label(RichText::new("Качество JPEG").color(TEXT_DIM));
                ui.add(egui::Slider::new(&mut s.jpeg_quality, 40..=100));
                ui.end_row();
            }
        });

        if pick_model {
            self.act(Action::PickBgModel);
        }

        // Preview.
        ui.add_space(10.0);
        self.update_preview(now);
        let line = match &self.export.preview {
            Some((k, Ok((w, h, bytes, count)))) if *k == self.export.preview_key => {
                let more = if *count > 1 { format!(", и ещё {} размер(а)", count - 1) } else { String::new() };
                format!("Результат: {w} × {h}, {}{more}", crate::meta::human_size(*bytes as u64))
            }
            Some((k, Err(e))) if *k == self.export.preview_key => format!("Не получится: {e}"),
            _ if self.current().is_some_and(|c| c.kind != Kind::Image) => "Этот файл экспортировать нельзя".into(),
            _ => "Считаю результат…".into(),
        };
        ui.label(RichText::new(line).size(16.0).strong().color(TEXT));
        if self.s.export.apply != ApplyTo::Current {
            ui.label(RichText::new("Расчёт показан для текущего файла.").size(11.0).color(TEXT_DIM));
        }
        if self.s.export.remove_bg {
            ui.label(RichText::new("Размер файла посчитан без удаления фона.").size(11.0).color(TEXT_DIM));
        }

        ui.add_space(8.0);
        if let Some(t) = &self.export.task {
            let done = t.done.load(Ordering::Relaxed);
            let secs = (now - t.started).max(0.0) as u32;
            ui.label(RichText::new(format!("{}: {done} из {}, {secs} с", t.label, t.total)).color(TEXT));
            ui.add(egui::ProgressBar::new(done as f32 / t.total.max(1) as f32).animate(true));
            if t.total > 1 && ui.add_enabled(!t.cancel.load(Ordering::Relaxed), egui::Button::new("Остановить")).clicked() {
                t.cancel.store(true, Ordering::Relaxed);
            }
            ui.ctx().request_repaint_after_secs(0.2);
            return;
        }
        let jobs = self.export_jobs().len();
        let batch = jobs > 1;
        let confirming = now < self.export.confirm_until;
        let mut actions: Vec<u8> = Vec::new();
        let modify = self.s.export.mode == ExportMode::Modify;
        let over = if confirming { format!("Точно перезаписать {jobs}?") } else { "Перезаписать".to_string() };
        let as_label = if batch { format!("Экспортировать {jobs} {}…", files_word(jobs)) } else { "Экспортировать как…".to_string() };
        let w = ui.available_width();
        let can_clip = self.current().is_some_and(|c| c.kind == Kind::Image);
        ui.add_enabled_ui(jobs > 0, |ui| {
            // The main action for the chosen mode comes first and is filled with the accent.
            let order: [(u8, &str, &str); 2] = if modify { [(1, &over, ""), (0, &as_label, "")] } else { [(0, &as_label, ""), (1, &over, "")] };
            for (k, (code, label, hint)) in order.into_iter().enumerate() {
                if crate::panels::big_button(ui, label, hint, k == 0, w).clicked() {
                    actions.push(code);
                }
                ui.add_space(2.0);
            }
        });
        ui.add_enabled_ui(can_clip, |ui| {
            if crate::panels::big_button(ui, "В буфер обмена", "текущий файл", false, w).clicked() {
                actions.push(2);
            }
        });
        if jobs == 0 {
            ui.label(RichText::new("Нет файлов для экспорта.").size(11.0).color(TEXT_DIM));
        }
        for a in actions {
            match a {
                0 => self.export_as(),
                1 if batch && !confirming => self.export.confirm_until = now + 4.0,
                1 => {
                    self.export.confirm_until = 0.0;
                    self.start_export(Dest::InPlace, self.export_jobs());
                }
                _ => self.export_clipboard(),
            }
        }
    }

    fn update_preview(&mut self, now: f64) {
        let Some(cur) = self.current().filter(|c| c.kind == Kind::Image).map(|c| c.path.clone()) else { return };
        let job = Job { path: cur.clone(), rot: self.view.rot, flip: self.view.flip, crop: self.crop_for_jobs() };
        let key = format!("{}|{}|{}|{}|{:?}", cur.display(), serde_json::to_string(&self.s.export).unwrap_or_default(), job.rot, job.flip, job.crop);
        if key != self.export.preview_key {
            self.export.preview_key = key.clone();
            self.export.preview_due = now + 0.35;
        }
        if let Some((k, rx)) = &self.export.preview_running {
            if let Ok(r) = rx.try_recv() {
                self.export.preview = Some((k.clone(), r));
                self.export.preview_running = None;
            } else {
                self.ctx.request_repaint_after_secs(0.1);
                return;
            }
        }
        let fresh = self.export.preview.as_ref().is_some_and(|(k, _)| *k == key);
        if fresh {
            return;
        }
        if now < self.export.preview_due {
            self.ctx.request_repaint_after_secs((self.export.preview_due - now) as f32 + 0.01);
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        // The size estimate skips the neural network: it takes seconds per photo.
        let settings = ExportSettings { remove_bg: false, ..self.s.export.clone() };
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| export::render(&job, &settings))
                .unwrap_or_else(|_| Err("ошибка при обработке".into()))
                .and_then(|v| v.first().map(|e| (e.width, e.height, e.bytes.len(), v.len())).ok_or_else(|| "нет результата".into()));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.export.preview_running = Some((key, rx));
    }

    fn export_as(&mut self) {
        let jobs = self.export_jobs();
        let Some(first) = jobs.first() else { return };
        let s = &self.s.export;
        let dest = if let (true, Some(f)) = (s.reuse_folder, s.folder.clone().filter(|f| f.is_dir())) {
            Dest::Folder(f)
        } else if jobs.len() == 1 {
            let ext = s.out_format(&first.path).ext();
            let stem = first.path.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let name = format!("{}.{ext}", s.name_template.replace("{name}", &stem).replace("{w}", "").replace("{h}", "").replace("{date}", &chrono::Local::now().format("%Y-%m-%d").to_string()));
            let dir = s.folder.clone().filter(|f| f.is_dir()).or_else(|| first.path.parent().map(|p| p.to_path_buf()));
            let mut dlg = rfd::FileDialog::new().set_title("Экспортировать как").set_file_name(name).add_filter(ext.to_uppercase(), &[ext]);
            if let Some(d) = dir {
                dlg = dlg.set_directory(d);
            }
            let Some(f) = dlg.save_file() else { return };
            self.s.export.folder = f.parent().map(|p| p.to_path_buf());
            Dest::File(f)
        } else {
            let Some(d) = rfd::FileDialog::new().set_title(format!("Папка для {} файлов", jobs.len())).pick_folder() else { return };
            self.s.export.folder = Some(d.clone());
            Dest::Folder(d)
        };
        self.start_export(dest, jobs);
    }

    fn start_export(&mut self, dest: Dest, jobs: Vec<Job>) {
        let label = if self.s.export.remove_bg { "Удаление фона" } else { "Экспорт" };
        if jobs.is_empty() {
            return;
        }
        let in_place = matches!(dest, Dest::InPlace);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let done = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let total = jobs.len();
        let (settings, ctx, d, c) = (self.s.export.clone(), self.ctx.clone(), done.clone(), cancel.clone());
        std::thread::spawn(move || run_batch(jobs, settings, dest, d, c, tx, ctx));
        let started = self.ctx.input(|i| i.time);
        self.export.task = Some(Task { kind: TaskKind::Files { in_place }, label, started, done, cancel, total, rx });
    }

    /// Progress card shown while a task runs and the export panel is closed.
    pub fn task_overlay(&mut self, ui: &mut Ui, full: Rect, now: f64) {
        if self.panel == crate::app::Panel::Export {
            return;
        }
        let Some(t) = &self.export.task else { return };
        let done = t.done.load(Ordering::Relaxed);
        let secs = (now - t.started).max(0.0) as u32;
        let text = if t.total > 1 { format!("{}: {done} из {}, {secs} с", t.label, t.total) } else { format!("{}… {secs} с", t.label) };
        let r = Rect::from_center_size(pos2(full.center().x, full.bottom() - 160.0), vec2(360.0, if t.total > 1 { 92.0 } else { 62.0 }));
        let mut stop = false;
        egui::Area::new(egui::Id::NULL.with("task_overlay")).fixed_pos(r.min).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_premultiplied(30, 26, 40, 240))
                .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.5)))
                .corner_radius(10.0)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.set_width(r.width() - 28.0);
                    ui.label(RichText::new(text).size(15.0).color(TEXT));
                    ui.add(egui::ProgressBar::new(done as f32 / t.total.max(1) as f32).desired_height(6.0).animate(true));
                    if t.total > 1 {
                        stop = ui.add_enabled(!t.cancel.load(Ordering::Relaxed), egui::Button::new("Остановить")).clicked();
                    }
                });
        });
        if stop {
            t.cancel.store(true, Ordering::Relaxed);
        }
        ui.ctx().request_repaint_after_secs(0.25);
    }

    fn export_clipboard(&mut self) {
        let Some(job) = self.export_jobs().into_iter().find(|j| Some(&j.path) == self.current_path().as_ref()).or_else(|| {
            let it = self.current()?;
            Some(Job { path: it.path.clone(), rot: self.view.rot, flip: self.view.flip, crop: self.crop_for_jobs() })
        }) else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let settings = self.s.export.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| -> Result<(), String> {
                let out = export::render(&job, &settings)?;
                let e = out.first().ok_or("нет результата")?;
                let rgba = e.pixels.to_rgba8();
                let mut cb = arboard::Clipboard::new().map_err(|x| x.to_string())?;
                cb.set_image(arboard::ImageData { width: rgba.width() as usize, height: rgba.height() as usize, bytes: std::borrow::Cow::Owned(rgba.into_raw()) })
                    .map_err(|x| x.to_string())
            })
            .unwrap_or_else(|_| Err("ошибка при обработке".into()));
            let _ = tx.send((Vec::new(), r.err().into_iter().collect()));
            ctx.request_repaint();
        });
        let started = self.ctx.input(|i| i.time);
        let label = if self.s.export.remove_bg { "Удаление фона" } else { "Копирование" };
        self.export.task = Some(Task { kind: TaskKind::Clipboard, label, started, done: Arc::new(AtomicUsize::new(0)), cancel: Arc::new(AtomicBool::new(false)), total: 1, rx });
    }

    /// Picks up a finished export; also called every frame from the UI so it completes with the panel closed.
    pub fn poll_export(&mut self, _now: f64) {
        let Some(t) = &self.export.task else { return };
        let Ok((written, errors)) = t.rx.try_recv() else { return };
        let task = self.export.task.take().unwrap();
        match task.kind {
            TaskKind::Clipboard => {
                if errors.is_empty() {
                    self.toast("Результат скопирован в буфер обмена");
                } else {
                    self.toast(format!("Не удалось: {}", errors[0]));
                }
                return;
            }
            TaskKind::Files { in_place } => {
                let stopped = if task.cancel.load(Ordering::Relaxed) { ", остановлено" } else { "" };
                let msg = match (written.as_slice(), errors.len()) {
                    ([one], 0) if task.label == "Удаление фона" => {
                        format!("Фон удалён: {}", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                    }
                    (w, 0) => format!("Готово: {} {}{stopped}", w.len(), files_word(w.len())),
                    (w, e) => format!("Готово: {}, ошибок: {e}{stopped}. {}", w.len(), errors[0]),
                };
                self.toast(msg);
                let moved = self.s.export.mode == ExportMode::Move;
                if in_place || moved {
                    for p in &written {
                        self.invalidate(p);
                    }
                    self.act(Action::Reload);
                }
                if self.s.export.open_after {
                    if written.len() == 1 {
                        self.open_path(&written[0]);
                    } else if let Some(p) = written.first() {
                        crate::platform::show_in_explorer(p);
                    }
                }
            }
        }
    }
}

fn run_batch(jobs: Vec<Job>, s: ExportSettings, dest: Dest, done: Arc<AtomicUsize>, cancel: Arc<AtomicBool>, tx: Sender<(Vec<PathBuf>, Vec<String>)>, ctx: egui::Context) {
    let mut written = Vec::new();
    let mut errors = Vec::new();
    for job in &jobs {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export::export(job, &s, &dest))).unwrap_or_else(|_| Err("ошибка при обработке".into()));
        match r {
            Ok(w) => written.extend(w),
            Err(e) => errors.push(format!("{}: {e}", job.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
        }
        done.fetch_add(1, Ordering::Relaxed);
        ctx.request_repaint();
    }
    let _ = tx.send((written, errors));
    ctx.request_repaint();
}

fn files_word(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (1, x) if x != 11 => "файл",
        (2..=4, x) if !(12..=14).contains(&x) => "файла",
        _ => "файлов",
    }
}

fn size_value(v: &mut u32) -> egui::DragValue<'_> {
    egui::DragValue::new(v)
        .range(0..=65535)
        .speed(4.0)
        .custom_formatter(|x, _| if x == 0.0 { "Авто".into() } else { format!("{x:.0}") })
        .custom_parser(|t| {
            let t = t.trim().to_lowercase();
            if t.is_empty() || t.starts_with('а') || t.starts_with('a') { Some(0.0) } else { t.parse().ok() }
        })
}
