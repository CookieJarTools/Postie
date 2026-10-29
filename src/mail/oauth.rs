//! Signing in with a Microsoft account, for Outlook.com and Microsoft 365, which no longer accept
//! passwords over IMAP and SMTP.
//!
//! The browser does the sign-in (authorization code with PKCE) and hands the code back to a
//! one-shot listener on localhost. Only the refresh token is kept, in the credentials file in place
//! of a password; access tokens last an hour and are cached in memory.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::config::Account;
use crate::credentials;

/// The CookieJarTools app registration. Not a secret: desktop apps can't keep one, which is what
/// PKCE is for. A build can set its own with POSTIE_MICROSOFT_CLIENT_ID.
const CLIENT_ID: &str = match option_env!("POSTIE_MICROSOFT_CLIENT_ID") {
    Some(id) => id,
    None => "5f39388f-2293-40c1-aedf-6cb8624e3e64",
};
const AUTHORIZE: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const SCOPES: &str =
    "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

pub fn client_id() -> String {
    std::env::var("POSTIE_MICROSOFT_CLIENT_ID")
        .ok()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| CLIENT_ID.to_owned())
}

/// Whether this build can sign in with Microsoft at all.
pub fn available() -> bool {
    !client_id().is_empty()
}

pub struct Tokens {
    pub access: String,
    pub refresh: String,
    pub expires: Instant,
}

static CACHE: Mutex<Option<HashMap<String, (String, Instant)>>> = Mutex::new(None);

/// A current access token for the account, refreshed when it's near its hour.
pub fn access_token(account: &Account) -> Result<String> {
    let cached = CACHE.lock().unwrap().as_ref().and_then(|cache| cache.get(&account.id).cloned());
    if let Some((token, expires)) = cached
        && expires > Instant::now() + Duration::from_secs(300)
    {
        return Ok(token);
    }
    let refresh = credentials::password(&account.id)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("Sign in to {} with Microsoft again, in Settings.", account.email))?;
    let tokens = post_token(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)])?;
    // Microsoft hands back a new refresh token each time; keep the newest.
    if tokens.refresh != refresh {
        credentials::set_password(&account.id, &tokens.refresh)?;
    }
    remember(&account.id, &tokens);
    Ok(tokens.access)
}

pub fn remember(account_id: &str, tokens: &Tokens) {
    CACHE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(account_id.to_owned(), (tokens.access.clone(), tokens.expires));
}

pub fn forget(account_id: &str) {
    if let Some(cache) = CACHE.lock().unwrap().as_mut() {
        cache.remove(account_id);
    }
}

/// Opens the browser at Microsoft's sign-in page and waits for it to come back. Blocks.
pub fn sign_in(email: &str) -> Result<Tokens> {
    if !available() {
        bail!("This build of Postie has no Microsoft app ID, so it can't sign in to Outlook.");
    }
    let listener = TcpListener::bind("127.0.0.1:0").context("opening a local port for the sign-in")?;
    let redirect = format!("http://localhost:{}", listener.local_addr()?.port());
    let verifier = random_text(48);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_text(16);
    let url = format!(
        "{AUTHORIZE}?client_id={}&response_type=code&response_mode=query&redirect_uri={}&scope={}\
         &code_challenge={challenge}&code_challenge_method=S256&state={state}&login_hint={}&prompt=select_account",
        encode(&client_id()),
        encode(&redirect),
        encode(SCOPES),
        encode(email.trim()),
    );
    open::that_detached(&url).context("opening the browser")?;

    let code = wait_for_code(&listener, &state)?;
    post_token(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect),
        ("code_verifier", &verifier),
    ])
}

fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    bail!("The Microsoft sign-in took too long. Try again.");
                }
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(err) => return Err(err.into()),
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut line = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut line)?;
        let target = line.split_whitespace().nth(1).unwrap_or("/");
        let query: HashMap<String, String> = target
            .split_once('?')
            .map(|(_, query)| query)
            .unwrap_or("")
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.to_owned(), decode(value)))
            .collect();
        // Browsers also ask for /favicon.ico; only the redirect carries a state.
        if !query.contains_key("state") && !query.contains_key("error") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let result = match (query.get("code"), query.get("error")) {
            _ if query.get("state").map(String::as_str) != Some(state) => {
                Err(anyhow!("The sign-in reply didn't match. Try again."))
            }
            (Some(code), _) => Ok(code.clone()),
            (None, Some(error)) => {
                let detail = query.get("error_description").cloned().unwrap_or_default();
                Err(anyhow!("Microsoft said no: {error}. {}", detail.lines().next().unwrap_or("")))
            }
            (None, None) => Err(anyhow!("Microsoft sent no sign-in code.")),
        };
        let (title, text) = match &result {
            Ok(_) => ("You're signed in", "You can close this tab and go back to Postie."),
            Err(_) => ("That didn't work", "Go back to Postie to see what happened and try again."),
        };
        let _ = respond(&mut stream, title, text);
        return result;
    }
}

/// A page in the house colours for the browser tab the sign-in ends on.
fn respond(stream: &mut impl Write, title: &str, text: &str) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>Postie</title><body style=\"margin:0;height:100vh;display:grid;\
         place-items:center;background:#08090a;color:#f7f8f8;font:15px Inter,system-ui,sans-serif\"><div style=\"\
         text-align:center\"><div style=\"width:44px;height:44px;margin:0 auto 16px;border-radius:12px;background:#5e6ad2\">\
         </div><h1 style=\"font-size:20px;font-weight:600;margin:0 0 6px\">{title}</h1><p style=\"color:#8a8f98;margin:0\">\
         {text}</p></div>"
    );
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn post_token(fields: &[(&str, &str)]) -> Result<Tokens> {
    let client_id = client_id();
    let mut form: Vec<(&str, &str)> = vec![("client_id", &client_id), ("scope", SCOPES)];
    form.extend_from_slice(fields);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent.post(TOKEN).send_form(form).context("reaching Microsoft's sign-in service")?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string()?;
    let json: serde_json::Value = serde_json::from_str(&body).with_context(|| format!("Microsoft sent {status}"))?;
    if status != 200 {
        let error = json["error"].as_str().unwrap_or("error");
        if error == "invalid_grant" {
            bail!("Microsoft sign-in has expired or was revoked. Sign in again in Settings.");
        }
        let detail = json["error_description"].as_str().unwrap_or("").lines().next().unwrap_or("");
        bail!("Microsoft said {error}: {detail}");
    }
    let field =
        |name: &str| json[name].as_str().map(str::to_owned).ok_or_else(|| anyhow!("Microsoft's reply had no {name}"));
    Ok(Tokens {
        access: field("access_token")?,
        refresh: field("refresh_token")?,
        expires: Instant::now() + Duration::from_secs(json["expires_in"].as_u64().unwrap_or(3600)),
    })
}

/// The SASL XOAUTH2 initial response, before base64.
pub fn xoauth2(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

fn random_text(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut buffer));
    if filled.is_err() {
        // Never expected on Linux; still unpredictable enough for a one-off state value.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        for (i, byte) in buffer.iter_mut().enumerate() {
            *byte = (seed.rotate_left(i as u32 * 7) as u8) ^ (std::process::id() as u8).wrapping_mul(i as u8 + 1);
        }
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buffer)
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(text: &str) -> String {
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
    use super::*;

    #[test]
    fn xoauth2_is_the_sasl_shape() {
        assert_eq!(xoauth2("joe@outlook.com", "abc"), "user=joe@outlook.com\x01auth=Bearer abc\x01\x01");
    }

    #[test]
    fn encodes_and_decodes_query_values() {
        assert_eq!(encode("http://localhost:1234"), "http%3A%2F%2Flocalhost%3A1234");
        assert_eq!(decode("M.C5%2Bab%3D"), "M.C5+ab=");
    }

    #[test]
    fn local_redirect_hands_back_the_code() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            let mut favicon = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            favicon.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut reply = String::new();
            let _ = favicon.read_to_string(&mut reply);
            let mut redirect = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            redirect.write_all(b"GET /?code=M.abc%2Fdef&state=xyz HTTP/1.1\r\n\r\n").unwrap();
            let mut page = String::new();
            let _ = redirect.read_to_string(&mut page);
            page
        });
        assert_eq!(wait_for_code(&listener, "xyz").unwrap(), "M.abc/def");
        assert!(browser.join().unwrap().contains("You're signed in"));
    }
}
