use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Align2, CornerRadius, CursorIcon, Frame, Layout, Margin, Rect, Response, Sense, Stroke, Ui, vec2};

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets::{self, Kind};
use super::{Action, App, Nav};
use crate::store::{Category, Filter, Role, Summary, View};

const ROW: f32 = 60.0;

pub fn show(app: &mut App, ui: &mut Ui, out: &mut Vec<Action>) {
    Frame::new().inner_margin(Margin { left: 16, right: 12, top: 14, bottom: 10 }).show(ui, |ui| header(app, ui, out));
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), Stroke::new(1.0, LINE));
    ui.add_space(4.0);

    match app.nav.clone() {
        Nav::Drafts => drafts(app, ui, out),
        Nav::Outbox => outbox(app, ui, out),
        Nav::Mailbox(view) => messages(app, ui, out, &view),
    }
}

fn header(app: &mut App, ui: &mut Ui, out: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        widgets::label(ui, app.nav_title(), theme::semibold(16.0), FG);
        let unread = view_unread(app);
        if unread > 0 {
            widgets::label(ui, format!("{unread} unread"), theme::regular(12.0), FG_SUBTLE);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let more = widgets::icon_button(ui, Icon::More, "More");
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(200.0);
                if widgets::menu_row(ui, Some(Icon::Check), "Mark all as read", "", false).clicked() {
                    out.push(Action::MarkAllRead);
                    ui.close();
                }
                if widgets::menu_row(ui, Some(Icon::Refresh), "Check for new mail", "F5", false).clicked() {
                    out.push(Action::SyncNow);
                    ui.close();
                }
            });
        });
    });
    if !matches!(app.nav, Nav::Mailbox(_)) {
        return;
    }
    ui.add_space(10.0);
    let focus = std::mem::take(&mut app.focus_search);
    let response = widgets::search_box(ui, &mut app.search, "Search mail", ui.available_width(), focus);
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.search.clear();
        response.surrender_focus();
    }
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        response.surrender_focus();
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if !app.search.trim().is_empty() {
            let mut everywhere = app.search_everywhere;
            let scopes = [(true, "Everywhere", None), (false, "This folder", None)];
            if widgets::segmented(ui, &mut everywhere, &scopes) {
                out.push(Action::SearchEverywhere(everywhere));
            }
        } else if app.inbox_split() {
            let (primary, other) = split_unread(app);
            let mut category = app.category;
            let tabs = [(Category::Primary, "Primary", Some(primary)), (Category::Other, "Other", Some(other))];
            if widgets::segmented(ui, &mut category, &tabs) {
                out.push(Action::SetCategoryTab(category));
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| filter_menu(app, ui, out));
    });
}

/// The All / Unread / Flagged / Files filter, folded into one button so the tabs keep the room.
fn filter_menu(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let label = match app.filter {
        Filter::All => "All mail",
        Filter::Unread => "Unread",
        Filter::Flagged => "Flagged",
        Filter::Attachments => "With files",
    };
    let kind = if app.filter == Filter::All { Kind::Ghost } else { Kind::Accent };
    let button = widgets::icon_text_button(ui, Icon::ChevronDown, label, kind);
    egui::Popup::menu(&button).show(|ui| {
        ui.set_min_width(170.0);
        for (filter, text, icon) in [
            (Filter::All, "All mail", Icon::Inbox),
            (Filter::Unread, "Unread", Icon::Mail),
            (Filter::Flagged, "Flagged", Icon::Flag),
            (Filter::Attachments, "With files", Icon::Paperclip),
        ] {
            if widgets::menu_row(ui, Some(icon), text, "", app.filter == filter).clicked() {
                out.push(Action::SetFilter(filter));
                ui.close();
            }
        }
    });
}

/// Unread in Primary and in Other for the inbox being shown.
fn split_unread(app: &App) -> (i64, i64) {
    let inboxes: Vec<_> = match &app.nav {
        Nav::Mailbox(View::Folder(id)) => app.folder(*id).into_iter().collect(),
        _ => app.folders.iter().filter(|folder| folder.role == Role::Inbox).collect(),
    };
    let primary: i64 = inboxes.iter().map(|folder| folder.unread_primary).sum();
    let all: i64 = inboxes.iter().map(|folder| folder.unread).sum();
    (primary, all - primary)
}

