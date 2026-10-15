//! Consistent slate surfaces, readable text and a single blue action accent.
//! The app owns its dark palette instead of mixing it with OS light widgets.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const CANVAS_BG: Color32 = Color32::from_rgb(0x10, 0x17, 0x22);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x1b, 0x26, 0x36);
pub const FIELD_BG: Color32 = Color32::from_rgb(0x12, 0x1c, 0x2b);
pub const FIELD_BORDER: Color32 = Color32::from_rgb(0x46, 0x56, 0x6f);
pub const SEPARATOR: Color32 = Color32::from_rgb(0x32, 0x42, 0x59);
pub const TEXT: Color32 = Color32::from_rgb(0xf1, 0xf5, 0xfb);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0xba, 0xc8, 0xda);
pub const ACCENT: Color32 = Color32::from_rgb(0x28, 0x5a, 0xcc);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x38, 0x6d, 0xdf);
pub const HOVER: Color32 = Color32::from_rgb(0x32, 0x46, 0x61);
pub const PRESSED: Color32 = Color32::from_rgb(0x20, 0x36, 0x54);
pub const SUCCESS: Color32 = Color32::from_rgb(0x59, 0xc7, 0x9e);
pub const DANGER: Color32 = Color32::from_rgb(0xf0, 0x78, 0x68);

/// Install the app palette in both styles and opt out of OS theme switching.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut v = Visuals::dark();
    v.panel_fill = PANEL_BG;
    v.window_fill = PANEL_BG;
    v.extreme_bg_color = FIELD_BG; // text fields, log view
    v.faint_bg_color = PANEL_BG;
    v.text_edit_bg_color = Some(FIELD_BG);
    v.weak_text_color = Some(TEXT_DIM);
    v.hyperlink_color = Color32::from_rgb(0x8b, 0xb6, 0xff);
    v.error_fg_color = DANGER;
    v.warn_fg_color = Color32::from_rgb(0xf4, 0xc2, 0x6b);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.widgets.noninteractive.bg_fill = PANEL_BG;
    v.widgets.noninteractive.weak_bg_fill = PANEL_BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, SEPARATOR);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x26, 0x36, 0x4d);
    v.widgets.inactive.weak_bg_fill = v.widgets.inactive.bg_fill;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, FIELD_BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, FIELD_BORDER);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.active.bg_fill = PRESSED;
    v.widgets.active.weak_bg_fill = PRESSED;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT_HOVER);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.open.bg_fill = HOVER;
    v.widgets.open.weak_bg_fill = HOVER;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, ACCENT_HOVER);
    v.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(4);
    }
    ctx.all_styles_mut(|s| {
        s.visuals = v.clone();
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

/// Muted section label and a subtle separator.
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
    fn os_light_mode_does_not_replace_the_app_palette() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        apply(&ctx);
        for system_theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
            let _ = ctx.run(
                egui::RawInput {
                    system_theme: Some(system_theme),
                    ..Default::default()
                },
                |ctx| {
                    assert_eq!(ctx.theme(), egui::Theme::Dark);
                    let style = ctx.style();
                    assert_eq!(style.visuals.override_text_color, Some(TEXT));
                    assert_eq!(style.visuals.text_edit_bg_color(), FIELD_BG);
                    assert_eq!(style.visuals.selection.bg_fill, ACCENT);
                    assert_eq!(style.visuals.widgets.inactive.fg_stroke.color, TEXT);
                },
            );
        }
    }

    #[test]
    fn text_and_action_colors_meet_normal_text_contrast() {
        fn luminance(color: Color32) -> f64 {
            let linear = |channel: u8| {
                let value = channel as f64 / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
        }
        for (foreground, background) in [
            (TEXT, PANEL_BG),
            (TEXT_DIM, PANEL_BG),
            (TEXT, FIELD_BG),
            (TEXT_DIM, CANVAS_BG),
            (SUCCESS, PANEL_BG),
            (DANGER, PANEL_BG),
            (Color32::WHITE, ACCENT),
            (Color32::WHITE, ACCENT_HOVER),
        ] {
            let a = luminance(foreground);
            let b = luminance(background);
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
        }
    }

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
