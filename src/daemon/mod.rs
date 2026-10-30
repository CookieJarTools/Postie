//! The background service started at login: one sync worker per account, the tray icon, desktop
//! notifications for new mail, and the control socket the window and command line talk to.
//!
//! The window is a separate process the daemon starts on demand, so closing it frees all of its
//! memory and GPU context while notifications carry on.

mod notify;
mod tray;

use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::config::Settings;
use crate::ipc::{self, Event, Request};
use crate::mail::sync::{self, Command as WorkerCommand};
use crate::{autostart, paths, store};

pub struct Hub {
    settings: Mutex<Settings>,
    workers: Mutex<HashMap<String, sync::Handle>>,
    subscribers: Mutex<Vec<UnboundedSender<Event>>>,
    /// Events for a window that's still starting, handed over when it subscribes.
    pending: Mutex<Vec<Event>>,
    window: Mutex<Option<Child>>,
    worker_events: UnboundedSender<sync::Event>,
    tray: Mutex<Option<ksni::Handle<tray::PostieTray>>>,
    runtime: tokio::runtime::Handle,
    quit: Notify,
}

impl Hub {
    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn reload(&self) {
        let settings = Settings::load();
        // Workers read these when they start, so a change to any of them restarts every worker.
        let sync_settings = |s: &Settings| (s.download_all, s.initial_messages, s.sync_minutes);
        let resync = sync_settings(&self.settings.lock().unwrap()) != sync_settings(&settings);
        let mut workers = self.workers.lock().unwrap();
        let stale: Vec<String> = workers
            .iter()
            .filter(|(id, handle)| resync || settings.account(id) != Some(&handle.account))
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(handle) = workers.remove(&id) {
                handle.stop();
            }
            crate::mail::oauth::forget(&id);
        }
        for account in &settings.accounts {
            if !workers.contains_key(&account.id) {
                let handle = sync::spawn(account.clone(), &settings, self.worker_events.clone());
                workers.insert(account.id.clone(), handle);
            }
        }
        drop(workers);
        let dnd = settings.do_not_disturb;
        *self.settings.lock().unwrap() = settings;
        self.update_tray(move |tray| tray.dnd = dnd);
    }

    fn tell_workers(&self, command: fn() -> WorkerCommand) {
        for handle in self.workers.lock().unwrap().values() {
            handle.send(command());
        }
    }

    pub fn wake(&self) {
        self.tell_workers(|| WorkerCommand::Wake);
    }

    pub fn sync_now(&self) {
        self.tell_workers(|| WorkerCommand::SyncNow);
    }

    fn broadcast(&self, event: Event) {
        self.subscribers.lock().unwrap().retain(|subscriber| subscriber.send(event.clone()).is_ok());
    }

    fn window_open(&self) -> bool {
        let mut window = self.window.lock().unwrap();
        let alive = window.as_mut().is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !alive {
            *window = None;
        }
        alive
    }

    /// Brings the window up, starting it if needed, and passes it `then` once it's listening.
    pub fn show(&self, then: Option<Event>) {
        let listening = {
            let mut subscribers = self.subscribers.lock().unwrap();
            subscribers.retain(|subscriber| !subscriber.is_closed());
            !subscribers.is_empty()
        };
        if listening {
            self.broadcast(Event::Focus);
            if let Some(event) = then {
                self.broadcast(event);
            }
            return;
        }
        if let Some(event) = then {
            self.pending.lock().unwrap().push(event);
        }
        if self.window_open() {
            return;
        }
        match std::env::current_exe().and_then(|exe| Command::new(exe).arg("window").spawn()) {
            Ok(child) => *self.window.lock().unwrap() = Some(child),
            Err(err) => log::error!("couldn't open the window: {err}"),
        }
    }

    pub fn set_do_not_disturb(&self, on: bool) {
        let mut settings = Settings::load();
        settings.do_not_disturb = on;
        if let Err(err) = settings.save() {
            log::warn!("couldn't save settings: {err}");
        }
        self.settings.lock().unwrap().do_not_disturb = on;
        self.broadcast(Event::Changed);
    }

    pub fn quit(&self) {
        self.broadcast(Event::Quit);
        self.quit.notify_one();
    }

    fn update_tray(&self, change: impl FnOnce(&mut tray::PostieTray) + Send + 'static) {
        let Some(handle) = self.tray.lock().unwrap().clone() else { return };
        self.runtime.spawn(async move {
            handle.update(change).await;
        });
    }

    fn refresh_unread(&self, db: &rusqlite::Connection) {
        let primary_only = self.settings.lock().unwrap().split_inbox;
        let unread = store::unread_in_inboxes(db, primary_only).unwrap_or(0);
        self.update_tray(move |tray| tray.unread = unread);
    }

    fn handle_request(self: &Arc<Self>, request: Request) {
        match request {
            Request::Ping | Request::Subscribe => {}
            Request::Show { message } => self.show(message.map(|message| Event::Open { message })),
            Request::Compose { to, subject, body } => self.show(Some(Event::Compose { to, subject, body })),
            Request::Sync => self.sync_now(),
            Request::Wake => {
                self.wake();
                // The window changed the store: the tray's unread count may be different now.
                let _ = self.worker_events.send(sync::Event::Changed);
            }
            Request::Reload => {
                let _ = self.worker_events.send(sync::Event::Changed);
                let hub = self.clone();
                // Stopping a worker joins its thread, so keep that off the async workers.
                self.runtime.spawn_blocking(move || hub.reload());
            }
            Request::Quit => self.quit(),
        }
    }
}