fn view_unread(app: &App) -> i64 {
    match &app.nav {
        Nav::Mailbox(View::Folder(id)) => app.folder(*id).map_or(0, |folder| {
            if folder.role == Role::Inbox && app.settings.split_inbox { folder.unread_primary } else { folder.unread }
        }),
        Nav::Mailbox(View::Unified(Role::Inbox)) => app.unified_unread(),
        _ => 0,
    }
}

fn messages(app: &App, ui: &mut Ui, out: &mut Vec<Action>, view: &View) {
    if app.settings.accounts.is_empty() {
        ui.add_space(80.0);
        if widgets::empty_state(
            ui,
            Icon::Mail,
            "Add your first account",
            "Postie syncs it here and tells you when new mail arrives.",
            Some("Add account"),
        ) {
            out.push(Action::AddAccount);
        }
        return;
    }
    if app.messages.is_empty() {
        ui.add_space(80.0);
        empty(app, ui, view);
        ui.add_space(20.0);
        older_on_server(app, ui, out);
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut group = String::new();
        for message in &app.messages {
            let this = date_group(message.date);
            if this != group {
                ui.add_space(if group.is_empty() { 4.0 } else { 10.0 });
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    widgets::label(ui, &this, theme::medium(11.5), FG_SUBTLE);
                });
                ui.add_space(4.0);
                group = this;
            }
            row(app, ui, message, out);
        }
        if app.messages.len() as i64 >= app.limit {
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                if widgets::button(ui, "Show older messages", Kind::Ghost).clicked() {
                    out.push(Action::LoadMore);
                }
            });
        } else {
            ui.add_space(16.0);
            older_on_server(app, ui, out);
        }
        ui.add_space(16.0);
    });
}

/// The end of what's on this computer: how much older mail the server still has, and buttons to
/// fetch some or all of it. While searching it says the search couldn't look there.
fn older_on_server(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let scope = app.download_scope();
    let remaining: i64 = scope.iter().map(|folder| folder.on_server_only()).sum();
    let downloading: i64 = scope.iter().map(|folder| folder.downloading()).sum();
    if remaining == 0 {
        return;
    }
    ui.vertical_centered(|ui| {
        ui.set_max_width(ui.available_width() - 40.0);
        if downloading > 0 {
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 250.0).max(0.0) / 2.0);
                widgets::spinner(ui, 12.0);
                let text = format!("Downloading {} older emails…", super::thousands(downloading));
                widgets::label(ui, text, theme::regular(12.5), FG_MUTED);
            });
            return;
        }
        let many = super::thousands(remaining);
        let text = if app.search.trim().is_empty() {
            format!("{many} older emails are still on the server.")
        } else {
            format!("This search only looked at emails on this computer. {many} older ones are still on the server.")
        };
        ui.add(egui::Label::new(egui::RichText::new(text).font(theme::regular(12.5)).color(FG_SUBTLE)).wrap());
        ui.add_space(8.0);
        let several = scope.iter().filter(|folder| folder.on_server_only() > 0).count() > 1;
        // With a batch or less to go, "more" and "all" would be the same button.
        if !several && remaining <= super::OLDER_BATCH {
            let all = format!("Download all {many}");
            let width = ui.painter().layout_no_wrap(all.clone(), theme::medium(13.0), FG).size().x + 24.0;
            ui.allocate_ui_with_layout(vec2(width, 28.0), Layout::left_to_right(Align::Center), |ui| {
                if widgets::button(ui, &all, Kind::Secondary).clicked() {
                    out.push(Action::DownloadOlder { all: true });
                }
            });
            return;
        }
        let more = if several {
            format!("Download {} more from each", super::OLDER_BATCH)
        } else {
            format!("Download {} more", super::OLDER_BATCH)
        };
        let width = ui.painter().layout_no_wrap(more.clone(), theme::medium(13.0), FG).size().x + 140.0;
        ui.allocate_ui_with_layout(vec2(width, 28.0), Layout::left_to_right(Align::Center), |ui| {
            if widgets::button(ui, &more, Kind::Secondary).clicked() {
                out.push(Action::DownloadOlder { all: false });
            }
            if widgets::button(ui, "Download all", Kind::Ghost).clicked() {
                out.push(Action::DownloadOlder { all: true });
            }
        });
    });
}

