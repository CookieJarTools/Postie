//! The window's own title bar, as CrumbTree has: logo, app menu, where you are, the command menu,
//! and minimise, maximise and close. The window has no system frame, so this is also where it's
//! dragged from and double-clicked to maximise.

use egui::{Align, Color32, CornerRadius, CursorIcon, Id, Layout, Rect, Sense, Stroke, Ui, ViewportCommand, vec2};

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets;
use super::{Action, App, Nav, Screen};

pub const HEIGHT: f32 = 36.0;
const BUTTON_WIDTH: f32 = 46.0;
const CLOSE_HOVER: Color32 = Color32::from_rgb(0xc4, 0x31, 0x4b);

pub fn show(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let bar = ui.max_rect();
    // Registered first, so every control drawn on top of it takes its own clicks.
    let drag = ui.interact(bar, Id::unique("titlebar-drag"), Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        out.push(Action::WindowMaximize);
    }

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_space(10.0);
        ui.add(egui::Image::new(app.logo()).fit_to_exact_size(vec2(18.0, 18.0)).corner_radius(5));
        ui.add_space(2.0);
        let menu = widgets::icon_button(ui, Icon::Menu, "Menu");
        egui::Popup::menu(&menu).show(|ui| app_menu(app, ui, out));
        ui.add_space(4.0);
        location_tab(app, ui);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Hyprland's floating windows report themselves maximised, so there it's always the square,
            // as on the desktop's own title bars.
            let hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
            let maximized = !hyprland && ui.input(|i| i.viewport().maximized.unwrap_or(false));
            if window_button(ui, Icon::Close, "Close", true).clicked() {
                out.push(Action::WindowClose);
            }
            let (icon, tip) = if maximized { (Icon::Restore, "Restore") } else { (Icon::Maximize, "Maximise") };
            if window_button(ui, icon, tip, false).clicked() {
                out.push(Action::WindowMaximize);
            }
            if window_button(ui, Icon::Minimize, "Minimise", false).clicked() {
                out.push(Action::WindowMinimize);
            }
            ui.add_space(8.0);
            if palette_chip(ui).clicked() {
                out.push(Action::OpenPalette);
            }
        });
    });
}

fn app_menu(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    ui.set_min_width(230.0);
    let mut item = |ui: &mut Ui, icon: Icon, text: &str, keys: &str, action: Action| {
        if widgets::menu_row(ui, Some(icon), text, keys, false).clicked() {
            out.push(action);
            ui.close();
        }
    };
    item(ui, Icon::Compose, "New message", "C", Action::Compose);
    item(ui, Icon::Refresh, "Check for new mail", "F5", Action::SyncNow);
    item(ui, Icon::Search, "Search mail", "/", Action::FocusSearch);
    item(ui, Icon::Search, "Command menu", "Ctrl+K", Action::OpenPalette);
    ui.separator();
    item(ui, Icon::Plus, "Add account", "", Action::AddAccount);
    item(ui, Icon::Settings, "Settings", "", Action::ShowScreen(Screen::Settings));
    let dnd = app.settings.do_not_disturb;
    let (icon, text) = if dnd { (Icon::Bell, "Turn notifications back on") } else { (Icon::BellOff, "Do not disturb") };
    item(ui, icon, text, "", Action::SetDoNotDisturb(!dnd));
    ui.separator();
    item(ui, Icon::Close, "Close window", "", Action::WindowClose);
    item(ui, Icon::Close, "Quit Postie", "", Action::QuitEverything);
}

/// Where you are, in the shape of CrumbTree's repository tab.
fn location_tab(app: &App, ui: &mut Ui) {
    let (icon, title) = match app.screen {
        Screen::Settings => (Icon::Settings, "Settings".to_owned()),
        Screen::Mail => {
            let icon = match &app.nav {
                Nav::Drafts => Icon::File,
                Nav::Outbox => Icon::Clock,
                _ if app.inbox_split() || matches!(app.nav, Nav::Mailbox(crate::store::View::Unified(_))) => {
                    Icon::Inbox
                }
                _ => Icon::Folder,
            };
            (icon, app.nav_title())
        }
    };
    let galley = ui.painter().layout_no_wrap(title, theme::medium(12.0), FG);
    let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 36.0, 24.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), BG_ACTIVE);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(13.0, 13.0)),
        icon,
        FG_MUTED,
    );
    ui.painter().galley(rect.left_center() + vec2(26.0, -galley.size().y / 2.0), galley, FG);
}

fn palette_chip(ui: &mut Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap("Ctrl K".to_owned(), egui::FontId::monospace(11.0), FG_SUBTLE);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 16.0, 22.0), Sense::click());
    let stroke = if response.hovered() { FG_SUBTLE } else { LINE_STRONG };
    ui.painter().rect(rect, CornerRadius::same(5), BG_SOFT, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, FG_SUBTLE);
    response.on_hover_text("Command menu").on_hover_cursor(CursorIcon::PointingHand)
}

fn window_button(ui: &mut Ui, icon: Icon, tip: &str, close: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON_WIDTH, HEIGHT), Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, if close { CLOSE_HOVER } else { BG_HOVER });
    }
    let color = match (close, hovered) {
        (true, true) => Color32::WHITE,
        (_, true) => FG,
        _ => FG_MUTED,
    };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(14.0, 14.0)), icon, color);
    response.on_hover_text(tip)
}

/// Window commands. On Hyprland they go through its own dispatchers, the same ones the desktop's
/// title bars use, so minimising lands where the taskbar expects it.
pub fn window_command(ctx: &egui::Context, action: &Action) {
    let hyprland = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
    match action {
        Action::WindowMinimize if hyprland => dispatch(
            r#"hl.dsp.window.move({ workspace = "special:minimized", follow = false })"#,
            &["movetoworkspacesilent", "special:minimized"],
        ),
        Action::WindowMinimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
        Action::WindowMaximize if hyprland => {
            dispatch(r#"hl.dsp.window.fullscreen({ mode = "maximized", action = "toggle" })"#, &["fullscreen", "1"])
        }
        Action::WindowMaximize => {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        Action::WindowClose => ctx.send_viewport_cmd(ViewportCommand::Close),
        _ => {}
    }
}

/// Newer Hyprland takes Lua dispatchers; older ones the classic form.
fn dispatch(lua: &'static str, classic: &'static [&'static str]) {
    std::thread::spawn(move || {
        let run = |args: &[&str]| {
            std::process::Command::new("hyprctl")
                .args(args)
                .output()
                .ok()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        };
        if run(&["dispatch", lua]).as_deref() != Some("ok") {
            run(&[&["dispatch"], classic].concat());
        }
    });
}
