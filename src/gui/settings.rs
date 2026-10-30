use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, Sense, Stroke, StrokeKind, TextEdit, Ui, vec2,
};

use super::icons::{self, Icon};
use super::theme::{self, *};
use super::widgets::{self, Kind};
use super::{Action, App, Background, Dialog};
use crate::config::{self, Account, Auth, Security, Server, Settings};
use crate::mail::{Secret, oauth};
use crate::{autostart, credentials};

const COLUMN: f32 = 660.0;

pub fn show(app: &mut App, ui: &mut Ui, out: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let side = ((ui.available_width() - COLUMN) / 2.0).max(0.0);
        ui.horizontal(|ui| {
            ui.add_space(side);
            ui.vertical(|ui| {
                ui.set_width(COLUMN.min(ui.available_width()));
                page(app, ui, out);
            });
        });
    });
}

fn page(app: &mut App, ui: &mut Ui, out: &mut Vec<Action>) {
    widgets::label(ui, "Settings", theme::semibold(21.0), FG);
    ui.add_space(24.0);

    widgets::section_title(ui, "Accounts");
    ui.add_space(6.0);
    card(ui, |ui| {
        if app.settings.accounts.is_empty() {
            ui.add_space(8.0);
            widgets::note(ui, "No accounts yet. Add one to start getting your mail.");
            ui.add_space(8.0);
        }
        for account in app.settings.accounts.clone() {
            account_row(app, ui, &account, out);
        }
    });
    ui.add_space(10.0);
    if widgets::icon_text_button(ui, Icon::Plus, "Add account", Kind::Secondary).clicked() {
        out.push(Action::AddAccount);
    }
    ui.add_space(28.0);

    let mut changed = false;
    widgets::section_title(ui, "Notifications");
    ui.add_space(6.0);
    card(ui, |ui| {
        changed |= toggle_row(
            ui,
            "Notify me about new mail",
            "A desktop notification for each new message in your inboxes.",
            &mut app.settings.notifications,
        );
        divider(ui);
        changed |= toggle_row(
            ui,
            "Show a preview",
            "Include the first line of the message in the notification.",
            &mut app.settings.notification_preview,
        );
        divider(ui);
        let before = app.settings.do_not_disturb;
        toggle_row(
            ui,
            "Do not disturb",
            "Pause notifications. Mail keeps syncing; the tray menu has this too.",
            &mut app.settings.do_not_disturb,
        );
        if before != app.settings.do_not_disturb {
            out.push(Action::SetDoNotDisturb(app.settings.do_not_disturb));
        }
    });
    ui.add_space(28.0);

    widgets::section_title(ui, "Startup");
    ui.add_space(6.0);
    card(ui, |ui| {
        let how = if autostart::has_systemd_session() {
            "Runs as the postie.service user service."
        } else {
            "Uses an autostart entry."
        };
        let note = format!(
            "Postie waits in the tray and tells you about new mail. Closing this window doesn't stop it; quit from the tray. {how}"
        );
        let before = app.settings.launch_at_login;
        toggle_row(ui, "Start when you log in", &note, &mut app.settings.launch_at_login);
        if before != app.settings.launch_at_login {
            match autostart::set_launch_at_login(app.settings.launch_at_login) {
                Ok(()) => changed = true,
                Err(err) => {
                    app.settings.launch_at_login = before;
                    out.push(Action::Toast(format!("{err:#}")));
                }
            }
        }
    });
    ui.add_space(28.0);

    widgets::section_title(ui, "Inbox");
    ui.add_space(6.0);
    card(ui, |ui| {
        changed |= toggle_row(
            ui,
            "Split into Primary and Other",
            "People you write to, replies to you and urgent mail stay in Primary. Newsletters, mailing lists and automated mail go to Other. Move any message to teach it a sender.",
            &mut app.settings.split_inbox,
        );
        if app.settings.split_inbox {
            divider(ui);
            changed |= toggle_row(
                ui,
                "Notify about Other too",
                "Off: only mail that lands in Primary gets a notification and counts toward the unread badge.",
                &mut app.settings.notify_other,
            );
        }
    });
    ui.add_space(28.0);

    widgets::section_title(ui, "Reading and sending");
    ui.add_space(6.0);
    card(ui, |ui| {
        changed |= toggle_row(
            ui,
            "Mark as read when opened",
            "Otherwise messages stay unread until you mark them.",
            &mut app.settings.mark_read_on_open,
        );
        divider(ui);
        changed |= toggle_row(
            ui,
            "Monospace message text",
            "Show the text of every message, and what you write, in a fixed-width font. Menus and lists stay as they are.",
            &mut app.settings.monospace_bodies,
        );
        divider(ui);
        changed |= choice_row(
            ui,
            "Undo send",
            "How long a sent message waits, so you can take it back.",
            &mut app.settings.undo_send_seconds,
            &[(0, "Off"), (5, "5s"), (10, "10s"), (20, "20s"), (30, "30s")],
        );
    });
    ui.add_space(28.0);

    widgets::section_title(ui, "Sync");
    ui.add_space(6.0);
    card(ui, |ui| {
        changed |= choice_row(
            ui,
            "Check other folders every",
            "Inboxes update the moment mail arrives; this is for everything else.",
            &mut app.settings.sync_minutes,
            &[(2, "2 min"), (5, "5 min"), (15, "15 min"), (30, "30 min")],
        );
        divider(ui);
        changed |= toggle_row(
            ui,
            "Keep all mail on this computer",
            "Postie downloads your newest mail first, then the rest in the background, so search finds old emails too. Turn it off to keep only the newest messages of each folder.",
            &mut app.settings.download_all,
        );
        if !app.settings.download_all {
            divider(ui);
            changed |= choice_row(
                ui,
                "Messages per folder",
                "How many of the newest messages of each folder to keep.",
                &mut app.settings.initial_messages,
                &[(200, "200"), (400, "400"), (1000, "1,000"), (2500, "2,500")],
            );
        }
    });
    ui.add_space(28.0);

    widgets::section_title(ui, "Keyboard");
    ui.add_space(6.0);
    card(ui, shortcuts);
    ui.add_space(28.0);

    ui.horizontal(|ui| {
        widgets::label(ui, format!("Postie {}", env!("CARGO_PKG_VERSION")), theme::regular(12.0), FG_FAINT);
        widgets::label(ui, "·", theme::regular(12.0), FG_FAINT);
        let data = crate::paths::data_dir();
        if ui
            .add(
                egui::Label::new(
                    egui::RichText::new(data.display().to_string()).font(theme::regular(12.0)).color(FG_FAINT),
                )
                .sense(Sense::click()),
            )
            .on_hover_text("Open the data folder")
            .clicked()
        {
            out.push(Action::OpenUrl(data.display().to_string()));
        }
    });
    ui.add_space(30.0);

    if changed {
        out.push(Action::SettingsChanged);
    }
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(BG_SOFT)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(16, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn divider(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(rect.x_range(), rect.top(), Stroke::new(1.0, LINE));
    ui.add_space(1.0);
}

fn row_text(ui: &mut Ui, title: &str, note: &str, control_width: f32) {
    ui.vertical(|ui| {
        ui.set_width((ui.available_width() - control_width - 16.0).max(120.0));
        ui.spacing_mut().item_spacing.y = 2.0;
        widgets::label(ui, title, theme::medium(13.0), FG);
        widgets::note(ui, note);
    });
}

fn toggle_row(ui: &mut Ui, title: &str, note: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        row_text(ui, title, note, 40.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = widgets::toggle(ui, value).changed();
        });
    });
    ui.add_space(10.0);
    changed
}