fn empty(app: &App, ui: &mut Ui, view: &View) {
    let syncing = app.statuses.iter().any(|status| status.state == "syncing" || status.state == "connecting");
    if !app.search.trim().is_empty() {
        widgets::empty_state(
            ui,
            Icon::Search,
            "No messages match",
            "Try fewer words, or from:, to:, subject:, is:unread and has:attachment.",
            None,
        );
    } else if app.inbox_split() && app.category == Category::Other && app.filter == Filter::All {
        widgets::empty_state(
            ui,
            Icon::Layers,
            "Nothing in Other",
            "Newsletters, mailing lists and automated mail land here, so Primary stays for people.",
            None,
        );
    } else if app.filter == Filter::Unread {
        widgets::empty_state(ui, Icon::Check, "All caught up", "Nothing unread here.", None);
    } else if app.filter != Filter::All {
        widgets::empty_state(ui, Icon::Flag, "Nothing here", "No messages match this filter.", None);
    } else if syncing && app.folders.is_empty() {
        widgets::empty_state(
            ui,
            Icon::Refresh,
            "Downloading your mail",
            "The newest messages come first, so this fills in quickly.",
            None,
        );
    } else if matches!(view, View::Unified(Role::Inbox))
        || matches!(view, View::Folder(id) if app.folder(*id).is_some_and(|f| f.role == Role::Inbox))
    {
        widgets::empty_state(
            ui,
            Icon::Inbox,
            "Inbox zero",
            "Nothing waiting. New mail shows up here and as a notification.",
            None,
        );
    } else if matches!(view, View::Flagged) {
        widgets::empty_state(
            ui,
            Icon::Flag,
            "No flagged messages",
            "Flag a message with S to keep it here until it's dealt with.",
            None,
        );
    } else {
        widgets::empty_state(
            ui,
            Icon::Folder,
            "Nothing in this folder",
            "Messages moved here will show up in this list.",
            None,
        );
    }
}

fn date_group(timestamp: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(date) = Local.timestamp_opt(timestamp, 0).single() else { return "Older".into() };
    let today = Local::now().date_naive();
    let day = date.date_naive();
    let days = (today - day).num_days();
    let week_start = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    if days <= 0 {
        "Today".into()
    } else if days == 1 {
        "Yesterday".into()
    } else if day >= week_start {
        "This week".into()
    } else if day >= week_start - chrono::Duration::days(7) {
        "Last week".into()
    } else if day.year() == today.year() && day.month() == today.month() {
        "Earlier this month".into()
    } else if day.year() == today.year() {
        date.format("%B").to_string()
    } else {
        date.format("%B %Y").to_string()
    }
}

