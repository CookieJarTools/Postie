pub mod compose;
pub mod net;
pub mod oauth;
pub mod parse;
pub mod smtp;
pub mod sync;

use anyhow::{Result, anyhow};

use crate::config::{Account, Auth};
use crate::credentials;

/// What an account signs in with.
#[derive(Clone)]
pub enum Secret {
    Password(String),
    /// A Microsoft access token, sent with SASL XOAUTH2.
    Bearer(String),
}

/// The account's sign-in, fetching a fresh access token for Microsoft accounts when needed. Blocks
/// on the network for that, which only ever holds up the account's own thread.
pub fn secret(account: &Account) -> Result<Secret> {
    match account.auth {
        Auth::Password => credentials::password(&account.id)
            .filter(|password| !password.is_empty())
            .map(Secret::Password)
            .ok_or_else(|| anyhow!("No password is saved for {}.", account.email)),
        Auth::Microsoft => oauth::access_token(account).map(Secret::Bearer),
    }
}

/// Signs in over IMAP and checks SMTP answers, for the add-account form.
pub fn check_account(account: &Account, secret: &Secret) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async {
        let mut session = net::connect(account, secret).await?;
        let _ = session.logout().await;
        smtp::check(account, secret).await
    })
}

/// IMAP folder names are in "modified UTF-7": `&` starts base64 of UTF-16, with `,` for `/`.
pub fn decode_utf7(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded = &after[..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            out.push_str(&decode_utf16_base64(encoded).unwrap_or_else(|| format!("&{encoded}-")));
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn decode_utf16_base64(encoded: &str) -> Option<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";
    let mut bits = 0u32;
    let mut count = 0;
    let mut bytes = Vec::new();
    for c in encoded.bytes() {
        let value = ALPHABET.iter().position(|&a| a == c)? as u32;
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes([pair[0], pair[1]])).collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::decode_utf7;

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode_utf7("INBOX"), "INBOX");
        assert_eq!(decode_utf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_utf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_utf7("&ZeVnLIqe-"), "日本語");
    }
}
