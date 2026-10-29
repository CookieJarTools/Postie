use std::path::{Path, PathBuf};

pub const APP_ID: &str = "postie";

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| home().join(".config")).join(APP_ID)
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| home().join(".local/share")).join(APP_ID)
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn credentials_file() -> PathBuf {
    config_dir().join("credentials.json")
}

pub fn database() -> PathBuf {
    data_dir().join("mail.db")
}

/// The daemon's control socket. Lives in the runtime dir so it disappears at logout.
pub fn socket() -> PathBuf {
    let runtime = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    runtime.join(format!("{APP_ID}.sock"))
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Replaces `path` whole: a temporary file beside it, then a rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// Like [`write_atomic`], readable only by the user.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)
}
