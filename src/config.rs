//! Settings and accounts, saved as JSON beside the credentials. Passwords live in `credentials`.

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Security {
    /// TLS from the first byte (IMAP 993, SMTP 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143, SMTP 587).
    StartTls,
    /// No encryption. Only offered for local bridges such as Proton Bridge.
    None,
}

/// How an account signs in. The saved credential is a password, or Microsoft's refresh token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Auth {
    #[default]
    Password,
    Microsoft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    pub username: String,
    pub auth: Auth,
    pub imap: Server,
    pub smtp: Server,
    pub signature: String,
    pub notify: bool,
    /// Gmail and Outlook keep a copy of what SMTP sends, so appending one to Sent would duplicate it.
    pub server_saves_sent: bool,
}

impl Default for Account {
    fn default() -> Self {
        let blank = Server { host: String::new(), port: 993, security: Security::Tls };
        Self {
            id: String::new(),
            name: String::new(),
            email: String::new(),
            username: String::new(),
            auth: Auth::Password,
            imap: blank.clone(),
            smtp: Server { port: 465, ..blank },
            signature: String::new(),
            notify: true,
            server_saves_sent: false,
        }
    }
}

impl Account {
    pub fn label(&self) -> &str {
        if self.email.is_empty() { &self.name } else { &self.email }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub accounts: Vec<Account>,
    pub notifications: bool,
    pub notification_preview: bool,
    pub do_not_disturb: bool,
    pub launch_at_login: bool,
    pub sync_minutes: u32,
    /// How many of the newest messages per folder the first sync downloads; older ones come when asked
    /// for at the bottom of a list.
    pub initial_messages: u32,
    pub undo_send_seconds: u32,
    pub mark_read_on_open: bool,
    /// Inboxes split into Primary (people) and Other (newsletters, lists, machines).
    pub split_inbox: bool,
    /// Notify about mail sorted into Other too, not just Primary.
    pub notify_other: bool,
    /// Message text, read and written, in a fixed-width font.
    pub monospace_bodies: bool,
    /// Senders whose mail opens in the original view, images and all.
    pub image_senders: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            accounts: Vec::new(),
            notifications: true,
            notification_preview: true,
            do_not_disturb: false,
            launch_at_login: true,
            sync_minutes: 5,
            initial_messages: 400,
            undo_send_seconds: 8,
            mark_read_on_open: true,
            split_inbox: true,
            notify_other: false,
            monospace_bodies: false,
            image_senders: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read(paths::settings_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).expect("settings serialize");
        paths::write_atomic(&paths::settings_file(), &json)
    }

    pub fn shows_images_from(&self, sender: &str) -> bool {
        self.image_senders.iter().any(|trusted| trusted.eq_ignore_ascii_case(sender.trim()))
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.id == id)
    }
}

pub struct Preset {
    pub name: &'static str,
    pub domains: &'static [&'static str],
    pub imap: (&'static str, u16, Security),
    pub smtp: (&'static str, u16, Security),
    pub server_saves_sent: bool,
    /// What to tell someone about signing in, such as needing an app password.
    pub hint: &'static str,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "Gmail",
        domains: &["gmail.com", "googlemail.com"],
        imap: ("imap.gmail.com", 993, Security::Tls),
        smtp: ("smtp.gmail.com", 465, Security::Tls),
        server_saves_sent: true,
        hint: "Gmail needs an app password: turn on 2-Step Verification, then create one at myaccount.google.com/apppasswords.",
    },
    Preset {
        name: "iCloud",
        domains: &["icloud.com", "me.com", "mac.com"],
        imap: ("imap.mail.me.com", 993, Security::Tls),
        smtp: ("smtp.mail.me.com", 587, Security::StartTls),
        server_saves_sent: false,
        hint: "iCloud needs an app-specific password from account.apple.com.",
    },
    Preset {
        name: "Yahoo",
        domains: &["yahoo.com", "yahoo.co.uk", "ymail.com"],
        imap: ("imap.mail.yahoo.com", 993, Security::Tls),
        smtp: ("smtp.mail.yahoo.com", 465, Security::Tls),
        server_saves_sent: false,
        hint: "Yahoo needs an app password from Account Security.",
    },
    Preset {
        name: "Fastmail",
        domains: &["fastmail.com", "fastmail.fm"],
        imap: ("imap.fastmail.com", 993, Security::Tls),
        smtp: ("smtp.fastmail.com", 465, Security::Tls),
        server_saves_sent: false,
        hint: "Fastmail needs an app password from Settings > Privacy & Security.",
    },
    Preset {
        name: "Outlook",
        domains: &["outlook.com", "hotmail.com", "live.com", "msn.com", "hotmail.co.uk", "live.co.uk"],
        imap: ("outlook.office365.com", 993, Security::Tls),
        smtp: ("smtp-mail.outlook.com", 587, Security::StartTls),
        server_saves_sent: true,
        hint: "Outlook.com signs in with your Microsoft account in the browser; Postie never sees the password.",
    },
    Preset {
        name: "Zoho",
        domains: &["zoho.com", "zohomail.com"],
        imap: ("imap.zoho.com", 993, Security::Tls),
        smtp: ("smtp.zoho.com", 465, Security::Tls),
        server_saves_sent: false,
        hint: "Zoho needs IMAP turned on in its mail settings, and an app password if two-factor sign-in is on.",
    },
    Preset {
        name: "GMX",
        domains: &["gmx.com", "gmx.net", "gmx.de", "gmx.co.uk"],
        imap: ("imap.gmx.com", 993, Security::Tls),
        smtp: ("mail.gmx.com", 465, Security::Tls),
        server_saves_sent: false,
        hint: "GMX needs IMAP access turned on in its settings.",
    },
];

pub fn preset_for(email: &str) -> Option<&'static Preset> {
    let domain = email.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    PRESETS.iter().find(|preset| preset.domains.contains(&domain.as_str()))
}

/// A best guess for a domain with no preset: imap.<domain> and smtp.<domain>.
pub fn guess_servers(email: &str) -> (Server, Server) {
    let domain = email.rsplit_once('@').map(|(_, domain)| domain.trim().to_ascii_lowercase()).unwrap_or_default();
    (
        Server { host: format!("imap.{domain}"), port: 993, security: Security::Tls },
        Server { host: format!("smtp.{domain}"), port: 465, security: Security::Tls },
    )
}

pub fn new_account_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("a{:x}{:04x}", nanos, std::process::id() & 0xffff)
}
