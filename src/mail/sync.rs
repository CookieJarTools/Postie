//! One worker per account, on its own thread: keeps an IMAP session open, replays queued changes,
//! sends what's due, syncs folders, and watches the inbox with IDLE so new mail shows up at once.

use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use async_imap::extensions::idle::IdleResponse;
use async_imap::types::{Fetch, Flag, Name, NameAttribute};
use futures::TryStreamExt;
use rusqlite::Connection;
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;

use super::net::{self, Imap};
use super::{parse, smtp};
use crate::config::{Account, Settings};
use crate::store::{self, Category, Flags, Op, Role};

/// Bodies up to this size download in the background; bigger ones wait until opened.
const AUTO_BODY_BYTES: i64 = 4 * 1024 * 1024;
const BODY_BATCH: i64 = 40;
const POLL_WITHOUT_IDLE: Duration = Duration::from_secs(120);
/// Under the 29 minutes RFC 2177 allows, and short enough for home routers that forget quiet connections.
const IDLE_RENEW: Duration = Duration::from_secs(9 * 60);
/// The longest the main connection sits unused; an inbox check then keeps it alive.
const KEEPALIVE: Duration = Duration::from_secs(4 * 60);
const MAX_ATTEMPTS: i64 = 3;

pub enum Command {
    /// Something was queued: ops, or a message to send.
    Wake,
    SyncNow,
    Shutdown,
}

#[derive(Clone, Debug)]
pub struct NewMail {
    pub id: i64,
    pub from: String,
    pub subject: String,
    pub snippet: String,
    pub category: Category,
}

#[derive(Debug)]
pub enum Event {
    Changed,
    NewMail { account: String, items: Vec<NewMail> },
    SendFailed { subject: String, error: String },
}

pub struct Handle {
    pub account: Account,
    commands: UnboundedSender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn stop(mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn spawn(account: Account, settings: &Settings, events: UnboundedSender<Event>) -> Handle {
    let (commands, receiver) = unbounded_channel();
    let sync_every = Duration::from_secs(u64::from(settings.sync_minutes.max(1)) * 60);
    let initial = settings.initial_messages.max(50);
    let worker_account = account.clone();
    let thread = std::thread::Builder::new()
        .name(format!("sync {}", account.email))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, run(worker_account, sync_every, initial, events, receiver));
        })
        .expect("spawn sync thread");
    Handle { account, commands, thread: Some(thread) }
}

async fn run(
    account: Account,
    sync_every: Duration,
    initial: u32,
    events: UnboundedSender<Event>,
    mut commands: UnboundedReceiver<Command>,
) {
    let db = match store::open() {
        Ok(db) => db,
        Err(err) => {
            log::error!("{}: can't open the mail store: {err:#}", account.email);
            return;
        }
    };
    let mut worker = Worker {
        account,
        sync_every,
        initial,
        db,
        events,
        wake: Rc::new(Notify::new()),
        full_sync: Rc::new(Cell::new(true)),
        selected: None,
        caps: Caps::default(),
        moved: HashSet::new(),
        backoff: Duration::from_secs(10),
        retry_quietly: false,
    };
    let wake = worker.wake.clone();
    let full_sync = worker.full_sync.clone();
    let pump = async move {
        while let Some(command) = commands.recv().await {
            match command {
                Command::Wake => wake.notify_one(),
                Command::SyncNow => {
                    full_sync.set(true);
                    wake.notify_one();
                }
                Command::Shutdown => break,
            }
        }
    };
    tokio::select! {
        _ = pump => {}
        _ = worker.keep_running() => {}
    }
}

#[derive(Default)]
struct Caps {
    idle: bool,
    mv: bool,
    uidplus: bool,
}

