use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, vec2,
};

use super::icons::{self, Icon};
use super::theme::{self, *};

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// The one main action on screen.
    Primary,
    Accent,
    Secondary,
    Ghost,
    Danger,
    /// The confirm button in a dialog that deletes something.
    Destructive,
}

pub fn button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    button_sized(ui, None, text, kind, 28.0)
}

pub fn small_button(ui: &mut Ui, text: &str, kind: Kind) -> Response {
    button_sized(ui, None, text, kind, 24.0)
}

pub fn icon_text_button(ui: &mut Ui, icon: Icon, text: &str, kind: Kind) -> Response {
    button_sized(ui, Some(icon), text, kind, 28.0)
}

fn button_sized(ui: &mut Ui, icon: Option<Icon>, text: &str, kind: Kind, height: f32) -> Response {
    let font = theme::medium(if height < 26.0 { 12.0 } else { 13.0 });
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let padding = if height < 26.0 { 9.0 } else { 12.0 };
    let icon_space = if icon.is_some() { 20.0 } else { 0.0 };
    let width = galley.size().x + padding * 2.0 + icon_space;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let enabled = ui.is_enabled();

    if ui.is_rect_visible(rect) {
        let hovered = enabled && response.hovered();
        let pressed = enabled && response.is_pointer_button_down_on();
        let (fill, stroke, text_color) = button_colors(kind, hovered, pressed);
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let stroke = stroke.map_or(Stroke::NONE, |c| Stroke::new(1.0, fade(c)));
        ui.painter().rect(rect, CornerRadius::same(RADIUS), fade(fill), stroke, StrokeKind::Inside);
        let mut x = rect.left() + padding;
        if let Some(icon) = icon {
            let icon_rect = Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(15.0, 15.0));
            icons::paint(ui.painter(), icon_rect, icon, fade(text_color));
            x += icon_space;
        }
        let pos = egui::pos2(x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(pos, galley, fade(text_color));
    }
    if enabled { response.on_hover_cursor(CursorIcon::PointingHand) } else { response }
}

fn button_colors(kind: Kind, hovered: bool, pressed: bool) -> (Color32, Option<Color32>, Color32) {
    let pick = |rest: Color32, hover: Color32, down: Color32| match (pressed, hovered) {
        (true, _) => down,
        (_, true) => hover,
        _ => rest,
    };
    match kind {
        Kind::Primary => (pick(ACCENT, ACCENT_HOVER, ACCENT_PRESSED), None, Color32::WHITE),
        Kind::Accent => (pick(ACCENT_SOFT, ACCENT_SOFT_HOVER, ACCENT_SOFT), None, ACCENT_TEXT),
        Kind::Secondary => (pick(BG_HOVER, BG_ACTIVE, BG_PRESSED), Some(LINE_STRONG), FG),
        Kind::Ghost => (pick(Color32::TRANSPARENT, BG_HOVER, BG_ACTIVE), None, if hovered { FG } else { FG_MUTED }),
        Kind::Destructive => {
            (pick(DANGER, DANGER.gamma_multiply(0.9), DANGER.gamma_multiply(0.8)), None, Color32::WHITE)
        }
        Kind::Danger => {
            (pick(Color32::TRANSPARENT, DANGER.gamma_multiply(0.12), DANGER.gamma_multiply(0.2)), None, DANGER)
        }
    }
}

/// A square ghost button holding one icon, with its name as the tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    icon_button_tinted(ui, icon, tooltip, None)
}

pub fn icon_button_tinted(ui: &mut Ui, icon: Icon, tooltip: &str, tint: Option<Color32>) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered() && ui.is_enabled();
        if hovered {
            let fill = if response.is_pointer_button_down_on() { BG_ACTIVE } else { BG_HOVER };
            ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), fill);
        }
        let color = tint.unwrap_or(if hovered { FG } else { FG_MUTED });
        let color = if ui.is_enabled() { color } else { color.gamma_multiply(0.4) };
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, color);
    }
    response.on_hover_text(tooltip).on_hover_cursor(CursorIcon::PointingHand)
}

pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(30.0, 18.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    let track = egui::lerp(egui::Rgba::from(BG_ACTIVE)..=egui::Rgba::from(ACCENT), t);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(9),
        Color32::from(track),
        Stroke::new(1.0, if *on { Color32::TRANSPARENT } else { LINE_STRONG }),
        StrokeKind::Inside,
    );
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
    painter.circle_filled(egui::pos2(x, rect.center().y), 6.0, FG);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A small pill, such as "Draft" or "Failed".
