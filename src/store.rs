//! The local mail store, shared by the daemon (which syncs into it) and the window (which reads it and
//! queues changes). SQLite in WAL mode, so both processes can have it open at once.
//!
//! Changes made in the window are applied here straight away and queued in `ops`; the daemon replays
//! them on the server. That keeps the window instant and working offline.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::mail::parse::Parsed;
use crate::paths;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS folders(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT '',
    delimiter TEXT,
    uidvalidity INTEGER NOT NULL DEFAULT 0,
    last_uid INTEGER NOT NULL DEFAULT 0,
    synced INTEGER NOT NULL DEFAULT 0,
    server_count INTEGER NOT NULL DEFAULT 0,
    fetch_older INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account, path)
);
CREATE TABLE IF NOT EXISTS messages(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    folder INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL,
    message_id TEXT NOT NULL DEFAULT '',
    in_reply_to TEXT NOT NULL DEFAULT '',
    refs TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    from_name TEXT NOT NULL DEFAULT '',
    from_addr TEXT NOT NULL DEFAULT '',
    to_list TEXT NOT NULL DEFAULT '[]',
    cc_list TEXT NOT NULL DEFAULT '[]',
    reply_to TEXT NOT NULL DEFAULT '[]',
    date INTEGER NOT NULL DEFAULT 0,
    snippet TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL DEFAULT 0,
    seen INTEGER NOT NULL DEFAULT 0,
    flagged INTEGER NOT NULL DEFAULT 0,
    answered INTEGER NOT NULL DEFAULT 0,
    attachments INTEGER NOT NULL DEFAULT 0,
    has_body INTEGER NOT NULL DEFAULT 0,
    hidden INTEGER NOT NULL DEFAULT 0,
    category INTEGER NOT NULL DEFAULT 0,
    UNIQUE(folder, uid)
);
CREATE INDEX IF NOT EXISTS messages_by_folder ON messages(folder, hidden, date DESC);
CREATE INDEX IF NOT EXISTS messages_by_date ON messages(date DESC);
CREATE TABLE IF NOT EXISTS raw(
    message INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    data BLOB NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(subject, people, body, tokenize = 'unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS messages_unsearch AFTER DELETE ON messages BEGIN
    DELETE FROM search WHERE rowid = old.id;
END;
CREATE TABLE IF NOT EXISTS ops(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    message INTEGER NOT NULL,
    op TEXT NOT NULL,
    created INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT
);
CREATE TABLE IF NOT EXISTS outbox(
    id INTEGER PRIMARY KEY,
    account TEXT NOT NULL,
    raw BLOB NOT NULL,
    sender TEXT NOT NULL,
    recipients TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    replying_to INTEGER,
    send_after INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    error TEXT
);
CREATE TABLE IF NOT EXISTS drafts(
    id INTEGER PRIMARY KEY,
    data TEXT NOT NULL,
    updated INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS contacts(
    addr TEXT PRIMARY KEY,
    name TEXT NOT NULL DEFAULT '',
    weight INTEGER NOT NULL DEFAULT 0,
    last INTEGER NOT NULL DEFAULT 0,
    sent INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sender_rules(
    addr TEXT PRIMARY KEY,
    category INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS status(
    account TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    error TEXT,
    at INTEGER NOT NULL
);
"#;

pub fn open() -> Result<Connection> {
    std::fs::create_dir_all(paths::data_dir())?;
    prepare(Connection::open(paths::database())?)
}

fn prepare(conn: Connection) -> Result<Connection> {
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    conn.execute_batch(SCHEMA)?;
    add_column(&conn, "messages", "category", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(&conn, "contacts", "sent", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(&conn, "folders", "server_count", "INTEGER NOT NULL DEFAULT 0")?;
    add_column(&conn, "folders", "fetch_older", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(conn)
}

/// Brings a store made by an older version up to date.
fn add_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        &format!("SELECT COUNT(*) > 0 FROM pragma_table_info('{table}') WHERE name = ?1"),
        [column],
        |row| row.get(0),
    )?;
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"))?;
    }
    Ok(())
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    /// Gmail's All Mail: everything, so not synced, but it is where archiving goes.
    All,
    /// Gmail's Starred and similar views of flagged mail. Not synced; flags cover it.
    Flagged,
    Other,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Inbox => "inbox",
            Role::Drafts => "drafts",
            Role::Sent => "sent",
            Role::Archive => "archive",
            Role::Junk => "junk",
            Role::Trash => "trash",
            Role::All => "all",
            Role::Flagged => "flagged",
            Role::Other => "",
        }
    }

    pub fn parse(text: &str) -> Role {
        match text {
            "inbox" => Role::Inbox,
            "drafts" => Role::Drafts,
            "sent" => Role::Sent,
            "archive" => Role::Archive,
            "junk" => Role::Junk,
            "trash" => Role::Trash,
            "all" => Role::All,
            "flagged" => Role::Flagged,
            _ => Role::Other,
        }
    }

    /// Order in the sidebar; other folders follow alphabetically.
    pub fn rank(self) -> u8 {
        match self {
            Role::Inbox => 0,
            Role::Drafts => 1,
            Role::Sent => 2,
            Role::Archive => 3,
            Role::Junk => 4,
            Role::Trash => 5,
            Role::All => 6,
            Role::Flagged => 7,
            Role::Other => 8,
        }
    }

    pub fn synced(self) -> bool {
        !matches!(self, Role::All | Role::Flagged)
    }
}

/// Which part of a split inbox a message sits in, as Outlook's Focused and Other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Category {
    #[default]
    Primary,
    Other,
}

impl Category {
    pub fn to_sql(self) -> i64 {
        match self {
            Category::Primary => 0,
            Category::Other => 1,
        }
    }

    pub fn from_sql(value: i64) -> Category {
        if value == 1 { Category::Other } else { Category::Primary }
    }

    pub fn name(self) -> &'static str {
        match self {
            Category::Primary => "Primary",
            Category::Other => "Other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Folder {
    pub id: i64,
    pub account: String,
    pub path: String,
    pub name: String,
    pub role: Role,
    pub delimiter: Option<String>,
    pub unread: i64,
    /// Unread in Primary: what an inbox's badge shows when inboxes are split.
    pub unread_primary: i64,
    pub total: i64,
    /// How many messages the server had at the last sync, to tell how much is still to download.
    pub server_count: i64,
    /// Older messages asked for and not downloaded yet: a count, [`ALL_OLDER`], or 0.
    pub fetch_older: i64,
}

impl Folder {
    /// Messages on the server that aren't on this computer.
    pub fn on_server_only(&self) -> i64 {
        (self.server_count - self.total).max(0)
    }

    /// How many of those are on their way, having been asked for.
    pub fn downloading(&self) -> i64 {
        match self.fetch_older {
            ALL_OLDER => self.on_server_only(),
            wanted => wanted.min(self.on_server_only()),
        }
    }

    /// How deep the folder sits under its parents, for indenting in the sidebar.
    pub fn depth(&self) -> usize {
        match &self.delimiter {
            Some(delim) if !delim.is_empty() => self.path.matches(delim.as_str()).count(),
            _ => 0,
        }
    }
}

pub fn folders(conn: &Connection) -> Result<Vec<Folder>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.account, f.path, f.name, f.role, f.delimiter,
                (SELECT COUNT(*) FROM messages m WHERE m.folder = f.id AND m.hidden = 0 AND m.seen = 0),
                (SELECT COUNT(*) FROM messages m WHERE m.folder = f.id AND m.hidden = 0),
                (SELECT COUNT(*) FROM messages m WHERE m.folder = f.id AND m.hidden = 0 AND m.seen = 0 AND m.category = 0),
                f.server_count, f.fetch_older
         FROM folders f",
    )?;
    let mut folders: Vec<Folder> = stmt
        .query_map([], |row| {
            Ok(Folder {
                id: row.get(0)?,
                account: row.get(1)?,
                path: row.get(2)?,
                name: row.get(3)?,
                role: Role::parse(&row.get::<_, String>(4)?),
                delimiter: row.get(5)?,
                unread: row.get(6)?,
                total: row.get(7)?,
                unread_primary: row.get(8)?,
                server_count: row.get(9)?,
                fetch_older: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    folders.sort_by_key(|folder| (folder.role.rank(), folder.path.to_lowercase()));
    Ok(folders)
}

pub fn upsert_folder(
    conn: &Connection,
    account: &str,
    path: &str,
    name: &str,
    role: Role,
    delimiter: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO folders(account, path, name, role, delimiter) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(account, path) DO UPDATE SET name = excluded.name, role = excluded.role, delimiter = excluded.delimiter",
        params![account, path, name, role.as_str(), delimiter],
    )?;
    Ok(conn.query_row("SELECT id FROM folders WHERE account = ?1 AND path = ?2", params![account, path], |row| {
        row.get(0)
    })?)
}

/// Drops folders the server no longer lists, with their messages.
pub fn retain_folders(conn: &Connection, account: &str, keep: &[String]) -> Result<()> {
    let existing: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT id, path FROM folders WHERE account = ?1")?;
        stmt.query_map([account], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };
    for (id, path) in existing {
        if !keep.contains(&path) {
            conn.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

pub fn folder_by_role(conn: &Connection, account: &str, role: Role) -> Result<Option<(i64, String)>> {
    Ok(conn
        .query_row(
            "SELECT id, path FROM folders WHERE account = ?1 AND role = ?2 LIMIT 1",
            params![account, role.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn folder_path(conn: &Connection, folder: i64) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT path FROM folders WHERE id = ?1", [folder], |row| row.get(0)).optional()?)
}

pub struct FolderState {
    pub uidvalidity: u32,
    pub last_uid: u32,
    pub synced: bool,
}

pub fn folder_state(conn: &Connection, folder: i64) -> Result<FolderState> {
    Ok(conn.query_row("SELECT uidvalidity, last_uid, synced FROM folders WHERE id = ?1", [folder], |row| {
        Ok(FolderState {
            uidvalidity: row.get::<_, i64>(0)? as u32,
            last_uid: row.get::<_, i64>(1)? as u32,
            synced: row.get::<_, i64>(2)? != 0,
        })
    })?)
}

pub fn set_folder_state(conn: &Connection, folder: i64, uidvalidity: u32, last_uid: u32) -> Result<()> {
    conn.execute(
        "UPDATE folders SET uidvalidity = ?2, last_uid = ?3, synced = 1 WHERE id = ?1",
        params![folder, uidvalidity as i64, last_uid as i64],
    )?;
    Ok(())
}

pub fn set_server_count(conn: &Connection, folder: i64, count: u32) -> Result<()> {
    conn.execute("UPDATE folders SET server_count = ?2 WHERE id = ?1", params![folder, count as i64])?;
    Ok(())
}

/// `fetch_older` meaning every older message the server has.
pub const ALL_OLDER: i64 = -1;

/// Asks for `count` more older messages of a folder, or all of them with [`ALL_OLDER`].
pub fn request_older(conn: &Connection, folder: i64, count: i64) -> Result<()> {
    conn.execute(
        "UPDATE folders SET fetch_older = CASE
             WHEN ?2 = -1 OR fetch_older = -1 THEN -1
             ELSE fetch_older + ?2 END
         WHERE id = ?1",
        params![folder, count],
    )?;
    Ok(())
}

/// Folders of an account with older messages asked for, most important first, with how many.
pub fn folders_wanting_older(conn: &Connection, account: &str) -> Result<Vec<(i64, String, Role, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, role, fetch_older FROM folders WHERE account = ?1 AND synced = 1 AND fetch_older != 0",
    )?;
    let mut folders: Vec<(i64, String, Role, i64)> = stmt
        .query_map([account], |row| {
            Ok((row.get(0)?, row.get(1)?, Role::parse(&row.get::<_, String>(2)?), row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    folders.retain(|(_, _, role, _)| role.synced());
    folders.sort_by_key(|(_, _, role, _)| role.rank());
    Ok(folders)
}

/// Counts off older messages that have arrived; `finished` when the server has no more.
pub fn fetched_older(conn: &Connection, folder: i64, count: usize, finished: bool) -> Result<()> {
    conn.execute(
        "UPDATE folders SET fetch_older = CASE
             WHEN ?3 THEN 0
             WHEN fetch_older = -1 THEN -1
             ELSE MAX(fetch_older - ?2, 0) END
         WHERE id = ?1",
        params![folder, count as i64, finished],
    )?;
    Ok(())
}

pub fn clear_folder(conn: &Connection, folder: i64) -> Result<()> {
    conn.execute("DELETE FROM messages WHERE folder = ?1", [folder])?;
    conn.execute("UPDATE folders SET uidvalidity = 0, last_uid = 0, synced = 0 WHERE id = ?1", [folder])?;
    Ok(())
}

pub fn known_uids(conn: &Connection, folder: i64) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare("SELECT uid FROM messages WHERE folder = ?1")?;
    let uids = stmt.query_map([folder], |row| row.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(uids.into_iter().map(|uid| uid as u32).collect())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
}

/// Saves a message the server sent. Returns its id and whether it is new to the store.
#[allow(clippy::too_many_arguments)]
pub fn upsert_message(
    tx: &Transaction,
    account: &str,
    folder: i64,
    uid: u32,
    flags: Flags,
    size: u32,
    parsed: &Parsed,
    raw: Option<&[u8]>,
    category: Category,
) -> Result<(i64, bool)> {
    let existing: Option<i64> = tx
        .query_row("SELECT id FROM messages WHERE folder = ?1 AND uid = ?2", params![folder, uid as i64], |row| {
            row.get(0)
        })
        .optional()?;
    if let Some(id) = existing {
        return Ok((id, false));
    }
    let people = |list: &[(String, String)]| serde_json::to_string(list).unwrap_or_else(|_| "[]".into());
    tx.execute(
        "INSERT INTO messages(account, folder, uid, message_id, in_reply_to, refs, subject, from_name, from_addr,
                              to_list, cc_list, reply_to, date, snippet, size, seen, flagged, answered, attachments, has_body,
                              category)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            account,
            folder,
            uid as i64,
            parsed.message_id,
            parsed.in_reply_to,
            parsed.references,
            parsed.subject,
            parsed.from.0,
            parsed.from.1,
            people(&parsed.to),
            people(&parsed.cc),
            people(&parsed.reply_to),
            parsed.date,
            parsed.snippet,
            size as i64,
            flags.seen,
            flags.flagged,
            flags.answered,
            parsed.has_attachments,
            raw.is_some(),
            category.to_sql(),
        ],
    )?;
    let id = tx.last_insert_rowid();
    if let Some(raw) = raw {
        tx.execute("INSERT OR REPLACE INTO raw(message, data) VALUES (?1, ?2)", params![id, raw])?;
    }
    let people_text = std::iter::once(&parsed.from)
        .chain(parsed.to.iter())
        .chain(parsed.cc.iter())
        .map(|(name, addr)| format!("{name} {addr}"))
        .collect::<Vec<_>>()
        .join(" ");
    tx.execute(
        "INSERT INTO search(rowid, subject, people, body) VALUES (?1, ?2, ?3, ?4)",
        params![id, parsed.subject, people_text, parsed.search_text],
    )?;
    Ok((id, true))
}

/// Primary or Other for a new message to `me`, in order of what the user has said and done:
/// a choice made for this sender, a reply to something they sent, high importance, someone they
/// write to, and then the marks of bulk mail.
pub fn classify(conn: &Connection, parsed: &Parsed, me: &str) -> Result<Category> {
    let sender = parsed.from.1.trim().to_lowercase();
    let rule: Option<i64> =
        conn.query_row("SELECT category FROM sender_rules WHERE addr = ?1", [&sender], |row| row.get(0)).optional()?;
    if let Some(rule) = rule {
        return Ok(Category::from_sql(rule));
    }
    if sender == me.to_lowercase() || parsed.important {
        return Ok(Category::Primary);
    }
    if !parsed.in_reply_to.is_empty() {
        let replying_to_me: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM messages WHERE message_id = ?1 AND lower(from_addr) = ?2",
            params![parsed.in_reply_to, me.to_lowercase()],
            |row| row.get(0),
        )?;
        if replying_to_me {
            return Ok(Category::Primary);
        }
    }
    let written_to: bool =
        conn.query_row("SELECT COUNT(*) > 0 FROM contacts WHERE addr = ?1 AND sent > 0", [&sender], |row| row.get(0))?;
    if written_to {
        return Ok(Category::Primary);
    }
    Ok(if parsed.bulk { Category::Other } else { Category::Primary })
}

/// Files this sender's mail under `category` from now on, and moves what's already here.
pub fn set_sender_category(conn: &Connection, sender: &str, category: Category) -> Result<usize> {
    let sender = sender.trim().to_lowercase();
    conn.execute(
        "INSERT INTO sender_rules(addr, category) VALUES (?1, ?2)
         ON CONFLICT(addr) DO UPDATE SET category = excluded.category",
        params![sender, category.to_sql()],
    )?;
    Ok(conn
        .execute("UPDATE messages SET category = ?2 WHERE lower(from_addr) = ?1", params![sender, category.to_sql()])?)
}

/// Fills in the body of a message first saved with headers only.
pub fn store_body(conn: &Connection, id: i64, parsed: &Parsed, raw: &[u8]) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO raw(message, data) VALUES (?1, ?2)", params![id, raw])?;
    conn.execute(
        "UPDATE messages SET has_body = 1, snippet = ?2, attachments = ?3 WHERE id = ?1",
        params![id, parsed.snippet, parsed.has_attachments],
    )?;
    conn.execute("UPDATE search SET body = ?2 WHERE rowid = ?1", params![id, parsed.search_text])?;
    Ok(())
}

/// Takes the server's flags, except on messages whose own change is still waiting to reach it.
pub fn update_flags(conn: &Connection, folder: i64, uid: u32, flags: Flags) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE messages SET seen = ?3, flagged = ?4, answered = ?5
         WHERE folder = ?1 AND uid = ?2 AND (seen != ?3 OR flagged != ?4 OR answered != ?5)
           AND id NOT IN (SELECT message FROM ops)",
        params![folder, uid as i64, flags.seen, flags.flagged, flags.answered],
    )?;
    Ok(changed > 0)
}

pub fn delete_uids(conn: &Connection, folder: i64, uids: &[u32]) -> Result<()> {
    let mut stmt = conn.prepare("DELETE FROM messages WHERE folder = ?1 AND uid = ?2")?;
    for uid in uids {
        stmt.execute(params![folder, *uid as i64])?;
    }
    Ok(())
}

pub fn delete_message(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
    Ok(())
}

pub struct Missing {
    pub id: i64,
    pub folder_path: String,
    pub uid: u32,
}

/// Messages saved with headers only, newest first, small enough to fetch without being asked.
pub fn missing_bodies(conn: &Connection, account: &str, max_size: i64, limit: i64) -> Result<Vec<Missing>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, f.path, m.uid FROM messages m JOIN folders f ON f.id = m.folder
         WHERE m.account = ?1 AND m.has_body = 0 AND m.hidden = 0 AND m.size <= ?2
         ORDER BY m.date DESC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(params![account, max_size, limit], |row| {
            Ok(Missing { id: row.get(0)?, folder_path: row.get(1)?, uid: row.get::<_, i64>(2)? as u32 })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

// ---------- reading, for the window ----------

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Folder(i64),
    /// Every account's folder with this role, such as all inboxes.
    Unified(Role),
    /// Flagged mail across every account, outside Trash and Junk.
    Flagged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Unread,
    Flagged,
    Attachments,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub id: i64,
    pub account: String,
    pub folder: i64,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: Vec<(String, String)>,
    pub date: i64,
    pub snippet: String,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub attachments: bool,
    pub category: Category,
}

impl Summary {
    pub fn sender(&self) -> &str {
        if self.from_name.is_empty() { &self.from_addr } else { &self.from_name }
    }
}

const SUMMARY_COLUMNS: &str = "m.id, m.account, m.folder, m.subject, m.from_name, m.from_addr, m.to_list, m.date,
                               m.snippet, m.seen, m.flagged, m.answered, m.attachments, m.category";

fn summary_row(row: &rusqlite::Row) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: row.get(0)?,
        account: row.get(1)?,
        folder: row.get(2)?,
        subject: row.get(3)?,
        from_name: row.get(4)?,
        from_addr: row.get(5)?,
        to: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
        date: row.get(7)?,
        snippet: row.get(8)?,
        seen: row.get(9)?,
        flagged: row.get(10)?,
        answered: row.get(11)?,
        attachments: row.get(12)?,
        category: Category::from_sql(row.get(13)?),
    })
}

/// Search text split into its `from:`, `is:` and `has:` parts and the words left for full-text search.
#[derive(Default)]
struct Query {
    words: Vec<String>,
    from: Vec<String>,
    to: Vec<String>,
    subject: Vec<String>,
    unread: bool,
    flagged: bool,
    attachments: bool,
}

fn parse_query(text: &str) -> Query {
    let mut query = Query::default();
    for token in text.split_whitespace() {
        let lower = token.to_lowercase();
        match lower.split_once(':') {
            Some(("from", value)) if !value.is_empty() => query.from.push(value.to_owned()),
            Some(("to", value)) if !value.is_empty() => query.to.push(value.to_owned()),
            Some(("subject", value)) if !value.is_empty() => query.subject.push(value.to_owned()),
            Some(("is", "unread")) => query.unread = true,
            Some(("is", "flagged" | "starred")) => query.flagged = true,
            Some(("has", "attachment" | "attachments")) => query.attachments = true,
            _ => {
                let word: String = token.chars().filter(|c| c.is_alphanumeric() || *c == '@' || *c == '.').collect();
                if !word.is_empty() {
                    query.words.push(word);
                }
            }
        }
    }
    query
}

/// Messages in a view, newest first. `category` narrows an inbox to Primary or Other; a search
/// looks through both.
pub fn list(
    conn: &Connection,
    view: &View,
    category: Option<Category>,
    filter: Filter,
    search: &str,
    everywhere: bool,
    limit: i64,
) -> Result<Vec<Summary>> {
    let mut sql =
        format!("SELECT {SUMMARY_COLUMNS} FROM messages m JOIN folders f ON f.id = m.folder WHERE m.hidden = 0");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    // A search everywhere looks past the folder being shown, to every folder but Trash and Junk.
    let everywhere = everywhere && !search.trim().is_empty();
    match view {
        _ if everywhere => sql.push_str(" AND f.role NOT IN ('trash', 'junk')"),
        View::Folder(id) => {
            sql.push_str(" AND m.folder = ?");
            args.push(Box::new(*id));
        }
        View::Unified(role) => {
            sql.push_str(" AND f.role = ?");
            args.push(Box::new(role.as_str()));
        }
        View::Flagged => sql.push_str(" AND m.flagged = 1 AND f.role NOT IN ('trash', 'junk')"),
    }
    match filter {
        Filter::All => {}
        Filter::Unread => sql.push_str(" AND m.seen = 0"),
        Filter::Flagged => sql.push_str(" AND m.flagged = 1"),
        Filter::Attachments => sql.push_str(" AND m.attachments = 1"),
    }
    if let Some(category) = category.filter(|_| search.trim().is_empty()) {
        sql.push_str(" AND m.category = ?");
        args.push(Box::new(category.to_sql()));
    }

    let query = parse_query(search);
    if query.unread {
        sql.push_str(" AND m.seen = 0");
    }
    if query.flagged {
        sql.push_str(" AND m.flagged = 1");
    }
    if query.attachments {
        sql.push_str(" AND m.attachments = 1");
    }
    for from in &query.from {
        sql.push_str(" AND (m.from_addr LIKE ? OR m.from_name LIKE ?)");
        args.push(Box::new(format!("%{from}%")));
        args.push(Box::new(format!("%{from}%")));
    }
    for to in &query.to {
        sql.push_str(" AND (m.to_list LIKE ? OR m.cc_list LIKE ?)");
        args.push(Box::new(format!("%{to}%")));
        args.push(Box::new(format!("%{to}%")));
    }
    for subject in &query.subject {
        sql.push_str(" AND m.subject LIKE ?");
        args.push(Box::new(format!("%{subject}%")));
    }
    if !query.words.is_empty() {
        let fts =
            query.words.iter().map(|word| format!("\"{}\"*", word.replace('"', ""))).collect::<Vec<_>>().join(" ");
        sql.push_str(" AND m.id IN (SELECT rowid FROM search WHERE search MATCH ?)");
        args.push(Box::new(fts));
    }
    sql.push_str(" ORDER BY m.date DESC LIMIT ?");
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> = args.iter().map(|arg| arg.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), summary_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

#[derive(Clone, Debug)]
pub struct Detail {
    pub summary: Summary,
    pub cc: Vec<(String, String)>,
    pub reply_to: Vec<(String, String)>,
    pub message_id: String,
    pub references: String,
    pub has_body: bool,
}

pub fn detail(conn: &Connection, id: i64) -> Result<Option<Detail>> {
    let sql = format!(
        "SELECT {SUMMARY_COLUMNS}, m.cc_list, m.reply_to, m.message_id, m.refs, m.has_body FROM messages m WHERE m.id = ?1"
    );
    Ok(conn
        .query_row(&sql, [id], |row| {
            Ok(Detail {
                summary: summary_row(row)?,
                cc: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default(),
                reply_to: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
                message_id: row.get(16)?,
                references: row.get(17)?,
                has_body: row.get(18)?,
            })
        })
        .optional()?)
}

pub fn raw(conn: &Connection, id: i64) -> Result<Option<Vec<u8>>> {
    Ok(conn.query_row("SELECT data FROM raw WHERE message = ?1", [id], |row| row.get(0)).optional()?)
}

pub fn unread_in_inboxes(conn: &Connection, primary_only: bool) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM messages m JOIN folders f ON f.id = m.folder
         WHERE f.role = 'inbox' AND m.seen = 0 AND m.hidden = 0 AND (?1 = 0 OR m.category = 0)",
        [primary_only],
        |row| row.get(0),
    )?)
}

// ---------- queued changes ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Seen(bool),
    Flagged(bool),
    Answered,
    Move {
        to: i64,
    },
    /// To the account's Archive, Gmail's All Mail, or a new Archive folder.
    Archive,
    /// Into Trash, or gone for good when already there.
    Delete,
    /// The window opened a message whose body hasn't been downloaded.
    FetchBody,
}

pub struct QueuedOp {
    pub id: i64,
    pub message: i64,
    pub op: Op,
    pub attempts: i64,
}

pub fn queue(conn: &Connection, account: &str, message: i64, op: &Op) -> Result<()> {
    conn.execute(
        "INSERT INTO ops(account, message, op, created) VALUES (?1, ?2, ?3, ?4)",
        params![account, message, serde_json::to_string(op)?, now()],
    )?;
    Ok(())
}

pub fn pending_ops(conn: &Connection, account: &str) -> Result<Vec<QueuedOp>> {
    let mut stmt = conn.prepare("SELECT id, message, op, attempts FROM ops WHERE account = ?1 ORDER BY id")?;
    let rows = stmt
        .query_map([account], |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, String>(2)?, row.get(3)?)))?
        .collect::<rusqlite::Result<Vec<(i64, i64, String, i64)>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, message, op, attempts)| {
            Some(QueuedOp { id, message, op: serde_json::from_str(&op).ok()?, attempts })
        })
        .collect())
}

