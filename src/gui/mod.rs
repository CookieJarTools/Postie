//! The window: sidebar of accounts and folders, the message list, and the reading pane, in the house
//! Linear look. It reads the shared store directly and queues changes for the daemon to replay.

mod compose;
mod html;
mod icons;
mod list;
mod palette;
mod reader;
mod settings;
mod sidebar;
mod theme;
mod titlebar;
mod widgets;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use egui::{Align2, Color32, Frame, Key, Margin, Modifiers, Stroke, vec2};
use rusqlite::Connection;

use crate::config::Settings;
use crate::ipc::{self, Request};
use crate::mail::compose::{Draft, DraftAttachment};
use crate::mail::parse;
use crate::store::{self, Category, Filter, Folder, Op, Outgoing, Role, Status, Summary, View};
use compose::Composer;
use reader::Reader;
use theme::*;
use widgets::Kind;

const PAGE: i64 = 400;

pub fn run(message: Option<i64>) -> Result<()> {
    let icon = egui::IconData { rgba: crate::icon::rgba(256, false), width: 256, height: 256 };
    let viewport = egui::ViewportBuilder::default()
        .with_title("Postie")
        .with_app_id("postie")
        .with_inner_size([1320.0, 820.0])
        .with_min_inner_size([900.0, 540.0])
        // Postie draws its own title bar, as CrumbTree does; Hyprland's bars leave such windows alone.
        .with_decorations(false)
        .with_icon(icon);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Postie", options, Box::new(move |cc| Ok(Box::new(App::new(cc, message)?))))
        .map_err(|err| anyhow!("{err}"))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Nav {
    Mailbox(View),
    Drafts,
    Outbox,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Screen {
    Mail,
    Settings,
}

#[derive(Clone, Debug)]
pub enum Action {
    Navigate(Nav),
    Select(i64),
    SetCategoryTab(Category),
    /// Moves a message, and its sender from now on, to Primary or Other.
    SetCategory(i64, Category),
    SetFilter(Filter),
    Seen(i64, bool),
    Flag(i64, bool),
    Archive(i64),
    Delete(i64),
    Junk(i64),
    MoveTo(i64, i64),
    Reply(i64, bool),
    Forward(i64),
    Compose,
    ComposeTo {
        to: String,
        subject: String,
        body: String,
    },
    OpenDraft(i64),
    DeleteDraft(i64),
    Send,
    CloseComposer,
    DiscardComposer,
    Attach,
    SaveAttachment(i64, usize),
    OpenAttachment(i64, usize),
    OpenInBrowser(i64),
    TogglePlain,
    /// The original view for the open message; `always` remembers its sender.
    ShowImages {
        always: bool,
    },
    /// Back to the simplified view, and stop showing this sender's images on their own.
    HideImages,
    OpenUrl(String),
    SyncNow,
    ShowScreen(Screen),
    MarkAllRead,
    UndoSend(i64),
    RetrySend(i64),
    EditOutgoing(i64),
    OpenPalette,
    FocusSearch,
    WindowMinimize,
    WindowMaximize,
    WindowClose,
    /// Closes the window and stops the background service too.
    QuitEverything,
    Next,
    Previous,
    StartDaemon,
    LoadMore,
    ToggleAccount(String),
    /// An account dragged from its place to before the one at `.1` (or to the end).
    MoveAccount(usize, usize),
    AddAccount,
    EditAccount(String),
    RemoveAccount(String),
    RemoveAccountConfirmed(String),
    SettingsChanged,
    SetDoNotDisturb(bool),
    Toast(String),
    CloseDialog,
}

/// Results from work done off the UI thread.
pub enum Background {
    Attach(Vec<PathBuf>),
    /// The add-account check finished; a Microsoft sign-in brings its refresh token.
    AccountChecked(Result<Option<String>, String>),
    Toast(String),
}

pub struct Toast {
    text: String,
    action: Option<(String, Action)>,
    seconds: f64,
    /// Set the first time it's drawn, so a toast queued while busy still gets its full time.
    shown_at: Option<f64>,
}

/// An image a message refers to, as the original view loads it.
#[derive(Clone)]
pub enum ImageState {
    Loading,
    /// Loaded, under this `bytes://` address in egui's image cache.
    Ready(String),
    Failed,
}

pub enum Dialog {
    Account(Box<settings::AccountForm>),
    RemoveAccount(String),
}

pub struct App {
    db: Connection,
    pub settings: Settings,
    pub folders: Vec<Folder>,
    pub statuses: Vec<Status>,
    pub outbox: Vec<Outgoing>,
    pub drafts: Vec<(i64, Draft, i64)>,
    pub nav: Nav,
    pub screen: Screen,
    pub filter: Filter,
    /// Primary or Other, in an inbox that's split.
    pub category: Category,
    pub search: String,
    searched: String,
    pub messages: Vec<Summary>,
    pub limit: i64,
    pub selected: Option<i64>,
    pub reader: Option<Reader>,
    pub composer: Option<Composer>,
    pub palette: Option<palette::Palette>,
    pub dialog: Option<Dialog>,
    pub collapsed: HashSet<String>,
    pub focus_search: bool,
    pub daemon: bool,
    toasts: Vec<Toast>,
    /// Drafts of messages waiting in the outbox, so Undo brings back exactly what was written.
    pending_drafts: Vec<(i64, Draft)>,
    events: Receiver<ipc::Event>,
    last_connect: f64,
    background: (Sender<Background>, Receiver<Background>),
    dirty: bool,
    last_reload: f64,
    title: String,
    logo: egui::TextureHandle,
    /// Images loaded for the original view, by their address in the message. Shared with the
    /// threads that fetch them.
    images: Arc<Mutex<HashMap<String, ImageState>>>,
    /// Set by POSTIE_SCREENSHOT: save the window's own pixels there after a few seconds, then
    /// quit. For the README's screenshots, unaffected by whatever is on top of the window.
    screenshot: Option<(PathBuf, bool)>,
}

impl App {
    fn new(cc: &eframe::CreationContext, message: Option<i64>) -> Result<Self> {
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let db = store::open()?;
        let (events_tx, events) = channel();
        let daemon = subscribe(&cc.egui_ctx, events_tx);
        let logo = cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &crate::icon::rgba(64, false)),
            egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            db,
            settings: Settings::load(),
            folders: Vec::new(),
            statuses: Vec::new(),
            outbox: Vec::new(),
            drafts: Vec::new(),
            nav: Nav::Mailbox(View::Unified(Role::Inbox)),
            screen: Screen::Mail,
            filter: Filter::All,
            category: Category::Primary,
            search: String::new(),
            searched: String::new(),
            messages: Vec::new(),
            limit: PAGE,
            selected: None,
            reader: None,
            composer: None,
            palette: None,
            dialog: None,
            collapsed: HashSet::new(),
            focus_search: false,
            daemon,
            toasts: Vec::new(),
            pending_drafts: Vec::new(),
            events,
            last_connect: 0.0,
            background: channel(),
            dirty: true,
            last_reload: -1.0,
            title: String::new(),
            logo,
            images: Arc::new(Mutex::new(HashMap::new())),
            screenshot: std::env::var_os("POSTIE_SCREENSHOT").map(|path| (PathBuf::from(path), false)),
        };
        app.reload();
        if app.settings.accounts.is_empty() {
            app.screen = Screen::Settings;
            app.dialog = Some(Dialog::Account(Box::new(settings::AccountForm::new(None))));
        }
        if let Some(message) = message {
            app.open_message(message);
        }
        Ok(app)
    }

    pub fn logo(&self) -> &egui::TextureHandle {
        &self.logo
    }

    pub fn background(&self) -> Sender<Background> {
        self.background.0.clone()
    }

    fn reload(&mut self) {
        self.dirty = false;
        if let Err(err) = self.try_reload() {
            log::warn!("reload failed: {err:#}");
        }
    }

    fn try_reload(&mut self) -> Result<()> {
        self.settings = Settings::load();
        self.folders = store::folders(&self.db)?;
        self.statuses = store::statuses(&self.db)?;
        self.outbox = store::outbox(&self.db)?;
        self.drafts = store::drafts(&self.db)?
            .into_iter()
            .filter_map(|(id, json, updated)| Some((id, serde_json::from_str(&json).ok()?, updated)))
            .collect();
        if let Nav::Mailbox(View::Folder(id)) = &self.nav
            && !self.folders.iter().any(|folder| folder.id == *id)
        {
            self.nav = Nav::Mailbox(View::Unified(Role::Inbox));
        }
        let category = self.inbox_split().then_some(self.category);
        self.messages = match &self.nav {
            Nav::Mailbox(view) => store::list(&self.db, view, category, self.filter, &self.search, self.limit)?,
            Nav::Drafts | Nav::Outbox => Vec::new(),
        };
        self.searched = self.search.clone();
        if let Some(reader) = &mut self.reader {
            match store::detail(&self.db, reader.id())? {
                Some(detail) => {
                    reader.set_mono(self.settings.monospace_bodies);
                    reader.refresh(&self.db, detail);
                }
                None => {
                    self.reader = None;
                    self.selected = None;
                }
            }
        }
        Ok(())
    }

    pub fn image(&self, src: &str) -> Option<ImageState> {
        self.images.lock().unwrap().get(src).cloned()
    }

    /// Starts loading the images the open message shows in its original view: from the message
    /// itself for `cid:` and `data:` addresses, from the web otherwise.
    fn load_images(&mut self, ctx: &egui::Context) {
        let Some(reader) = &self.reader else { return };
        let Some(page) = reader.page.as_ref().filter(|page| page.original) else { return };
        let wanted: Vec<String> = {
            let cache = self.images.lock().unwrap();
            page.images().filter(|src| !cache.contains_key(*src)).map(str::to_owned).collect()
        };
        if wanted.is_empty() {
            return;
        }
        let inline = if wanted.iter().any(|src| src.starts_with("cid:")) {
            store::raw(&self.db, reader.id()).ok().flatten().map(|raw| parse::inline_images(&raw)).unwrap_or_default()
        } else {
            Vec::new()
        };
        for src in wanted {
            if let Some(id) = src.strip_prefix("cid:") {
                let bytes = inline.iter().find(|(cid, _)| cid.eq_ignore_ascii_case(id.trim_matches(['<', '>'])));
                let state = bytes.map_or(ImageState::Failed, |(_, bytes)| show_image(ctx, &src, bytes.clone()));
                self.images.lock().unwrap().insert(src, state);
            } else if src.starts_with("data:") {
                let state = data_uri(&src).map_or(ImageState::Failed, |bytes| show_image(ctx, &src, bytes));
                self.images.lock().unwrap().insert(src, state);
            } else {
                self.images.lock().unwrap().insert(src.clone(), ImageState::Loading);
                let images = self.images.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let url = if src.starts_with("//") { format!("https:{src}") } else { src.clone() };
                    let state = fetch_image(&url).map_or(ImageState::Failed, |bytes| show_image(&ctx, &src, bytes));
                    images.lock().unwrap().insert(src, state);
                    ctx.request_repaint();
                });
            }
        }
    }

    pub fn folder(&self, id: i64) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    pub fn account_folder(&self, account: &str, role: Role) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.account == account && folder.role == role)
    }

    pub fn nav_title(&self) -> String {
        match &self.nav {
            Nav::Mailbox(View::Unified(Role::Inbox)) => "All inboxes".into(),
            Nav::Mailbox(View::Unified(role)) => format!("All {}", role.as_str()),
            Nav::Mailbox(View::Flagged) => "Flagged".into(),
            Nav::Mailbox(View::Folder(id)) => self.folder(*id).map(|folder| folder.name.clone()).unwrap_or_default(),
            Nav::Drafts => "Drafts".into(),
            Nav::Outbox => "Outbox".into(),
        }
    }

    /// Unread across inboxes: Primary only when inboxes are split, as the badge and tray count.
    pub fn unified_unread(&self) -> i64 {
        let inboxes = self.folders.iter().filter(|folder| folder.role == Role::Inbox);
        if self.settings.split_inbox {
            inboxes.map(|folder| folder.unread_primary).sum()
        } else {
            inboxes.map(|folder| folder.unread).sum()
        }
    }

    /// Whether the list shows an inbox with its Primary and Other tabs.
    pub fn inbox_split(&self) -> bool {
        self.settings.split_inbox
            && match &self.nav {
                Nav::Mailbox(View::Unified(Role::Inbox)) => true,
                Nav::Mailbox(View::Folder(id)) => self.folder(*id).is_some_and(|folder| folder.role == Role::Inbox),
                _ => false,
            }
    }

    fn tell_daemon(&mut self, request: Request) {
        let sender = self.background.0.clone();
        std::thread::spawn(move || {
            if ipc::send(&request).is_err() && matches!(request, Request::Sync) {
                let _ = sender.send(Background::Toast("The background service isn't running.".into()));
            }
        });
    }

    fn queue(&mut self, id: i64, op: Op) {
        let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
        let result = (|| -> Result<()> {
            match &op {
                Op::Seen(seen) => store::set_seen(&self.db, id, *seen)?,
                Op::Flagged(flagged) => store::set_flagged(&self.db, id, *flagged)?,
                Op::Move { .. } | Op::Archive | Op::Delete => store::hide(&self.db, id)?,
                Op::Answered | Op::FetchBody => {}
            }
            store::queue(&self.db, &detail.summary.account, id, &op)
        })();
        if let Err(err) = result {
            self.toast(format!("Couldn't save that change: {err:#}"));
        }
        self.tell_daemon(Request::Wake);
        self.dirty = true;
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), action: None, seconds: 3.0, shown_at: None });
    }

    fn toast_with(&mut self, text: impl Into<String>, label: &str, action: Action, seconds: f64) {
        self.toasts.push(Toast { text: text.into(), action: Some((label.into(), action)), seconds, shown_at: None });
    }

    fn open_message(&mut self, id: i64) {
        let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
        if let Some(folder) = self.folder(detail.summary.folder) {
            let in_view = self.messages.iter().any(|message| message.id == id);
            if !in_view {
                self.nav = Nav::Mailbox(View::Folder(folder.id));
                self.filter = Filter::All;
                self.search.clear();
            }
        }
        self.screen = Screen::Mail;
        self.category = detail.summary.category;
        self.dirty = true;
        self.try_reload().ok();
        self.select(id);
    }

    fn select(&mut self, id: i64) {
        let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
        if !detail.summary.seen && self.settings.mark_read_on_open {
            self.queue(id, Op::Seen(true));
        }
        if !detail.has_body {
            let already = store::pending_ops(&self.db, &detail.summary.account)
                .map(|ops| ops.iter().any(|op| op.message == id && matches!(op.op, Op::FetchBody)))
                .unwrap_or(false);
            if !already {
                let _ = store::queue(&self.db, &detail.summary.account, id, &Op::FetchBody);
                self.tell_daemon(Request::Wake);
            }
        }
        self.selected = Some(id);
        let original = self.settings.shows_images_from(&detail.summary.from_addr);
        self.reader = Some(Reader::new(&self.db, detail, self.settings.monospace_bodies, original));
    }

    /// After a message leaves the list, select the one that took its place.
    fn select_after_removing(&mut self, id: i64) {
        if self.selected != Some(id) {
            return;
        }
        let index = self.messages.iter().position(|message| message.id == id);
        let next = index
            .and_then(|i| self.messages.get(i + 1).or_else(|| i.checked_sub(1).and_then(|p| self.messages.get(p))));
        match next.map(|message| message.id) {
            Some(next) => self.select(next),
            None => {
                self.selected = None;
                self.reader = None;
            }
        }
    }

    fn move_selection(&mut self, step: isize) {
        if self.messages.is_empty() {
            return;
        }
        let index = self.selected.and_then(|id| self.messages.iter().position(|message| message.id == id));
        let next = match index {
            Some(i) => (i as isize + step).clamp(0, self.messages.len() as isize - 1) as usize,
            None => 0,
        };
        let id = self.messages[next].id;
        if Some(id) != self.selected {
            self.select(id);
        }
    }

    fn start_compose(&mut self, draft: Draft, draft_id: Option<i64>, title: &str) {
        if let Some(composer) = self.composer.take() {
            composer.save(&self.db);
        }
        self.composer = Some(Composer::new(draft, draft_id, title));
    }

    fn reply_text(&self, id: i64) -> String {
        store::raw(&self.db, id)
            .ok()
            .flatten()
            .map(|raw| {
                let body = parse::body(&raw);
                body.text.or_else(|| body.html.map(|html| html::to_plain(&html))).unwrap_or_default()
            })
            .unwrap_or_default()
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Navigate(nav) => {
                self.screen = Screen::Mail;
                if self.nav != nav {
                    self.nav = nav;
                    self.category = Category::Primary;
                    self.limit = PAGE;
                    self.search.clear();
                    self.selected = None;
                    self.reader = None;
                }
                self.dirty = true;
            }
            Action::Select(id) => self.select(id),
            Action::SetCategoryTab(category) => {
                self.category = category;
                self.selected = None;
                self.reader = None;
                self.limit = PAGE;
                self.dirty = true;
            }
            Action::SetCategory(id, category) => {
                let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
                let sender = detail.summary.sender().to_owned();
                if self.inbox_split() && self.category != category {
                    self.select_after_removing(id);
                }
                match store::set_sender_category(&self.db, &detail.summary.from_addr, category) {
                    Ok(_) => self.toast(format!("Mail from {sender} now goes to {}", category.name())),
                    Err(err) => self.toast(format!("Couldn't move it: {err:#}")),
                }
                // The tray's unread dot counts Primary only.
                self.tell_daemon(Request::Wake);
                self.dirty = true;
            }
            Action::SetFilter(filter) => {
                self.filter = filter;
                self.dirty = true;
            }
            Action::Seen(id, seen) => self.queue(id, Op::Seen(seen)),
            Action::Flag(id, flagged) => self.queue(id, Op::Flagged(flagged)),
            Action::Archive(id) => {
                self.select_after_removing(id);
                self.queue(id, Op::Archive);
                self.toast("Archived");
            }
            Action::Delete(id) => {
                self.select_after_removing(id);
                self.queue(id, Op::Delete);
                self.toast("Deleted");
            }
            Action::Junk(id) => {
                let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
                let Some(junk) = self.account_folder(&detail.summary.account, Role::Junk).map(|folder| folder.id)
                else {
                    self.toast("This account has no Junk folder.");
                    return;
                };
                self.select_after_removing(id);
                self.queue(id, Op::Move { to: junk });
                self.toast("Moved to Junk");
            }
            Action::MoveTo(id, folder) => {
                let name = self.folder(folder).map(|folder| folder.name.clone()).unwrap_or_default();
                self.select_after_removing(id);
                self.queue(id, Op::Move { to: folder });
                self.toast(format!("Moved to {name}"));
            }
            Action::Reply(id, all) => {
                let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
                let Some(account) = self.settings.account(&detail.summary.account).cloned() else { return };
                let text = self.reply_text(id);
                let draft = crate::mail::compose::reply(&account, &detail, &text, all);
                self.start_compose(draft, None, if all { "Reply all" } else { "Reply" });
            }
            Action::Forward(id) => {
                let Ok(Some(detail)) = store::detail(&self.db, id) else { return };
                let Some(account) = self.settings.account(&detail.summary.account).cloned() else { return };
                let text = self.reply_text(id);
                let attachments = forwarded_attachments(&self.db, id);
                let draft = crate::mail::compose::forward(&account, &detail, &text, attachments);
                self.start_compose(draft, None, "Forward");
            }
            Action::Compose => self.compose_new(String::new(), String::new(), String::new()),
            Action::ComposeTo { to, subject, body } => self.compose_new(to, subject, body),
            Action::OpenDraft(id) => {
                if let Some((_, draft, _)) = self.drafts.iter().find(|(draft_id, _, _)| *draft_id == id).cloned() {
                    self.start_compose(draft, Some(id), "Draft");
                }
            }
            Action::DeleteDraft(id) => {
                let _ = store::delete_draft(&self.db, id);
                if self.composer.as_ref().is_some_and(|composer| composer.draft_id == Some(id)) {
                    self.composer = None;
                }
                self.dirty = true;
            }
            Action::Send => self.send(),
            Action::CloseComposer => {
                if let Some(composer) = self.composer.take()
                    && composer.save(&self.db)
                {
                    self.toast("Saved to Drafts");
                }
                self.dirty = true;
            }
            Action::DiscardComposer => {
                if let Some(composer) = self.composer.take() {
                    composer.discard(&self.db);
                }
                self.dirty = true;
            }
            Action::Attach => {
                let sender = self.background.0.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    if let Some(files) = rfd::FileDialog::new().set_title("Attach files").pick_files() {
                        let _ = sender.send(Background::Attach(files));
                        ctx.request_repaint();
                    }
                });
            }
            Action::SaveAttachment(id, index) => self.save_attachment(ctx, id, index),
            Action::OpenAttachment(id, index) => self.open_attachment(id, index),
            Action::OpenInBrowser(id) => self.open_in_browser(id),
            Action::ShowImages { always } => {
                let Some(reader) = &mut self.reader else { return };
                reader.set_original(true);
                let sender = reader.detail.summary.from_addr.trim().to_lowercase();
                let name = reader.detail.summary.sender().to_owned();
                if always && !sender.is_empty() && !self.settings.shows_images_from(&sender) {
                    self.settings.image_senders.push(sender);
                    if let Err(err) = self.settings.save() {
                        self.toast(format!("Couldn't save settings: {err}"));
                    }
                    self.toast(format!("Mail from {name} will show images from now on"));
                }
            }
            Action::HideImages => {
                let Some(reader) = &mut self.reader else { return };
                reader.set_original(false);
                let sender = reader.detail.summary.from_addr.clone();
                let name = reader.detail.summary.sender().to_owned();
                if self.settings.shows_images_from(&sender) {
                    self.settings.image_senders.retain(|trusted| !trusted.eq_ignore_ascii_case(&sender));
                    if let Err(err) = self.settings.save() {
                        self.toast(format!("Couldn't save settings: {err}"));
                    }
                    self.toast(format!("Images from {name} are hidden again"));
                }
            }
            Action::TogglePlain => {
                if let Some(reader) = &mut self.reader {
                    reader.toggle_plain();
                }
            }
            Action::OpenUrl(url) => {
                if let Err(err) = open::that_detached(&url) {
                    self.toast(format!("Couldn't open the link: {err}"));
                }
            }
            Action::SyncNow => {
                self.tell_daemon(Request::Sync);
                self.toast("Checking for new mail");
            }
            Action::ShowScreen(screen) => self.screen = screen,
            Action::MarkAllRead => {
                let unread: Vec<i64> =
                    self.messages.iter().filter(|message| !message.seen).map(|message| message.id).collect();
                let count = unread.len();
                for id in unread {
                    self.queue(id, Op::Seen(true));
                }
                self.toast(format!("Marked {count} as read"));
            }
            Action::UndoSend(id) => match store::cancel_send(&self.db, id) {
                Ok(Some(outgoing)) => {
                    self.toasts
                        .retain(|toast| !matches!(toast.action, Some((_, Action::UndoSend(undo))) if undo == id));
                    self.reopen_outgoing(outgoing);
                }
                _ => self.toast("Too late, it has already gone."),
            },
            Action::RetrySend(id) => {
                let _ = store::retry_send(&self.db, id);
                self.tell_daemon(Request::Wake);
                self.dirty = true;
            }
            Action::EditOutgoing(id) => {
                if let Ok(Some(outgoing)) = store::cancel_send(&self.db, id) {
                    self.reopen_outgoing(outgoing);
                }
                self.dirty = true;
            }
            Action::OpenPalette => self.palette = Some(palette::Palette::default()),
            Action::WindowMinimize | Action::WindowMaximize | Action::WindowClose => {
                titlebar::window_command(ctx, &action)
            }
            Action::QuitEverything => {
                self.tell_daemon(Request::Quit);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::Next => self.move_selection(1),
            Action::Previous => self.move_selection(-1),
            Action::FocusSearch => {
                self.screen = Screen::Mail;
                self.focus_search = true;
            }
            Action::StartDaemon => match crate::autostart::start_daemon() {
                // The reconnect check in ui() picks it up within a second.
                Ok(()) => self.last_connect = ctx.input(|i| i.time) - 4.0,
                Err(err) => self.toast(format!("{err:#}")),
            },
            Action::LoadMore => {
                self.limit += PAGE;
                self.dirty = true;
            }
            Action::ToggleAccount(id) => {
                if !self.collapsed.remove(&id) {
                    self.collapsed.insert(id);
                }
            }
            Action::MoveAccount(from, slot) => {
                let to = moved_index(from, slot);
                if from < self.settings.accounts.len() && to != from {
                    let account = self.settings.accounts.remove(from);
                    self.settings.accounts.insert(to.min(self.settings.accounts.len()), account);
                    if let Err(err) = self.settings.save() {
                        self.toast(format!("Couldn't save the new order: {err}"));
                    }
                    self.tell_daemon(Request::Reload);
                }
            }
            Action::AddAccount => self.dialog = Some(Dialog::Account(Box::new(settings::AccountForm::new(None)))),
            Action::EditAccount(id) => {
                if let Some(account) = self.settings.account(&id).cloned() {
                    self.dialog = Some(Dialog::Account(Box::new(settings::AccountForm::new(Some(account)))));
                }
            }
            Action::RemoveAccount(id) => self.dialog = Some(Dialog::RemoveAccount(id)),
            Action::RemoveAccountConfirmed(id) => {
                self.dialog = None;
                self.settings.accounts.retain(|account| account.id != id);
                if let Err(err) = self.settings.save() {
                    self.toast(format!("Couldn't save settings: {err}"));
                }
                let _ = crate::credentials::forget(&id);
                let _ = store::forget_account(&self.db, &id);
                self.tell_daemon(Request::Reload);
                self.nav = Nav::Mailbox(View::Unified(Role::Inbox));
                self.reader = None;
                self.selected = None;
                self.dirty = true;
            }
            Action::SettingsChanged => {
                if let Err(err) = self.settings.save() {
                    self.toast(format!("Couldn't save settings: {err}"));
                }
                self.dirty = true;
                self.tell_daemon(Request::Reload);
            }
            Action::SetDoNotDisturb(on) => {
                self.settings.do_not_disturb = on;
                if let Err(err) = self.settings.save() {
                    self.toast(format!("Couldn't save settings: {err}"));
                }
                self.tell_daemon(Request::Reload);
                self.toast(if on { "Notifications paused" } else { "Notifications back on" });
            }
            Action::Toast(text) => self.toast(text),
            Action::CloseDialog => self.dialog = None,
        }
    }

    fn compose_new(&mut self, to: String, subject: String, body: String) {
        let account = match &self.nav {
            Nav::Mailbox(View::Folder(id)) => {
                self.folder(*id).and_then(|folder| self.settings.account(&folder.account))
            }
            _ => None,
        }
        .or(self.settings.accounts.first())
        .cloned();
        let Some(account) = account else {
            self.toast("Add an account first.");
            self.dialog = Some(Dialog::Account(Box::new(settings::AccountForm::new(None))));
            return;
        };
        let mut draft = crate::mail::compose::new_message(&account);
        draft.to = to;
        draft.subject = subject;
        if !body.is_empty() {
            draft.body = format!("{body}{}", draft.body);
        }
        self.start_compose(draft, None, "New message");
    }

    fn send(&mut self) {
        let Some(composer) = &mut self.composer else { return };
        let Some(account) = self.settings.account(&composer.draft.account).cloned() else {
            composer.error = Some("Choose an account to send from.".into());
            return;
        };
        let built = match crate::mail::compose::build(&account, &composer.draft) {
            Ok(built) => built,
            Err(err) => {
                composer.error = Some(format!("{err:#}"));
                return;
            }
        };
        let delay = i64::from(self.settings.undo_send_seconds);
        let subject = if composer.draft.subject.trim().is_empty() {
            "(no subject)".to_owned()
        } else {
            composer.draft.subject.clone()
        };
        let queued = store::queue_send(
            &self.db,
            &account.id,
            &built.raw,
            &built.sender,
            &built.recipients,
            &subject,
            composer.draft.replying_to,
            store::now() + delay,
        );
        match queued {
            Ok(id) => {
                let composer = self.composer.take().expect("composer");
                // Keep the draft text with the outgoing message, so Undo can bring it back.
                composer.discard(&self.db);
                self.pending_drafts.push((id, composer.draft));
                if delay > 0 {
                    self.toast_with("Sending…", "Undo", Action::UndoSend(id), delay as f64);
                } else {
                    self.toast("Sending…");
                }
                self.tell_daemon(Request::Wake);
                self.dirty = true;
            }
            Err(err) => composer.error = Some(format!("Couldn't queue the message: {err:#}")),
        }
    }

    fn reopen_outgoing(&mut self, outgoing: Outgoing) {
        let draft = self
            .pending_drafts
            .iter()
            .position(|(id, _)| *id == outgoing.id)
            .map(|index| self.pending_drafts.remove(index).1)
            .unwrap_or_else(|| draft_from_raw(&outgoing));
        self.start_compose(draft, None, "New message");
        self.dirty = true;
    }

    fn save_attachment(&mut self, ctx: &egui::Context, id: i64, index: usize) {
        let Some((name, bytes)) =
            store::raw(&self.db, id).ok().flatten().and_then(|raw| parse::attachment(&raw, index))
        else {
            self.toast("That attachment hasn't downloaded yet.");
            return;
        };
        let sender = self.background.0.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new().set_file_name(&name);
            let dialog = match dirs::download_dir() {
                Some(dir) => dialog.set_directory(dir),
                None => dialog,
            };
            if let Some(path) = dialog.save_file() {
                let text = match std::fs::write(&path, bytes) {
                    Ok(()) => format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy()),
                    Err(err) => format!("Couldn't save: {err}"),
                };
                let _ = sender.send(Background::Toast(text));
                ctx.request_repaint();
            }
        });
    }

    fn open_attachment(&mut self, id: i64, index: usize) {
        let Some((name, bytes)) =
            store::raw(&self.db, id).ok().flatten().and_then(|raw| parse::attachment(&raw, index))
        else {
            self.toast("That attachment hasn't downloaded yet.");
            return;
        };
        let dir = std::env::temp_dir().join("postie").join(id.to_string());
        let path = dir.join(sanitize(&name));
        let result = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, bytes));
        match result
            .map_err(|err| err.to_string())
            .and_then(|_| open::that_detached(&path).map_err(|err| err.to_string()))
        {
            Ok(()) => {}
            Err(err) => self.toast(format!("Couldn't open {name}: {err}")),
        }
    }

    fn open_in_browser(&mut self, id: i64) {
        let Some(html) = store::raw(&self.db, id).ok().flatten().and_then(|raw| parse::body(&raw).html) else { return };
        let path = std::env::temp_dir().join("postie").join(format!("message-{id}.html"));
        let result = crate::paths::write_atomic(&path, html.as_bytes()).map_err(|err| err.to_string());
        if let Err(err) = result.and_then(|_| open::that_detached(&path).map_err(|err| err.to_string())) {
            self.toast(format!("Couldn't open the browser: {err}"));
        }
    }

    fn handle_background(&mut self, message: Background) {
        match message {
            Background::Attach(files) => {
                if let Some(composer) = &mut self.composer {
                    for path in files {
                        let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        composer.draft.attachments.push(DraftAttachment { name, path, size });
                    }
                }
            }
            Background::AccountChecked(result) => {
                if let Some(Dialog::Account(form)) = &mut self.dialog {
                    form.checking = false;
                    match result {
                        Ok(token) => {
                            if let Some(name) = form.commit(&mut self.settings, token) {
                                self.dialog = None;
                                self.tell_daemon(Request::Reload);
                                self.toast(format!("{name} added. Downloading your mail now."));
                                self.screen = Screen::Mail;
                                self.dirty = true;
                            }
                        }
                        Err(err) => form.error = Some(err),
                    }
                }
            }
            Background::Toast(text) => self.toast(text),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let command_k = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K));
        if command_k {
            actions.push(Action::OpenPalette);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N)) {
            actions.push(Action::Compose);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F5)) {
            actions.push(Action::SyncNow);
        }
        let typing = ctx.egui_wants_keyboard_input();
        if typing || self.palette.is_some() || self.dialog.is_some() || self.screen != Screen::Mail {
            return;
        }
        let selected = self.selected;
        let seen = self.reader.as_ref().map(|reader| reader.detail.summary.seen);
        let flagged = self.reader.as_ref().map(|reader| reader.detail.summary.flagged);
        ctx.input(|input| {
            for event in &input.events {
                let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
                if modifiers.command || modifiers.alt {
                    continue;
                }
                match (key, selected) {
                    (Key::J | Key::ArrowDown, _) => actions.push(Action::Next),
                    (Key::K | Key::ArrowUp, _) => actions.push(Action::Previous),
                    (Key::C, _) => actions.push(Action::Compose),
                    (Key::Slash, _) => actions.push(Action::FocusSearch),
                    (Key::E, Some(id)) => actions.push(Action::Archive(id)),
                    (Key::Delete | Key::Backspace, Some(id)) => actions.push(Action::Delete(id)),
                    (Key::R, Some(id)) => actions.push(Action::Reply(id, false)),
                    (Key::A, Some(id)) => actions.push(Action::Reply(id, true)),
                    (Key::F, Some(id)) => actions.push(Action::Forward(id)),
                    (Key::S, Some(id)) => actions.push(Action::Flag(id, !flagged.unwrap_or(false))),
                    (Key::U, Some(id)) => actions.push(Action::Seen(id, !seen.unwrap_or(true))),
                    (Key::Escape, _) if !self.search.is_empty() => {
                        self.search.clear();
                        self.dirty = true;
                    }
                    _ => {}
                }
            }
        });
    }

    fn toasts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let now = ctx.input(|i| i.time);
        // Only the newest toast shows; an Undo toast stays put while a plain one passes over it.
        if let Some(toast) = self.toasts.last_mut() {
            toast.shown_at.get_or_insert(now);
        }
        self.toasts.retain(|toast| toast.shown_at.is_none_or(|shown| now < shown + toast.seconds));
        if self.toasts.len() > 1 {
            let last = self.toasts.len() - 1;
            let keep_undo = self.toasts[..last].iter().any(|toast| toast.action.is_some());
            if !keep_undo {
                self.toasts.drain(..last);
            }
        }
        let Some(toast) = self.toasts.last() else { return };
        let remaining = toast.shown_at.map_or(toast.seconds, |shown| shown + toast.seconds - now);
        let text = toast.text.clone();
        let action = toast.action.clone();
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(BG_ACTIVE)
                    .stroke(Stroke::new(1.0, LINE_STRONG))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 8))
                    .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            widgets::label(ui, text, theme::medium(13.0), FG);
                            if let Some((label, action)) = action {
                                ui.add_space(6.0);
                                if widgets::small_button(ui, &label, Kind::Accent).clicked() {
                                    actions.push(action);
                                }
                            }
                        });
                    });
            });
        ctx.request_repaint_after(Duration::from_secs_f64(remaining.clamp(0.05, 1.0)));
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let unread = self.unified_unread();
        let title = if unread > 0 { format!("Postie ({unread})") } else { "Postie".into() };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn raise(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        // Wayland lets a window take focus only with a token; Hyprland's own dispatcher doesn't need one.
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            std::thread::spawn(|| {
                let hyprctl = |args: &[&str]| {
                    std::process::Command::new("hyprctl")
                        .args(args)
                        .output()
                        .ok()
                        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
                };
                // Newer Hyprland takes Lua dispatchers; older takes the classic form.
                if hyprctl(&["dispatch", r#"hl.dsp.focus({ window = "class:^(postie)$" })"#]).as_deref() != Some("ok") {
                    hyprctl(&["dispatch", "focuswindow", "class:^(postie)$"]);
                }
            });
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();
        loop {
            let event = match self.events.try_recv() {
                Ok(event) => event,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.daemon = false;
                    break;
                }
            };
            match event {
                ipc::Event::Changed => self.dirty = true,
                ipc::Event::Focus => self.raise(&ctx),
                ipc::Event::Open { message } => self.open_message(message),
                ipc::Event::Compose { to, subject, body } => actions.push(Action::ComposeTo { to, subject, body }),
                ipc::Event::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
        while let Ok(message) = self.background.1.try_recv() {
            self.handle_background(message);
        }
        let now = ctx.input(|i| i.time);
        if self.search != self.searched {
            self.dirty = true;
        }
        if self.dirty && now - self.last_reload > 0.2 {
            self.last_reload = now;
            self.reload();
        } else if self.dirty {
            ctx.request_repaint_after(Duration::from_millis(220));
        }
        if !self.daemon && now - self.last_connect > 5.0 {
            self.last_connect = now;
            let (tx, rx) = channel();
            if subscribe(&ctx, tx) {
                self.events = rx;
                self.daemon = true;
                self.dirty = true;
            }
        }

        self.shortcuts(&ctx, &mut actions);
        egui::Panel::top("titlebar")
            .exact_size(titlebar::HEIGHT)
            .resizable(false)
            .frame(Frame::new().fill(BG).inner_margin(Margin::ZERO))
            .show(ui, |ui| titlebar::show(self, ui, &mut actions));
        egui::Panel::left("sidebar")
            .exact_size(236.0)
            .resizable(false)
            .frame(Frame::new().fill(BG_SOFT).inner_margin(Margin::symmetric(10, 14)))
            .show(ui, |ui| sidebar::show(self, ui, &mut actions));
        match self.screen {
            Screen::Mail => {
                egui::Panel::left("list")
                    .default_size(400.0)
                    .size_range(320.0..=600.0)
                    .resizable(true)
                    .frame(Frame::new().fill(BG).inner_margin(Margin::ZERO))
                    .show(ui, |ui| list::show(self, ui, &mut actions));
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(BG).inner_margin(Margin::ZERO))
                    .show(ui, |ui| reader::show(self, ui, &mut actions));
            }
            Screen::Settings => {
                egui::CentralPanel::default()
                    .frame(Frame::new().fill(BG).inner_margin(Margin::symmetric(32, 28)))
                    .show(ui, |ui| settings::show(self, ui, &mut actions));
            }
        }
        self.load_images(&ctx);
        compose::window(self, &ctx, &mut actions);
        palette::show(self, &ctx, &mut actions);
        settings::dialogs(self, &ctx, &mut actions);
        self.toasts(&ctx, &mut actions);
        self.update_title(&ctx);

        for action in actions {
            self.apply(&ctx, action);
        }
        if let Some((path, requested)) = &mut self.screenshot {
            if now > 7.0 && !*requested {
                *requested = true;
                let path = path.clone();
                ctx.request_screenshot(move |image| {
                    if let Err(err) = save_ppm(&path, &image) {
                        eprintln!("postie: couldn't save the screenshot: {err}");
                    }
                    std::process::exit(0);
                });
            }
            ctx.request_repaint();
        }
        ctx.request_repaint_after(Duration::from_secs(30));
    }
}