pub fn chip(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::medium(11.0), fg);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 12.0, 18.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
}

/// A count in a soft pill, as on sidebar items and tabs.
pub fn paint_count(ui: &Ui, right_center: egui::Pos2, count: i64, strong: bool) -> f32 {
    let (fg, bg) = if strong { (ACCENT_TEXT, ACCENT_SOFT) } else { (FG_MUTED, BG_ACTIVE) };
    let text = if count > 999 { "999+".to_owned() } else { count.to_string() };
    let galley = ui.painter().layout_no_wrap(text, theme::medium(11.0), fg);
    let width = galley.size().x.max(8.0) + 10.0;
    let pill = Rect::from_center_size(right_center - vec2(width / 2.0, 0.0), vec2(width, 18.0));
    ui.painter().rect_filled(pill, CornerRadius::same(9), bg);
    ui.painter().galley(pill.center() - galley.size() / 2.0, galley, fg);
    width
}

/// The tinted rounded badge with a name's first letter, giving each sender a colour of their own.
pub fn paint_avatar(ui: &Ui, rect: Rect, name: &str) {
    let (bg, fg) = theme::tint(name);
    let size = rect.width();
    ui.painter().rect_filled(rect, CornerRadius::same((size * 0.3) as u8), bg);
    let initial = name.chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().to_string();
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, initial, theme::semibold(size * 0.44), fg);
}

/// Text on one line that ends in "…" rather than spilling past `width`.
pub fn paint_truncated(ui: &Ui, pos: egui::Pos2, text: &str, font: FontId, color: Color32, width: f32) -> Rect {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping::truncate_at_width(width.max(10.0));
    let galley = ui.painter().layout_job(job);
    let rect = Rect::from_min_size(pos, galley.size());
    ui.painter().galley(pos, galley, color);
    rect
}

pub fn label(ui: &mut Ui, text: impl Into<String>, font: FontId, color: Color32) -> Response {
    ui.label(egui::RichText::new(text).font(font).color(color))
}

/// A wrapping paragraph in the muted colour.
pub fn note(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text).font(theme::regular(12.0)).color(FG_MUTED)).wrap())
}

pub fn section_title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::medium(11.5)).color(FG_SUBTLE));
}

/// A text field in the house style: soft fill, hairline outline, accent outline while focused.
pub fn field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    field_with(ui, text, hint, width, |edit| edit)
}

pub fn password_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    field_with(ui, text, hint, width, |edit| edit.password(true))
}

pub fn field_with<'t>(
    ui: &mut Ui,
    text: &'t mut String,
    hint: &str,
    width: f32,
    configure: impl FnOnce(TextEdit<'t>) -> TextEdit<'t>,
) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let bg = ui.painter().add(egui::Shape::Noop);
    let edit = TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .hint_text(egui::RichText::new(hint).color(FG_FAINT))
        .font(theme::regular(13.0))
        .text_color(FG)
        .desired_width(width - 20.0)
        .align(egui::Align2::LEFT_CENTER);
    let response = ui.put(rect.shrink2(vec2(10.0, 4.0)), configure(edit));
    let stroke = if response.has_focus() { ACCENT } else { LINE_STRONG };
    ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(RADIUS), BG_SOFT));
    ui.painter().rect_stroke(rect, CornerRadius::same(RADIUS), Stroke::new(1.0, stroke), StrokeKind::Inside);
    response
}

/// A search box: magnifier, borderless field, and a clear button while it has text.
pub fn search_box(ui: &mut Ui, text: &mut String, hint: &str, width: f32, focus: bool) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let bg = ui.painter().add(egui::Shape::Noop);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.center().y), vec2(14.0, 14.0)),
        Icon::Search,
        FG_SUBTLE,
    );
    let clear_space = if text.is_empty() { 0.0 } else { 26.0 };
    let inner = Rect::from_min_max(
        egui::pos2(rect.left() + 30.0, rect.top() + 4.0),
        egui::pos2(rect.right() - 8.0 - clear_space, rect.bottom() - 4.0),
    );
    let edit = TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .hint_text(egui::RichText::new(hint).color(FG_FAINT))
        .font(theme::regular(13.0))
        .text_color(FG)
        .desired_width(inner.width())
        .align(egui::Align2::LEFT_CENTER);
    let response = ui.put(inner, edit);
    if focus {
        response.request_focus();
    }
    if !text.is_empty() {
        let clear = Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), vec2(20.0, 20.0));
        let clicked = ui.interact(clear, response.id.with("clear"), Sense::click());
        let color = if clicked.hovered() { FG } else { FG_SUBTLE };
        icons::paint(ui.painter(), clear.shrink(4.0), Icon::Close, color);
        if clicked.clicked() {
            text.clear();
        }
    }
    let stroke = if response.has_focus() { ACCENT } else { LINE_STRONG };
    ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(RADIUS), BG_SOFT));
    ui.painter().rect_stroke(rect, CornerRadius::same(RADIUS), Stroke::new(1.0, stroke), StrokeKind::Inside);
    response
}

