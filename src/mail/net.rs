//! Connecting and signing in to IMAP over TLS, STARTTLS, or plain TCP for local bridges.

use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use async_imap::{Client, Session};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore, pki_types::ServerName};

use super::{Secret, oauth};
use crate::config::{Account, Security, Server};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug)]
pub enum Stream {
    Tls(Box<TlsStream<TcpStream>>),
    Plain(TcpStream),
}

impl AsyncRead for Stream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Tls(stream) => Pin::new(stream.as_mut()).poll_read(cx, buf),
            Stream::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Stream::Tls(stream) => Pin::new(stream.as_mut()).poll_write(cx, buf),
            Stream::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Tls(stream) => Pin::new(stream.as_mut()).poll_flush(cx),
            Stream::Plain(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Tls(stream) => Pin::new(stream.as_mut()).poll_shutdown(cx),
            Stream::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

pub type Imap = Session<Stream>;

pub fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("ring supports the default TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

async fn tls(tcp: TcpStream, host: &str) -> Result<Stream> {
    let name = ServerName::try_from(host.to_owned()).map_err(|_| anyhow!("{host} isn't a valid server name"))?;
    let stream =
        TlsConnector::from(tls_config()).connect(name, tcp).await.with_context(|| format!("TLS with {host}"))?;
    Ok(Stream::Tls(Box::new(stream)))
}

async fn tcp(server: &Server) -> Result<TcpStream> {
    let address = (server.host.as_str(), server.port);
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| anyhow!("{} didn't answer", server.host))?
        .with_context(|| format!("couldn't reach {}:{}", server.host, server.port))?;
    stream.set_nodelay(true).ok();
    Ok(stream)
}

pub async fn connect(account: &Account, secret: &Secret) -> Result<Imap> {
    let server = &account.imap;
    if server.host.trim().is_empty() {
        bail!("No IMAP server is set for {}", account.label());
    }
    let tcp = tcp(server).await?;
    let client = match server.security {
        Security::Tls => {
            let mut client = Client::new(tls(tcp, &server.host).await?);
            greeting(&mut client).await?;
            client
        }
        Security::StartTls => {
            let mut plain = Client::new(Stream::Plain(tcp));
            greeting(&mut plain).await?;
            plain.run_command_and_check_ok("STARTTLS", None).await.context("STARTTLS")?;
            let Stream::Plain(tcp) = plain.into_inner() else { unreachable!() };
            Client::new(tls(tcp, &server.host).await?)
        }
        Security::None => {
            let mut client = Client::new(Stream::Plain(tcp));
            greeting(&mut client).await?;
            client
        }
    };
    let username = if account.username.is_empty() { &account.email } else { &account.username };
    let signing_in = async {
        match secret {
            Secret::Password(password) => client.login(username, password).await,
            Secret::Bearer(token) => client.authenticate("XOAUTH2", XOAuth2(oauth::xoauth2(username, token))).await,
        }
    };
    let session = tokio::time::timeout(CONNECT_TIMEOUT, signing_in)
        .await
        .map_err(|_| anyhow!("{} took too long to sign in", server.host))?
        .map_err(|(err, _)| anyhow!("Sign-in failed: {}", sign_in_reason(&err)))?;
    Ok(session)
}

/// Sends the XOAUTH2 response once; any further challenge is the server's error report, which
/// wants an empty reply before it says NO.
struct XOAuth2(String);

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        std::mem::take(&mut self.0)
    }
}

async fn greeting(client: &mut Client<Stream>) -> Result<()> {
    tokio::time::timeout(CONNECT_TIMEOUT, client.read_response())
        .await
        .map_err(|_| anyhow!("the server sent no greeting"))??
        .ok_or_else(|| anyhow!("the server closed the connection"))?;
    Ok(())
}

fn sign_in_reason(err: &async_imap::error::Error) -> String {
    match err {
        async_imap::error::Error::No(text) | async_imap::error::Error::Bad(text) => text.trim().to_owned(),
        other => other.to_string(),
    }
}

/// Whether an error means the connection is gone, rather than one command being refused.
pub fn is_connection_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<async_imap::error::Error>(),
            Some(async_imap::error::Error::Io(_) | async_imap::error::Error::ConnectionLost)
        ) || cause.downcast_ref::<std::io::Error>().is_some()
    })
}