struct Worker {
    account: Account,
    sync_every: Duration,
    initial: u32,
    db: Connection,
    events: UnboundedSender<Event>,
    wake: Rc<Notify>,
    full_sync: Rc<Cell<bool>>,
    selected: Option<String>,
    caps: Caps,
    /// Message ids this worker moved, so one moved back into the inbox isn't announced as new.
    moved: HashSet<String>,
    backoff: Duration,
    /// A connection that worked and then dropped gets one quiet reconnect before anyone is told.
    retry_quietly: bool,
}

/// Stops the IDLE watcher when the session it belongs to ends.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Worker {
    async fn keep_running(&mut self) {
        loop {
            if let Err(err) = self.session().await {
                if net::is_connection_error(&err) && std::mem::take(&mut self.retry_quietly) {
                    log::info!("{}: connection dropped ({err:#}), reconnecting", self.account.email);
                    continue;
                }
                log::warn!("{}: {err:#}", self.account.email);
                self.status("error", Some(&format!("{err:#}")));
                let wake = self.wake.clone();
                tokio::select! {
                    _ = tokio::time::sleep(self.backoff) => {}
                    _ = wake.notified() => {}
                }
                self.backoff = (self.backoff * 2).min(Duration::from_secs(600));
            }
        }
    }

    fn status(&self, state: &str, error: Option<&str>) {
        let _ = store::set_status(&self.db, &self.account.id, state, error);
        let _ = self.events.send(Event::Changed);
    }

    fn changed(&self) {
        let _ = self.events.send(Event::Changed);
    }

    async fn session(&mut self) -> Result<()> {
        self.status("connecting", None);
        let secret = super::secret(&self.account)?;
        let mut session = net::connect(&self.account, &secret).await?;
        self.backoff = Duration::from_secs(10);
        self.retry_quietly = true;
        self.selected = None;
        let caps = session.capabilities().await?;
        self.caps = Caps { idle: caps.has_str("IDLE"), mv: caps.has_str("MOVE"), uidplus: caps.has_str("UIDPLUS") };

        let _idle = self
            .caps
            .idle
            .then(|| AbortOnDrop(tokio::task::spawn_local(watch_inbox(self.account.clone(), self.wake.clone()))));
        let mut next_full = Instant::now();
        loop {
            self.run_ops(&mut session).await?;
            self.send_due(&mut session).await?;
            if self.full_sync.replace(false) || Instant::now() >= next_full {
                self.status("syncing", None);
                self.list_folders(&mut session).await?;
                self.sync_folders(&mut session, None).await?;
                next_full = Instant::now() + self.sync_every;
            } else {
                self.sync_folders(&mut session, Some(Role::Inbox)).await?;
            }
            let more_bodies = self.fetch_bodies(&mut session).await?;
            self.status("idle", None);
            if more_bodies {
                continue;
            }

            let mut deadline = next_full.min(Instant::now() + KEEPALIVE);
            if !self.caps.idle {
                deadline = deadline.min(Instant::now() + POLL_WITHOUT_IDLE);
            }
            if let Ok(Some(at)) = store::next_send_at(&self.db) {
                let wait = (at - store::now()).max(0) as u64;
                deadline = deadline.min(Instant::now() + Duration::from_secs(wait));
            }
            let wake = self.wake.clone();
            let waiting = Instant::now();
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep_until(deadline) => {}
            }
            if waiting.elapsed() > Duration::from_secs(30) {
                confirm(&mut session).await?;
            }
        }
    }

    async fn select(&mut self, session: &mut Imap, path: &str) -> Result<async_imap::types::Mailbox> {
        let mailbox = session.select(path).await?;
        self.selected = Some(path.to_owned());
        Ok(mailbox)
    }

    async fn ensure_selected(&mut self, session: &mut Imap, path: &str) -> Result<()> {
        if self.selected.as_deref() != Some(path) {
            self.select(session, path).await?;
        }
        Ok(())
    }

    // ---------- folders ----------

    async fn list_folders(&mut self, session: &mut Imap) -> Result<()> {
        let names: Vec<Name> = session.list(Some(""), Some("*")).await?.try_collect().await?;
        let selectable: Vec<&Name> = names
            .iter()
            .filter(|name| {
                !name.attributes().iter().any(|attr| match attr {
                    NameAttribute::NoSelect => true,
                    // Gmail's Important is a view of mail already in other folders.
                    NameAttribute::Extension(ext) => {
                        ext.eq_ignore_ascii_case("\\NonExistent") || ext.eq_ignore_ascii_case("\\Important")
                    }
                    _ => false,
                })
            })
            .collect();

        let mut roles: BTreeMap<String, Role> = BTreeMap::new();
        let mut taken: HashSet<Role> = HashSet::new();
        for name in &selectable {
            if let Some(role) = special_use(name).filter(|role| !taken.contains(role)) {
                roles.insert(name.name().to_owned(), role);
                taken.insert(role);
            }
        }
        for name in &selectable {
            if roles.contains_key(name.name()) {
                continue;
            }
            let role = guess_role(name.name(), name.delimiter()).filter(|role| !taken.contains(role));
            if let Some(role) = role {
                taken.insert(role);
            }
            roles.insert(name.name().to_owned(), role.unwrap_or(Role::Other));
        }

        let mut keep = Vec::new();
        for name in &selectable {
            let path = name.name();
            let role = roles[path];
            let display = display_name(path, name.delimiter(), role);
            store::upsert_folder(&self.db, &self.account.id, path, &display, role, name.delimiter())?;
            keep.push(path.to_owned());
        }
        store::retain_folders(&self.db, &self.account.id, &keep)?;
        self.changed();
        Ok(())
    }

    async fn sync_folders(&mut self, session: &mut Imap, only: Option<Role>) -> Result<()> {
        let mut folders: Vec<(i64, String, Role)> = store::folders(&self.db)?
            .into_iter()
            .filter(|folder| folder.account == self.account.id && folder.role.synced())
            .filter(|folder| only.is_none_or(|role| folder.role == role))
            .map(|folder| (folder.id, folder.path, folder.role))
            .collect();
        // Sent first: who you write to and what you sent decide what's Primary in the inbox.
        folders.sort_by_key(|(_, _, role)| *role != Role::Sent);
        for (id, path, role) in folders {
            let new_mail = self.sync_folder(session, id, &path, role).await?;
            if !new_mail.is_empty() {
                let _ = self.events.send(Event::NewMail { account: self.account.id.clone(), items: new_mail });
            }
        }
        Ok(())
    }

    async fn sync_folder(&mut self, session: &mut Imap, folder: i64, path: &str, role: Role) -> Result<Vec<NewMail>> {
        let mailbox = self.select(session, path).await?;
        let validity = mailbox.uid_validity.unwrap_or(0);
        let mut state = store::folder_state(&self.db, folder)?;
        if state.synced && state.uidvalidity != validity {
            store::clear_folder(&self.db, folder)?;
            state = store::folder_state(&self.db, folder)?;
            self.changed();
        }
        let first = !state.synced;

        if !first {
            self.sync_flags(session, folder, mailbox.exists).await?;
        }

        let mut uids: Vec<u32> = if mailbox.exists == 0 {
            Vec::new()
        } else if first {
            let mut all: Vec<u32> = session.uid_search("ALL").await?.into_iter().collect();
            all.sort_unstable();
            all.split_off(all.len().saturating_sub(self.initial as usize))
        } else if mailbox.uid_next.is_some_and(|next| next <= state.last_uid + 1) {
            Vec::new()
        } else {
            let found = session.uid_search(format!("UID {}:*", state.last_uid + 1)).await?;
            let mut newer: Vec<u32> = found.into_iter().filter(|uid| *uid > state.last_uid).collect();
            newer.sort_unstable();
            newer
        };
        uids.reverse();

        // The first sync takes headers only so the list appears quickly; bodies follow in the background.
        let query = if first {
            "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER])"
        } else {
            "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[])"
        };
        let mut last_uid = state.last_uid;
        let mut new_mail = Vec::new();
        for chunk in uids.chunks(if first { 100 } else { 20 }) {
            let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            let fetches: Vec<Fetch> = session.uid_fetch(&set, query).await?.try_collect().await?;
            if fetches.len() < chunk.len() {
                confirm(session).await?;
            }
            let tx = self.db.transaction()?;
            for fetch in &fetches {
                let Some(uid) = fetch.uid else { continue };
                let Some(bytes) = (if first { fetch.header() } else { fetch.body() }) else { continue };
                let received = fetch.internal_date().map(|date| date.timestamp());
                let mut parsed = parse::parse(bytes, received.unwrap_or_else(store::now));
                // Sorted by when it arrived, as Outlook does: a sender's clock can be wrong.
                if let Some(received) = received {
                    parsed.date = received;
                }
                let flags = flags_of(fetch);
                let raw = (!first).then_some(bytes);
                let size = fetch.size.unwrap_or(bytes.len() as u32);
                let category = store::classify(&tx, &parsed, &self.account.email)?;
                let (id, inserted) =
                    store::upsert_message(&tx, &self.account.id, folder, uid, flags, size, &parsed, raw, category)?;
                last_uid = last_uid.max(uid);
                if !inserted {
                    continue;
                }
                store::remember_contacts(&tx, std::slice::from_ref(&parsed.from), 1, false, parsed.date)?;
                if role == Role::Sent {
                    let recipients = [parsed.to.clone(), parsed.cc.clone()].concat();
                    store::remember_contacts(&tx, &recipients, 2, true, parsed.date)?;
                }
                let moved_here = self.moved.remove(&parsed.message_id);
                if !first && role == Role::Inbox && !flags.seen && !moved_here {
                    let from = if parsed.from.0.is_empty() { parsed.from.1.clone() } else { parsed.from.0.clone() };
                    new_mail.push(NewMail {
                        id,
                        from,
                        subject: parsed.subject.clone(),
                        snippet: parsed.snippet.clone(),
                        category,
                    });
                }
            }
            tx.commit()?;
            self.changed();
        }
        store::set_folder_state(&self.db, folder, validity, last_uid)?;
        new_mail.reverse();
        Ok(new_mail)
    }

    /// Picks up flags changed elsewhere and drops what was deleted elsewhere.
    async fn sync_flags(&mut self, session: &mut Imap, folder: i64, exists: u32) -> Result<()> {
        let known = store::known_uids(&self.db, folder)?;
        let Some(lowest) = known.iter().min().copied() else { return Ok(()) };
        let mut present = HashSet::new();
        let mut changed = false;
        if exists > 0 {
            let fetches: Vec<Fetch> =
                session.uid_fetch(format!("{lowest}:*"), "(UID FLAGS)").await?.try_collect().await?;
            confirm(session).await?;
            for fetch in &fetches {
                if let Some(uid) = fetch.uid {
                    present.insert(uid);
                    changed |= store::update_flags(&self.db, folder, uid, flags_of(fetch))?;
                }
            }
            if present.is_empty() {
                // `n:*` always matches the newest message, so an empty answer is the server misbehaving.
                log::warn!(
                    "{}: empty flag sync for a folder with {exists} messages, keeping everything",
                    self.account.email
                );
                return Ok(());
            }
        }
        let gone: Vec<u32> = known.into_iter().filter(|uid| !present.contains(uid)).collect();
        if !gone.is_empty() {
            store::delete_uids(&self.db, folder, &gone)?;
            changed = true;
        }
        if changed {
            self.changed();
        }
        Ok(())
    }

    /// Downloads a batch of bodies the first sync skipped. True when more are waiting.
    async fn fetch_bodies(&mut self, session: &mut Imap) -> Result<bool> {
        let missing = store::missing_bodies(&self.db, &self.account.id, AUTO_BODY_BYTES, BODY_BATCH)?;
        if missing.is_empty() {
            return Ok(false);
        }
        let mut by_folder: BTreeMap<String, Vec<(i64, u32)>> = BTreeMap::new();
        for item in &missing {
            by_folder.entry(item.folder_path.clone()).or_default().push((item.id, item.uid));
        }
        for (path, items) in by_folder {
            self.ensure_selected(session, &path).await?;
            self.fetch_into(session, &items).await?;
        }
        self.changed();
        Ok(missing.len() as i64 == BODY_BATCH)
    }

    async fn fetch_into(&mut self, session: &mut Imap, items: &[(i64, u32)]) -> Result<()> {
        let set = items.iter().map(|(_, uid)| uid.to_string()).collect::<Vec<_>>().join(",");
        let fetches: Vec<Fetch> = session.uid_fetch(&set, "(UID BODY.PEEK[])").await?.try_collect().await?;
        if fetches.len() < items.len() {
            confirm(session).await?;
        }
        let mut got = HashSet::new();
        for fetch in &fetches {
            let (Some(uid), Some(raw)) = (fetch.uid, fetch.body()) else { continue };
            let Some((id, _)) = items.iter().find(|(_, wanted)| *wanted == uid) else { continue };
            let fallback = fetch.internal_date().map(|date| date.timestamp()).unwrap_or_else(store::now);
            store::store_body(&self.db, *id, &parse::parse(raw, fallback), raw)?;
            got.insert(uid);
        }
        // Gone from the server since; the next flag sync removes them. Until then, stop asking.
        for (id, uid) in items {
            if !got.contains(uid) {
                self.db.execute("UPDATE messages SET has_body = 2 WHERE id = ?1", [id])?;
            }
        }
        Ok(())
    }

    // ---------- queued changes ----------

    async fn run_ops(&mut self, session: &mut Imap) -> Result<()> {
        let ops = store::pending_ops(&self.db, &self.account.id)?;
        if ops.is_empty() {
            return Ok(());
        }
        for queued in ops {
            let Some((folder, path, uid)) = store::location(&self.db, queued.message)? else {
                store::finish_op(&self.db, queued.id)?;
                continue;
            };
            let result = self.apply(session, &queued.op, queued.message, folder, &path, uid).await;
            log::debug!("{}: {:?} on {path} uid {uid}: {result:?}", self.account.email, queued.op);
            match result {
                Ok(()) => store::finish_op(&self.db, queued.id)?,
                Err(err) if net::is_connection_error(&err) => return Err(err),
                Err(err) => {
                    log::warn!("{}: {:?} failed: {err:#}", self.account.email, queued.op);
                    store::fail_op(&self.db, queued.id, &format!("{err:#}"))?;
                    if queued.attempts + 1 >= MAX_ATTEMPTS {
                        store::finish_op(&self.db, queued.id)?;
                        store::unhide(&self.db, queued.message)?;
                    }
                }
            }
        }
        self.changed();
        Ok(())
    }

    async fn apply(
        &mut self,
        session: &mut Imap,
        op: &Op,
        message: i64,
        folder: i64,
        path: &str,
        uid: u32,
    ) -> Result<()> {
        self.ensure_selected(session, path).await?;
        match op {
            Op::Seen(on) => self.store_flag(session, uid, "\\Seen", *on).await,
            Op::Flagged(on) => self.store_flag(session, uid, "\\Flagged", *on).await,
            Op::Answered => self.store_flag(session, uid, "\\Answered", true).await,
            Op::FetchBody => self.fetch_into(session, &[(message, uid)]).await,
            Op::Move { to } => {
                let dest = store::folder_path(&self.db, *to)?.ok_or_else(|| anyhow!("that folder is gone"))?;
                self.move_message(session, message, uid, &dest).await
            }
            Op::Archive => {
                let dest = self.archive_folder(session).await?;
                if dest == path {
                    return Ok(());
                }
                self.move_message(session, message, uid, &dest).await
            }
            Op::Delete => {
                let trash = store::folder_by_role(&self.db, &self.account.id, Role::Trash)?;
                match trash {
                    Some((trash_id, trash_path)) if trash_id != folder => {
                        self.move_message(session, message, uid, &trash_path).await
                    }
                    _ => {
                        self.expunge(session, uid).await?;
                        store::delete_message(&self.db, message)
                    }
                }
            }
        }
    }

    async fn store_flag(&mut self, session: &mut Imap, uid: u32, flag: &str, on: bool) -> Result<()> {
        let sign = if on { '+' } else { '-' };
        // Run so the tagged reply is checked: a dead socket or a NO must not pass as done.
        session.run_command_and_check_ok(format!("UID STORE {uid} {sign}FLAGS.SILENT ({flag})")).await?;
        Ok(())
    }

    async fn expunge(&mut self, session: &mut Imap, uid: u32) -> Result<()> {
        self.store_flag(session, uid, "\\Deleted", true).await?;
        let command = if self.caps.uidplus { format!("UID EXPUNGE {uid}") } else { "EXPUNGE".to_owned() };
        session.run_command_and_check_ok(command).await?;
        Ok(())
    }

    async fn move_message(&mut self, session: &mut Imap, message: i64, uid: u32, dest: &str) -> Result<()> {
        if self.caps.mv {
            session.uid_mv(uid.to_string(), dest).await?;
        } else {
            session.uid_copy(uid.to_string(), dest).await?;
            self.expunge(session, uid).await?;
        }
        let message_id: String =
            self.db.query_row("SELECT message_id FROM messages WHERE id = ?1", [message], |row| row.get(0))?;
        if !message_id.is_empty() {
            self.moved.insert(message_id);
        }
        store::delete_message(&self.db, message)
    }

    /// The account's archive: its Archive folder, Gmail's All Mail, or a new "Archive".
    async fn archive_folder(&mut self, session: &mut Imap) -> Result<String> {
        for role in [Role::Archive, Role::All] {
            if let Some((_, path)) = store::folder_by_role(&self.db, &self.account.id, role)? {
                return Ok(path);
            }
        }
        session.create("Archive").await?;
        store::upsert_folder(&self.db, &self.account.id, "Archive", "Archive", Role::Archive, None)?;
        Ok("Archive".into())
    }

    // ---------- sending ----------

    async fn send_due(&mut self, session: &mut Imap) -> Result<()> {
        let due = store::claim_due(&self.db, &self.account.id)?;
        for outgoing in due {
            let sent = match super::secret(&self.account) {
                Ok(secret) => smtp::send(&self.account, &secret, &outgoing).await,
                Err(err) => Err(err),
            };
            match sent {
                Ok(()) => {
                    store::sent(&self.db, outgoing.id)?;
                    let people: Vec<(String, String)> =
                        outgoing.recipients.iter().map(|addr| (String::new(), addr.clone())).collect();
                    store::remember_contacts(&self.db, &people, 3, true, store::now())?;
                    if let Some(original) = outgoing.replying_to {
                        store::set_answered(&self.db, original)?;
                        store::queue(&self.db, &self.account.id, original, &Op::Answered)?;
                    }
                    if !self.account.server_saves_sent {
                        self.save_sent_copy(session, &outgoing.raw).await;
                    }
                }
                Err(err) => {
                    log::warn!("{}: sending failed: {err:#}", self.account.email);
                    store::send_failed(&self.db, outgoing.id, &format!("{err:#}"))?;
                    let _ =
                        self.events.send(Event::SendFailed { subject: outgoing.subject, error: format!("{err:#}") });
                }
            }
            self.changed();
        }
        Ok(())
    }

    async fn save_sent_copy(&mut self, session: &mut Imap, raw: &[u8]) {
        let Ok(Some((_, sent))) = store::folder_by_role(&self.db, &self.account.id, Role::Sent) else { return };
        if let Err(err) = session.append(&sent, Some("(\\Seen)"), None, raw).await {
            log::warn!("{}: couldn't save to {sent}: {err}", self.account.email);
        }
    }
}

