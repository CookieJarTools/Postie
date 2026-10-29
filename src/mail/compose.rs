//! Drafts, replies and forwards, and turning a draft into the bytes SMTP sends.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use lettre::Message;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use serde::{Deserialize, Serialize};

use crate::config::Account;
use crate::store::Detail;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub account: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<DraftAttachment>,
    pub in_reply_to: String,
    pub references: String,
    pub replying_to: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftAttachment {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

impl Draft {
    pub fn is_empty(&self) -> bool {
        self.to.trim().is_empty()
            && self.cc.trim().is_empty()
            && self.subject.trim().is_empty()
            && self.body.trim().is_empty()
            && self.attachments.is_empty()
    }
}

pub fn signature_block(account: &Account) -> String {
    if account.signature.trim().is_empty() {
        String::new()
    } else {
        format!("\n\n-- \n{}", account.signature.trim_end())
    }
}

pub fn new_message(account: &Account) -> Draft {
    Draft { account: account.id.clone(), body: signature_block(account), ..Default::default() }
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let lower = subject.trim_start().to_lowercase();
    let already =
        ["re:", "fwd:", "fw:"].iter().any(|p| lower.starts_with(p) && prefix.to_lowercase().starts_with(&p[..2]));
    if already { subject.to_owned() } else { format!("{prefix} {subject}") }
}

pub fn format_person((name, addr): &(String, String)) -> String {
    if name.is_empty() {
        addr.clone()
    } else if name.contains([',', ';', '"', '<']) {
        format!("\"{}\" <{addr}>", name.replace('"', ""))
    } else {
        format!("{name} <{addr}>")
    }
}

fn join_people(people: &[(String, String)]) -> String {
    people.iter().map(format_person).collect::<Vec<_>>().join(", ")
}

fn quote(detail: &Detail, text: &str) -> String {
    let when = chrono::DateTime::from_timestamp(detail.summary.date, 0)
        .map(|date| date.with_timezone(&chrono::Local).format("%a %-d %b %Y at %H:%M").to_string())
        .unwrap_or_default();
    let sender = format_person(&(detail.summary.from_name.clone(), detail.summary.from_addr.clone()));
    let quoted = text.trim_end().lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n");
    format!("\n\nOn {when}, {sender} wrote:\n{quoted}\n")
}

pub fn reply(account: &Account, detail: &Detail, text: &str, all: bool) -> Draft {
    let me = account.email.to_lowercase();
    let summary = &detail.summary;
    let mut to: Vec<(String, String)> = if detail.reply_to.is_empty() {
        vec![(summary.from_name.clone(), summary.from_addr.clone())]
    } else {
        detail.reply_to.clone()
    };
    // Replying to your own sent message goes back to whoever it was sent to.
    if to.iter().all(|(_, addr)| addr.to_lowercase() == me) {
        to = summary.to.clone();
    }
    let mut cc = Vec::new();
    if all {
        let mut seen: Vec<String> = to.iter().map(|(_, addr)| addr.to_lowercase()).collect();
        seen.push(me.clone());
        for person in summary.to.iter().chain(detail.cc.iter()) {
            let addr = person.1.to_lowercase();
            if !seen.contains(&addr) {
                seen.push(addr);
                cc.push(person.clone());
            }
        }
    }
    let references = format!("{} {}", detail.references, angle(&detail.message_id)).trim().to_owned();
    Draft {
        account: account.id.clone(),
        to: join_people(&to),
        cc: join_people(&cc),
        subject: prefixed("Re:", &summary.subject),
        body: format!("{}{}", signature_block(account), quote(detail, text)),
        in_reply_to: angle(&detail.message_id),
        references,
        replying_to: Some(summary.id),
        ..Default::default()
    }
}

pub fn forward(account: &Account, detail: &Detail, text: &str, attachments: Vec<DraftAttachment>) -> Draft {
    let summary = &detail.summary;
    let when = chrono::DateTime::from_timestamp(summary.date, 0)
        .map(|date| date.with_timezone(&chrono::Local).format("%a %-d %b %Y at %H:%M").to_string())
        .unwrap_or_default();
    let mut header = format!(
        "\n\n---------- Forwarded message ----------\nFrom: {}\nDate: {when}\nSubject: {}\nTo: {}\n",
        format_person(&(summary.from_name.clone(), summary.from_addr.clone())),
        summary.subject,
        join_people(&summary.to),
    );
    if !detail.cc.is_empty() {
        header.push_str(&format!("Cc: {}\n", join_people(&detail.cc)));
    }
    Draft {
        account: account.id.clone(),
        subject: prefixed("Fwd:", &summary.subject),
        body: format!("{}{header}\n{}", signature_block(account), text.trim_end()),
        attachments,
        ..Default::default()
    }
}

fn angle(id: &str) -> String {
    let id = id.trim();
    if id.is_empty() || id.starts_with('<') { id.to_owned() } else { format!("<{id}>") }
}

/// "Ann <ann@x.com>, bob@y.com; …" into mailboxes, naming the first one that doesn't parse.
pub fn parse_recipients(text: &str) -> Result<Vec<Mailbox>> {
    split_recipients(text)
        .into_iter()
        .map(|item| item.parse::<Mailbox>().with_context(|| format!("\"{item}\" isn't an email address")))
        .collect()
}

pub fn split_recipients(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' | ';' | '\n' if !quoted => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_owned());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
    out
}

pub struct Built {
    pub raw: Vec<u8>,
    pub sender: String,
    pub recipients: Vec<String>,
}

pub fn build(account: &Account, draft: &Draft) -> Result<Built> {
    let to = parse_recipients(&draft.to)?;
    let cc = parse_recipients(&draft.cc)?;
    let bcc = parse_recipients(&draft.bcc)?;
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        bail!("Add at least one recipient.");
    }
    let from = Mailbox::new(
        (!account.name.trim().is_empty()).then(|| account.name.trim().to_owned()),
        account.email.parse().with_context(|| format!("{} isn't a valid address", account.email))?,
    );
    let mut builder = Message::builder()
        .from(from.clone())
        .subject(draft.subject.trim())
        .date_now()
        .message_id(None)
        .user_agent(format!("Postie {}", env!("CARGO_PKG_VERSION")));
    for mailbox in &to {
        builder = builder.to(mailbox.clone());
    }
    for mailbox in &cc {
        builder = builder.cc(mailbox.clone());
    }
    for mailbox in &bcc {
        builder = builder.bcc(mailbox.clone());
    }
    if !draft.in_reply_to.is_empty() {
        builder = builder.in_reply_to(draft.in_reply_to.clone());
    }
    if !draft.references.is_empty() {
        builder = builder.references(draft.references.clone());
    }

    let text = SinglePart::plain(draft.body.clone());
    let message = if draft.attachments.is_empty() {
        builder.singlepart(text)?
    } else {
        let mut mixed = MultiPart::mixed().singlepart(text);
        for attachment in &draft.attachments {
            let bytes = std::fs::read(&attachment.path)
                .with_context(|| format!("couldn't read {}", attachment.path.display()))?;
            let content_type = ContentType::parse(mime_for(&attachment.name)).unwrap_or(ContentType::TEXT_PLAIN);
            mixed = mixed.singlepart(Attachment::new(attachment.name.clone()).body(bytes, content_type));
        }
        builder.multipart(mixed)?
    };
    let recipients = to.iter().chain(cc.iter()).chain(bcc.iter()).map(|mailbox| mailbox.email.to_string()).collect();
    Ok(Built { raw: message.formatted(), sender: from.email.to_string(), recipients })
}

pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ics" => "text/calendar",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_recipients_outside_quotes() {
        assert_eq!(
            split_recipients("\"Cook, Joe\" <joe@x.com>, ann@y.com; bob@z.com"),
            vec!["\"Cook, Joe\" <joe@x.com>", "ann@y.com", "bob@z.com"]
        );
    }

    #[test]
    fn prefixes_subjects_once() {
        assert_eq!(prefixed("Re:", "Hello"), "Re: Hello");
        assert_eq!(prefixed("Re:", "RE: Hello"), "RE: Hello");
        assert_eq!(prefixed("Fwd:", "Re: Hello"), "Fwd: Re: Hello");
    }
}