/// Tabs as a segmented control, each with its count.
pub fn segmented<T: Copy + PartialEq>(ui: &mut Ui, current: &mut T, items: &[(T, &str, Option<i64>)]) -> bool {
    let mut changed = false;
    let font = theme::medium(12.0);
    let widths: Vec<f32> = items
        .iter()
        .map(|(_, label, count)| {
            let text = ui.painter().layout_no_wrap(label.to_string(), font.clone(), FG).size().x;
            text + 20.0 + count.filter(|n| *n > 0).map_or(0.0, |n| 10.0 + n.to_string().len() as f32 * 7.0 + 4.0)
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(RADIUS), BG_SOFT, Stroke::new(1.0, LINE), StrokeKind::Inside);
    let mut x = rect.left() + 2.0;
    for ((value, label, count), width) in items.iter().zip(widths) {
        let segment = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(width, 24.0));
        let response = ui.interact(segment, ui.scope_id().with(("segment", *label)), Sense::click());
        let selected = *current == *value;
        if selected {
            ui.painter().rect(
                segment,
                CornerRadius::same(5),
                BG_ACTIVE,
                Stroke::new(1.0, LINE_STRONG),
                StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter().rect_filled(segment, CornerRadius::same(5), BG_HOVER);
        }
        let color = if selected { FG } else { FG_MUTED };
        let galley = ui.painter().layout_no_wrap(label.to_string(), font.clone(), color);
        let text_pos = egui::pos2(segment.left() + 10.0, segment.center().y - galley.size().y / 2.0);
        let text_width = galley.size().x;
        ui.painter().galley(text_pos, galley, color);
        if let Some(n) = count.filter(|n| *n > 0) {
            let right =
                egui::pos2(text_pos.x + text_width + 8.0 + n.to_string().len() as f32 * 7.0 + 8.0, segment.center().y);
            paint_count(ui, right, n, selected);
        }
        if response.on_hover_cursor(CursorIcon::PointingHand).clicked() && !selected {
            *current = *value;
            changed = true;
        }
        x += width;
    }
    changed
}

/// The designed empty state: an icon in a rounded frame, a title, one sentence, and maybe an action.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, text: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(12), BG_PANEL, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(20.0, 20.0)), icon, FG_MUTED);
        ui.add_space(12.0);
        label(ui, title, theme::semibold(15.0), FG);
        ui.add_space(2.0);
        ui.add(egui::Label::new(egui::RichText::new(text).font(theme::regular(12.5)).color(FG_MUTED)).wrap());
        if let Some(action) = action {
            ui.add_space(12.0);
            clicked = button(ui, action, Kind::Secondary).clicked();
        }
    });
    clicked
}

/// A row in a menu or the command palette: icon, label, and a shortcut on the right.
pub fn menu_row(ui: &mut Ui, icon: Option<Icon>, text: &str, shortcut: &str, highlighted: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if highlighted || response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), if highlighted { BG_ACTIVE } else { BG_HOVER });
    }
    let mut x = rect.left() + 10.0;
    if let Some(icon) = icon {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)),
            icon,
            FG_MUTED,
        );
        x += 24.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, text, theme::regular(13.0), FG);
    if !shortcut.is_empty() {
        ui.painter().text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            shortcut,
            theme::medium(11.5),
            FG_SUBTLE,
        );
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn spinner(ui: &mut Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    let center = rect.center();
    let radius = size / 2.0 - 1.5;
    let points: Vec<egui::Pos2> = (0..=20)
        .map(|i| {
            let a = t * 5.0 + i as f32 / 20.0 * std::f32::consts::PI * 1.4;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(1.6, ACCENT_TEXT)));
    ui.ctx().request_repaint();
}
