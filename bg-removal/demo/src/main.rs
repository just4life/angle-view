#![cfg_attr(windows, windows_subsystem = "windows")]

//! Small window for trying the background remover: open (or drop) a picture,
//! look at the cut-out over different backdrops, save it as PNG.

use bg_removal::{BgRemover, Options};
use eframe::egui;
use image::RgbaImage;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

const MODEL_FILE: &str = "birefnet-lite.safetensors";

fn main() -> eframe::Result {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Удаление фона")
            .with_inner_size([1200.0, 820.0])
            .with_min_inner_size([520.0, 360.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("Удаление фона", options, Box::new(move |cc| Ok(Box::new(App::new(cc, arg)))))
}

#[derive(Clone, Copy, PartialEq)]
enum Backdrop {
    Checker,
    White,
    Black,
    Green,
}

enum Msg {
    Model(Result<BgRemover, String>),
    Done(RgbaImage, Duration),
}

struct App {
    remover: Option<Arc<BgRemover>>,
    status: String,
    rx: Receiver<Msg>,
    tx: std::sync::mpsc::Sender<Msg>,
    busy_since: Option<Instant>,
    path: Option<PathBuf>,
    src: Option<(RgbaImage, egui::TextureHandle)>,
    out: Option<(RgbaImage, egui::TextureHandle)>,
    show_result: bool,
    backdrop: Backdrop,
    opts: Options,
    pending: Option<PathBuf>,
}

fn model_path() -> Option<PathBuf> {
    let mut c = vec![];
    if let Some(p) = std::env::var_os("BG_REMOVAL_MODEL") {
        c.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            c.push(d.join(MODEL_FILE));
        }
    }
    c.push(PathBuf::from(MODEL_FILE));
    c.into_iter().find(|p| p.exists())
}

impl App {
    fn new(_cc: &eframe::CreationContext<'_>, arg: Option<PathBuf>) -> Self {
        let (tx, rx) = channel();
        let mut app = App {
            remover: None,
            status: String::new(),
            rx,
            tx,
            busy_since: None,
            path: None,
            src: None,
            out: None,
            show_result: true,
            backdrop: Backdrop::Checker,
            opts: Options { release_memory: false, ..Options::default() },
            pending: arg,
        };
        match model_path() {
            Some(p) => app.load_model(p),
            None => app.status = format!("Не найден файл модели {MODEL_FILE} рядом с программой"),
        }
        app
    }

    fn load_model(&mut self, p: PathBuf) {
        self.status = "Загрузка модели…".into();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Model(BgRemover::from_file(&p).map_err(|e| e.to_string())));
        });
    }

    fn open(&mut self, ctx: &egui::Context, p: PathBuf) {
        match image::open(&p) {
            Ok(img) => {
                let img = img.to_rgba8();
                let tex = texture(ctx, "src", &img);
                self.src = Some((img, tex));
                self.out = None;
                self.status = p.display().to_string();
                self.path = Some(p);
                self.run();
            }
            Err(e) => self.status = format!("Не удалось открыть {}: {e}", p.display()),
        }
    }

    fn run(&mut self) {
        let (Some(r), Some((img, _))) = (&self.remover, &self.src) else { return };
        if self.busy_since.is_some() {
            return;
        }
        let (r, img, opts, tx) = (r.clone(), img.clone(), self.opts.clone(), self.tx.clone());
        self.busy_since = Some(Instant::now());
        std::thread::spawn(move || {
            let t = Instant::now();
            let out = r.remove_background(&img, &opts);
            let _ = tx.send(Msg::Done(out, t.elapsed()));
        });
    }

    fn save(&mut self) {
        let Some((img, _)) = &self.out else { return };
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| format!("{}-nobg.png", s.to_string_lossy()))
            .unwrap_or_else(|| "nobg.png".into());
        let mut dlg = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(name);
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dlg = dlg.set_directory(dir);
        }
        if let Some(p) = dlg.save_file() {
            self.status = match img.save(&p) {
                Ok(()) => format!("Сохранено: {}", p.display()),
                Err(e) => format!("Ошибка сохранения: {e}"),
            };
        }
    }
}