pub fn finish_op(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM ops WHERE id = ?1", [id])?;
    Ok(())
}

pub fn fail_op(conn: &Connection, id: i64, error: &str) -> Result<()> {
    conn.execute("UPDATE ops SET attempts = attempts + 1, error = ?2 WHERE id = ?1", params![id, error])?;
    Ok(())
}

/// Where a message lives on the server, for replaying an op.
pub fn location(conn: &Connection, message: i64) -> Result<Option<(i64, String, u32)>> {
    Ok(conn
        .query_row(
            "SELECT m.folder, f.path, m.uid FROM messages m JOIN folders f ON f.id = m.folder WHERE m.id = ?1",
            [message],
            |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? as u32)),
        )
        .optional()?)
}

pub fn set_seen(conn: &Connection, id: i64, seen: bool) -> Result<()> {
    conn.execute("UPDATE messages SET seen = ?2 WHERE id = ?1", params![id, seen])?;
    Ok(())
}

pub fn set_flagged(conn: &Connection, id: i64, flagged: bool) -> Result<()> {
    conn.execute("UPDATE messages SET flagged = ?2 WHERE id = ?1", params![id, flagged])?;
    Ok(())
}

pub fn set_answered(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("UPDATE messages SET answered = 1 WHERE id = ?1", [id])?;
    Ok(())
}

