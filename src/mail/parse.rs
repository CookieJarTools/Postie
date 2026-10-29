//! Turning raw RFC 5322 bytes into what the store keeps and what the reading pane shows.

use mail_parser::{Address, HeaderValue, Message, MessageParser, MimeHeaders, PartType};

const SNIPPET_CHARS: usize = 180;
const SEARCH_CHARS: usize = 32 * 1024;

#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub message_id: String,
    pub in_reply_to: String,
    pub references: String,
    pub subject: String,
    pub from: (String, String),
    pub to: Vec<(String, String)>,
    pub cc: Vec<(String, String)>,
    pub reply_to: Vec<(String, String)>,
    pub date: i64,
    pub snippet: String,
    pub search_text: String,
    pub has_attachments: bool,
    /// Sent in bulk: a newsletter, mailing list, or a machine's notification.
    pub bulk: bool,
    /// Marked high importance by the sender.
    pub important: bool,
}

pub fn parse(raw: &[u8], fallback_date: i64) -> Parsed {
    let Some(message) = MessageParser::default().parse(raw) else {
        return Parsed { date: fallback_date, subject: "(unreadable message)".into(), ..Default::default() };
    };
    let text = message.body_text(0).map(|text| text.into_owned()).unwrap_or_default();
    let has_attachments = message.attachments().any(is_real_attachment)
        || (text.is_empty() && content_type_is(&message, "multipart", "mixed"));
    Parsed {
        message_id: message.message_id().unwrap_or_default().to_owned(),
        in_reply_to: header_ids(message.in_reply_to()).into_iter().next().unwrap_or_default(),
        references: header_ids(message.references()).join(" "),
        subject: message.subject().unwrap_or_default().trim().to_owned(),
        from: people(message.from()).into_iter().next().unwrap_or_default(),
        to: people(message.to()),
        cc: people(message.cc()),
        reply_to: people(message.reply_to()),
        date: message.date().map(|date| date.to_timestamp()).filter(|date| *date > 0).unwrap_or(fallback_date),
        snippet: snippet(&text),
        search_text: text.chars().take(SEARCH_CHARS).collect(),
        has_attachments,
        bulk: is_bulk(&message),
        important: is_important(&message),
    }
}

/// The marks bulk senders leave: the headers mailing-list software and email services add, and
/// the addresses machines send from.
fn is_bulk(message: &Message) -> bool {
    let header = |name: &str| message.header_raw(name).map(|value| value.trim().to_ascii_lowercase());
    let listed =
        ["List-Unsubscribe", "List-Id", "List-Post", "Feedback-ID", "X-Campaign", "X-Mailgun-Tag", "X-SES-Outgoing"]
            .iter()
            .any(|name| message.header_raw(*name).is_some());
    let precedence = header("Precedence").is_some_and(|value| matches!(value.as_str(), "bulk" | "list" | "junk"));
    let automatic = header("Auto-Submitted").is_some_and(|value| value != "no");
    let from = message.from().and_then(|from| from.first()).and_then(|addr| addr.address()).unwrap_or_default();
    listed || precedence || automatic || is_machine_address(from)
}

pub fn is_machine_address(address: &str) -> bool {
    let local = address.split('@').next().unwrap_or_default().to_ascii_lowercase();
    const MACHINES: &[&str] = &[
        "noreply",
        "no-reply",
        "no_reply",
        "donotreply",
        "do-not-reply",
        "do_not_reply",
        "notifications",
        "notification",
        "notify",
        "newsletter",
        "newsletters",
        "news",
        "marketing",
        "mailer-daemon",
        "postmaster",
        "bounce",
        "updates",
        "digest",
        "alerts",
        "automated",
        "promotions",
        "offers",
        "deals",
    ];
    MACHINES.iter().any(|machine| {
        local == *machine
            || local.starts_with(&format!("{machine}-"))
            || local.starts_with(&format!("{machine}+"))
            || local.starts_with(&format!("{machine}."))
    })
}

fn is_important(message: &Message) -> bool {
    let importance = message.header_raw("Importance").is_some_and(|value| value.trim().eq_ignore_ascii_case("high"));
    let priority =
        message.header_raw("X-Priority").is_some_and(|value| matches!(value.trim().chars().next(), Some('1' | '2')));
    importance || priority
}

fn content_type_is(message: &Message, ctype: &str, subtype: &str) -> bool {
    message.content_type().is_some_and(|ct| {
        ct.ctype().eq_ignore_ascii_case(ctype) && ct.subtype().is_some_and(|sub| sub.eq_ignore_ascii_case(subtype))
    })
}

