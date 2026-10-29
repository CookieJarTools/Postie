use egui::{Align, Align2, CornerRadius, CursorIcon, Layout, Rect, Response, Sense, Ui, vec2};

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets::{self, Kind};
use super::{Action, App, Nav, Screen};
use crate::store::{Role, View};

pub fn show(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.add(egui::Image::new(app.logo()).fit_to_exact_size(vec2(20.0, 20.0)).corner_radius(5));
        ui.add_space(2.0);
        widgets::label(ui, "Postie", theme::semibold(14.0), FG);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, Icon::Refresh, "Check for new mail (F5)").clicked() {
                out.push(Action::SyncNow);
            }
            if widgets::icon_button(ui, Icon::Search, "Search (/)").clicked() {
                out.push(Action::FocusSearch);
            }
        });
    });
    ui.add_space(12.0);
    if compose_button(ui).clicked() {
        out.push(Action::Compose);
    }
    ui.add_space(14.0);

    ui.spacing_mut().item_spacing.y = 1.0;
    let mail = app.screen == Screen::Mail;
    let inbox = Nav::Mailbox(View::Unified(Role::Inbox));
    if nav_item(ui, Icon::Inbox, "All inboxes", mail && app.nav == inbox, app.unified_unread(), true, 0).clicked() {
        out.push(Action::Navigate(inbox));
    }
    let flagged = Nav::Mailbox(View::Flagged);
    if nav_item(ui, Icon::Flag, "Flagged", mail && app.nav == flagged, 0, false, 0).clicked() {
        out.push(Action::Navigate(flagged));
    }
    if !app.drafts.is_empty()
        && nav_item(ui, Icon::File, "Drafts", mail && app.nav == Nav::Drafts, app.drafts.len() as i64, false, 0)
            .clicked()
    {
        out.push(Action::Navigate(Nav::Drafts));
    }
    if !app.outbox.is_empty() {
        let failed = app.outbox.iter().any(|outgoing| outgoing.state == "failed");
        let response =
            nav_item(ui, Icon::Clock, "Outbox", mail && app.nav == Nav::Outbox, app.outbox.len() as i64, failed, 0);
        if response.clicked() {
            out.push(Action::Navigate(Nav::Outbox));
        }
    }

    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if nav_item(ui, Icon::Settings, "Settings", app.screen == Screen::Settings, 0, false, 0).clicked() {
            out.push(Action::ShowScreen(if app.screen == Screen::Settings { Screen::Mail } else { Screen::Settings }));
        }
        ui.add_space(6.0);
        status_line(app, ui, out);
        ui.add_space(8.0);
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.add_space(14.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let mut sections = Vec::new();
                for (index, account) in app.settings.accounts.iter().enumerate() {
                    sections.push(accounts_section(app, ui, out, index, &account.id, account.label()));
                    ui.add_space(10.0);
                }
                account_drop(app, ui, &sections, out);
            });
        });
    });
}

fn compose_button(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let fill = if response.is_pointer_button_down_on() {
        ACCENT_PRESSED
    } else if response.hovered() {
        ACCENT_HOVER
    } else {
        ACCENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), fill);
    let icon = Rect::from_center_size(rect.left_center() + vec2(18.0, 0.0), vec2(15.0, 15.0));
    icons::paint(ui.painter(), icon, Icon::Compose, egui::Color32::WHITE);
    ui.painter().text(
        rect.left_center() + vec2(34.0, 0.0),
        Align2::LEFT_CENTER,
        "New message",
        theme::medium(13.0),
        egui::Color32::WHITE,
    );
    ui.painter().text(
        rect.right_center() - vec2(12.0, 0.0),
        Align2::RIGHT_CENTER,
        "C",
        theme::medium(11.5),
        ACCENT_TEXT,
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// An account being dragged by its header to a new place in the list.
struct AccountDrag(usize);

/// Draws an account and its folders, and returns the space it took, for dropping a dragged
/// account above or below it.
fn accounts_section(app: &App, ui: &mut Ui, out: &mut Vec<Action>, index: usize, id: &str, label: &str) -> Rect {
    let collapsed = app.collapsed.contains(id);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    response.dnd_set_drag_payload(AccountDrag(index));
    let dragging = egui::DragAndDrop::payload::<AccountDrag>(ui.ctx()).is_some_and(|drag| drag.0 == index);
    let color = if dragging {
        FG_FAINT
    } else if response.hovered() {
        FG_MUTED
    } else {
        FG_SUBTLE
    };
    let avatar = Rect::from_center_size(rect.left_center() + vec2(10.0, 0.0), vec2(14.0, 14.0));
    widgets::paint_avatar(ui, avatar, label);
    widgets::paint_truncated(
        ui,
        rect.left_center() + vec2(24.0, -7.5),
        label,
        theme::medium(11.5),
        color,
        rect.width() - 46.0,
    );
    let chevron = Rect::from_center_size(rect.right_center() - vec2(12.0, 0.0), vec2(12.0, 12.0));
    icons::paint(ui.painter(), chevron, if collapsed { Icon::ChevronRight } else { Icon::ChevronDown }, color);
    let response = response.on_hover_text("Click to fold, drag to reorder");
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        out.push(Action::ToggleAccount(id.to_owned()));
    }
    if collapsed {
        return rect;
    }
    let mail = app.screen == Screen::Mail;
    for folder in app.folders.iter().filter(|folder| folder.account == id && folder.role.synced()) {
        let icon = match folder.role {
            Role::Inbox => Icon::Inbox,
            Role::Drafts => Icon::File,
            Role::Sent => Icon::Send,
            Role::Archive | Role::All => Icon::Archive,
            Role::Junk => Icon::Junk,
            Role::Trash => Icon::Trash,
            Role::Flagged | Role::Other => Icon::Folder,
        };
        let count = match folder.role {
            Role::Inbox if app.settings.split_inbox => folder.unread_primary,
            Role::Inbox | Role::Other | Role::Archive => folder.unread,
            Role::Drafts => folder.total,
            _ => 0,
        };
        let nav = Nav::Mailbox(View::Folder(folder.id));
        let selected = mail && app.nav == nav;
        let response = nav_item(ui, icon, &folder.name, selected, count, folder.role == Role::Inbox, folder.depth());
        if response.clicked() {
            out.push(Action::Navigate(nav));
        }
    }
    Rect::from_min_max(rect.min, egui::pos2(rect.right(), ui.min_rect().bottom().max(rect.bottom())))
}

