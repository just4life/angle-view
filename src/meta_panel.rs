//! The "all metadata" panel: grouped, filterable tables plus a viewer for long values.

use crate::app::App;
use crate::style::*;
use eframe::egui::{self, vec2, Align2, Context, Id, Order, RichText, Ui};

/// Values longer than this are cut in the table and opened in the text viewer.
const SHORT: usize = 160;

impl App {
    pub fn meta_panel(&mut self, ui: &mut Ui) {
        let Some(path) = self.current_path() else {
            ui.label("Нет файла");
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Фильтр").color(TEXT_DIM));
            ui.add(egui::TextEdit::singleline(&mut self.meta_filter).hint_text("по названию или значению").desired_width(f32::INFINITY));
        });
        ui.add_space(6.0);
        let Some((p, m)) = &self.meta else {
            ui.label(RichText::new("Читаю метаданные…").color(TEXT_DIM));
            return;
        };
        if p != &path {
            ui.label(RichText::new("Читаю метаданные…").color(TEXT_DIM));
            return;
        }
        let filter = self.meta_filter.trim().to_lowercase();
        let matches = |k: &str, v: &str| filter.is_empty() || k.to_lowercase().contains(&filter) || v.to_lowercase().contains(&filter);
        let mut open_text = None;
        let mut copy = None;
        if let Some(prompt) = &m.prompt {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Промпт найден").color(ACCENT));
                if ui.small_button("Копировать промпт").clicked() {
                    copy = Some(prompt.clone());
                }
                if ui.small_button("Открыть").clicked() {
                    open_text = Some(("Промпт".to_string(), prompt.clone()));
                }
            });
            ui.add_space(4.0);
        }
        let value_w = (ui.available_width() - 250.0).max(160.0);
        for (gi, g) in m.groups.iter().enumerate() {
            let shown: usize = g.sections.iter().map(|s| s.rows.iter().filter(|(k, v)| matches(k, v)).count()).sum();
            if shown == 0 {
                continue;
            }
            egui::CollapsingHeader::new(RichText::new(&g.title).size(15.0).strong().color(ACCENT))
                .id_salt(("meta_group", gi))
                .default_open(true)
                .open((!filter.is_empty()).then_some(true))
                .show(ui, |ui| {
                    for (si, sec) in g.sections.iter().enumerate() {
                        let rows: Vec<&(String, String)> = sec.rows.iter().filter(|(k, v)| matches(k, v)).collect();
                        if rows.is_empty() {
                            continue;
                        }
                        let body = |ui: &mut Ui, open_text: &mut Option<(String, String)>| {
                            let grid = egui::Grid::new(("meta_grid", gi, si)).num_columns(2).min_col_width(180.0).spacing([14.0, 4.0]).striped(sec.table);
                            grid.show(ui, |ui| {
                                if sec.table {
                                    ui.label(RichText::new("Id / название").size(11.0).color(TEXT_DIM));
                                    ui.label(RichText::new("Значение").size(11.0).color(TEXT_DIM));
                                    ui.end_row();
                                }
                                for (k, v) in rows {
                                    ui.add(egui::Label::new(RichText::new(k).color(TEXT_DIM)).wrap());
                                    ui.horizontal(|ui| {
                                        ui.set_max_width(value_w);
                                        let long = v.chars().count() > SHORT || v.lines().count() > 3;
                                        let text = if long { format!("{}…", v.chars().take(SHORT).collect::<String>().replace('\n', " ")) } else { v.clone() };
                                        ui.add(egui::Label::new(RichText::new(text).monospace().size(12.0)).wrap());
                                        if long && ui.small_button("Открыть").clicked() {
                                            *open_text = Some((k.clone(), v.clone()));
                                        }
                                    });
                                    ui.end_row();
                                }
                            });
                        };
                        match &sec.title {
                            Some(t) => {
                                egui::CollapsingHeader::new(t).id_salt(("meta_sec", gi, si)).default_open(true).show(ui, |ui| body(ui, &mut open_text));
                            }
                            None => body(ui, &mut open_text),
                        }
                    }
                });
            ui.add_space(4.0);
        }
        if let Some(t) = copy {
            self.copy_text(t);
            self.toast("Промпт скопирован");
        }
        if open_text.is_some() {
            self.text_view = open_text;
        }
    }

    pub fn save_meta_txt(&mut self) {
        let Some((path, m)) = &self.meta else { return };
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(out) = rfd::FileDialog::new()
            .set_title("Сохранить метаданные")
            .add_filter("Текст", &["txt"])
            .set_file_name(format!("{name}.metadata.txt"))
            .save_file()
        else {
            return;
        };
        let mut s = format!("{}\n\n", path.display());
        for g in &m.groups {
            s += &format!("== {} ==\n", g.title);
            for sec in &g.sections {
                if let Some(t) = &sec.title {
                    s += &format!("-- {t} --\n");
                }
                for (k, v) in &sec.rows {
                    s += &format!("{k}: {v}\n");
                }
            }
            s.push('\n');
        }
        match std::fs::write(&out, s) {
            Ok(()) => self.toast("Метаданные сохранены"),
            Err(e) => self.toast(format!("Не удалось сохранить: {e}")),
        }
    }

    /// Modal viewer for a long value (JSON is pretty-printed).
    pub fn text_view_ui(&mut self, ctx: &Context) {
        let Some((title, text)) = &self.text_view else { return };
        let pretty = crate::meta_full::parse_json_lenient(text)
            .filter(|v| v.is_object() || v.is_array())
            .and_then(|v| serde_json::to_string_pretty(&v).ok());
        let shown = pretty.as_deref().unwrap_or(text);
        let screen = ctx.content_rect();
        let size = vec2((screen.width() * 0.7).clamp(360.0, 1100.0), (screen.height() * 0.8).max(240.0));
        let mut close = false;
        let mut action = None;
        egui::Area::new(Id::NULL.with("text_view"))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                dialog_frame().show(ui, |ui| {
                    roomy(ui);
                    ui.set_width(size.x);
                    ui.set_height(size.y);
                    ui.horizontal(|ui| {
                        dialog_title(ui, title);
                        if pretty.is_some() {
                            ui.label(RichText::new("JSON").color(TEXT_DIM));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Закрыть").clicked() {
                                close = true;
                            }
                            if ui.button("Сохранить…").clicked() {
                                action = Some(false);
                            }
                            if ui.add(accent_button("Копировать")).clicked() {
                                action = Some(true);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        let mut s: &str = shown;
                        ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_width(f32::INFINITY));
                    });
                });
            });
        match action {
            Some(true) => {
                self.copy_text(shown.to_string());
                self.toast("Скопировано");
            }
            Some(false) => {
                let ext = if pretty.is_some() { "json" } else { "txt" };
                let name = format!("{}.{ext}", title.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_"));
                if let Some(out) = rfd::FileDialog::new().set_title("Сохранить").set_file_name(name).save_file() {
                    match std::fs::write(&out, shown) {
                        Ok(()) => self.toast("Сохранено"),
                        Err(e) => self.toast(format!("Не удалось сохранить: {e}")),
                    }
                }
            }
            None => {}
        }
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.text_view = None;
        }
    }
}