pub fn run() -> Result<()> {
    if ipc::daemon_running() {
        log::info!("already running");
        return Ok(());
    }
    let socket = paths::socket();
    let _ = std::fs::remove_file(&socket);
    if let Err(err) = autostart::integrate() {
        log::warn!("couldn't write the desktop entry: {err:#}");
    }
    store::open().context("opening the mail store")?;

    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(serve(socket))
}

async fn serve(socket: std::path::PathBuf) -> Result<()> {
    let listener = UnixListener::bind(&socket).with_context(|| format!("binding {}", socket.display()))?;
    let (worker_events, mut events) = unbounded_channel();
    let settings = Settings::load();
    let hub = Arc::new(Hub {
        settings: Mutex::new(settings.clone()),
        workers: Mutex::new(HashMap::new()),
        subscribers: Mutex::new(Vec::new()),
        pending: Mutex::new(Vec::new()),
        window: Mutex::new(None),
        worker_events,
        tray: Mutex::new(None),
        runtime: tokio::runtime::Handle::current(),
        quit: Notify::new(),
    });

    match tray::spawn(hub.clone(), settings.do_not_disturb).await {
        Ok(handle) => *hub.tray.lock().unwrap() = Some(handle),
        Err(err) => log::warn!("no tray icon: {err}"),
    }
    {
        let hub = hub.clone();
        tokio::task::spawn_blocking(move || hub.reload()).await?;
    }
    tokio::spawn(accept(listener, hub.clone()));

    let db = store::open()?;
    hub.refresh_unread(&db);
    let mut tick = tokio::time::interval(Duration::from_millis(400));
    let mut dirty = false;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            Some(event) = events.recv() => match event {
                sync::Event::Changed => dirty = true,
                sync::Event::NewMail { account, items } => {
                    dirty = true;
                    notify::new_mail(&hub, &account, items);
                }
                sync::Event::SendFailed { subject, error } => notify::send_failed(&hub, &subject, &error),
            },
            _ = tick.tick() => {
                if dirty {
                    dirty = false;
                    hub.broadcast(Event::Changed);
                    hub.refresh_unread(&db);
                }
            }
            _ = hub.quit.notified() => break,
            _ = terminate.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    let _ = std::fs::remove_file(&socket);
    let workers: Vec<sync::Handle> = hub.workers.lock().unwrap().drain().map(|(_, handle)| handle).collect();
    tokio::task::spawn_blocking(move || workers.into_iter().for_each(sync::Handle::stop)).await?;
    let tray = hub.tray.lock().unwrap().take();
    if let Some(tray) = tray {
        tray.shutdown().await;
    }
    Ok(())
}

async fn accept(listener: UnixListener, hub: Arc<Hub>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(client(stream, hub.clone()));
            }
            Err(err) => {
                log::warn!("control socket: {err}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn client(stream: UnixStream, hub: Arc<Hub>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(request) = serde_json::from_str::<Request>(&line) else {
            let _ = write.write_all(b"error\n").await;
            continue;
        };
        if let Request::Subscribe = request {
            let (sender, mut receiver) = unbounded_channel();
            for event in hub.pending.lock().unwrap().drain(..) {
                let _ = sender.send(event);
            }
            hub.subscribers.lock().unwrap().push(sender);
            // Until the window goes away: its end of the socket closing is how we find out.
            loop {
                tokio::select! {
                    event = receiver.recv() => {
                        let Some(event) = event else { break };
                        let mut line = serde_json::to_string(&event).unwrap_or_default();
                        line.push('\n');
                        if write.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    line = lines.next_line() => {
                        if !matches!(line, Ok(Some(_))) {
                            break;
                        }
                    }
                }
            }
            return;
        }
        hub.handle_request(request);
        if write.write_all(b"ok\n").await.is_err() {
            return;
        }
    }
}
