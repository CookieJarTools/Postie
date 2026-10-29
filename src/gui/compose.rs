use egui::{
    Align, Align2, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Modifiers, Rect, Sense, Stroke, TextEdit,
    Ui, vec2,
};
use rusqlite::Connection;

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets::{self, Kind};
use super::{Action, App};
use crate::config::Settings;
use crate::mail::compose::{Draft, split_recipients};
use crate::store;

const LABEL_WIDTH: f32 = 64.0;
const AUTOSAVE_SECONDS: f64 = 3.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Field {
    To,
    Cc,
    Bcc,
    Body,
}

pub struct Composer {
    pub draft: Draft,
    pub draft_id: Option<i64>,
    pub error: Option<String>,
    title: String,
    show_cc: bool,
    saved: Draft,
    last_save: f64,
    initial_focus: Option<Field>,
    suggestions: Vec<(String, String)>,
    suggest_query: String,
    highlighted: usize,
}

impl Composer {
    pub fn new(draft: Draft, draft_id: Option<i64>, title: &str) -> Self {
        let initial_focus = if draft.to.trim().is_empty() { Field::To } else { Field::Body };
        Self {
            show_cc: !draft.cc.is_empty() || !draft.bcc.is_empty(),
            saved: draft.clone(),
            draft,
            draft_id,
            error: None,
            title: title.to_owned(),
            last_save: 0.0,
            initial_focus: Some(initial_focus),
            suggestions: Vec::new(),
            suggest_query: String::new(),
            highlighted: 0,
        }
    }

    /// Keeps it in Drafts. False when there was nothing worth keeping.
    pub fn save(&self, db: &Connection) -> bool {
        if self.draft.is_empty() {
            self.discard(db);
            return false;
        }
        let json = serde_json::to_string(&self.draft).unwrap_or_default();
        store::save_draft(db, self.draft_id, &json).is_ok()
    }

    pub fn discard(&self, db: &Connection) {
        if let Some(id) = self.draft_id {
            let _ = store::delete_draft(db, id);
        }
    }

    fn autosave(&mut self, db: &Connection, now: f64) {
        if self.draft == self.saved || now - self.last_save < AUTOSAVE_SECONDS || self.draft.is_empty() {
            return;
        }
        let json = serde_json::to_string(&self.draft).unwrap_or_default();
        if let Ok(id) = store::save_draft(db, self.draft_id, &json) {
            self.draft_id = Some(id);
            self.saved = self.draft.clone();
            self.last_save = now;
        }
    }
}

pub fn window(app: &mut App, ctx: &egui::Context, out: &mut Vec<Action>) {
    let App { composer, settings, db, .. } = app;
    let Some(composer) = composer.as_mut() else { return };
    let now = ctx.input(|i| i.time);
    composer.autosave(db, now);

    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        out.push(Action::Send);
    }
    if composer.suggestions.is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        out.push(Action::CloseComposer);
    }

    let screen = ctx.content_rect();
    let size = vec2(screen.width().min(760.0) - 40.0, (screen.height() - 80.0).min(680.0));
    let frame = Frame::new()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0, LINE_STRONG))
        .corner_radius(12)
        .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: egui::Color32::from_black_alpha(170) });
    egui::Window::new("composer")
        .id(Id::unique("composer"))
        .title_bar(false)
        .frame(frame)
        .fixed_size(size)
        .collapsible(false)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(screen.center())
        .show(ctx, |ui| contents(composer, settings, db, ui, size, out));
}

