//! The daemon's control socket: one JSON request per line, answered with `ok`. A window subscribes
//! and then hears about changes, focus requests and messages to open, one JSON event per line.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    Ping,
    /// Open the window, optionally at a message.
    Show {
        message: Option<i64>,
    },
    Compose {
        to: String,
        subject: String,
        body: String,
    },
    /// Check every account now.
    Sync,
    /// Something was queued in the store: replay changes and send what's due.
    Wake,
    /// Settings changed: restart workers for accounts that were added, edited or removed.
    Reload,
    Subscribe,
    Quit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Changed,
    Focus,
    Open { message: i64 },
    Compose { to: String, subject: String, body: String },
    Quit,
}

fn connect() -> std::io::Result<UnixStream> {
    let stream = UnixStream::connect(paths::socket())?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(stream)
}

pub fn send(request: &Request) -> std::io::Result<()> {
    let mut stream = connect()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    match answer.trim() {
        "ok" => Ok(()),
        // A newer Postie was installed and the daemon is restarting into it; it's back in a moment.
        "restarting" => {
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "the background service is restarting"))
        }
        _ => Err(std::io::Error::other(format!("daemon said {answer:?}"))),
    }
}

pub fn daemon_running() -> bool {
    send(&Request::Ping).is_ok()
}

/// Events for a window, read on a background thread until the daemon goes away.
pub fn subscribe(mut on_event: impl FnMut(Event) + Send + 'static, on_gone: impl FnOnce() + Send + 'static) -> bool {
    let Ok(mut stream) = connect() else { return false };
    let mut line = serde_json::to_string(&Request::Subscribe).expect("serialize");
    line.push('\n');
    if stream.write_all(line.as_bytes()).is_err() {
        return false;
    }
    std::thread::Builder::new()
        .name("daemon events".into())
        .spawn(move || {
            let reader = BufReader::new(stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Ok(event) = serde_json::from_str::<Event>(&line) {
                    on_event(event);
                }
            }
            on_gone();
        })
        .is_ok()
}
