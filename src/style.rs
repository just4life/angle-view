use eframe::egui::{self, Color32, Context, FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

pub const TOOLBAR_BG: Color32 = Color32::from_rgba_premultiplied(24, 24, 24, 235);
pub const MENU_BG: Color32 = Color32::from_rgb(43, 43, 43);
pub const MENU_SEP: Color32 = Color32::from_rgb(28, 28, 28);
pub const HOVER: Color32 = Color32::from_rgb(62, 62, 62);
pub const TEXT: Color32 = Color32::from_rgb(230, 230, 230);
pub const TEXT_DIM: Color32 = Color32::from_rgb(170, 170, 170);
/// Violet from the app icon.
pub const ACCENT: Color32 = Color32::from_rgb(168, 110, 240);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(110, 58, 190);

pub fn setup(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    // Prefer the native Windows UI font; egui's bundled fonts remain as fallback.
    for (name, path) in [("segoe", r"C:\Windows\Fonts\segoeui.ttf"), ("segoe_sym", r"C:\Windows\Fonts\seguisym.ttf")] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            let fam = fonts.families.entry(FontFamily::Proportional).or_default();
            if name == "segoe" {
                fam.insert(0, name.into());
            } else {
                fam.push(name.into());
            }
        }
    }
    ctx.set_fonts(fonts);
    ctx.options_mut(|o| o.zoom_with_keyboard = false);

    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::BLACK;
    v.window_fill = MENU_BG;
    v.extreme_bg_color = Color32::from_rgb(24, 24, 24);
    v.selection.bg_fill = ACCENT_DARK;
    v.selection.stroke.color = Color32::from_rgb(225, 205, 255);
    v.hyperlink_color = ACCENT;
    v.widgets.active.bg_fill = ACCENT_DARK;
    v.widgets.active.weak_bg_fill = ACCENT_DARK;
    v.widgets.hovered.bg_stroke.color = ACCENT.gamma_multiply(0.6);
    v.text_cursor.stroke.color = ACCENT;
    v.widgets.noninteractive.fg_stroke.color = TEXT;
    v.widgets.inactive.bg_fill = Color32::from_rgb(58, 58, 58);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(52, 52, 52);
    v.widgets.hovered.bg_fill = Color32::from_rgb(72, 72, 72);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(68, 68, 68);
    ctx.set_visuals_of(egui::Theme::Dark, v.clone());
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 3.0);
        s.interaction.tooltip_delay = 0.4;
    });
}

/// Section title used by every panel and dialog: accent text over a thin rule.
pub fn heading(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(egui::RichText::new(title).size(13.0).strong().color(ACCENT));
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, Color32::from_white_alpha(22));
    ui.add_space(2.0);
}

/// Larger text, controls and spacing for side panels and dialogs.
pub fn roomy(ui: &mut egui::Ui) {
    let st = ui.style_mut();
    st.spacing.item_spacing = egui::vec2(10.0, 8.0);
    st.spacing.button_padding = egui::vec2(12.0, 6.0);
    st.spacing.interact_size.y = 30.0;
    for (k, f) in st.text_styles.iter_mut() {
        if matches!(k, egui::TextStyle::Body | egui::TextStyle::Button) {
            f.size = 15.0;
        }
    }
}

/// Frame of every pop-up window: menus, dialogs, the text viewer.
pub fn dialog_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(MENU_BG)
        .stroke(egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.45)))
        .corner_radius(10.0)
        .inner_margin(20.0)
        .shadow(egui::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
}

/// Title line of a dialog.
pub fn dialog_title(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).size(18.0).color(TEXT));
}

/// Button filled with the accent color, for the main action.
pub fn accent_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).color(Color32::WHITE)).fill(ACCENT_DARK)
}

/// A row (or rows) of equal-width toggle buttons that always fits the available width.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str, bool)], per_row: usize) -> bool {
    let per_row = per_row.clamp(1, options.len().max(1));
    let gap = 6.0;
    let w = ((ui.available_width() - gap * (per_row as f32 - 1.0)) / per_row as f32).floor();
    let mut changed = false;
    for row in options.chunks(per_row) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (v, label, enabled) in row {
                let on = *value == *v;
                let text = egui::RichText::new(*label).color(if on { Color32::WHITE } else { TEXT });
                let b = egui::Button::new(text).truncate().fill(if on { ACCENT_DARK } else { Color32::from_rgb(44, 44, 46) });
                let r = ui.add_enabled_ui(*enabled, |ui| ui.add_sized([w, 32.0], b)).inner;
                if r.clicked() && !on {
                    *value = *v;
                    changed = true;
                }
            }
        });
    }
    changed
}