fn choice_row(ui: &mut Ui, title: &str, note: &str, value: &mut u32, options: &[(u32, &str)]) -> bool {
    let mut changed = false;
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        row_text(ui, title, note, 270.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let items: Vec<(u32, &str, Option<i64>)> = options.iter().map(|(v, label)| (*v, *label, None)).collect();
            changed = widgets::segmented(ui, value, &items);
        });
    });
    ui.add_space(10.0);
    changed
}

fn account_row(app: &App, ui: &mut Ui, account: &Account, out: &mut Vec<Action>) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
    let avatar = Rect::from_min_size(rect.left_top() + vec2(0.0, 12.0), vec2(30.0, 30.0));
    widgets::paint_avatar(ui, avatar, account.label());
    let name = if account.name.is_empty() { account.email.as_str() } else { account.name.as_str() };
    widgets::paint_truncated(
        ui,
        rect.left_top() + vec2(42.0, 10.0),
        name,
        theme::medium(13.0),
        FG,
        rect.width() - 260.0,
    );
    let status = app.statuses.iter().find(|status| status.account == account.id);
    let (detail, color) = match status {
        Some(status) if status.state == "error" => {
            (status.error.clone().unwrap_or_else(|| "Can't connect".into()), DANGER)
        }
        Some(status) if status.state == "idle" => {
            (format!("{} · synced {}", account.email, super::ago(status.at)), FG_SUBTLE)
        }
        Some(_) => (format!("{} · syncing", account.email), FG_SUBTLE),
        None => (format!("{} · waiting for the background service", account.email), FG_SUBTLE),
    };
    widgets::paint_truncated(
        ui,
        rect.left_top() + vec2(42.0, 29.0),
        &detail,
        theme::regular(12.0),
        color,
        rect.width() - 200.0,
    );

    let hovered = response.hovered() || ui.rect_contains_pointer(rect);
    let buttons = Rect::from_min_max(rect.right_top() + vec2(-170.0, 13.0), rect.right_bottom() - vec2(0.0, 13.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(buttons).layout(Layout::right_to_left(Align::Center)), |ui| {
        if !hovered {
            if !account.notify {
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(buttons.right_center() - vec2(12.0, 0.0), vec2(14.0, 14.0)),
                    Icon::BellOff,
                    FG_SUBTLE,
                );
            }
            return;
        }
        if widgets::small_button(ui, "Remove", Kind::Danger).clicked() {
            out.push(Action::RemoveAccount(account.id.clone()));
        }
        if widgets::small_button(ui, "Edit", Kind::Ghost).clicked() {
            out.push(Action::EditAccount(account.id.clone()));
        }
    });
}