fn row(app: &App, ui: &mut Ui, message: &Summary, out: &mut Vec<Action>) {
    let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if !ui.is_rect_visible(outer) {
        return;
    }
    let rect = outer.shrink2(vec2(8.0, 1.0));
    let selected = app.selected == Some(message.id);
    let hovered = response.hovered();
    if selected || hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), if selected { BG_ACTIVE } else { BG_HOVER });
    }
    let unread = !message.seen;
    if unread {
        ui.painter().circle_filled(egui::pos2(rect.left() + 8.0, rect.top() + 19.0), 3.5, ACCENT);
    }
    let in_sent = app.folder(message.folder).is_some_and(|folder| matches!(folder.role, Role::Sent | Role::Drafts));
    // In Sent and Drafts the person that matters is who it went to, which a draft may not have yet.
    let recipient = if in_sent {
        message.to.first().map(|(name, addr)| if name.is_empty() { addr.clone() } else { name.clone() })
    } else {
        None
    };
    let who = match &recipient {
        Some(to) => format!("To {to}"),
        None if in_sent => "No recipients yet".to_owned(),
        None if message.sender().is_empty() => "(unknown sender)".to_owned(),
        None => message.sender().to_owned(),
    };

    let avatar = Rect::from_min_size(egui::pos2(rect.left() + 16.0, rect.top() + 11.0), vec2(30.0, 30.0));
    widgets::paint_avatar(ui, avatar, recipient.as_deref().unwrap_or(message.sender()));
    let left = avatar.right() + 11.0;
    let actions_visible = hovered || selected;
    let right_reserved = if actions_visible { 128.0 } else { 70.0 };

    let name_font = if unread { theme::semibold(13.0) } else { theme::medium(13.0) };
    let name_color = if unread { FG } else { FG_MUTED };
    widgets::paint_truncated(
        ui,
        egui::pos2(left, rect.top() + 10.0),
        &who,
        name_font,
        name_color,
        rect.right() - left - right_reserved,
    );

    let mut job = LayoutJob::default();
    let subject = if message.subject.is_empty() { "(no subject)" } else { &message.subject };
    let subject_color = if unread { FG } else { FG_MUTED };
    job.append(subject, 0.0, TextFormat::simple(theme::regular(12.5), subject_color));
    if !message.snippet.is_empty() {
        job.append(&format!("  {}", message.snippet), 0.0, TextFormat::simple(theme::regular(12.5), FG_SUBTLE));
    }
    job.wrap = TextWrapping::truncate_at_width((rect.right() - left - 12.0).max(10.0));
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(left, rect.top() + 32.0), galley, FG);

    if actions_visible {
        row_actions(ui, rect, message, out);
    } else {
        let date_right = rect.right() - 10.0;
        let date = ui.painter().text(
            egui::pos2(date_right, rect.top() + 18.0),
            Align2::RIGHT_CENTER,
            super::short_date(message.date),
            theme::regular(11.5),
            if unread { FG_MUTED } else { FG_SUBTLE },
        );
        let mut x = date.left() - 8.0;
        if message.flagged {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(egui::pos2(x - 6.0, rect.top() + 18.0), vec2(12.0, 12.0)),
                Icon::FlagFilled,
                WARN,
            );
            x -= 18.0;
        }
        if message.attachments {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(egui::pos2(x - 6.0, rect.top() + 18.0), vec2(12.0, 12.0)),
                Icon::Paperclip,
                FG_SUBTLE,
            );
        }
    }

    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if response.clicked() {
        out.push(Action::Select(message.id));
    }
    context_menu(app, &response, message, out);
}

/// Archive, delete, read and flag, shown on the hovered or selected row instead of the date.
fn row_actions(ui: &mut Ui, rect: Rect, message: &Summary, out: &mut Vec<Action>) {
    let buttons = [
        (Icon::Archive, "Archive (E)", Action::Archive(message.id)),
        (Icon::Trash, "Delete (#)", Action::Delete(message.id)),
        (
            if message.seen { Icon::Mail } else { Icon::MailOpen },
            if message.seen { "Mark as unread (U)" } else { "Mark as read (U)" },
            Action::Seen(message.id, !message.seen),
        ),
        (
            if message.flagged { Icon::FlagFilled } else { Icon::Flag },
            if message.flagged { "Unflag (S)" } else { "Flag (S)" },
            Action::Flag(message.id, !message.flagged),
        ),
    ];
    let mut x = rect.right() - 6.0 - 26.0 * buttons.len() as f32;
    for (icon, tip, action) in buttons {
        let hit = Rect::from_min_size(egui::pos2(x, rect.top() + 6.0), vec2(24.0, 24.0));
        let response = ui.interact(hit, ui.scope_id().with(("row-action", message.id, tip)), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(hit, CornerRadius::same(5), BG_PRESSED);
        }
        let color = if icon == Icon::FlagFilled {
            WARN
        } else if response.hovered() {
            FG
        } else {
            FG_MUTED
        };
        icons::paint(ui.painter(), hit.shrink(5.0), icon, color);
        if response.on_hover_text(tip).on_hover_cursor(CursorIcon::PointingHand).clicked() {
            out.push(action);
        }
        x += 26.0;
    }
}

