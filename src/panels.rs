//! Popup menus (context, tools, zoom, folders), side panels (settings, inspector) and dialogs.

use crate::app::{Action, App, Menu, Panel};
use crate::icons::{self, Icon};
use crate::settings::{ClockStyle, SortKey, Strip, Transition, WheelAction, ZoomMode};
use crate::style::*;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Context, FontId, Id, Order, Rect, RichText, Sense, Stroke, Ui,
};

const MENU_W: f32 = 380.0;

struct MenuBuilder<'a> {
    ui: &'a mut Ui,
    action: Option<Action>,
    n: usize,
}

impl MenuBuilder<'_> {
    fn sep(&mut self) {
        let (r, _) = self.ui.allocate_exact_size(vec2(MENU_W, 2.0), Sense::hover());
        self.ui.painter().rect_filled(r, 0.0, MENU_SEP);
    }

    fn row(&mut self, icon: Option<Icon>, items: &[(&str, Action, bool)], hint: Option<&str>) {
        let (row, _) = self.ui.allocate_exact_size(vec2(MENU_W, 32.0), Sense::hover());
        let p = self.ui.painter().clone();
        if let Some(icon) = icon {
            icons::paint(&p, Rect::from_center_size(pos2(row.left() + 20.0, row.center().y), vec2(17.0, 17.0)), icon, TEXT);
        }
        let mut x = row.left() + 38.0;
        let font = FontId::proportional(14.0);
        for (label, action, checked) in items {
            let g = p.layout_no_wrap(label.to_string(), font.clone(), TEXT);
            let w = g.size().x + 14.0 + if *checked { 18.0 } else { 0.0 };
            let r = Rect::from_min_size(pos2(x, row.top() + 3.0), vec2(w, row.height() - 6.0));
            self.n += 1;
            let resp = self.ui.interact(r, Id::NULL.with(("menu_item", self.n)), Sense::click());
            if resp.hovered() {
                p.rect_filled(r, 3.0, HOVER);
            }
            if *checked {
                p.rect_filled(r, 4.0, ACCENT_DARK.gamma_multiply(0.45));
            }
            let color = if *checked { Color32::WHITE } else { TEXT };
            p.galley(pos2(r.left() + 7.0, r.center().y - g.size().y / 2.0), g.clone(), color);
            if *checked {
                icons::paint(&p, Rect::from_center_size(pos2(r.right() - 12.0, r.center().y), vec2(13.0, 13.0)), Icon::Check, ACCENT);
            }
            if resp.clicked() {
                self.action = Some(action.clone());
            }
            x += w + 2.0;
        }
        if let Some(hint) = hint {
            p.text(pos2(x + 4.0, row.center().y + 1.0), Align2::LEFT_CENTER, hint, FontId::proportional(11.0), TEXT_DIM);
        }
    }

    fn icon_row(&mut self, items: &[(Icon, Action, &str)], right: &[(Icon, Action, &str)]) {
        let (row, _) = self.ui.allocate_exact_size(vec2(MENU_W, 36.0), Sense::hover());
        let p = self.ui.painter().clone();
        let place = |x: f32, icon: Icon, action: &Action, tip: &str, me: &mut Self| {
            let r = Rect::from_min_size(pos2(x, row.top() + 3.0), vec2(34.0, 30.0));
            me.n += 1;
            let resp = me.ui.interact(r, Id::NULL.with(("menu_icon", me.n)), Sense::click());
            if resp.hovered() {
                p.rect_filled(r, 3.0, HOVER);
            }
            icons::paint(&p, r.shrink(7.0), icon, TEXT);
            if resp.on_hover_text(tip).clicked() {
                me.action = Some(action.clone());
            }
        };
        for (k, (icon, a, tip)) in items.iter().enumerate() {
            place(row.left() + 34.0 + k as f32 * 36.0, *icon, a, tip, self);
        }
        for (k, (icon, a, tip)) in right.iter().enumerate() {
            place(row.right() - 40.0 - k as f32 * 36.0, *icon, a, tip, self);
        }
    }

    fn label(&mut self, text: &str) {
        let (r, _) = self.ui.allocate_exact_size(vec2(MENU_W, 24.0), Sense::hover());
        self.ui.painter().text(pos2(r.left() + 14.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(12.0), ACCENT);
    }
}