fn shortcuts(ui: &mut Ui) {
    let rows = [
        ("Next / previous message", "J / K"),
        ("Archive", "E"),
        ("Delete", "#  or  Delete"),
        ("Reply / reply all / forward", "R / A / F"),
        ("Flag", "S"),
        ("Mark read or unread", "U"),
        ("New message", "C"),
        ("Send", "Ctrl+Enter"),
        ("Search", "/"),
        ("Command menu", "Ctrl+K"),
        ("Check for new mail", "F5"),
    ];
    ui.add_space(8.0);
    for (what, keys) in rows {
        ui.horizontal(|ui| {
            widgets::label(ui, what, theme::regular(13.0), FG_MUTED);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                widgets::chip(ui, keys, FG, BG_ACTIVE);
            });
        });
        ui.add_space(4.0);
    }
    ui.add_space(6.0);
}

// ---------- the account form ----------

pub struct AccountForm {
    editing: Option<String>,
    account: Account,
    password: String,
    imap_port: String,
    smtp_port: String,
    advanced: bool,
    servers_for: String,
    pub checking: bool,
    pub error: Option<String>,
}

impl AccountForm {
    pub fn new(existing: Option<Account>) -> Self {
        let editing = existing.as_ref().map(|account| account.id.clone());
        let account = existing.unwrap_or_default();
        Self {
            imap_port: account.imap.port.to_string(),
            smtp_port: account.smtp.port.to_string(),
            servers_for: account.email.clone(),
            advanced: false,
            editing,
            account,
            password: String::new(),
            checking: false,
            error: None,
        }
    }

    /// Fills in servers from the provider presets as the address is typed, until they're edited by hand.
    fn guess_servers(&mut self) {
        let email = self.account.email.trim().to_owned();
        if self.editing.is_some() || email == self.servers_for || !email.contains('@') {
            return;
        }
        self.servers_for = email.clone();
        let (imap, smtp, saves) = match config::preset_for(&email) {
            Some(preset) => (
                Server { host: preset.imap.0.into(), port: preset.imap.1, security: preset.imap.2 },
                Server { host: preset.smtp.0.into(), port: preset.smtp.1, security: preset.smtp.2 },
                preset.server_saves_sent,
            ),
            None => {
                let (imap, smtp) = config::guess_servers(&email);
                (imap, smtp, false)
            }
        };
        self.imap_port = imap.port.to_string();
        self.smtp_port = smtp.port.to_string();
        self.account.imap = imap;
        self.account.smtp = smtp;
        self.account.server_saves_sent = saves;
        let microsoft = config::preset_for(&email).is_some_and(|preset| preset.name == "Outlook");
        self.account.auth = if microsoft { Auth::Microsoft } else { Auth::Password };
    }

