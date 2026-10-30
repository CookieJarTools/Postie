//! Desktop integration: the launcher entry and icon, the mailto handler, and starting the background
//! service at login. A systemd user service is preferred (it restarts the daemon if it ever dies);
//! sessions without one get an XDG autostart entry.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::{icon, paths};

const SERVICE: &str = "postie.service";

fn systemd_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| paths::home().join(".config")).join("systemd/user")
}

fn service_file() -> PathBuf {
    systemd_dir().join(SERVICE)
}

fn xdg_autostart_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| paths::home().join(".config")).join("autostart/postie.desktop")
}

fn applications_file() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| paths::home().join(".local/share")).join("applications/postie.desktop")
}

pub fn icon_file() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| paths::home().join(".local/share"))
        .join("icons/hicolor/scalable/apps/postie.svg")
}

pub fn installed_binary() -> PathBuf {
    dirs::executable_dir().unwrap_or_else(|| paths::home().join(".local/bin")).join("postie")
}

/// What the launcher entry and the login service run. An AppImage's own binary lives in a
/// temporary mount, and The Jar replaces the file itself with a newly named one on each update,
/// so for an AppImage this is a small script that starts the newest AppImage in its folder.
fn exe() -> PathBuf {
    match std::env::var_os("APPIMAGE").map(PathBuf::from) {
        Some(appimage) => appimage_launcher(&appimage).unwrap_or(appimage),
        None => std::env::current_exe().unwrap_or_else(|_| installed_binary()),
    }
}

