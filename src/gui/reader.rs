use egui::{Align, CornerRadius, CursorIcon, Frame, Layout, Margin, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use rusqlite::Connection;

use super::html::{self, Page, Piece, Rendered};
use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets::{self, Kind};
use super::{Action, App};
use crate::mail::parse::{self, Body};
use crate::store::{self, Detail};

pub struct Reader {
    pub detail: Detail,
    body: Option<Body>,
    pub page: Option<Page>,
    pub plain: bool,
    mono: bool,
    /// The sender's own layout colours and images, on a white page.
    pub original: bool,
}

impl Reader {
    pub fn new(db: &Connection, detail: Detail, mono: bool, original: bool) -> Self {
        let mut reader = Self { detail, body: None, page: None, plain: false, mono, original };
        reader.load_body(db);
        reader
    }

    pub fn id(&self) -> i64 {
        self.detail.summary.id
    }

    pub fn refresh(&mut self, db: &Connection, detail: Detail) {
        self.detail = detail;
        if self.body.is_none() {
            self.load_body(db);
        }
    }

    pub fn toggle_plain(&mut self) {
        self.plain = !self.plain;
        self.rerender();
    }

    pub fn set_mono(&mut self, mono: bool) {
        if self.mono != mono {
            self.mono = mono;
            self.rerender();
        }
    }

    pub fn set_original(&mut self, original: bool) {
        if self.original != original {
            self.original = original;
            self.plain = false;
            self.rerender();
        }
    }

    pub fn has_html(&self) -> bool {
        self.body.as_ref().is_some_and(|body| body.html.is_some())
    }

    fn rerender(&mut self) {
        if let Some(body) = &self.body {
            self.page = Some(render(body, self.plain, self.mono, self.original));
        }
    }

    fn load_body(&mut self, db: &Connection) {
        if let Ok(Some(raw)) = store::raw(db, self.id()) {
            let body = parse::body(&raw);
            self.page = Some(render(&body, self.plain, self.mono, self.original));
            self.body = Some(body);
        }
    }
}

fn render(body: &Body, plain: bool, mono: bool, original: bool) -> Page {
    match (&body.html, &body.text) {
        (Some(html), _) if !plain => html::from_html(html, mono, original),
        (_, Some(text)) => html::from_text(text, mono),
        (Some(html), None) => html::from_html(html, mono, false),
        (None, None) => html::from_text("", mono),
    }
}

pub fn show(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let Some(reader) = &app.reader else {
        ui.add_space((ui.available_height() / 2.0 - 90.0).max(40.0));
        if widgets::empty_state(
            ui,
            Icon::Mail,
            "No message selected",
            "Pick one from the list, or press C to write something new.",
            Some("New message"),
        ) {
            out.push(Action::Compose);
        }
        return;
    };
    let summary = &reader.detail.summary;
    let id = summary.id;

    Frame::new().inner_margin(Margin { left: 16, right: 16, top: 10, bottom: 10 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            if widgets::icon_text_button(ui, Icon::Reply, "Reply", Kind::Ghost).on_hover_text("R").clicked() {
                out.push(Action::Reply(id, false));
            }
            if widgets::icon_text_button(ui, Icon::ReplyAll, "Reply all", Kind::Ghost).on_hover_text("A").clicked() {
                out.push(Action::Reply(id, true));
            }
            if widgets::icon_text_button(ui, Icon::Forward, "Forward", Kind::Ghost).on_hover_text("F").clicked() {
                out.push(Action::Forward(id));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let more = widgets::icon_button(ui, Icon::More, "More");
                egui::Popup::menu(&more).show(|ui| {
                    ui.set_min_width(220.0);
                    super::list::message_menu(app, ui, summary, out);
                    if reader.has_html() {
                        ui.separator();
                        let (label, action) = if reader.original {
                            ("Show simplified", Action::HideImages)
                        } else {
                            ("Show original, with images", Action::ShowImages { always: false })
                        };
                        if widgets::menu_row(ui, Some(Icon::Layers), label, "", false).clicked() {
                            out.push(action);
                            ui.close();
                        }
                        let label = if reader.plain { "Show formatted" } else { "Show plain text" };
                        if widgets::menu_row(ui, Some(Icon::File), label, "", false).clicked() {
                            out.push(Action::TogglePlain);
                            ui.close();
                        }
                        if widgets::menu_row(ui, Some(Icon::External), "Open in browser", "", false).clicked() {
                            out.push(Action::OpenInBrowser(id));
                            ui.close();
                        }
                    }
                });
                let flag = if summary.flagged { Icon::FlagFilled } else { Icon::Flag };
                let tint = summary.flagged.then_some(WARN);
                if widgets::icon_button_tinted(ui, flag, if summary.flagged { "Unflag (S)" } else { "Flag (S)" }, tint)
                    .clicked()
                {
                    out.push(Action::Flag(id, !summary.flagged));
                }
                let (read_icon, read_tip) = if summary.seen {
                    (Icon::Mail, "Mark as unread (U)")
                } else {
                    (Icon::MailOpen, "Mark as read (U)")
                };
                if widgets::icon_button(ui, read_icon, read_tip).clicked() {
                    out.push(Action::Seen(id, !summary.seen));
                }
                if widgets::icon_button(ui, Icon::Trash, "Delete (#)").clicked() {
                    out.push(Action::Delete(id));
                }
                if widgets::icon_button(ui, Icon::Archive, "Archive (E)").clicked() {
                    out.push(Action::Archive(id));
                }
            });
        });
    });
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, LINE));

    egui::ScrollArea::vertical().id_salt(("reader", id)).auto_shrink([false, false]).show(ui, |ui| {
        Frame::new().inner_margin(Margin { left: 32, right: 32, top: 22, bottom: 40 }).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(860.0));
            header(app, reader, ui, out);
            ui.add_space(18.0);
            body(app, reader, ui, out);
        });
    });
}