fn contents(
    composer: &mut Composer,
    settings: &Settings,
    db: &Connection,
    ui: &mut Ui,
    size: egui::Vec2,
    out: &mut Vec<Action>,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    Frame::new().inner_margin(Margin { left: 18, right: 10, top: 10, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            widgets::label(ui, &composer.title, theme::semibold(14.0), FG);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Close, "Close and keep as draft (Esc)").clicked() {
                    out.push(Action::CloseComposer);
                }
            });
        });
    });
    hairline(ui);

    let account = settings.account(&composer.draft.account).cloned();
    field_row(ui, "From", |ui| {
        let label = account.as_ref().map(|account| {
            if account.name.is_empty() {
                account.email.clone()
            } else {
                format!("{} <{}>", account.name, account.email)
            }
        });
        let response = account_picker(ui, label.as_deref().unwrap_or("Choose an account"), settings.accounts.len() > 1);
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(280.0);
            for option in &settings.accounts {
                let selected = option.id == composer.draft.account;
                if widgets::menu_row(ui, Some(Icon::Mail), option.label(), "", selected).clicked() {
                    switch_signature(&mut composer.draft, account.as_ref(), option);
                    composer.draft.account = option.id.clone();
                    ui.close();
                }
            }
        });
    });
    hairline(ui);

    recipients(composer, db, ui, Field::To, "To");
    hairline(ui);
    if composer.show_cc {
        recipients(composer, db, ui, Field::Cc, "Cc");
        hairline(ui);
        recipients(composer, db, ui, Field::Bcc, "Bcc");
        hairline(ui);
    }
    field_row(ui, "Subject", |ui| {
        ui.add(
            TextEdit::singleline(&mut composer.draft.subject)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .min_size(vec2(0.0, 26.0))
                .align(Align2::LEFT_CENTER)
                .font(theme::medium(13.5))
                .text_color(FG)
                .hint_text(egui::RichText::new("What's it about?").color(FG_FAINT))
                .desired_width(f32::INFINITY),
        );
    });
    hairline(ui);

    let attachments_height = if composer.draft.attachments.is_empty() { 0.0 } else { 48.0 };
    let error_height = if composer.error.is_some() { 30.0 } else { 0.0 };
    let used = ui.min_rect().height();
    let body_height = (size.y - used - 56.0 - attachments_height - error_height).max(120.0);
    let body_id = Id::unique("compose-body");
    Frame::new().inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
        egui::ScrollArea::vertical().max_height(body_height - 24.0).auto_shrink([false, false]).show(ui, |ui| {
            let font = if settings.monospace_bodies { egui::FontId::monospace(13.0) } else { theme::regular(13.5) };
            let edit = TextEdit::multiline(&mut composer.draft.body)
                .id(body_id)
                .frame(Frame::NONE)
                .font(font)
                .text_color(FG)
                .lock_focus(true)
                .desired_width(f32::INFINITY);
            ui.add_sized([ui.available_width(), body_height - 24.0], edit);
        });
    });
    if composer.initial_focus == Some(Field::Body) {
        composer.initial_focus = None;
        let mut state = TextEdit::load_state(ui.ctx(), body_id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(0))));
        state.store(ui.ctx(), body_id);
        ui.memory_mut(|memory| memory.request_focus(body_id));
    }

    if !composer.draft.attachments.is_empty() {
        Frame::new().inner_margin(Margin::symmetric(18, 6)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let mut remove = None;
                for (index, attachment) in composer.draft.attachments.iter().enumerate() {
                    if attachment_pill(ui, &attachment.name, attachment.size) {
                        remove = Some(index);
                    }
                }
                if let Some(index) = remove {
                    composer.draft.attachments.remove(index);
                }
            });
        });
    }
    if let Some(error) = &composer.error {
        Frame::new().inner_margin(Margin::symmetric(18, 4)).show(ui, |ui| {
            widgets::label(ui, error, theme::regular(12.5), DANGER);
        });
    }
    hairline(ui);
    Frame::new().inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if widgets::icon_text_button(ui, Icon::Paperclip, "Attach", Kind::Ghost).clicked() {
                out.push(Action::Attach);
            }
            if !composer.show_cc && widgets::button(ui, "Cc / Bcc", Kind::Ghost).clicked() {
                composer.show_cc = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_text_button(ui, Icon::Send, "Send", Kind::Primary)
                    .on_hover_text("Ctrl+Enter")
                    .clicked()
                {
                    out.push(Action::Send);
                }
                if widgets::button(ui, "Discard", Kind::Ghost).clicked() {
                    out.push(Action::DiscardComposer);
                }
                if let Some(id) = composer.draft_id {
                    let _ = id;
                    widgets::label(ui, "Draft saved", theme::regular(11.5), FG_FAINT);
                }
            });
        });
    });
}

