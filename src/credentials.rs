//! Account passwords and Microsoft refresh tokens, kept apart from settings in a file only the user
//! can read (0600). What goes in here is app passwords and tokens, each of which can be revoked from
//! the provider without touching the real account password.

use std::collections::BTreeMap;

use crate::paths;

fn load_all() -> BTreeMap<String, String> {
    std::fs::read(paths::credentials_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_all(all: &BTreeMap<String, String>) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(all).expect("credentials serialize");
    paths::write_private(&paths::credentials_file(), &json)
}

pub fn password(account_id: &str) -> Option<String> {
    load_all().remove(account_id)
}

pub fn set_password(account_id: &str, password: &str) -> std::io::Result<()> {
    let mut all = load_all();
    all.insert(account_id.to_owned(), password.to_owned());
    save_all(&all)
}

pub fn forget(account_id: &str) -> std::io::Result<()> {
    let mut all = load_all();
    if all.remove(account_id).is_some() {
        save_all(&all)?;
    }
    Ok(())
}