/// Everything a row can do, so nothing is out of reach when the row actions are hidden.
pub fn context_menu(app: &App, response: &Response, message: &Summary, out: &mut Vec<Action>) {
    response.context_menu(|ui| {
        ui.set_min_width(210.0);
        message_menu(app, ui, message, out);
    });
}

pub fn message_menu(app: &App, ui: &mut Ui, message: &Summary, out: &mut Vec<Action>) {
    let (id, account, folder, seen, flagged) =
        (message.id, message.account.as_str(), message.folder, message.seen, message.flagged);
    let mut item = |ui: &mut Ui, icon: Icon, text: &str, key: &str, action: Action| {
        if widgets::menu_row(ui, Some(icon), text, key, false).clicked() {
            out.push(action);
            ui.close();
        }
    };
    item(ui, Icon::Reply, "Reply", "R", Action::Reply(id, false));
    item(ui, Icon::ReplyAll, "Reply all", "A", Action::Reply(id, true));
    item(ui, Icon::Forward, "Forward", "F", Action::Forward(id));
    ui.separator();
    if seen {
        item(ui, Icon::Mail, "Mark as unread", "U", Action::Seen(id, false));
    } else {
        item(ui, Icon::MailOpen, "Mark as read", "U", Action::Seen(id, true));
    }
    if flagged {
        item(ui, Icon::Flag, "Unflag", "S", Action::Flag(id, false));
    } else {
        item(ui, Icon::Flag, "Flag", "S", Action::Flag(id, true));
    }
    let in_inbox = app.folder(folder).is_some_and(|f| f.role == Role::Inbox);
    if app.settings.split_inbox && in_inbox {
        let (target, icon) = match message.category {
            Category::Primary => (Category::Other, Icon::Layers),
            Category::Other => (Category::Primary, Icon::Inbox),
        };
        item(ui, icon, &format!("Move to {}", target.name()), "", Action::SetCategory(id, target));
    }
    ui.separator();
    item(ui, Icon::Archive, "Archive", "E", Action::Archive(id));
    let targets: Vec<(i64, String)> = app
        .folders
        .iter()
        .filter(|f| f.account == account && f.id != folder && f.role.synced())
        .map(|f| (f.id, f.name.clone()))
        .collect();
    ui.menu_button("      Move to", |ui| {
        ui.set_min_width(180.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (target, name) in targets {
                if widgets::menu_row(ui, Some(Icon::Folder), &name, "", false).clicked() {
                    out.push(Action::MoveTo(id, target));
                    ui.close();
                }
            }
        });
    });
    let mut item = |ui: &mut Ui, icon: Icon, text: &str, key: &str, action: Action| {
        if widgets::menu_row(ui, Some(icon), text, key, false).clicked() {
            out.push(action);
            ui.close();
        }
    };
    item(ui, Icon::Junk, "Move to Junk", "", Action::Junk(id));
    item(ui, Icon::Trash, "Delete", "#", Action::Delete(id));
}

fn drafts(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    if app.drafts.is_empty() {
        ui.add_space(80.0);
        widgets::empty_state(ui, Icon::File, "No drafts", "Anything you start writing and close is kept here.", None);
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(4.0);
        for (id, draft, updated) in &app.drafts {
            let subject = if draft.subject.trim().is_empty() { "(no subject)" } else { draft.subject.trim() };
            let to = if draft.to.trim().is_empty() {
                "No recipients yet".to_owned()
            } else {
                format!("To {}", draft.to.trim())
            };
            let (rect, response, hovered) = simple_row(ui, subject, &to, &super::ago(*updated), None);
            if hovered {
                let hit = Rect::from_center_size(rect.right_center() - vec2(22.0, 0.0), vec2(24.0, 24.0));
                let delete = ui.interact(hit, ui.scope_id().with(("draft-delete", id)), Sense::click());
                icons::paint(
                    ui.painter(),
                    hit.shrink(5.0),
                    Icon::Trash,
                    if delete.hovered() { DANGER } else { FG_MUTED },
                );
                if delete.on_hover_text("Delete draft").clicked() {
                    out.push(Action::DeleteDraft(*id));
                    continue;
                }
            }
            if response.clicked() {
                out.push(Action::OpenDraft(*id));
            }
        }
    });
}