    /// Microsoft 365 work accounts live on their own domains, so choosing Microsoft sign-in also
    /// points the servers at Exchange Online when they were guessed for something else.
    fn use_microsoft_servers(&mut self) {
        if self.account.imap.host.contains("office365.com") || self.account.imap.host.contains("outlook.com") {
            return;
        }
        self.account.imap = Server { host: "outlook.office365.com".into(), port: 993, security: Security::Tls };
        self.account.smtp = Server { host: "smtp.office365.com".into(), port: 587, security: Security::StartTls };
        self.imap_port = "993".into();
        self.smtp_port = "587".into();
        self.account.server_saves_sent = true;
    }

    /// The account as entered, and the password to check it with (none for Microsoft sign-in).
    fn prepared(&self) -> Result<(Account, Option<String>), String> {
        let mut account = self.account.clone();
        account.email = account.email.trim().to_owned();
        account.name = account.name.trim().to_owned();
        if !account.email.contains('@') {
            return Err("Enter your email address.".into());
        }
        account.imap.host = account.imap.host.trim().to_owned();
        account.smtp.host = account.smtp.host.trim().to_owned();
        account.imap.port =
            self.imap_port.trim().parse().map_err(|_| "The IMAP port should be a number.".to_owned())?;
        account.smtp.port =
            self.smtp_port.trim().parse().map_err(|_| "The SMTP port should be a number.".to_owned())?;
        if account.username.trim().is_empty() {
            account.username = account.email.clone();
        }
        if account.auth == Auth::Microsoft {
            if !oauth::available() {
                return Err("This build of Postie has no Microsoft app ID, so it can't sign in to Outlook.".into());
            }
            return Ok((account, None));
        }
        let password = if self.password.is_empty() {
            self.editing.as_deref().and_then(credentials::password).ok_or_else(|| "Enter the password.".to_owned())?
        } else {
            self.password.clone()
        };
        Ok((account, Some(password)))
    }

    /// Saves the checked account with its password, or the Microsoft token the check came back
    /// with. Returns its label.
    pub fn commit(&mut self, settings: &mut Settings, token: Option<String>) -> Option<String> {
        let (mut account, password) = self.prepared().ok()?;
        match &self.editing {
            Some(id) => {
                account.id = id.clone();
                if let Some(slot) = settings.accounts.iter_mut().find(|existing| existing.id == *id) {
                    *slot = account.clone();
                }
            }
            None => {
                account.id = config::new_account_id();
                settings.accounts.push(account.clone());
            }
        }
        // Editing a Microsoft account that still signs in keeps its token as it is.
        if let Some(secret) = token.or(password)
            && let Err(err) = credentials::set_password(&account.id, &secret)
        {
            self.error = Some(format!("Couldn't save the sign-in: {err}"));
            return None;
        }
        if let Err(err) = settings.save() {
            self.error = Some(format!("Couldn't save settings: {err}"));
            return None;
        }
        if settings.launch_at_login && settings.accounts.len() == 1 {
            let _ = autostart::set_launch_at_login(true);
        }
        Some(account.label().to_owned())
    }
}

pub fn dialogs(app: &mut App, ctx: &egui::Context, out: &mut Vec<Action>) {
    let background = app.background();
    match &mut app.dialog {
        Some(Dialog::Account(form)) => account_dialog(form, ctx, out, background),
        Some(Dialog::RemoveAccount(id)) => {
            let label = app.settings.account(id).map(|account| account.label().to_owned()).unwrap_or_default();
            let id = id.clone();
            let response = modal(ctx, "remove-account", 400.0, |ui| {
                widgets::label(ui, format!("Remove {label}?"), theme::semibold(15.0), FG);
                ui.add_space(4.0);
                widgets::note(
                    ui,
                    "Its mail is removed from this computer and its saved password is forgotten. Nothing is deleted from the server.",
                );
                ui.add_space(18.0);
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 28.0),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        if widgets::button(ui, "Remove account", Kind::Destructive).clicked() {
                            out.push(Action::RemoveAccountConfirmed(id.clone()));
                        }
                        if widgets::button(ui, "Cancel", Kind::Ghost).clicked() {
                            out.push(Action::CloseDialog);
                        }
                    },
                );
            });
            if response {
                out.push(Action::CloseDialog);
            }
        }
        None => {}
    }
}