fn header(app: &App, reader: &Reader, ui: &mut Ui, out: &mut Vec<Action>) {
    let summary = &reader.detail.summary;
    let subject = if summary.subject.is_empty() { "(no subject)" } else { &summary.subject };
    ui.add(egui::Label::new(egui::RichText::new(subject).font(theme::semibold(20.0)).color(FG)).wrap());
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if let Some(folder) = app.folder(summary.folder) {
            widgets::chip(ui, &folder.name, FG_MUTED, BG_ACTIVE);
        }
        if app.settings.accounts.len() > 1
            && let Some(account) = app.settings.account(&summary.account)
        {
            widgets::chip(ui, account.label(), FG_MUTED, BG_ACTIVE);
        }
        if summary.flagged {
            widgets::chip(ui, "Flagged", WARN, WARN.gamma_multiply(0.14));
        }
        if summary.answered {
            widgets::chip(ui, "Replied", ACCENT_TEXT, ACCENT_SOFT);
        }
    });
    ui.add_space(14.0);

    let me = app.settings.account(&summary.account).map(|account| account.email.to_lowercase()).unwrap_or_default();
    ui.horizontal(|ui| {
        let (avatar, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
        widgets::paint_avatar(ui, avatar, summary.sender());
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                widgets::label(ui, summary.sender(), theme::semibold(13.5), FG);
                if !summary.from_name.is_empty() {
                    widgets::label(ui, format!("<{}>", summary.from_addr), theme::regular(12.5), FG_SUBTLE);
                }
            });
            let mut people: Vec<String> = summary
                .to
                .iter()
                .chain(reader.detail.cc.iter())
                .map(|(name, addr)| {
                    if addr.to_lowercase() == me {
                        "me".to_owned()
                    } else if name.is_empty() {
                        addr.clone()
                    } else {
                        name.clone()
                    }
                })
                .collect();
            let extra = people.len().saturating_sub(4);
            people.truncate(4);
            let mut to = format!("to {}", people.join(", "));
            if extra > 0 {
                to.push_str(&format!(" and {extra} more"));
            }
            let all: Vec<String> =
                summary.to.iter().chain(reader.detail.cc.iter()).map(crate::mail::compose::format_person).collect();
            widgets::label(ui, to, theme::regular(12.0), FG_MUTED).on_hover_text(all.join("\n"));
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            widgets::label(ui, super::long_date(summary.date), theme::regular(12.0), FG_SUBTLE);
        });
    });

    let Some(body) = &reader.body else { return };
    if let Some(link) =
        body.list_unsubscribe.as_deref().filter(|link| link.starts_with("http") || link.starts_with("mailto:"))
    {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(44.0);
            if widgets::small_button(ui, "Unsubscribe", Kind::Ghost).on_hover_text(link).clicked() {
                out.push(Action::OpenUrl(link.to_owned()));
            }
        });
    }
    let files: Vec<_> = body.attachments.iter().filter(|attachment| !attachment.inline).collect();
    if !files.is_empty() {
        ui.add_space(14.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for attachment in files {
                attachment_chip(ui, summary.id, attachment, out);
            }
        });
    }
}