fn texture(ctx: &egui::Context, name: &str, img: &RgbaImage) -> egui::TextureHandle {
    // keep textures within what every GPU accepts
    let max = 4096;
    let shown;
    let img = if img.width() > max || img.height() > max {
        let s = max as f32 / img.width().max(img.height()) as f32;
        shown = image::imageops::resize(
            img,
            (img.width() as f32 * s) as u32,
            (img.height() as f32 * s) as u32,
            image::imageops::FilterType::Triangle,
        );
        &shown
    } else {
        img
    };
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    ctx.load_texture(name, ci, egui::TextureOptions::LINEAR)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Model(Ok(r)) => {
                    self.remover = Some(Arc::new(r));
                    self.status = "Модель загружена. Откройте или перетащите изображение.".into();
                    if let Some(p) = self.pending.take() {
                        self.open(&ctx, p);
                    } else {
                        self.run();
                    }
                }
                Msg::Model(Err(e)) => self.status = format!("Ошибка модели: {e}"),
                Msg::Done(img, dt) => {
                    let tex = texture(&ctx, "out", &img);
                    self.out = Some((img, tex));
                    self.busy_since = None;
                    self.show_result = true;
                    self.status = format!("Готово за {:.1} с", dt.as_secs_f32());
                }
            }
        }
        if let Some(f) = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty())) {
            if self.remover.is_some() {
                self.open(&ctx, f);
            } else {
                self.pending = Some(f);
            }
        }

        egui::Panel::top("bar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Открыть…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Изображения", &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"])
                        .pick_file()
                    {
                        if self.remover.is_some() {
                            self.open(&ctx, p);
                        } else {
                            self.pending = Some(p);
                        }
                    }
                }
                let can_run = self.remover.is_some() && self.src.is_some() && self.busy_since.is_none();
                if ui.add_enabled(can_run, egui::Button::new("Удалить фон")).clicked() {
                    self.run();
                }
                if ui.add_enabled(self.out.is_some(), egui::Button::new("Сохранить PNG…")).clicked() {
                    self.save();
                }
                ui.separator();
                ui.add_enabled(self.out.is_some(), egui::Checkbox::new(&mut self.show_result, "Результат"));
                ui.separator();
                ui.label("Фон:");
                ui.selectable_value(&mut self.backdrop, Backdrop::Checker, "клетка");
                ui.selectable_value(&mut self.backdrop, Backdrop::White, "белый");
                ui.selectable_value(&mut self.backdrop, Backdrop::Black, "чёрный");
                ui.selectable_value(&mut self.backdrop, Backdrop::Green, "зелёный");
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.opts.refine_edges, "Уточнять края");
                ui.checkbox(&mut self.opts.decontaminate_colors, "Очищать цвет волос/краёв");
                ui.checkbox(&mut self.opts.main_subject_only, "Только главный объект");
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(t) = self.busy_since {
                    ui.spinner();
                    ui.label(format!("Обработка… {:.0} с", t.elapsed().as_secs_f32()));
                    ctx.request_repaint_after(Duration::from_millis(200));
                } else if self.remover.is_none() && !self.status.starts_with("Не") && !self.status.starts_with("Ошибка") {
                    ui.spinner();
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
                ui.label(&self.status);
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let tex = match (&self.out, &self.src) {
                (Some((_, t)), _) if self.show_result => t,
                (_, Some((_, t))) => t,
                _ => {
                    ui.centered_and_justified(|ui| ui.heading("Перетащите сюда фотографию"));
                    return;
                }
            };
            let avail = ui.available_rect_before_wrap();
            let ts = tex.size_vec2();
            let s = (avail.width() / ts.x).min(avail.height() / ts.y);
            let rect = egui::Rect::from_center_size(avail.center(), ts * s);
            let p = ui.painter_at(avail);
            let showing_cut = self.out.is_some() && self.show_result;
            if showing_cut {
                match self.backdrop {
                    Backdrop::Checker => {
                        p.rect_filled(rect, 0.0, egui::Color32::from_gray(200));
                        let cell = 12.0;
                        let (nx, ny) = ((rect.width() / cell).ceil() as i32, (rect.height() / cell).ceil() as i32);
                        for y in 0..ny {
                            for x in (y % 2..nx).step_by(2) {
                                let r = egui::Rect::from_min_size(rect.min + egui::vec2(x as f32 * cell, y as f32 * cell), egui::vec2(cell, cell))
                                    .intersect(rect);
                                p.rect_filled(r, 0.0, egui::Color32::from_gray(150));
                            }
                        }
                    }
                    Backdrop::White => {
                        p.rect_filled(rect, 0.0, egui::Color32::WHITE);
                    }
                    Backdrop::Black => {
                        p.rect_filled(rect, 0.0, egui::Color32::BLACK);
                    }
                    Backdrop::Green => {
                        p.rect_filled(rect, 0.0, egui::Color32::from_rgb(40, 200, 60));
                    }
                }
            }
            p.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        });
    }
}
