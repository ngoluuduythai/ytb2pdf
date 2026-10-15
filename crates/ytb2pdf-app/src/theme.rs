//! Design tokens for the ytb2pdf UI, inspired by FilmCraft's Premiere-style
//! dark theme (`storytold/filmcraft`, `crates/ui-egui/src/theme.rs`, "Darkest"):
//! near-black panels, hairline separators, one blue accent, sharp corners.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const CANVAS_BG: Color32 = Color32::from_rgb(0x14, 0x17, 0x1c);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x1b, 0x20, 0x27);
pub const FIELD_BG: Color32 = Color32::from_rgb(0x0e, 0x0e, 0x0e);
pub const FIELD_BORDER: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const SEPARATOR: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
pub const TEXT: Color32 = Color32::from_rgb(0xd1, 0xd1, 0xd1);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xb0, 0xb0, 0xb0);
pub const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x6b, 0xdf);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x3f, 0x7c, 0xe8);
pub const HOVER: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
pub const PRESSED: Color32 = Color32::from_rgb(0x4b, 0x4b, 0x4b);
pub const SUCCESS: Color32 = Color32::from_rgb(0x59, 0xc7, 0x9e);
pub const DANGER: Color32 = Color32::from_rgb(0xf0, 0x78, 0x68);

/// Install the FilmCraft-style dark visuals + spacing on the egui context.
pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = PANEL_BG;
    v.window_fill = PANEL_BG;
    v.extreme_bg_color = FIELD_BG; // text fields, log view
    v.faint_bg_color = Color32::from_rgb(0x24, 0x24, 0x24);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.noninteractive.bg_fill = PANEL_BG;
    v.widgets.noninteractive.weak_bg_fill = PANEL_BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, SEPARATOR);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.bg_fill = FIELD_BG;
    v.widgets.inactive.weak_bg_fill = FIELD_BG;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FIELD_BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, FIELD_BORDER);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.bg_fill = PRESSED;
    v.widgets.active.weak_bg_fill = PRESSED;
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.bg_fill = HOVER;
    v.widgets.open.weak_bg_fill = HOVER;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(4);
    }
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.wrap_mode = Some(egui::TextWrapMode::Wrap);
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 8.0);
        s.text_styles
            .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        s.text_styles.insert(
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Proportional),
        );
        s.text_styles.insert(
            TextStyle::Small,
            FontId::new(13.0, FontFamily::Proportional),
        );
        s.text_styles.insert(
            TextStyle::Heading,
            FontId::new(21.0, FontFamily::Proportional),
        );
        s.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        );
    });
}

/// Install the bundled Be Vietnam Pro (same faces the PDF embeds) as the UI font.
pub fn install_font(ctx: &egui::Context) {
    let (regular, bold) = ytb2pdf_core::pdf::bundled_ui_font();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "be-vn-regular".to_owned(),
        egui::FontData::from_owned(regular.to_vec()).into(),
    );
    fonts.font_data.insert(
        "be-vn-bold".to_owned(),
        egui::FontData::from_owned(bold.to_vec()).into(),
    );
    if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
        prop.insert(0, "be-vn-regular".to_owned());
    }
    // The log view uses Monospace — it needs the Vietnamese face too,
    // otherwise diacritics render as tofu boxes.
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.insert(0, "be-vn-regular".to_owned());
    }
    // Bold UI text falls back to the bold face.
    fonts
        .families
        .entry(FontFamily::Name("bold".into()))
        .or_default()
        .push("be-vn-bold".to_owned());
    ctx.set_fonts(fonts);
}

/// Card container: subtle raised panel with rounded corners, for grouping
/// related controls (less monotonous than a flat list).
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::default()
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0_f32, SEPARATOR))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Premiere-style panel section label: small dim caps + hairline rule.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).small().strong().color(TEXT_DIM));
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0_f32, SEPARATOR),
    );
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apply_sets_dark_visuals() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert_eq!(ctx.style().visuals.panel_fill, PANEL_BG);
        assert_eq!(ctx.style().visuals.extreme_bg_color, FIELD_BG);
    }
    #[test]
    fn install_font_does_not_panic() {
        let ctx = egui::Context::default();
        install_font(&ctx);
    }
    #[test]
    fn visuals_survive_install_font_sequence() {
        // mirrors App::new call order
        let ctx = egui::Context::default();
        apply(&ctx);
        install_font(&ctx);
        assert_eq!(ctx.style().visuals.panel_fill, PANEL_BG);
    }
}