fn attachment_chip(ui: &mut Ui, id: i64, attachment: &parse::Attachment, out: &mut Vec<Action>) {
    let name_galley = ui.painter().layout_no_wrap(attachment.name.clone(), theme::medium(12.5), FG);
    let size = super::human_size(attachment.size as u64);
    let size_galley = ui.painter().layout_no_wrap(size, theme::regular(11.5), FG_SUBTLE);
    let width = (name_galley.size().x.min(220.0) + size_galley.size().x + 86.0).max(160.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
    let hovered = response.hovered();
    ui.painter().rect(
        rect,
        CornerRadius::same(8),
        if hovered { BG_HOVER } else { BG_PANEL },
        Stroke::new(1.0, LINE_STRONG),
        StrokeKind::Inside,
    );
    let icon = Rect::from_min_size(rect.left_top() + vec2(8.0, 8.0), vec2(24.0, 24.0));
    ui.painter().rect_filled(icon, CornerRadius::same(6), BG_ACTIVE);
    icons::paint(ui.painter(), icon.shrink(5.0), Icon::Paperclip, FG_MUTED);
    widgets::paint_truncated(
        ui,
        rect.left_top() + vec2(40.0, 5.0),
        &attachment.name,
        theme::medium(12.5),
        FG,
        width - 80.0,
    );
    ui.painter().galley(rect.left_top() + vec2(40.0, 22.0), size_galley, FG_SUBTLE);

    let save = Rect::from_center_size(rect.right_center() - vec2(20.0, 0.0), vec2(26.0, 26.0));
    let save_response = ui.interact(save, ui.scope_id().with(("save", id, attachment.index)), Sense::click());
    if save_response.hovered() {
        ui.painter().rect_filled(save, CornerRadius::same(5), BG_PRESSED);
    }
    icons::paint(ui.painter(), save.shrink(6.0), Icon::Download, if save_response.hovered() { FG } else { FG_MUTED });
    if save_response.on_hover_text("Save as…").on_hover_cursor(CursorIcon::PointingHand).clicked() {
        out.push(Action::SaveAttachment(id, attachment.index));
    } else if response
        .on_hover_text(format!("Open {} ({})", attachment.name, attachment.mime))
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        out.push(Action::OpenAttachment(id, attachment.index));
    }
}

fn body(app: &App, reader: &Reader, ui: &mut Ui, out: &mut Vec<Action>) {
    let (Some(body), Some(page)) = (&reader.body, &reader.page) else {
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            widgets::spinner(ui, 14.0);
            widgets::label(ui, "Downloading this message…", theme::regular(13.0), FG_MUTED);
        });
        return;
    };
    let has_images = body.html.as_deref().is_some_and(|html| html.contains("<img"));
    if has_images && !page.original && !reader.plain {
        images_banner(reader, ui, out);
        ui.add_space(14.0);
    }
    if page.is_blank() {
        widgets::label(ui, "This message has no text.", theme::regular(13.0), FG_SUBTLE);
        return;
    }
    if !page.original {
        pieces(app, page, ui, out);
        return;
    }
    Frame::new().fill(html::PAPER).corner_radius(10).inner_margin(Margin::same(24)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        pieces(app, page, ui, out);
    });
}

fn images_banner(reader: &Reader, ui: &mut Ui, out: &mut Vec<Action>) {
    let sender = reader.detail.summary.sender().to_owned();
    Frame::new()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let (icon, _) = ui.allocate_exact_size(vec2(16.0, 24.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(icon.center(), vec2(14.0, 14.0)),
                    Icon::Alert,
                    FG_SUBTLE,
                );
                widgets::label(
                    ui,
                    "Images are hidden so the sender can't tell you've opened this.",
                    theme::regular(12.0),
                    FG_MUTED,
                );
                if widgets::small_button(ui, "Show images", Kind::Accent).clicked() {
                    out.push(Action::ShowImages { always: false });
                }
                if widgets::small_button(ui, &format!("Always for {sender}"), Kind::Ghost).clicked() {
                    out.push(Action::ShowImages { always: true });
                }
                if widgets::small_button(ui, "Open in browser", Kind::Ghost).clicked() {
                    out.push(Action::OpenInBrowser(reader.id()));
                }
            });
        });
}

fn pieces(app: &App, page: &Page, ui: &mut Ui, out: &mut Vec<Action>) {
    for piece in &page.pieces {
        match piece {
            Piece::Text(text) => text_piece(text, ui, out),
            Piece::Image { src, link } => image_piece(app, src, link.as_deref(), ui, out),
        }
    }
}

fn image_piece(app: &App, src: &str, link: Option<&str>, ui: &mut Ui, out: &mut Vec<Action>) {
    match app.image(src) {
        Some(super::ImageState::Ready(uri)) => {
            ui.add_space(6.0);
            let image = egui::Image::new(uri).fit_to_original_size(1.0).max_width(ui.available_width());
            let response = ui.add(image.sense(if link.is_some() { Sense::click() } else { Sense::hover() }));
            if let Some(link) = link {
                let response = response.on_hover_text_at_pointer(link).on_hover_cursor(CursorIcon::PointingHand);
                if response.clicked() {
                    out.push(Action::OpenUrl(link.to_owned()));
                }
            }
            ui.add_space(6.0);
        }
        Some(super::ImageState::Loading) | None => {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(220.0), 28.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(6), html::PAPER.gamma_multiply(0.94).to_opaque());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        Some(super::ImageState::Failed) => {}
    }
}

fn text_piece(rendered: &Rendered, ui: &mut Ui, out: &mut Vec<Action>) {
    let mut job = rendered.job.clone();
    job.wrap.max_width = ui.available_width();
    let galley = ui.painter().layout_job(job.clone());
    let response = ui.add(egui::Label::new(job).selectable(true));
    if let Some(pos) = response.hover_pos() {
        let cursor = galley.cursor_from_pos(pos - response.rect.min);
        if let Some(url) = rendered.link_at(cursor.index.into()) {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            let url = url.to_owned();
            let clicked = response.clicked();
            response.on_hover_text_at_pointer(&url);
            if clicked {
                out.push(Action::OpenUrl(url));
            }
        }
    }
}
