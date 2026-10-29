//! The house Linear-style palette and type scale, shared with The Jar and GameBridge.

use std::sync::Arc;

use egui::epaint::text::VariationCoords;
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Shadow, Stroke, TextStyle,
    Visuals, vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x08, 0x09, 0x0a);
pub const BG_SOFT: Color32 = Color32::from_rgb(0x0c, 0x0d, 0x0f);
pub const BG_PANEL: Color32 = Color32::from_rgb(0x13, 0x14, 0x16);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x1e);
pub const BG_ACTIVE: Color32 = Color32::from_rgb(0x20, 0x21, 0x24);
pub const BG_PRESSED: Color32 = Color32::from_rgb(0x26, 0x27, 0x2b);

pub const LINE: Color32 = Color32::from_rgb(0x1f, 0x20, 0x23);
pub const LINE_STRONG: Color32 = Color32::from_rgb(0x2a, 0x2c, 0x30);

pub const FG: Color32 = Color32::from_rgb(0xf7, 0xf8, 0xf8);
pub const FG_MUTED: Color32 = Color32::from_rgb(0x8a, 0x8f, 0x98);
pub const FG_SUBTLE: Color32 = Color32::from_rgb(0x62, 0x66, 0x6d);
pub const FG_FAINT: Color32 = Color32::from_rgb(0x4a, 0x4e, 0x55);

pub const ACCENT: Color32 = Color32::from_rgb(0x5e, 0x6a, 0xd2);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x6e, 0x79, 0xdb);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(0x55, 0x60, 0xc4);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x5c);
pub const ACCENT_SOFT_HOVER: Color32 = Color32::from_rgb(0x33, 0x39, 0x6e);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xb4, 0xba, 0xf6);

pub const WARN: Color32 = Color32::from_rgb(0xf2, 0xc9, 0x4c);
pub const DANGER: Color32 = Color32::from_rgb(0xf2, 0x55, 0x5a);
pub const SUCCESS: Color32 = Color32::from_rgb(0x4c, 0xb7, 0x82);

pub const RADIUS: u8 = 6;

const INTER: &[u8] = include_bytes!("../../assets/fonts/InterVariable.ttf");
const MEDIUM: &str = "Inter Medium";
const SEMIBOLD: &str = "Inter SemiBold";

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, regular(12.0)),
            (TextStyle::Body, regular(13.0)),
            (TextStyle::Button, medium(13.0)),
            (TextStyle::Heading, semibold(21.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(10.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.scroll.floating = true;
        style.spacing.scroll.bar_width = 6.0;
        style.visuals = visuals();
    });
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let weight = |wght: f32| {
        Arc::new(
            FontData::from_static(INTER)
                .tweak(FontTweak { coords: VariationCoords::new([(b"wght", wght)]), ..Default::default() }),
        )
    };
    fonts.font_data.insert("Inter".to_owned(), weight(400.0));
    fonts.font_data.insert("Inter-Medium".to_owned(), weight(510.0));
    fonts.font_data.insert("Inter-SemiBold".to_owned(), weight(590.0));

    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let family = |first: &str| std::iter::once(first.to_owned()).chain(fallbacks.iter().cloned()).collect();
    fonts.families.insert(FontFamily::Proportional, family("Inter"));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), family("Inter-Medium"));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family("Inter-SemiBold"));
    fonts
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG_PANEL;
    v.window_stroke = Stroke::new(1.0, LINE_STRONG);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 12], blur: 40, spread: 0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };
    v.extreme_bg_color = BG_SOFT;
    v.faint_bg_color = BG_SOFT;
    v.code_bg_color = BG_ACTIVE;
    v.hyperlink_color = ACCENT_TEXT;
    v.warn_fg_color = WARN;
    v.error_fg_color = DANGER;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = Stroke::new(1.0, FG);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT_TEXT);

    let radius = CornerRadius::same(RADIUS);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, FG);
    v.widgets.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut v.widgets.inactive, BG_HOVER),
        (&mut v.widgets.hovered, BG_ACTIVE),
        (&mut v.widgets.active, BG_PRESSED),
        (&mut v.widgets.open, BG_ACTIVE),
    ] {
        state.weak_bg_fill = fill;
        state.bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, LINE_STRONG);
        state.fg_stroke = Stroke::new(1.0, FG);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    v
}

/// A colour pair for a name, so rows get identity without anyone choosing colours.
pub fn tint(name: &str) -> (Color32, Color32) {
    let hash =
        name.to_lowercase().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let hue = (hash % 360) as f32 / 360.0;
    let bg = egui::ecolor::Hsva::new(hue, 0.42, 0.30, 1.0);
    let fg = egui::ecolor::Hsva::new(hue, 0.30, 0.95, 1.0);
    (bg.into(), fg.into())
}