/// Hands image bytes to egui under an address it can decode, named for the format the bytes are in.
fn show_image(ctx: &egui::Context, src: &str, bytes: Vec<u8>) -> ImageState {
    let extension = match bytes.as_slice() {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, ..] => "jpg",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        _ => return ImageState::Failed,
    };
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut hasher);
    let uri = format!("bytes://postie-image/{:016x}.{extension}", hasher.finish());
    ctx.include_bytes(uri.clone(), bytes);
    ImageState::Ready(uri)
}

/// Fetched without cookies or referrer, capped at 10 MB.
fn fetch_image(url: &str) -> Option<Vec<u8>> {
    if !url.starts_with("http") {
        return None;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let mut response = agent.get(url).call().ok()?;
    response.body_mut().with_config().limit(10 << 20).read_to_vec().ok()
}

fn data_uri(src: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let (header, data) = src.strip_prefix("data:")?.split_once(',')?;
    if header.ends_with(";base64") {
        base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()
    } else {
        Some(data.as_bytes().to_vec())
    }
}

/// A binary PPM: no encoder needed, and any image tool converts it.
fn save_ppm(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [width, height] = image.size;
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.extend(image.pixels.iter().flat_map(|pixel| [pixel.r(), pixel.g(), pixel.b()]));
    std::fs::write(path, bytes)
}

fn subscribe(ctx: &egui::Context, sender: Sender<ipc::Event>) -> bool {
    let on_event_ctx = ctx.clone();
    let on_gone_ctx = ctx.clone();
    ipc::subscribe(
        move |event| {
            let _ = sender.send(event);
            on_event_ctx.request_repaint();
        },
        move || on_gone_ctx.request_repaint(),
    )
}

fn forwarded_attachments(db: &Connection, id: i64) -> Vec<DraftAttachment> {
    let Some(raw) = store::raw(db, id).ok().flatten() else { return Vec::new() };
    let body = parse::body(&raw);
    let dir = crate::paths::data_dir().join("forwarded").join(id.to_string());
    body.attachments
        .iter()
        .filter(|attachment| !attachment.inline)
        .filter_map(|attachment| {
            let (name, bytes) = parse::attachment(&raw, attachment.index)?;
            let path = dir.join(sanitize(&name));
            std::fs::create_dir_all(&dir).ok()?;
            std::fs::write(&path, &bytes).ok()?;
            Some(DraftAttachment { name, path, size: bytes.len() as u64 })
        })
        .collect()
}

/// A draft rebuilt from a message taken back out of the outbox after the window that wrote it closed.
fn draft_from_raw(outgoing: &Outgoing) -> Draft {
    let parsed = parse::parse(&outgoing.raw, store::now());
    let body = parse::body(&outgoing.raw);
    let people =
        |list: &[(String, String)]| list.iter().map(crate::mail::compose::format_person).collect::<Vec<_>>().join(", ");
    Draft {
        account: outgoing.account.clone(),
        to: people(&parsed.to),
        cc: people(&parsed.cc),
        subject: parsed.subject,
        body: body.text.unwrap_or_default(),
        in_reply_to: parsed.in_reply_to,
        references: parsed.references,
        replying_to: outgoing.replying_to,
        ..Default::default()
    }
}

/// Where an item ends up when dragged from `from` to just before the item at `slot`: the gap it
/// leaves shifts everything after it up by one.
fn moved_index(from: usize, slot: usize) -> usize {
    if slot > from { slot - 1 } else { slot }
}

pub fn sanitize(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    if clean.trim().is_empty() || clean == "." || clean == ".." { "attachment".into() } else { clean }
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b / (1 << 10)),
        b => format!("{b} B"),
    }
}