/// While an account is dragged, a line shows where it would land; letting go moves it there.
fn account_drop(app: &App, ui: &mut Ui, sections: &[Rect], out: &mut Vec<Action>) {
    let Some(drag) = egui::DragAndDrop::payload::<AccountDrag>(ui.ctx()) else { return };
    let (Some(pointer), Some(first), Some(last)) = (ui.ctx().pointer_latest_pos(), sections.first(), sections.last())
    else {
        return;
    };
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    let slot = sections.iter().position(|section| pointer.y < section.center().y).unwrap_or(sections.len());
    let y = match sections.get(slot) {
        Some(section) => section.top() - 5.0,
        None => last.bottom() + 5.0,
    };
    let x = first.left() + 4.0..=first.right() - 4.0;
    ui.painter().hline(x, y, egui::Stroke::new(2.0, ACCENT));
    ui.painter().circle_filled(egui::pos2(first.left() + 4.0, y), 3.0, ACCENT);

    if let Some(account) = app.settings.accounts.get(drag.0) {
        egui::Area::new(egui::Id::unique("account-drag"))
            .fixed_pos(pointer + vec2(12.0, 6.0))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(BG_ACTIVE)
                    .stroke(egui::Stroke::new(1.0, LINE_STRONG))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| widgets::label(ui, account.label(), theme::medium(12.0), FG));
            });
    }
    if ui.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        out.push(Action::MoveAccount(drag.0, slot));
    }
}

pub fn nav_item(
    ui: &mut Ui,
    icon: Icon,
    text: &str,
    selected: bool,
    badge: i64,
    strong: bool,
    depth: usize,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let hovered = response.hovered();
    if selected || hovered {
        let fill = if selected { BG_ACTIVE } else { BG_HOVER };
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), fill);
    }
    let indent = depth.min(4) as f32 * 12.0;
    let color = if selected { FG } else { FG_MUTED };
    let icon_rect = Rect::from_center_size(rect.left_center() + vec2(17.0 + indent, 0.0), vec2(15.0, 15.0));
    icons::paint(ui.painter(), icon_rect, icon, if selected { FG } else { FG_SUBTLE });
    let font = if selected { theme::medium(13.0) } else { theme::regular(13.0) };
    let badge_room = if badge > 0 { 40.0 } else { 0.0 };
    let text_pos = rect.left_center() + vec2(34.0 + indent, -8.0);
    widgets::paint_truncated(ui, text_pos, text, font, color, rect.width() - 44.0 - indent - badge_room);
    if badge > 0 {
        widgets::paint_count(ui, rect.right_center() - vec2(6.0, 0.0), badge, strong);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn status_line(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    if !app.daemon {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            dot(ui, WARN);
            widgets::label(ui, "Sync paused", theme::regular(12.0), FG_MUTED).on_hover_text(
                "The background service isn't running, so mail isn't syncing and notifications are off.",
            );
            if widgets::small_button(ui, "Start", Kind::Ghost).clicked() {
                out.push(Action::StartDaemon);
            }
        });
        return;
    }
    let failing: Vec<_> = app.statuses.iter().filter(|status| status.state == "error").collect();
    let busy = app.statuses.iter().any(|status| status.state == "syncing" || status.state == "connecting");
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        if let Some(status) = failing.first() {
            dot(ui, DANGER);
            let label =
                app.settings.account(&status.account).map(|account| account.label().to_owned()).unwrap_or_default();
            let text = if failing.len() > 1 {
                format!("{} accounts need attention", failing.len())
            } else {
                format!("Can't reach {label}")
            };
            let response = widgets::label(ui, text, theme::regular(12.0), FG_MUTED);
            let details: Vec<String> = failing
                .iter()
                .map(|status| format!("{}: {}", status.account_label(app), status.error.clone().unwrap_or_default()))
                .collect();
            if response.on_hover_text(details.join("\n")).clicked() {
                out.push(Action::ShowScreen(Screen::Settings));
            }
        } else if busy {
            widgets::spinner(ui, 12.0);
            widgets::label(ui, "Syncing…", theme::regular(12.0), FG_MUTED);
        } else if let Some(at) = app.statuses.iter().map(|status| status.at).max() {
            dot(ui, SUCCESS);
            widgets::label(ui, format!("Up to date · {}", super::ago(at)), theme::regular(12.0), FG_SUBTLE);
        }
        if app.settings.do_not_disturb {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::BellOff, "Do not disturb is on. Click to turn it off.").clicked() {
                    out.push(Action::SetDoNotDisturb(false));
                }
            });
        }
    });
}

fn dot(ui: &mut Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 12.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

trait StatusLabel {
    fn account_label(&self, app: &App) -> String;
}

impl StatusLabel for crate::store::Status {
    fn account_label(&self, app: &App) -> String {
        app.settings
            .account(&self.account)
            .map(|account| account.label().to_owned())
            .unwrap_or_else(|| self.account.clone())
    }
}
