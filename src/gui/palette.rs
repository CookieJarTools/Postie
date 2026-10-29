//! The Ctrl+K command menu: every action and every folder, found by typing a few letters.

use egui::{Color32, Frame, Id, Key, Margin, Modifiers, Rect, Stroke, vec2};

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets;
use super::{Action, App, Nav, Screen};
use crate::store::{Category, Role, View};

#[derive(Default)]
pub struct Palette {
    query: String,
    highlighted: usize,
    focused: bool,
}

struct Command {
    icon: Icon,
    label: String,
    keys: &'static str,
    action: Action,
}

fn commands(app: &App) -> Vec<Command> {
    let mut list = Vec::new();
    let mut add = |icon, label: &str, keys, action| list.push(Command { icon, label: label.to_owned(), keys, action });
    if let Some(reader) = &app.reader {
        let summary = &reader.detail.summary;
        let id = summary.id;
        add(Icon::Reply, "Reply", "R", Action::Reply(id, false));
        add(Icon::ReplyAll, "Reply all", "A", Action::Reply(id, true));
        add(Icon::Forward, "Forward", "F", Action::Forward(id));
        add(Icon::Archive, "Archive", "E", Action::Archive(id));
        add(Icon::Trash, "Delete", "#", Action::Delete(id));
        if summary.seen {
            add(Icon::Mail, "Mark as unread", "U", Action::Seen(id, false));
        } else {
            add(Icon::MailOpen, "Mark as read", "U", Action::Seen(id, true));
        }
        add(Icon::Flag, if summary.flagged { "Unflag" } else { "Flag" }, "S", Action::Flag(id, !summary.flagged));
        add(Icon::Junk, "Move to Junk", "", Action::Junk(id));
        if app.settings.split_inbox {
            let target = if summary.category == Category::Primary { Category::Other } else { Category::Primary };
            add(Icon::Layers, &format!("Move to {}", target.name()), "", Action::SetCategory(id, target));
        }
    }
    add(Icon::Compose, "New message", "C", Action::Compose);
    add(Icon::Search, "Search mail", "/", Action::FocusSearch);
    add(Icon::Refresh, "Check for new mail", "F5", Action::SyncNow);
    add(Icon::Check, "Mark all as read", "", Action::MarkAllRead);
    add(Icon::Inbox, "Go to All inboxes", "", Action::Navigate(Nav::Mailbox(View::Unified(Role::Inbox))));
    add(Icon::Flag, "Go to Flagged", "", Action::Navigate(Nav::Mailbox(View::Flagged)));
    if app.inbox_split() {
        add(Icon::Inbox, "Show Primary", "", Action::SetCategoryTab(Category::Primary));
        add(Icon::Layers, "Show Other", "", Action::SetCategoryTab(Category::Other));
    }
    if !app.drafts.is_empty() {
        add(Icon::File, "Go to Drafts", "", Action::Navigate(Nav::Drafts));
    }
    let dnd = app.settings.do_not_disturb;
    add(
        if dnd { Icon::Bell } else { Icon::BellOff },
        if dnd { "Turn notifications back on" } else { "Do not disturb" },
        "",
        Action::SetDoNotDisturb(!dnd),
    );
    add(Icon::Settings, "Settings", "", Action::ShowScreen(Screen::Settings));
    add(Icon::Plus, "Add account", "", Action::AddAccount);
    let many = app.settings.accounts.len() > 1;
    for folder in app.folders.iter().filter(|folder| folder.role.synced()) {
        let label = match app.settings.account(&folder.account) {
            Some(account) if many => format!("Go to {} · {}", folder.name, account.label()),
            _ => format!("Go to {}", folder.name),
        };
        add(Icon::Folder, &label, "", Action::Navigate(Nav::Mailbox(View::Folder(folder.id))));
    }
    list
}

/// How well `query` matches `label`: whole-word starts beat substrings beat scattered letters.
fn score(label: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let label = label.to_lowercase();
    let query = query.to_lowercase();
    if let Some(at) = label.find(&query) {
        let word_start = at == 0 || label.as_bytes()[at - 1] == b' ';
        return Some(1000 - at as i32 + if word_start { 500 } else { 0 });
    }
    let mut chars = label.chars();
    let mut gaps = 0;
    for wanted in query.chars() {
        loop {
            let c = chars.next()?;
            if c == wanted {
                break;
            }
            gaps += 1;
        }
    }
    Some(100 - gaps)
}

pub fn show(app: &mut App, ctx: &egui::Context, out: &mut Vec<Action>) {
    if app.palette.is_none() {
        return;
    }
    let commands = commands(app);
    let Some(palette) = app.palette.as_mut() else { return };
    let mut matches: Vec<(i32, &Command)> =
        commands.iter().filter_map(|command| Some((score(&command.label, &palette.query)?, command))).collect();
    matches.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    palette.highlighted = palette.highlighted.min(matches.len().saturating_sub(1));

    let (down, up, enter) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    if down {
        palette.highlighted = (palette.highlighted + 1).min(matches.len().saturating_sub(1));
    }
    if up {
        palette.highlighted = palette.highlighted.saturating_sub(1);
    }
    let mut chosen =
        if enter { matches.get(palette.highlighted).map(|(_, command)| command.action.clone()) } else { None };

    let frame = Frame::new()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0, LINE_STRONG))
        .corner_radius(12)
        .inner_margin(Margin::same(6));
    let response = egui::Modal::new(Id::unique("palette"))
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(30.0, 40.0), egui::Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(rect.center() + vec2(4.0, 0.0), vec2(16.0, 16.0)),
                    Icon::Search,
                    FG_SUBTLE,
                );
                let edit = egui::TextEdit::singleline(&mut palette.query)
                    .frame(Frame::NONE)
                    .font(theme::regular(15.0))
                    .text_color(FG)
                    .hint_text(egui::RichText::new("Type a command or a folder").color(FG_FAINT))
                    .desired_width(f32::INFINITY);
                let field = ui.add_sized([ui.available_width(), 40.0], edit);
                if !palette.focused {
                    field.request_focus();
                    palette.focused = true;
                }
                if field.changed() {
                    palette.highlighted = 0;
                }
            });
            let rect = ui.available_rect_before_wrap();
            ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, LINE));
            ui.add_space(6.0);
            if matches.is_empty() {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| widgets::label(ui, "Nothing matches", theme::regular(13.0), FG_SUBTLE));
                ui.add_space(12.0);
                return;
            }
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (index, (_, command)) in matches.iter().enumerate() {
                    let highlighted = index == palette.highlighted;
                    let row = widgets::menu_row(ui, Some(command.icon), &command.label, command.keys, highlighted);
                    if highlighted && (down || up) {
                        row.scroll_to_me(None);
                    }
                    if row.clicked() {
                        chosen = Some(command.action.clone());
                    }
                }
            });
        });
    if let Some(action) = chosen {
        app.palette = None;
        out.push(action);
    } else if response.should_close() {
        app.palette = None;
    }
}