fn outbox(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    if app.outbox.is_empty() {
        ui.add_space(80.0);
        widgets::empty_state(ui, Icon::Send, "Outbox is empty", "Everything you've written has been sent.", None);
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(4.0);
        for outgoing in &app.outbox {
            let to = format!("To {}", outgoing.recipients.join(", "));
            let (pill, color) = match outgoing.state.as_str() {
                "failed" => ("Failed", DANGER),
                "sending" => ("Sending", ACCENT_TEXT),
                _ => ("Queued", FG_MUTED),
            };
            let detail = match (&outgoing.error, outgoing.state.as_str()) {
                (Some(error), _) => error.clone(),
                (None, "queued") if outgoing.send_after > crate::store::now() => {
                    format!("{to} · goes at {}", super::short_date(outgoing.send_after))
                }
                _ => to,
            };
            let (_, response, _) = simple_row(ui, &outgoing.subject, &detail, "", Some((pill, color)));
            response.context_menu(|ui| {
                if outgoing.state == "failed"
                    && widgets::menu_row(ui, Some(Icon::Refresh), "Try again", "", false).clicked()
                {
                    out.push(Action::RetrySend(outgoing.id));
                    ui.close();
                }
                if outgoing.state != "sending"
                    && widgets::menu_row(ui, Some(Icon::Compose), "Edit", "", false).clicked()
                {
                    out.push(Action::EditOutgoing(outgoing.id));
                    ui.close();
                }
            });
            if outgoing.state == "failed" {
                ui.horizontal(|ui| {
                    ui.add_space(62.0);
                    if widgets::small_button(ui, "Try again", Kind::Secondary).clicked() {
                        out.push(Action::RetrySend(outgoing.id));
                    }
                    if widgets::small_button(ui, "Edit", Kind::Ghost).clicked() {
                        out.push(Action::EditOutgoing(outgoing.id));
                    }
                });
                ui.add_space(6.0);
            }
        }
    });
}

fn simple_row(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    right: &str,
    pill: Option<(&str, egui::Color32)>,
) -> (Rect, Response, bool) {
    let (outer, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let rect = outer.shrink2(vec2(8.0, 1.0));
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), BG_HOVER);
    }
    let avatar = Rect::from_min_size(egui::pos2(rect.left() + 16.0, rect.top() + 11.0), vec2(30.0, 30.0));
    ui.painter().rect_filled(avatar, CornerRadius::same(9), BG_ACTIVE);
    icons::paint(ui.painter(), avatar.shrink(8.0), Icon::File, FG_MUTED);
    let left = avatar.right() + 11.0;
    widgets::paint_truncated(
        ui,
        egui::pos2(left, rect.top() + 10.0),
        title,
        theme::medium(13.0),
        FG,
        rect.right() - left - 90.0,
    );
    widgets::paint_truncated(
        ui,
        egui::pos2(left, rect.top() + 32.0),
        detail,
        theme::regular(12.5),
        FG_SUBTLE,
        rect.right() - left - 12.0,
    );
    if let Some((text, color)) = pill {
        let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::medium(11.0), color);
        let pill_rect = Rect::from_min_size(
            egui::pos2(rect.right() - galley.size().x - 22.0, rect.top() + 9.0),
            vec2(galley.size().x + 12.0, 18.0),
        );
        ui.painter().rect_filled(pill_rect, CornerRadius::same(7), color.gamma_multiply(0.16));
        ui.painter().galley(pill_rect.center() - galley.size() / 2.0, galley, color);
    } else if !right.is_empty() && !hovered {
        ui.painter().text(
            egui::pos2(rect.right() - 10.0, rect.top() + 18.0),
            Align2::RIGHT_CENTER,
            right,
            theme::regular(11.5),
            FG_SUBTLE,
        );
    }
    (rect, response.on_hover_cursor(CursorIcon::PointingHand), hovered)
}