fn menu_kind(m: &Menu) -> u8 {
    match m {
        Menu::None => 0,
        Menu::Context(_) => 1,
        Menu::Tools(_) => 2,
        Menu::Zoom(_) => 3,
        Menu::Folders(_) => 4,
        Menu::Sort(_) => 5,
    }
}

impl App {
    pub fn menus_ui(&mut self, ctx: &Context) {
        let pos = match &self.menu {
            Menu::None => return,
            Menu::Context(p) | Menu::Tools(p) | Menu::Zoom(p) | Menu::Folders(p) | Menu::Sort(p) => *p,
        };
        let kind = self.menu.clone();
        let inner = egui::Area::new(Id::NULL.with(("menu", menu_kind(&kind))))
            .fixed_pos(pos)
            .order(Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                dialog_frame()
                    .inner_margin(4.0)
                    .corner_radius(8.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                        let mut mb = MenuBuilder { ui, action: None, n: 0 };
                        match kind {
                            Menu::Context(_) => self.context_menu(&mut mb, true),
                            Menu::Tools(_) => self.context_menu(&mut mb, false),
                            Menu::Zoom(_) => self.zoom_menu(&mut mb),
                            Menu::Folders(_) => self.folders_menu(&mut mb),
                            Menu::Sort(_) => self.sort_menu(&mut mb),
                            Menu::None => {}
                        }
                        mb.action
                    })
                    .inner
            });
        let rect = inner.response.rect;
        if let Some(a) = inner.inner {
            self.act(a);
            return;
        }
        let pressed_outside = ctx.input(|i| {
            i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
        });
        if pressed_outside && !self.menu_just_opened {
            self.menu = Menu::None;
        }
    }

    fn context_menu(&self, mb: &mut MenuBuilder, full: bool) {
        let play = if self.slideshow { Icon::Pause } else { Icon::Play };
        if full {
            mb.icon_row(
                &[
                    (Icon::First, Action::First, "Первый (Home)"),
                    (Icon::Prev, Action::Prev, "Назад (←)"),
                    (play, Action::ToggleSlideshow, "Слайд-шоу (Пробел)"),
                    (Icon::Next, Action::Next, "Вперёд (→)"),
                    (Icon::Last, Action::Last, "Последний (End)"),
                ],
                &[
                    (Icon::Fullscreen, Action::ToggleFullscreen, "Весь экран (F11)"),
                    (Icon::Settings, Action::Settings, "Параметры (S)"),
                ],
            );
            mb.sep();
        }
        if self.multi_selected() {
            let n = self.selected.len();
            mb.row(Some(Icon::Check), &[(&format!("Выделено: {n}"), Action::OpenMenu(self.menu.clone()), false), ("Снять выделение", Action::ClearSelection, false)], Some("Esc"));
            mb.sep();
        } else if full && self.grid {
            mb.row(Some(Icon::Check), &[("Выделить все", Action::SelectAll, false)], Some("Ctrl+A, Ctrl / Shift + щелчок"));
            mb.sep();
        }
        mb.row(Some(Icon::Info), &[("Информация", Action::Info, self.panel == Panel::Info), ("Все метаданные", Action::AllMeta, self.panel == Panel::Meta)], None);
        mb.row(Some(Icon::Prompt), &[("Копировать промпт", Action::CopyPrompt, false)], Some("для сгенерированных"));
        mb.sep();
        mb.row(Some(Icon::Export), &[("Экспорт…", Action::Export, self.panel == Panel::Export)], Some("обрезка, размер, формат, Ctrl+S"));
        let nobg = if self.multi_selected() { "Удалить фон у выделенных" } else { "Удалить фон" };
        mb.row(Some(Icon::Wand), &[(nobg, Action::RemoveBg, false), ("В экспорте…", Action::RemoveBgExport, false)], Some("B"));
        if full {
            mb.sep();
            let m = self.view.mode;
            mb.row(
                Some(Icon::Zoom),
                &[
                    ("Вписать", Action::Zoom(ZoomMode::FitLarger), m == ZoomMode::FitLarger || m == ZoomMode::Fit),
                    ("Заполнить", Action::Zoom(ZoomMode::Fill), m == ZoomMode::Fill),
                    ("1:1", Action::Zoom(ZoomMode::Actual), m == ZoomMode::Actual),
                    ("По ширине", Action::Zoom(ZoomMode::FitWidth), m == ZoomMode::FitWidth),
                    ("Ещё", Action::OpenMenu(Menu::Zoom(self.menu_pos())), false),
                ],
                None,
            );
            mb.sep();
            let st = self.s.strip;
            mb.row(
                Some(Icon::Grid),
                &[
                    ("Один", Action::SetStrip(Strip::Off), !self.grid && st == Strip::Off),
                    ("Сетка", Action::SetGrid(true), self.grid),
                    ("Сбоку", Action::SetStrip(Strip::Side), !self.grid && st == Strip::Side),
                    ("Снизу", Action::SetStrip(Strip::Bottom), !self.grid && st == Strip::Bottom),
                ],
                None,
            );
        }
        mb.sep();
        mb.row(Some(Icon::Trash), &[("Удалить", Action::Delete, false)], Some("в корзину, Del"));
        mb.sep();
        mb.row(Some(Icon::Copy), &[("Копировать изображение", Action::CopyImage, false), ("Путь", Action::CopyPath, false)], None);
        mb.sep();
        mb.row(Some(Icon::Rotate), &[("Повернуть вправо", Action::RotateCw, false), ("влево", Action::RotateCcw, false), ("Отразить", Action::FlipH, self.view.flip)], None);
        mb.sep();
        mb.row(Some(Icon::OpenWith), &[("Открыть с помощью", Action::OpenWith, false), ("В проводнике", Action::ShowInExplorer, false)], None);
        mb.row(Some(Icon::Picture), &[("Сделать фоном рабочего стола", Action::SetWallpaper, false)], None);
        mb.sep();
        let flagged = self.current().is_some_and(|c| c.flagged);
        mb.row(Some(Icon::Star), &[(if flagged { "В избранном" } else { "В избранное" }, Action::Flag, flagged)], Some("P, список в «Недавних»"));
        let copy_label = if self.multi_selected() { "Копировать выделенные…" } else { "Копировать избранные…" };
        mb.row(None, &[(copy_label, Action::CopyFlagged, false), ("Переместить…", Action::MoveFlagged, false)], None);
        mb.sep();
        self.sort_rows(mb);
        mb.sep();
        mb.row(Some(Icon::Refresh), &[("Обновить", Action::Reload, false)], Some("F5"));
        if full {
            mb.row(None, &[(if self.s.toolbar_visible { "Скрыть панель" } else { "Показать панель" }, Action::ToggleToolbar, false)], Some("T"));
        }
    }

    fn menu_pos(&self) -> egui::Pos2 {
        match &self.menu {
            Menu::Context(p) | Menu::Tools(p) | Menu::Zoom(p) | Menu::Folders(p) | Menu::Sort(p) => *p,
            Menu::None => pos2(100.0, 50.0),
        }
    }

    fn sort_menu(&self, mb: &mut MenuBuilder) {
        self.sort_rows(mb);
    }

    fn sort_rows(&self, mb: &mut MenuBuilder) {
        let s = self.s.sort;
        mb.row(
            Some(Icon::Sort),
            &[
                ("Имя", Action::Sort(SortKey::Name), s == SortKey::Name),
                ("Дата", Action::Sort(SortKey::Modified), s == SortKey::Modified),
                ("Размер", Action::Sort(SortKey::Size), s == SortKey::Size),
                ("Тип", Action::Sort(SortKey::Type), s == SortKey::Type),
            ],
            None,
        );
        mb.row(None, &[("Перемешать", Action::Sort(SortKey::Random), s == SortKey::Random), ("По убыванию", Action::SortDesc, self.s.sort_desc)], None);
    }

    fn zoom_menu(&self, mb: &mut MenuBuilder) {
        let m = self.view.mode;
        mb.row(Some(Icon::Zoom), &[("Вписать", Action::Zoom(ZoomMode::Fit), m == ZoomMode::Fit), ("Только уменьшать", Action::Zoom(ZoomMode::FitLarger), m == ZoomMode::FitLarger)], None);
        mb.row(None, &[("Заполнить", Action::Zoom(ZoomMode::Fill), m == ZoomMode::Fill), ("1:1", Action::Zoom(ZoomMode::Actual), m == ZoomMode::Actual)], Some("0 / 1 / 2"));
        mb.row(None, &[("По ширине", Action::Zoom(ZoomMode::FitWidth), m == ZoomMode::FitWidth), ("По высоте", Action::Zoom(ZoomMode::FitHeight), m == ZoomMode::FitHeight)], None);
        mb.sep();
        mb.row(Some(Icon::ZoomIn), &[("Увеличить", Action::ZoomIn, false), ("Уменьшить", Action::ZoomOut, false)], Some("+ / −"));
        mb.sep();
        mb.label("Колесо мыши масштабирует, Ctrl + колесо листает");
    }

    fn folders_menu(&self, mb: &mut MenuBuilder) {
        mb.row(Some(Icon::Folder), &[("Открыть папку…", Action::OpenFolder, false), ("Открыть файлы…", Action::OpenFiles, false)], None);
        mb.row(None, &[("Включая подпапки", Action::IncludeSubfolders, self.s.include_subfolders)], None);
        mb.sep();
        mb.label("Быстрый доступ");
        if self.s.quick_access.is_empty() {
            mb.label("   пусто: добавьте текущую папку кнопкой ниже");
        }
        for p in &self.s.quick_access {
            let name = short_name(p);
            let icon = if p.is_dir() { Icon::Folder } else { Icon::File };
            mb.row(Some(icon), &[(&name, Action::OpenPath(p.clone()), false), ("убрать", Action::RemoveQuick(p.clone()), false)], None);
        }
        mb.row(Some(Icon::Folder), &[("Добавить текущую папку", Action::AddQuickFolder, false)], None);
        mb.sep();
        let flagged = self.flagged_files();
        mb.label(&format!("Избранное ({})", flagged.len()));
        if flagged.is_empty() {
            mb.label("   пусто: нажмите звёздочку у имени файла или P");
        } else {
            for p in flagged.iter().take(8) {
                let name = short_name(p);
                mb.row(Some(Icon::Star), &[(&name, Action::OpenPath(p.clone()), false), ("убрать", Action::Unflag(p.clone()), false)], None);
            }
            let all = if flagged.len() > 8 { format!("Показать все ({})", flagged.len()) } else { "Показать все сеткой".to_string() };
            mb.row(Some(Icon::Grid), &[(&all, Action::ShowFlagged, false), ("Очистить", Action::ClearFlags, false)], None);
        }
        if !self.s.recent.is_empty() {
            mb.sep();
            mb.label("Недавние папки");
            for p in self.s.recent.iter().take(8) {
                let name = short_name(p);
                mb.row(Some(Icon::Folder), &[(&name, Action::OpenPath(p.clone()), false)], None);
            }
        }
    }

    // ---------- side panels ----------

    pub fn panels_ui(&mut self, ctx: &Context, full: Rect) {
        if self.panel == Panel::None {
            return;
        }
        let top = full.top() + crate::ui::TOOLBAR_H;
        let w = match self.panel {
            Panel::Meta => (full.width() * 0.62).clamp(420.0, 980.0),
            _ => 440.0,
        };
        let rect = Rect::from_min_max(pos2(full.right() - w, top), full.max);
        egui::Area::new(Id::NULL.with("side_panel"))
            .fixed_pos(rect.min)
            .order(Order::Middle)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(30, 30, 32))
                    .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.35)))
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.set_width(w - 34.0);
                        ui.set_height(rect.height() - 28.0);
                        roomy(ui);
                        ui.horizontal(|ui| {
                            let title = match self.panel {
                                Panel::Info => "Информация",
                                Panel::Meta => "Все метаданные",
                                Panel::Export => "Экспорт",
                                _ => "Параметры",
                            };
                            ui.label(RichText::new(title).size(18.0).color(TEXT));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, Icon::Close, "Закрыть (Esc)").clicked() {
                                    self.panel = Panel::None;
                                }
                                if self.panel == Panel::Meta {
                                    if icon_button(ui, Icon::Save, "Сохранить все метаданные в текстовый файл").clicked() {
                                        self.save_meta_txt();
                                    }
                                    icon_button(ui, Icon::Info, "Все данные, которые нашлись в файле: свойства файла, контейнер, кадр, EXIF и текстовые блоки (например, prompt и workflow из ComfyUI). Длинные значения открываются кнопкой «Открыть».");
                                }
                            });
                        });
                        ui.add_space(6.0);
                        // A solid scroll bar takes its own space instead of covering buttons.
                        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                        ui.spacing_mut().scroll.bar_outer_margin = 2.0;
                        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("panel_scroll", self.panel as u8)).show(ui, |ui| {
                            match self.panel {
                                Panel::Info => self.info_panel(ui),
                                Panel::Meta => self.meta_panel(ui),
                                Panel::Export => self.export_panel(ui),
                                _ => self.settings_panel(ui),
                            }
                        });
                    });
            });
    }

    fn info_panel(&mut self, ui: &mut Ui) {
        let Some(it) = self.current() else {
            ui.label("Нет файла");
            return;
        };
        let (name, path) = (it.name.clone(), it.path.clone());
        heading(ui, "Файл");
        ui.label(RichText::new(&name).strong().size(16.0));
        ui.label(RichText::new(path.display().to_string()).size(12.0).color(TEXT_DIM));
        if ui.link("Показать в проводнике").clicked() {
            crate::platform::show_in_explorer(&path);
        }
        heading(ui, "Свойства");
        let full = self.current_full();
        egui::Grid::new("info_basic").num_columns(2).spacing([14.0, 6.0]).show(ui, |ui| {
            if let Some(img) = &full {
                ui.label(RichText::new("Размеры").color(TEXT_DIM));
                let mp = img.width as f64 * img.height as f64 / 1e6;
                ui.label(format!("{} × {} ({mp:.1} Мп)", img.width, img.height));
                ui.end_row();
                ui.label(RichText::new("Формат").color(TEXT_DIM));
                ui.label(&img.format);
                ui.end_row();
                if img.textures.len() > 1 {
                    ui.label(RichText::new("Кадров").color(TEXT_DIM));
                    let total: u32 = img.delays.iter().sum();
                    ui.label(format!("{} ({:.1} с)", img.textures.len(), total as f32 / 1000.0));
                    ui.end_row();
                }
            }
            if let Some((p, m)) = &self.meta {
                if p == &path {
                    for (k, v) in &m.summary {
                        ui.label(RichText::new(k).color(TEXT_DIM));
                        ui.label(v);
                        ui.end_row();
                    }
                }
            }
        });
        if let Some(img) = &full {
            heading(ui, "Гистограмма");
            histogram(ui, &img.histogram);
        }
        heading(ui, "Метаданные");
        let w = ui.available_width();
        if big_button(ui, "Все метаданные", "M", true, w).clicked() {
            self.act(Action::AllMeta);
        }
        if big_button(ui, "Копировать промпт", "", false, w).clicked() {
            self.act(Action::CopyPrompt);
        }
    }

    fn settings_panel(&mut self, ui: &mut Ui) {
        let before = (self.s.sort, self.s.sort_desc, self.s.include_subfolders, self.s.watch_folder);
        let s = &mut self.s;
        section(ui, "Интерфейс");
        ui.label("Размер текста");
        let mut preset = [0.9f32, 1.0, 1.15, 1.3, 1.5].into_iter().find(|k| (s.ui_scale - k).abs() < 0.01).unwrap_or(0.0);
        let opts: [(f32, &str, bool); 5] = [(0.9, "90 %", true), (1.0, "100 %", true), (1.15, "115 %", true), (1.3, "130 %", true), (1.5, "150 %", true)];
        if segmented(ui, &mut preset, &opts, 5) {
            s.ui_scale = preset;
        }
        ui.add(egui::Slider::new(&mut s.ui_scale, 0.8..=2.0).step_by(0.05).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)).text("Точно"));

        section(ui, "Слайд-шоу");
        ui.add(egui::Slider::new(&mut s.slideshow_interval, 1.0..=300.0).logarithmic(true).suffix(" с").text("Интервал"));
        ui.checkbox(&mut s.slideshow_random, "Случайный порядок");
        ui.checkbox(&mut s.slideshow_loop, "По кругу");
        ui.checkbox(&mut s.slideshow_fullscreen, "Запускать во весь экран");
        ui.label("Переход");
        segmented(ui, &mut s.transition, &[(Transition::None, "Нет", true), (Transition::Fade, "Затухание", true), (Transition::Slide, "Сдвиг", true), (Transition::Zoom, "Наплыв", true)], 4);
        ui.add(egui::Slider::new(&mut s.transition_ms, 100..=3000).suffix(" мс").text("Длительность"));
        ui.checkbox(&mut s.transition_on_manual, "Переходы и при ручном листании");

        section(ui, "Просмотр");
        ui.label("Масштаб нового фото");
        segmented(
            ui,
            &mut s.default_zoom,
            &[(ZoomMode::FitLarger, "Только уменьшать", true), (ZoomMode::Fit, "Вписать", true), (ZoomMode::Fill, "Заполнить", true), (ZoomMode::Actual, "1:1", true), (ZoomMode::FitWidth, "По ширине", true), (ZoomMode::FitHeight, "По высоте", true)],
            2,
        );
        ui.checkbox(&mut s.remember_zoom, "Сохранять масштаб при листании");
        ui.label("Колесо мыши");
        segmented(ui, &mut s.wheel, &[(WheelAction::Zoom, "Масштабирует", true), (WheelAction::Navigate, "Листает", true)], 2);
        ui.horizontal_wrapped(|ui| {
            ui.label("Фон");
            ui.color_edit_button_srgb(&mut s.background);
            for (name, c) in [("Чёрный", [0, 0, 0]), ("Графит", [24, 24, 24]), ("Серый", [64, 64, 64]), ("Белый", [255, 255, 255])] {
                if ui.small_button(name).clicked() {
                    s.background = c;
                }
            }
        });
        ui.checkbox(&mut s.autoplay_animations, "Сразу проигрывать анимацию GIF/WebP/APNG");

        section(ui, "Папки и файлы");
        ui.checkbox(&mut s.include_subfolders, "Включать подпапки");
        ui.label("Сортировка");
        segmented(
            ui,
            &mut s.sort,
            &[(SortKey::Name, "Имя", true), (SortKey::Modified, "Дата", true), (SortKey::Size, "Размер", true), (SortKey::Type, "Тип", true), (SortKey::Random, "Случайно", true)],
            5,
        );
        ui.checkbox(&mut s.sort_desc, "По убыванию");
        ui.checkbox(&mut s.watch_folder, "Следить за изменениями в папке");
        ui.checkbox(&mut s.jump_to_new, "Сразу показывать новые файлы");
        ui.checkbox(&mut s.restore_session, "Восстанавливать прошлый сеанс при запуске");
        ui.checkbox(&mut s.confirm_delete, "Спрашивать перед удалением");

        section(ui, "Миниатюры");
        ui.add(egui::Slider::new(&mut s.thumb_size, 64.0..=480.0).suffix(" pt").text("Размер (Ctrl + колесо)"));
        ui.add(egui::Slider::new(&mut s.grid_wheel_rows, 0.5..=5.0).step_by(0.5).text("Рядов за щелчок колеса"));

        section(ui, "Поверх изображения");
        ui.checkbox(&mut s.show_caption, "Подпись с именем файла");
        ui.horizontal(|ui| {
            ui.label("Часы");
            ui.selectable_value(&mut s.clock, ClockStyle::Off, "Нет");
            ui.selectable_value(&mut s.clock, ClockStyle::Digital, "Цифровые");
            ui.selectable_value(&mut s.clock, ClockStyle::Analog, "Стрелочные");
        });
        ui.checkbox(&mut s.show_calendar, "Календарь на месяц");
        ui.checkbox(&mut s.toolbar_visible, "Закрепить верхнюю панель");

        section(ui, "Фоторамка");
        ui.label(RichText::new("Весь экран, слайд-шоу по кругу, часы, экран не гаснет. Выход: Esc. Запуск сразу в этом режиме: параметр --frame.").size(12.0).color(TEXT_DIM));
        let frame_label = if self.frame_mode { "Выключить режим фоторамки" } else { "Включить режим фоторамки" };
        let frame_clicked = ui.button(frame_label).clicked();

        section(ui, "Клавиши");
        egui::Grid::new("keys").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
            for (k, v) in KEYS {
                ui.label(RichText::new(*k).size(12.0).strong());
                ui.label(RichText::new(*v).size(12.0).color(TEXT_DIM));
                ui.end_row();
            }
        });

        let after = (self.s.sort, self.s.sort_desc, self.s.include_subfolders, self.s.watch_folder);
        if before.0 != after.0 || before.1 != after.1 {
            self.resort();
        }
        if before.2 != after.2 {
            self.s.include_subfolders = before.2;
            self.act(Action::IncludeSubfolders);
        }
        if before.3 != after.3 {
            self.rewatch();
        }
        if frame_clicked {
            self.act(Action::PhotoFrame);
        }
        if ui.ctx().input(|i| i.pointer.any_released()) {
            self.s.save();
        }
    }

    /// Asks for the background-removal model when it is not next to the program.
    pub fn model_ui(&mut self, ctx: &Context) {
        if !self.ask_model {
            return;
        }
        let mut pick = false;
        egui::Area::new(Id::NULL.with("ask_model"))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                dialog_frame().show(ui, |ui| {
                    roomy(ui);
                    ui.set_width(420.0);
                    dialog_title(ui, "Не найдена нейросеть");
                    ui.label(RichText::new(format!(
                        "Для удаления фона нужен файл {} (89 МБ). Установщик Angle View кладёт его рядом с программой. Если он у вас есть отдельно, укажите его.",
                        crate::cutout::MODEL_FILE
                    )).color(TEXT_DIM));
                    ui.add_space(12.0);
                    let w = ui.available_width();
                    if big_button(ui, "Указать файл…", "", true, w).clicked() {
                        pick = true;
                    }
                    ui.add_space(6.0);
                    if big_button(ui, "Отмена", "Esc", false, w).clicked() {
                        self.ask_model = false;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ask_model = false;
        }
        if pick {
            self.act(Action::PickBgModel);
        }
    }

    pub fn subfolders_ui(&mut self, ctx: &Context) {
        let Some(dir) = self.ask_subfolders.clone() else { return };
        let name = short_name(&dir);
        let mut choice = None;
        egui::Area::new(Id::NULL.with("ask_subfolders"))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                dialog_frame().show(ui, |ui| {
                    roomy(ui);
                    ui.set_width(380.0);
                    dialog_title(ui, "В папке есть подпапки");
                    ui.label(RichText::new(name).color(TEXT_DIM));
                    ui.add_space(12.0);
                    let w = ui.available_width();
                    if big_button(ui, "Вместе с подпапками", "Enter", true, w).clicked() {
                        choice = Some(true);
                    }
                    ui.add_space(6.0);
                    if big_button(ui, "Только эту папку", "", false, w).clicked() {
                        choice = Some(false);
                    }
                    ui.add_space(6.0);
                    if big_button(ui, "Отмена", "Esc", false, w).clicked() {
                        self.ask_subfolders = None;
                    }
                });
            });
        if let Some(rec) = choice {
            self.open_folder(dir, rec);
        }
    }

    pub fn confirm_ui(&mut self, ctx: &Context) {
        let Some(path) = self.confirm_delete.clone() else { return };
        let (title, name) = match path.as_slice() {
            [one] => ("Удалить файл в корзину?".to_string(), one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            many => (format!("Удалить {} файлов в корзину?", many.len()), String::new()),
        };
        let mut choice = None;
        egui::Area::new(Id::NULL.with("confirm"))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .order(Order::Foreground)
            .show(ctx, |ui| {
                dialog_frame().show(ui, |ui| {
                    roomy(ui);
                    ui.set_width(340.0);
                    dialog_title(ui, &title);
                    if !name.is_empty() {
                        ui.label(RichText::new(name).color(TEXT_DIM));
                    }
                    ui.add_space(12.0);
                    let w = ui.available_width();
                    if big_button(ui, "Удалить", "Enter", true, w).clicked() {
                        choice = Some(true);
                    }
                    ui.add_space(6.0);
                    if big_button(ui, "Отмена", "Esc", false, w).clicked() {
                        choice = Some(false);
                    }
                    ui.add_space(4.0);
                    ui.checkbox(&mut self.s.confirm_delete, "Спрашивать в следующий раз");
                });
            });
        match choice {
            Some(true) => {
                self.confirm_delete = None;
                self.delete_now(path);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }
}

const KEYS: &[(&str, &str)] = &[
    ("← → / PgUp PgDn", "предыдущее / следующее"),
    ("Home / End", "первое / последнее"),
    ("Пробел", "слайд-шоу"),
    ("Enter / G", "сетка ↔ одно фото"),
    ("F11 / F, двойной щелчок", "весь экран"),
    ("Колесо", "масштаб (Ctrl — листать)"),
    ("+ / −", "масштаб"),
    ("0 / 1 / 2 / 3", "вписать / 1:1 / заполнить / по ширине"),
    ("Средняя кнопка", "1:1 ↔ вписать"),
    ("R / Shift+R, H", "поворот, отражение"),
    ("I / M", "информация / все метаданные"),
    ("P", "отметить файл"),
    (", / .  A", "кадр анимации назад/вперёд, пауза"),
    ("Del", "в корзину"),
    ("Ctrl+C / Ctrl+Shift+C", "копировать изображение / путь"),
    ("Ctrl+O / Ctrl+Shift+O", "открыть файлы / папку"),
    ("Ctrl+S", "экспорт"),
    ("K / N / T", "часы / подпись / панель"),
    ("E", "показать в проводнике"),
    ("F5", "обновить папку"),
    ("Esc", "назад"),
];

fn section(ui: &mut Ui, title: &str) {
    heading(ui, title);
}

fn short_name(p: &std::path::Path) -> String {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
    if name.chars().count() > 34 {
        let s: String = name.chars().take(33).collect();
        format!("{s}…")
    } else {
        name
    }
}

fn histogram(ui: &mut Ui, h: &[[u32; 256]; 4]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 100.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, Color32::from_rgb(18, 18, 18));
    // Ignore the two extreme bins when scaling so clipped blacks/whites don't flatten the curve.
    let max = h.iter().flat_map(|c| c[1..255].iter()).copied().max().unwrap_or(1).max(1) as f32;
    let bw = rect.width() / 256.0;
    let colors = [
        Color32::from_rgba_unmultiplied(255, 70, 70, 110),
        Color32::from_rgba_unmultiplied(70, 255, 70, 110),
        Color32::from_rgba_unmultiplied(80, 120, 255, 110),
    ];
    for (ch, col) in colors.iter().enumerate() {
        for i in 0..256 {
            let v = (h[ch][i] as f32 / max).min(1.0) * rect.height();
            if v > 0.5 {
                let x = rect.left() + i as f32 * bw;
                p.rect_filled(Rect::from_min_max(pos2(x, rect.bottom() - v), pos2(x + bw.max(1.0), rect.bottom())), 0.0, *col);
            }
        }
    }
    let pts: Vec<egui::Pos2> = (0..256)
        .map(|i| pos2(rect.left() + (i as f32 + 0.5) * bw, rect.bottom() - (h[3][i] as f32 / max).min(1.0) * rect.height()))
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_gray(220))));
}

pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, HOVER);
    }
    icons::paint(ui.painter(), r.shrink(5.0), icon, TEXT);
    resp.on_hover_text(tip)
}

/// Full-width dialog button; the primary one is filled with the accent color.
pub fn big_button(ui: &mut Ui, label: &str, hint: &str, primary: bool, width: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let fill = match (primary, resp.hovered()) {
        (true, false) => ACCENT_DARK,
        (true, true) => ACCENT_DARK.linear_multiply(1.25),
        (false, false) => Color32::from_rgb(56, 56, 56),
        (false, true) => Color32::from_rgb(72, 72, 72),
    };
    let p = ui.painter();
    p.rect_filled(r, 6.0, fill);
    p.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(15.0), Color32::WHITE);
    if !hint.is_empty() {
        p.text(pos2(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, hint, FontId::proportional(11.0), Color32::from_white_alpha(150));
    }
    resp
}