/// A modal in the house frame. True when it should close (Escape or a click outside).
fn modal(ctx: &egui::Context, id: &str, width: f32, add: impl FnOnce(&mut Ui)) -> bool {
    let frame = Frame::new()
        .fill(BG_PANEL)
        .stroke(Stroke::new(1.0, LINE_STRONG))
        .corner_radius(12)
        .inner_margin(Margin::same(22));
    let response =
        egui::Modal::new(Id::unique(id)).backdrop_color(Color32::from_black_alpha(150)).frame(frame).show(ctx, |ui| {
            ui.set_width(width);
            add(ui);
        });
    response.should_close()
}

fn account_dialog(
    form: &mut AccountForm,
    ctx: &egui::Context,
    out: &mut Vec<Action>,
    background: std::sync::mpsc::Sender<Background>,
) {
    let title = if form.editing.is_some() { "Edit account" } else { "Add an account" };
    let closing = modal(ctx, "account-form", 440.0, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        widgets::label(ui, title, theme::semibold(16.0), FG);
        widgets::note(
            ui,
            "Postie signs in over IMAP and SMTP. Most providers want an app password here rather than your usual one.",
        );
        ui.add_space(12.0);
        let width = ui.available_width();

        widgets::label(ui, "Email address", theme::medium(12.0), FG_MUTED);
        let email = widgets::field(ui, &mut form.account.email, "you@example.com", width);
        if form.editing.is_none() && form.account.email.is_empty() && !email.has_focus() && !form.checking {
            email.request_focus();
        }
        form.guess_servers();
        ui.add_space(4.0);
        widgets::label(ui, "Your name", theme::medium(12.0), FG_MUTED);
        widgets::field(ui, &mut form.account.name, "As people you email will see it", width);
        ui.add_space(4.0);
        widgets::label(ui, "Sign in with", theme::medium(12.0), FG_MUTED);
        let before = form.account.auth;
        widgets::segmented(
            ui,
            &mut form.account.auth,
            &[(Auth::Password, "Password", None), (Auth::Microsoft, "Microsoft account", None)],
        );
        if before != form.account.auth && form.account.auth == Auth::Microsoft {
            form.use_microsoft_servers();
        }
        ui.add_space(4.0);
        let mut submit_by_enter = false;
        if form.account.auth == Auth::Password {
            widgets::label(ui, "Password", theme::medium(12.0), FG_MUTED);
            let hint = if form.editing.is_some() { "Leave empty to keep the saved one" } else { "App password" };
            let password = widgets::password_field(ui, &mut form.password, hint, width);
            submit_by_enter = password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        } else if oauth::available() {
            widgets::note(
                ui,
                "Your browser opens to sign in with Microsoft. Postie keeps a sign-in token, never your password, and you can revoke it from your Microsoft account.",
            );
        } else {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "This build of Postie has no Microsoft app ID yet, so it can't sign in to Outlook.",
                    )
                    .font(theme::regular(12.0))
                    .color(DANGER),
                )
                .wrap(),
            );
        }

        if let Some(preset) = config::preset_for(&form.account.email).filter(|_| form.account.auth == Auth::Password) {
            ui.add_space(6.0);
            Frame::new()
                .fill(ACCENT_SOFT.gamma_multiply(0.5))
                .corner_radius(8)
                .inner_margin(Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(preset.hint).font(theme::regular(12.0)).color(ACCENT_TEXT),
                        )
                        .wrap(),
                    );
                    if preset.name == "Gmail"
                        && ui
                            .link(
                                egui::RichText::new("Create an app password")
                                    .font(theme::medium(12.0))
                                    .color(ACCENT_TEXT),
                            )
                            .clicked()
                    {
                        out.push(Action::OpenUrl("https://myaccount.google.com/apppasswords".into()));
                    }
                });
        }

        ui.add_space(8.0);
        let chevron = if form.advanced { Icon::ChevronDown } else { Icon::ChevronRight };
        if widgets::icon_text_button(ui, chevron, "Server settings", Kind::Ghost).clicked() {
            form.advanced = !form.advanced;
        }
        if form.advanced {
            servers(ui, form, width);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            widgets::toggle(ui, &mut form.account.notify);
            widgets::label(ui, "Notify me about new mail in this account", theme::regular(12.5), FG_MUTED);
        });
        ui.add_space(6.0);
        widgets::label(ui, "Signature", theme::medium(12.0), FG_MUTED);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 64.0), Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(RADIUS), BG_SOFT, Stroke::new(1.0, LINE_STRONG), StrokeKind::Inside);
        ui.put(
            rect.shrink2(vec2(10.0, 6.0)),
            TextEdit::multiline(&mut form.account.signature)
                .frame(Frame::NONE)
                .font(theme::regular(12.5))
                .hint_text(egui::RichText::new("Added to the end of new messages").color(FG_FAINT))
                .desired_rows(3),
        );

        if let Some(error) = &form.error {
            ui.add_space(6.0);
            ui.add(egui::Label::new(egui::RichText::new(error).font(theme::regular(12.5)).color(DANGER)).wrap());
        }
        ui.add_space(14.0);
        ui.allocate_ui_with_layout(vec2(width, 28.0), Layout::right_to_left(Align::Center), |ui| {
            if form.checking {
                widgets::spinner(ui, 16.0);
                let text = if form.account.auth == Auth::Microsoft {
                    "Finish signing in in your browser…"
                } else {
                    "Signing in…"
                };
                widgets::label(ui, text, theme::regular(12.5), FG_MUTED);
                return;
            }
            let microsoft = form.account.auth == Auth::Microsoft;
            let label = match (microsoft, form.editing.is_some()) {
                (true, _) => "Sign in with Microsoft",
                (false, true) => "Save",
                (false, false) => "Add account",
            };
            if widgets::button(ui, label, Kind::Primary).clicked() || submit_by_enter {
                match form.prepared() {
                    Ok((account, password)) => {
                        form.checking = true;
                        form.error = None;
                        let sender = background.clone();
                        let ctx = ui.ctx().clone();
                        let editing = form.editing.is_some();
                        std::thread::spawn(move || {
                            let result = check_sign_in(&account, password, editing).map_err(|err| format!("{err:#}"));
                            let _ = sender.send(Background::AccountChecked(result));
                            ctx.request_repaint();
                        });
                    }
                    Err(err) => form.error = Some(err),
                }
            }
            if widgets::button(ui, "Cancel", Kind::Ghost).clicked() {
                out.push(Action::CloseDialog);
            }
        });
    });
    if closing && !form.checking {
        out.push(Action::CloseDialog);
    }
}