/// Inline images referenced by the HTML aren't what anyone means by "has an attachment".
fn is_real_attachment(part: &mail_parser::MessagePart) -> bool {
    let inline = part.content_disposition().is_some_and(|cd| cd.ctype().eq_ignore_ascii_case("inline"));
    let image = part.content_type().is_some_and(|ct| ct.ctype().eq_ignore_ascii_case("image"));
    !(inline || (image && part.content_id().is_some()))
}

fn header_ids(value: &HeaderValue) -> Vec<String> {
    match value {
        HeaderValue::Text(text) => vec![text.to_string()],
        HeaderValue::TextList(list) => list.iter().map(|text| text.to_string()).collect(),
        _ => Vec::new(),
    }
}

pub fn people(address: Option<&Address>) -> Vec<(String, String)> {
    let Some(address) = address else { return Vec::new() };
    address
        .iter()
        .filter_map(|addr| {
            let email = addr.address()?.trim().to_owned();
            let name = addr.name().map(|name| name.trim().trim_matches('"').to_owned()).unwrap_or_default();
            Some((if name == email { String::new() } else { name }, email))
        })
        .collect()
}

/// One line of the body for the message list: quotes, signatures and runs of whitespace dropped.
pub fn snippet(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('>') || line.is_empty() {
            continue;
        }
        if line == "--" || line.starts_with("On ") && line.ends_with("wrote:") {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&line.split_whitespace().collect::<Vec<_>>().join(" "));
        if out.chars().count() >= SNIPPET_CHARS {
            break;
        }
    }
    out.chars().take(SNIPPET_CHARS).collect()
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub inline: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Body {
    pub text: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
    pub list_unsubscribe: Option<String>,
}

pub fn body(raw: &[u8]) -> Body {
    let Some(message) = MessageParser::default().parse(raw) else { return Body::default() };
    // Only real parts: mail-parser also lists a plain-text part as the HTML body (converted), and
    // an HTML part as the text body, the JMAP way.
    let html = message.html_bodies().find_map(|part| match &part.body {
        PartType::Html(html) => Some(html.to_string()),
        _ => None,
    });
    let text = message.text_bodies().find_map(|part| match &part.body {
        PartType::Text(text) => Some(text.to_string()),
        _ => None,
    });
    let attachments = message
        .attachments()
        .enumerate()
        .map(|(index, part)| Attachment {
            index,
            name: part.attachment_name().unwrap_or("attachment").to_owned(),
            mime: part
                .content_type()
                .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")))
                .unwrap_or_else(|| "application/octet-stream".into()),
            size: part.contents().len(),
            inline: !is_real_attachment(part),
        })
        .collect();
    let list_unsubscribe = match message.header("List-Unsubscribe") {
        Some(HeaderValue::Address(address)) => address.first().and_then(|addr| addr.address()).map(str::to_owned),
        Some(HeaderValue::Text(text)) => Some(text.to_string()),
        _ => None,
    };
    Body { text, html, attachments, list_unsubscribe }
}

pub fn attachment(raw: &[u8], index: usize) -> Option<(String, Vec<u8>)> {
    let message = MessageParser::default().parse(raw)?;
    let part = message.attachments().nth(index)?;
    let name = part.attachment_name().unwrap_or("attachment").to_owned();
    Some((name, part.contents().to_vec()))
}

/// Images carried inside the message, by content id, for `cid:` references in its HTML.
pub fn inline_images(raw: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Some(message) = MessageParser::default().parse(raw) else { return Vec::new() };
    message
        .attachments()
        .filter_map(|part| {
            let id = part.content_id()?.trim_matches(['<', '>']).to_owned();
            Some((id, part.contents().to_vec()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_mail_has_no_html_body() {
        let body = body(b"From: a@x.com\r\nContent-Type: text/plain\r\n\r\nSee https://x.com\r\n");
        assert!(body.html.is_none());
        assert_eq!(body.text.as_deref().map(str::trim), Some("See https://x.com"));
    }

    #[test]
    fn spots_bulk_and_important_mail() {
        let news = parse(
            b"From: Linear <news@linear.app>\r\nList-Unsubscribe: <https://x.com/u>\r\nSubject: Oct\r\n\r\nHi\r\n",
            0,
        );
        assert!(news.bulk);
        let person = parse(b"From: Ann <ann@example.com>\r\nSubject: Lunch\r\nImportance: High\r\n\r\nHi\r\n", 0);
        assert!(!person.bulk);
        assert!(person.important);
        let robot = parse(b"From: GitHub <notifications@github.com>\r\nSubject: PR\r\n\r\nHi\r\n", 0);
        assert!(robot.bulk);
    }

    #[test]
    fn html_only_mail_has_no_text_body() {
        let body =
            body(b"From: a@x.com\r\nContent-Type: text/html\r\n\r\n<p>Hi <a href='https://x.com'>there</a></p>\r\n");
        assert!(body.html.is_some());
        assert!(body.text.is_none());
    }
}