/// Moves the signature along when the From account changes, if it's still where it was put.
fn switch_signature(draft: &mut Draft, from: Option<&crate::config::Account>, to: &crate::config::Account) {
    let old = from.map(crate::mail::compose::signature_block).unwrap_or_default();
    let new = crate::mail::compose::signature_block(to);
    if !old.is_empty() && draft.body.contains(&old) {
        draft.body = draft.body.replacen(&old, &new, 1);
    } else if old.is_empty() && !new.is_empty() && draft.body.trim().is_empty() {
        draft.body = new;
    }
}

fn hairline(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    let (line, _) = ui.allocate_exact_size(vec2(rect.width(), 1.0), Sense::hover());
    ui.painter().hline(line.x_range(), line.center().y, Stroke::new(1.0, LINE));
}

fn field_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    Frame::new().inner_margin(Margin { left: 18, right: 14, top: 6, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.set_min_height(26.0);
            let (rect, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 26.0), Sense::hover());
            ui.painter().text(rect.left_center(), Align2::LEFT_CENTER, label, theme::regular(12.5), FG_SUBTLE);
            add(ui);
        });
    });
}

fn account_picker(ui: &mut Ui, label: &str, choosable: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), theme::medium(13.0), FG);
    let width = galley.size().x + if choosable { 34.0 } else { 12.0 };
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, 26.0), if choosable { Sense::click() } else { Sense::hover() });
    if choosable && response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), BG_HOVER);
    }
    ui.painter().galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, FG);
    if choosable {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(rect.right_center() - vec2(13.0, 0.0), vec2(12.0, 12.0)),
            Icon::ChevronDown,
            FG_MUTED,
        );
        return response.on_hover_cursor(CursorIcon::PointingHand);
    }
    response
}

