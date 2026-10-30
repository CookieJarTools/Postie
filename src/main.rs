//! Postie: a mail client in the house style. `postie daemon` runs at login with the tray icon and
//! notifications; `postie` opens the window through it.

mod autostart;
mod config;
mod credentials;
mod daemon;
mod gui;
mod icon;
mod ipc;
mod mail;
mod paths;
mod store;

use anyhow::Result;

use crate::ipc::Request;

const USAGE: &str = "\
Usage: postie [command]

  (none)            Open the window, starting the background service if needed
  mailto:...        Write a message to that address
  compose           Write a new message
  sync              Check every account now
  quit              Stop the background service
  daemon            Run the background service (tray icon and notifications)
  window            Open the window directly
  install           Copy postie to ~/.local/bin and start it at login (--no-login to skip that)
";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("postie=info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = run(&args) {
        eprintln!("postie: {err:#}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    let first = args.first().map(String::as_str);
    match first {
        None => open(Request::Show { message: None }),
        Some(link) if link.starts_with("mailto:") => {
            let (to, subject, body) = parse_mailto(link);
            open(Request::Compose { to, subject, body })
        }
        Some("compose") => open(Request::Compose { to: String::new(), subject: String::new(), body: String::new() }),
        Some("daemon") => daemon::run(),
        Some("window") => {
            let message = args.iter().position(|arg| arg == "--message").and_then(|i| args.get(i + 1)?.parse().ok());
            gui::run(message)
        }
        Some("sync") => Ok(ipc::send(&Request::Sync)?),
        Some("quit") => {
            if ipc::daemon_running() {
                ipc::send(&Request::Quit)?;
            }
            Ok(())
        }
        Some("install") => {
            let login = !args.iter().any(|arg| arg == "--no-login");
            let target = autostart::install(login)?;
            println!("Installed {}", target.display());
            if login {
                println!("Postie now starts at login and sits in the tray. Starting it now.");
            }
            autostart::start_daemon()?;
            Ok(())
        }
        Some("integrate") => {
            autostart::integrate()?;
            let login = !args.iter().any(|arg| arg == "--no-login");
            autostart::set_launch_at_login(login)?;
            let mut settings = config::Settings::load();
            settings.launch_at_login = login;
            settings.save()?;
            Ok(())
        }
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

/// Hands the request to the background service, starting it first if it isn't running. If it's
/// restarting into a newer version, it comes back by itself, so this waits for that instead.
fn open(request: Request) -> Result<()> {
    let (mut restarting, mut started) = (false, false);
    for _ in 0..200 {
        match ipc::send(&request) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => restarting = true,
            Err(_) if !restarting && !started => {
                autostart::start_daemon()?;
                started = true;
            }
            Err(_) => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    anyhow::bail!("the background service didn't start; try `postie daemon` to see why")
}

fn parse_mailto(link: &str) -> (String, String, String) {
    let rest = link.trim_start_matches("mailto:");
    let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut subject = String::new();
    let mut body = String::new();
    let mut to = percent_decode(to);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key.to_ascii_lowercase().as_str() {
            "subject" => subject = percent_decode(value),
            "body" => body = percent_decode(value),
            "to" | "cc" => {
                if !to.is_empty() {
                    to.push_str(", ");
                }
                to.push_str(&percent_decode(value));
            }
            _ => {}
        }
    }
    (to, subject, body)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::parse_mailto;

    #[test]
    fn reads_mailto_links() {
        let (to, subject, body) = parse_mailto("mailto:ann@x.com?subject=Hello%20there&body=Hi%2C+Ann");
        assert_eq!(to, "ann@x.com");
        assert_eq!(subject, "Hello there");
        assert_eq!(body, "Hi, Ann");
    }
}