fn appimage_launcher(appimage: &std::path::Path) -> Option<PathBuf> {
    let folder = appimage.parent()?.to_string_lossy().replace('\'', "'\\''");
    let fallback = appimage.to_string_lossy().replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\n# Starts the newest Postie AppImage here, so updates don't leave this pointing at an old one.\n\
         fallback='{fallback}'\napp=$(ls -1v '{folder}'/*.AppImage 2>/dev/null | tail -n 1)\n\
         exec \"${{app:-$fallback}}\" \"$@\"\n"
    );
    let path = paths::data_dir().join("launch");
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() != Some(script.as_str()) {
        paths::write_atomic(&path, script.as_bytes()).ok()?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok()?;
    }
    Some(path)
}

/// The Jar writes its own launcher entry for the apps it installs, marked as its own.
fn entry_belongs_to_the_jar() -> bool {
    std::fs::read_to_string(applications_file())
        .is_ok_and(|text| text.lines().any(|line| line.starts_with("X-TheJar-App=")))
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A systemd user session that follows the graphical session, as with uwsm or start-hyprland.
pub fn has_systemd_session() -> bool {
    systemctl(&["is-active", "--quiet", "graphical-session.target"])
}

pub fn service_installed() -> bool {
    service_file().exists()
}

/// Writes the launcher entry and icon, so Postie shows up in app launchers and as the mailto handler.
pub fn integrate() -> Result<()> {
    let exe = exe();
    paths::write_atomic(&icon_file(), icon::SVG.as_bytes())?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Postie\nGenericName=Mail\nComment=Read and send email\n\
         Exec={} %u\nIcon=postie\nTerminal=false\nCategories=Network;Email;Office;\n\
         MimeType=x-scheme-handler/mailto;\nStartupWMClass=postie\nKeywords=email;mail;inbox;outlook;\n\
         Actions=compose;\n\n[Desktop Action compose]\nName=New Message\nExec={} compose\n",
        exe.display(),
        exe.display()
    );
    if !entry_belongs_to_the_jar() {
        paths::write_atomic(&applications_file(), entry.as_bytes())?;
    }
    if service_installed() {
        write_service()?;
    }
    if let Err(err) = skip_desktop_title_bar() {
        log::warn!("couldn't turn off the desktop's title bar for Postie: {err:#}");
    }
    Ok(())
}

/// Postie draws its own title bar. The Graphite shell gives Hyprland's title bar to every window
/// except the app ids listed in this file (its taskbar's "Hide title bar" writes it), so Postie
/// adds itself, the same as choosing that menu item.
fn skip_desktop_title_bar() -> Result<()> {
    let list = paths::home().join(".config/graphite/no-titlebar");
    let Ok(current) = std::fs::read_to_string(&list) else { return Ok(()) };
    if current.lines().any(|line| line.trim() == paths::APP_ID) {
        return Ok(());
    }
    let mut updated = current;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(paths::APP_ID);
    updated.push('\n');
    std::fs::write(&list, updated)?;
    Command::new("hyprctl").arg("reload").stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    Ok(())
}

fn write_service() -> Result<()> {
    let unit = format!(
        "[Unit]\nDescription=Postie mail notifications\nPartOf=graphical-session.target\n\
         After=graphical-session.target\n\n[Service]\nExecStart={} daemon\nRestart=on-failure\nRestartSec=10\n\n\
         [Install]\nWantedBy=graphical-session.target\n",
        exe().display()
    );
    let path = service_file();
    let unchanged = std::fs::read_to_string(&path).is_ok_and(|existing| existing == unit);
    if !unchanged {
        paths::write_atomic(&path, unit.as_bytes())?;
        systemctl(&["daemon-reload"]);
    }
    Ok(())
}

pub fn set_launch_at_login(on: bool) -> Result<()> {
    if on {
        if has_systemd_session() {
            write_service()?;
            anyhow::ensure!(systemctl(&["enable", SERVICE]), "systemctl couldn't enable {SERVICE}");
            let _ = std::fs::remove_file(xdg_autostart_file());
        } else {
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName=Postie\nComment=Mail notifications\nExec={} daemon\n\
                 Icon=postie\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
                exe().display()
            );
            paths::write_atomic(&xdg_autostart_file(), entry.as_bytes()).context("writing the autostart entry")?;
        }
    } else {
        if service_installed() {
            systemctl(&["disable", SERVICE]);
        }
        let _ = std::fs::remove_file(xdg_autostart_file());
    }
    Ok(())
}

/// Starts the background service, through systemd when it manages it.
pub fn start_daemon() -> Result<()> {
    if service_installed() && systemctl(&["start", SERVICE]) {
        return Ok(());
    }
    use std::os::unix::process::CommandExt;
    Command::new(exe())
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(daemon_log())
        .process_group(0)
        .spawn()
        .context("starting the background service")?;
    Ok(())
}

fn daemon_log() -> Stdio {
    let log = std::fs::OpenOptions::new().create(true).append(true).open(paths::data_dir().join("daemon.log"));
    log.map(Stdio::from).unwrap_or_else(|_| Stdio::null())
}

/// Whether systemd runs this process as Postie's own service. Started any other way, it may sit
/// inside some other app's service, which mustn't be restarted.
fn run_by_systemd() -> bool {
    let unit = format!("/{SERVICE}");
    std::fs::read_to_string("/proc/self/cgroup").is_ok_and(|cgroups| cgroups.lines().any(|line| line.ends_with(&unit)))
}

/// Starts the background service again, for when a newer Postie has replaced `binary`, the one this
/// service started from. Under systemd that's a restart of the service, which stops this process
/// too. Otherwise a small shell waits for this process to exit and then starts the new copy, so two
/// never run at once. Returns whether this process has to quit by itself.
pub fn restart_daemon(binary: &std::path::Path) -> Result<bool> {
    if run_by_systemd() {
        anyhow::ensure!(systemctl(&["--no-block", "restart", SERVICE]), "systemctl couldn't restart {SERVICE}");
        return Ok(false);
    }
    // Once replaced, this process's own path reads "postie (deleted)", so start the one it came from.
    let program = if std::env::var_os("APPIMAGE").is_some() { exe() } else { binary.to_path_buf() };
    use std::os::unix::process::CommandExt;
    Command::new("sh")
        .arg("-c")
        .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$0" daemon"#)
        .arg(program)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(daemon_log())
        .process_group(0)
        .spawn()
        .context("starting the new background service")?;
    Ok(true)
}

/// Copies this binary to ~/.local/bin and sets up the launcher, icon and login item from there.
pub fn install(launch_at_login: bool) -> Result<PathBuf> {
    anyhow::ensure!(
        std::env::var_os("APPIMAGE").is_none(),
        "The AppImage sets itself up the first time you open it; `install` is for builds from source."
    );
    let target = installed_binary();
    let current = std::env::current_exe()?;
    if current.canonicalize().ok() != target.canonicalize().ok() {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = target.with_extension("new");
        std::fs::copy(&current, &tmp).with_context(|| format!("copying to {}", tmp.display()))?;
        std::fs::rename(&tmp, &target)?;
    }
    let status =
        Command::new(&target).arg("integrate").arg(if launch_at_login { "--login" } else { "--no-login" }).status()?;
    anyhow::ensure!(status.success(), "setting up the desktop entry failed");
    Ok(target)
}