fn recipients(composer: &mut Composer, db: &Connection, ui: &mut Ui, field: Field, label: &str) {
    let id = Id::unique(("compose-field", field as u8));
    let mut response = None;
    field_row(ui, label, |ui| {
        let text = match field {
            Field::To => &mut composer.draft.to,
            Field::Cc => &mut composer.draft.cc,
            _ => &mut composer.draft.bcc,
        };
        let edit = TextEdit::singleline(text)
            .id(id)
            .frame(Frame::NONE)
            .margin(Margin::ZERO)
            .min_size(vec2(0.0, 26.0))
            .align(Align2::LEFT_CENTER)
            .font(theme::regular(13.5))
            .text_color(FG)
            .hint_text(
                egui::RichText::new(if field == Field::To { "Name or email, comma between people" } else { "" })
                    .color(FG_FAINT),
            )
            .desired_width(f32::INFINITY);
        response = Some(ui.add(edit));
    });
    let Some(response) = response else { return };
    if composer.initial_focus == Some(field) {
        composer.initial_focus = None;
        response.request_focus();
    }
    if !response.has_focus() {
        return;
    }

    let text = match field {
        Field::To => composer.draft.to.clone(),
        Field::Cc => composer.draft.cc.clone(),
        _ => composer.draft.bcc.clone(),
    };
    let token = text.rsplit([',', ';']).next().unwrap_or("").trim().to_owned();
    if token != composer.suggest_query {
        composer.suggest_query = token.clone();
        composer.highlighted = 0;
        composer.suggestions = if token.chars().count() >= 1 {
            store::suggest_contacts(db, &token, 6).unwrap_or_default()
        } else {
            Vec::new()
        };
        let already: Vec<String> = split_recipients(&text).iter().map(|item| item.to_lowercase()).collect();
        composer
            .suggestions
            .retain(|(_, addr)| !already.iter().any(|item| item.contains(addr.as_str())) || addr.starts_with(&token));
    }
    if composer.suggestions.is_empty() {
        return;
    }

    let (down, up, accept, dismiss) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Tab) || i.key_pressed(Key::Enter),
            i.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if down {
        composer.highlighted = (composer.highlighted + 1).min(composer.suggestions.len() - 1);
    }
    if up {
        composer.highlighted = composer.highlighted.saturating_sub(1);
    }
    if dismiss {
        composer.suggestions.clear();
        return;
    }
    let mut chosen = accept.then_some(composer.highlighted);

    let anchor = egui::pos2(response.rect.left() - 6.0, response.rect.bottom() + 8.0);
    egui::Area::new(Id::unique(("suggestions", field as u8))).fixed_pos(anchor).order(egui::Order::Foreground).show(
        ui.ctx(),
        |ui| {
            Frame::new()
                .fill(BG_PANEL)
                .stroke(Stroke::new(1.0, LINE_STRONG))
                .corner_radius(8)
                .inner_margin(4)
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(140),
                })
                .show(ui, |ui| {
                    ui.set_width(360.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, (name, addr)) in composer.suggestions.iter().enumerate() {
                        let (rect, row) = ui.allocate_exact_size(vec2(360.0, 40.0), Sense::click());
                        if index == composer.highlighted || row.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), BG_ACTIVE);
                        }
                        let display = if name.is_empty() { addr.as_str() } else { name.as_str() };
                        let avatar = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(24.0, 24.0));
                        widgets::paint_avatar(ui, avatar, display);
                        widgets::paint_truncated(
                            ui,
                            rect.left_top() + vec2(38.0, 4.0),
                            display,
                            theme::medium(13.0),
                            FG,
                            300.0,
                        );
                        widgets::paint_truncated(
                            ui,
                            rect.left_top() + vec2(38.0, 21.0),
                            addr,
                            theme::regular(11.5),
                            FG_SUBTLE,
                            300.0,
                        );
                        if row.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                            chosen = Some(index);
                        }
                    }
                });
        },
    );

    if let Some(index) = chosen {
        let person = composer.suggestions[index].clone();
        let formatted = crate::mail::compose::format_person(&person);
        let target = match field {
            Field::To => &mut composer.draft.to,
            Field::Cc => &mut composer.draft.cc,
            _ => &mut composer.draft.bcc,
        };
        let keep = match target.rfind([',', ';']) {
            Some(cut) => format!("{}, ", target[..cut].trim_end()),
            None => String::new(),
        };
        *target = format!("{keep}{formatted}, ");
        composer.suggestions.clear();
        composer.suggest_query = String::new();
        let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        let end = target.chars().count();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
        state.store(ui.ctx(), id);
        ui.memory_mut(|memory| memory.request_focus(id));
    }
}

/// An attached file; true when its remove button was clicked.
fn attachment_pill(ui: &mut Ui, name: &str, size: u64) -> bool {
    let text = format!("{name}  ·  {}", super::human_size(size));
    let galley = ui.painter().layout_no_wrap(text, theme::regular(12.0), FG_MUTED);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x.min(260.0) + 54.0, 30.0), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(7), BG_SOFT, Stroke::new(1.0, LINE_STRONG), egui::StrokeKind::Inside);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(13.0, 13.0)),
        Icon::Paperclip,
        FG_SUBTLE,
    );
    widgets::paint_truncated(
        ui,
        rect.left_top() + vec2(28.0, 7.0),
        galley.text(),
        theme::regular(12.0),
        FG_MUTED,
        rect.width() - 54.0,
    );
    let close = Rect::from_center_size(rect.right_center() - vec2(14.0, 0.0), vec2(18.0, 18.0));
    let response = ui.interact(close, ui.scope_id().with(("remove-attachment", name)), Sense::click());
    icons::paint(ui.painter(), close.shrink(4.0), Icon::Close, if response.hovered() { FG } else { FG_SUBTLE });
    response.on_hover_text("Remove").on_hover_cursor(CursorIcon::PointingHand).clicked()
}