/// A second connection that sits in IDLE on the inbox and wakes the worker when anything changes.
async fn watch_inbox(account: Account, wake: Rc<Notify>) {
    let mut pause = Duration::from_secs(3);
    loop {
        let started = Instant::now();
        let watched = match super::secret(&account) {
            Ok(secret) => idle_session(&account, &secret, &wake).await,
            Err(err) => Err(err),
        };
        if let Err(err) = watched {
            log::info!("{}: IDLE stopped ({err:#}), retrying", account.email);
        }
        // Mail may have arrived while the watcher was down.
        wake.notify_one();
        pause = if started.elapsed() > Duration::from_secs(60) {
            Duration::from_secs(3)
        } else {
            (pause * 2).min(Duration::from_secs(120))
        };
        tokio::time::sleep(pause).await;
    }
}

async fn idle_session(account: &Account, secret: &super::Secret, wake: &Notify) -> Result<()> {
    let mut session = net::connect(account, secret).await?;
    session.select("INBOX").await?;
    loop {
        let mut idle = session.idle();
        idle.init().await?;
        let response = {
            let (wait, _stop) = idle.wait_with_timeout(IDLE_RENEW);
            wait.await?
        };
        session = idle.done().await?;
        if let IdleResponse::NewData(_) = response {
            wake.notify_one();
        }
    }
}