pub fn short_date(timestamp: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(date) = Local.timestamp_opt(timestamp, 0).single() else { return String::new() };
    let now = Local::now();
    if date.date_naive() == now.date_naive() {
        date.format("%H:%M").to_string()
    } else if (now.date_naive() - date.date_naive()).num_days() < 7 && date < now {
        date.format("%a").to_string()
    } else if date.year() == now.year() {
        date.format("%-d %b").to_string()
    } else {
        date.format("%-d %b %Y").to_string()
    }
}

pub fn long_date(timestamp: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|date| date.format("%a %-d %b %Y, %H:%M").to_string())
        .unwrap_or_default()
}

pub fn ago(timestamp: i64) -> String {
    let seconds = store::now() - timestamp;
    match seconds {
        s if s < 45 => "just now".into(),
        s if s < 3600 => format!("{}m ago", (s + 30) / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::moved_index;

    #[test]
    fn dragging_an_account_lands_where_the_line_was() {
        // Three accounts; the line sits before the item at `slot`, or at the end when slot == 3.
        assert_eq!(moved_index(0, 3), 2, "first to the end");
        assert_eq!(moved_index(2, 0), 0, "last to the top");
        assert_eq!(moved_index(1, 1), 1, "dropped where it was");
        assert_eq!(moved_index(1, 2), 1, "just below itself is still where it was");
        assert_eq!(moved_index(0, 2), 1, "first to between the other two");
    }
}
