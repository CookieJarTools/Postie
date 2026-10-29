//! Desktop notifications for new mail, with Open, Mark as read and Archive right on the notification.

use std::sync::Arc;

use notify_rust::{Hint, Notification, Timeout};

use super::Hub;
use crate::autostart;
use crate::ipc::Event;
use crate::mail::sync::NewMail;
use crate::store::{self, Category, Op};

/// More than this many at once become one summary notification instead of a flood.
const SEPARATE_LIMIT: usize = 3;

fn base(summary: &str, body: &str) -> Notification {
    let mut notification = Notification::new();
    notification
        .appname("Postie")
        .summary(summary)
        .body(body)
        .icon(&autostart::icon_file().to_string_lossy())
        .hint(Hint::DesktopEntry("postie".into()))
        .hint(Hint::Category("email.arrived".into()))
        .timeout(Timeout::Default);
    notification
}

/// Notification text is read as markup by most servers.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn new_mail(hub: &Arc<Hub>, account_id: &str, mut items: Vec<NewMail>) {
    let settings = hub.settings();
    let Some(account) = settings.account(account_id) else { return };
    if settings.split_inbox && !settings.notify_other {
        items.retain(|item| item.category == Category::Primary);
    }
    if !settings.notifications || settings.do_not_disturb || !account.notify || items.is_empty() {
        return;
    }
    if items.len() > SEPARATE_LIMIT {
        let mut senders: Vec<&str> = Vec::new();
        for item in &items {
            if !senders.contains(&item.from.as_str()) {
                senders.push(&item.from);
            }
        }
        let names = match senders.len() {
            1 => senders[0].to_owned(),
            2 => format!("{} and {}", senders[0], senders[1]),
            n => format!("{}, {} and {} others", senders[0], senders[1], n - 2),
        };
        let mut notification = base(&format!("{} new messages", items.len()), &escape(&format!("From {names}")));
        notification.action("default", "Open");
        show(hub.clone(), notification, None);
        return;
    }
    for item in items {
        let subject = if item.subject.is_empty() { "(no subject)" } else { &item.subject };
        let body = if settings.notification_preview && !item.snippet.is_empty() {
            format!("<b>{}</b>\n{}", escape(subject), escape(&item.snippet))
        } else {
            format!("<b>{}</b>", escape(subject))
        };
        let summary =
            if settings.accounts.len() > 1 { format!("{} · {}", item.from, account.email) } else { item.from.clone() };
        let mut notification = base(&summary, &body);
        notification.action("default", "Open").action("read", "Mark as read").action("archive", "Archive");
        show(hub.clone(), notification, Some((account_id.to_owned(), item.id)));
    }
}

pub fn send_failed(hub: &Arc<Hub>, subject: &str, error: &str) {
    let mut notification = base("Couldn't send your message", &escape(&format!("\u{201c}{subject}\u{201d}: {error}")));
    notification.hint(Hint::Category("email".into())).action("default", "Open");
    show(hub.clone(), notification, None);
}

/// Shows it on a thread of its own, which then waits for the click.
fn show(hub: Arc<Hub>, notification: Notification, message: Option<(String, i64)>) {
    std::thread::spawn(move || {
        let handle = match notification.show() {
            Ok(handle) => handle,
            Err(err) => {
                log::warn!("notification failed: {err}");
                return;
            }
        };
        handle.wait_for_action(|action| match (action, &message) {
            ("default", Some((_, id))) => hub.show(Some(Event::Open { message: *id })),
            ("default", None) => hub.show(None),
            ("read", Some((account, id))) => act(&hub, account, *id, Op::Seen(true)),
            ("archive", Some((account, id))) => act(&hub, account, *id, Op::Archive),
            _ => {}
        });
    });
}

fn act(hub: &Hub, account: &str, id: i64, op: Op) {
    let result = store::open().and_then(|db| {
        match op {
            Op::Seen(seen) => store::set_seen(&db, id, seen)?,
            _ => store::hide(&db, id)?,
        }
        store::queue(&db, account, id, &op)
    });
    match result {
        Ok(()) => hub.wake(),
        Err(err) => log::warn!("couldn't act on the notification: {err:#}"),
    }
}