/// Signs in the way the form says and checks both servers. For a Microsoft account, returns the
/// refresh token to keep; an account being edited tries its saved one before asking again.
fn check_sign_in(account: &Account, password: Option<String>, editing: bool) -> anyhow::Result<Option<String>> {
    if let Some(password) = password {
        crate::mail::check_account(account, &Secret::Password(password))?;
        return Ok(None);
    }
    if editing && let Ok(token) = oauth::access_token(account) {
        crate::mail::check_account(account, &Secret::Bearer(token))?;
        return Ok(None);
    }
    let tokens = oauth::sign_in(&account.email)?;
    crate::mail::check_account(account, &Secret::Bearer(tokens.access.clone()))?;
    Ok(Some(tokens.refresh))
}

fn servers(ui: &mut Ui, form: &mut AccountForm, width: f32) {
    let security_items =
        [(Security::Tls, "SSL/TLS", None), (Security::StartTls, "STARTTLS", None), (Security::None, "None", None)];
    for (label, server, port) in [
        ("Incoming (IMAP)", &mut form.account.imap, &mut form.imap_port),
        ("Outgoing (SMTP)", &mut form.account.smtp, &mut form.smtp_port),
    ] {
        ui.add_space(4.0);
        widgets::label(ui, label, theme::medium(12.0), FG_MUTED);
        ui.horizontal(|ui| {
            widgets::field(ui, &mut server.host, "Server", width - 86.0);
            widgets::field(ui, port, "Port", 78.0);
        });
        let before = server.security;
        widgets::segmented(ui, &mut server.security, &security_items);
        if before != server.security {
            let defaults = if label.starts_with("Incoming") { (993, 143, 143) } else { (465, 587, 25) };
            *port = match server.security {
                Security::Tls => defaults.0,
                Security::StartTls => defaults.1,
                Security::None => defaults.2,
            }
            .to_string();
        }
    }
    ui.add_space(4.0);
    widgets::label(ui, "Username", theme::medium(12.0), FG_MUTED);
    widgets::field(ui, &mut form.account.username, "Usually your email address", width);
    ui.horizontal(|ui| {
        widgets::toggle(ui, &mut form.account.server_saves_sent);
        widgets::label(ui, "The server files sent mail itself (Gmail, Outlook)", theme::regular(12.5), FG_MUTED);
    });
}
