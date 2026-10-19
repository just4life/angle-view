#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod clock;
mod cutout;
mod decode;
mod export;
mod export_ui;
mod formats;
mod icons;
mod loader;
mod meta;
mod meta_full;
mod meta_panel;
mod panels;
mod platform;
mod settings;
mod style;
mod ui;

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result {
    let arg = std::env::args().skip(1).find(|a| !a.starts_with("--") && a != "/frame").map(PathBuf::from);
    let options = |renderer| eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Angle View")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([480.0, 320.0])
            .with_drag_and_drop(true)
            .with_icon(app_icon()),
        renderer,
        ..Default::default()
    };
    // DirectX 12 / Vulkan through wgpu first; plain OpenGL if no such adapter exists.
    let a = arg.clone();
    let result = eframe::run_native(
        "Angle View",
        options(eframe::Renderer::Wgpu),
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, a)))),
    );
    if result.is_err() {
        return eframe::run_native(
            "Angle View",
            options(eframe::Renderer::Glow),
            Box::new(move |cc| Ok(Box::new(app::App::new(cc, arg)))),
        );
    }
    result
}

fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).map(|i| i.to_rgba8());
    match img {
        Ok(img) => egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() },
        Err(_) => egui::IconData::default(),
    }
}