/// async-imap ends a FETCH the same way on OK, NO and a dropped connection. A NOOP that comes back OK
/// shows the connection is alive, so a short answer before it was the server's real answer.
async fn confirm(session: &mut Imap) -> Result<()> {
    session.run_command_and_check_ok("NOOP").await?;
    Ok(())
}

fn flags_of(fetch: &Fetch) -> Flags {
    let mut flags = Flags::default();
    for flag in fetch.flags() {
        match flag {
            Flag::Seen => flags.seen = true,
            Flag::Flagged => flags.flagged = true,
            Flag::Answered => flags.answered = true,
            _ => {}
        }
    }
    flags
}

fn special_use(name: &Name) -> Option<Role> {
    if name.name().eq_ignore_ascii_case("INBOX") {
        return Some(Role::Inbox);
    }
    name.attributes().iter().find_map(|attr| match attr {
        NameAttribute::Sent => Some(Role::Sent),
        NameAttribute::Drafts => Some(Role::Drafts),
        NameAttribute::Trash => Some(Role::Trash),
        NameAttribute::Junk => Some(Role::Junk),
        NameAttribute::Archive => Some(Role::Archive),
        NameAttribute::All => Some(Role::All),
        NameAttribute::Flagged => Some(Role::Flagged),
        _ => None,
    })
}

fn leaf<'a>(path: &'a str, delimiter: Option<&str>) -> &'a str {
    match delimiter {
        Some(delim) if !delim.is_empty() => path.rsplit(delim).next().unwrap_or(path),
        _ => path,
    }
}

/// For servers without SPECIAL-USE, the usual names in English.
fn guess_role(path: &str, delimiter: Option<&str>) -> Option<Role> {
    let name = leaf(path, delimiter).to_lowercase();
    Some(match name.as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" => Role::Sent,
        "drafts" | "draft" => Role::Drafts,
        "trash" | "deleted items" | "deleted messages" | "bin" | "deleted" => Role::Trash,
        "junk" | "spam" | "junk email" | "junk e-mail" | "bulk mail" => Role::Junk,
        "archive" | "archives" => Role::Archive,
        _ => return None,
    })
}

fn display_name(path: &str, delimiter: Option<&str>, role: Role) -> String {
    match role {
        Role::Inbox => "Inbox".into(),
        Role::Sent => "Sent".into(),
        Role::Drafts => "Drafts".into(),
        Role::Trash => "Trash".into(),
        Role::Junk => "Junk".into(),
        Role::Archive => "Archive".into(),
        _ => super::decode_utf7(leaf(path, delimiter)),
    }
}