pub fn hide(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("UPDATE messages SET hidden = 1 WHERE id = ?1", [id])?;
    Ok(())
}

pub fn unhide(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("UPDATE messages SET hidden = 0 WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- sending ----------

#[derive(Clone, Debug)]
pub struct Outgoing {
    pub id: i64,
    pub account: String,
    pub raw: Vec<u8>,
    pub sender: String,
    pub recipients: Vec<String>,
    pub subject: String,
    pub replying_to: Option<i64>,
    pub send_after: i64,
    pub state: String,
    pub error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn queue_send(
    conn: &Connection,
    account: &str,
    raw: &[u8],
    sender: &str,
    recipients: &[String],
    subject: &str,
    replying_to: Option<i64>,
    send_after: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO outbox(account, raw, sender, recipients, subject, replying_to, send_after)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![account, raw, sender, serde_json::to_string(recipients)?, subject, replying_to, send_after],
    )?;
    Ok(conn.last_insert_rowid())
}

fn outgoing_row(row: &rusqlite::Row) -> rusqlite::Result<Outgoing> {
    Ok(Outgoing {
        id: row.get(0)?,
        account: row.get(1)?,
        raw: row.get(2)?,
        sender: row.get(3)?,
        recipients: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
        subject: row.get(5)?,
        replying_to: row.get(6)?,
        send_after: row.get(7)?,
        state: row.get(8)?,
        error: row.get(9)?,
    })
}

const OUTBOX_COLUMNS: &str = "id, account, raw, sender, recipients, subject, replying_to, send_after, state, error";

pub fn outbox(conn: &Connection) -> Result<Vec<Outgoing>> {
    let mut stmt = conn.prepare(&format!("SELECT {OUTBOX_COLUMNS} FROM outbox ORDER BY send_after"))?;
    let rows = stmt.query_map([], outgoing_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Claims every message due to go out for one account, so a second pass never sends it twice.
pub fn claim_due(conn: &Connection, account: &str) -> Result<Vec<Outgoing>> {
    let mut stmt = conn.prepare(&format!(
        "UPDATE outbox SET state = 'sending' WHERE account = ?1 AND state = 'queued' AND send_after <= ?2
         RETURNING {OUTBOX_COLUMNS}"
    ))?;
    let rows = stmt.query_map(params![account, now()], outgoing_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn next_send_at(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT MIN(send_after) FROM outbox WHERE state = 'queued'", [], |row| row.get(0))?)
}

pub fn sent(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
    Ok(())
}

pub fn send_failed(conn: &Connection, id: i64, error: &str) -> Result<()> {
    conn.execute("UPDATE outbox SET state = 'failed', error = ?2 WHERE id = ?1", params![id, error])?;
    Ok(())
}

pub fn retry_send(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE outbox SET state = 'queued', error = NULL, send_after = ?2 WHERE id = ?1",
        params![id, now()],
    )?;
    Ok(())
}

/// Takes a message back out of the outbox. False when it has already started sending.
pub fn cancel_send(conn: &Connection, id: i64) -> Result<Option<Outgoing>> {
    let mut stmt =
        conn.prepare(&format!("DELETE FROM outbox WHERE id = ?1 AND state != 'sending' RETURNING {OUTBOX_COLUMNS}"))?;
    Ok(stmt.query_row([id], outgoing_row).optional()?)
}

// ---------- drafts ----------

pub fn save_draft(conn: &Connection, id: Option<i64>, data: &str) -> Result<i64> {
    match id {
        Some(id) => {
            conn.execute(
                "INSERT INTO drafts(id, data, updated) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET data = excluded.data, updated = excluded.updated",
                params![id, data, now()],
            )?;
            Ok(id)
        }
        None => {
            conn.execute("INSERT INTO drafts(data, updated) VALUES (?1, ?2)", params![data, now()])?;
            Ok(conn.last_insert_rowid())
        }
    }
}

pub fn drafts(conn: &Connection) -> Result<Vec<(i64, String, i64)>> {
    let mut stmt = conn.prepare("SELECT id, data, updated FROM drafts ORDER BY updated DESC")?;
    let rows =
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn delete_draft(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM drafts WHERE id = ?1", [id])?;
    Ok(())
}

// ---------- contacts ----------

/// Notes people seen in mail, for address suggestions. `sent` means the user wrote to them, which
/// also puts their mail in Primary.
pub fn remember_contacts(
    conn: &Connection,
    people: &[(String, String)],
    weight: i64,
    sent: bool,
    when: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO contacts(addr, name, weight, last, sent) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(addr) DO UPDATE SET
             name = CASE WHEN excluded.name != '' THEN excluded.name ELSE contacts.name END,
             weight = contacts.weight + excluded.weight,
             last = MAX(contacts.last, excluded.last),
             sent = contacts.sent + excluded.sent",
    )?;
    for (name, addr) in people {
        let addr = addr.trim().to_lowercase();
        if addr.contains('@') && !crate::mail::parse::is_machine_address(&addr) {
            stmt.execute(params![addr, name.trim(), weight, when, sent as i64])?;
        }
    }
    Ok(())
}

pub fn suggest_contacts(conn: &Connection, prefix: &str, limit: i64) -> Result<Vec<(String, String)>> {
    let pattern = format!("%{}%", prefix.trim().to_lowercase());
    let mut stmt = conn.prepare(
        "SELECT name, addr FROM contacts WHERE addr LIKE ?1 OR lower(name) LIKE ?1
         ORDER BY weight DESC, last DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

// ---------- sync status ----------

#[derive(Clone, Debug)]
pub struct Status {
    pub account: String,
    pub state: String,
    pub error: Option<String>,
    pub at: i64,
}

pub fn set_status(conn: &Connection, account: &str, state: &str, error: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT INTO status(account, state, error, at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account) DO UPDATE SET state = excluded.state, error = excluded.error, at = excluded.at",
        params![account, state, error, now()],
    )?;
    Ok(())
}

pub fn statuses(conn: &Connection) -> Result<Vec<Status>> {
    let mut stmt = conn.prepare("SELECT account, state, error, at FROM status")?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Status { account: row.get(0)?, state: row.get(1)?, error: row.get(2)?, at: row.get(3)? })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Removes everything stored for an account that has been taken out of settings.
pub fn forget_account(conn: &Connection, account: &str) -> Result<()> {
    conn.execute("DELETE FROM folders WHERE account = ?1", [account])?;
    conn.execute("DELETE FROM ops WHERE account = ?1", [account])?;
    conn.execute("DELETE FROM outbox WHERE account = ?1", [account])?;
    conn.execute("DELETE FROM status WHERE account = ?1", [account])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(tx: &Transaction, folder: i64, uid: u32, subject: &str, date: i64) {
        let parsed = Parsed {
            subject: subject.into(),
            from: ("Ann".into(), "ann@example.com".into()),
            date,
            search_text: format!("{subject} body"),
            ..Default::default()
        };
        upsert_message(tx, "a", folder, uid, Flags::default(), 10, &parsed, None, Category::Primary).unwrap();
    }

    #[test]
    fn search_everywhere_finds_old_mail_outside_the_folder_shown() {
        let mut conn = prepare(Connection::open_in_memory().unwrap()).unwrap();
        let inbox = upsert_folder(&conn, "a", "INBOX", "Inbox", Role::Inbox, None).unwrap();
        let archive = upsert_folder(&conn, "a", "Archive", "Archive", Role::Archive, None).unwrap();
        let trash = upsert_folder(&conn, "a", "Trash", "Trash", Role::Trash, None).unwrap();
        let tx = conn.transaction().unwrap();
        message(&tx, inbox, 1, "Lunch", 3_000);
        message(&tx, archive, 1, "Invoice from 2019", 1_000);
        message(&tx, trash, 1, "Invoice duplicate", 2_000);
        tx.commit().unwrap();

        let inboxes = View::Unified(Role::Inbox);
        let found = |everywhere| -> Vec<String> {
            list(&conn, &inboxes, None, Filter::All, "invoice", everywhere, 50)
                .unwrap()
                .into_iter()
                .map(|summary| summary.subject)
                .collect()
        };
        assert_eq!(found(true), ["Invoice from 2019"], "everywhere, but not Trash");
        assert!(found(false).is_empty(), "this folder only");
        let all = list(&conn, &inboxes, None, Filter::All, "", true, 50).unwrap();
        assert_eq!(all.len(), 1, "without a search the view stays the inbox");
    }

    #[test]
    fn older_mail_downloads_only_when_asked_for() {
        let conn = prepare(Connection::open_in_memory().unwrap()).unwrap();
        let inbox = upsert_folder(&conn, "a", "INBOX", "Inbox", Role::Inbox, None).unwrap();
        let sent = upsert_folder(&conn, "a", "Sent", "Sent", Role::Sent, None).unwrap();
        set_folder_state(&conn, inbox, 1, 5).unwrap();
        set_folder_state(&conn, sent, 1, 5).unwrap();
        set_server_count(&conn, inbox, 900).unwrap();
        assert!(folders_wanting_older(&conn, "a").unwrap().is_empty(), "nothing until asked");

        request_older(&conn, inbox, 500).unwrap();
        request_older(&conn, inbox, 500).unwrap();
        request_older(&conn, sent, ALL_OLDER).unwrap();
        let wanted: Vec<(i64, i64)> =
            folders_wanting_older(&conn, "a").unwrap().into_iter().map(|(id, _, _, n)| (id, n)).collect();
        assert_eq!(wanted, [(inbox, 1000), (sent, ALL_OLDER)], "inbox first, requests add up");

        fetched_older(&conn, inbox, 300, false).unwrap();
        fetched_older(&conn, sent, 300, false).unwrap();
        let inbox_folder = folders(&conn).unwrap().into_iter().find(|f| f.id == inbox).unwrap();
        assert_eq!(inbox_folder.fetch_older, 700);
        assert_eq!(inbox_folder.downloading(), 700);
        fetched_older(&conn, sent, 1, true).unwrap();
        let wanted: Vec<i64> = folders_wanting_older(&conn, "a").unwrap().into_iter().map(|(id, ..)| id).collect();
        assert_eq!(wanted, [inbox], "all of Sent is done once the server runs out");
    }
}
