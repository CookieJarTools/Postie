use std::time::Duration;

use anyhow::{Context, Result};
use lettre::address::Envelope;
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use super::Secret;
use crate::config::{Account, Security};
use crate::store::Outgoing;

fn transport(account: &Account, secret: &Secret) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let server = &account.smtp;
    let builder = match server.security {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&server.host)?,
        Security::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&server.host)?,
        Security::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&server.host),
    };
    let username = if account.username.is_empty() { &account.email } else { &account.username };
    let builder = match secret {
        Secret::Password(password) => builder.credentials(Credentials::new(username.clone(), password.clone())),
        Secret::Bearer(token) => builder
            .credentials(Credentials::new(username.clone(), token.clone()))
            .authentication(vec![Mechanism::Xoauth2]),
    };
    Ok(builder.port(server.port).timeout(Some(Duration::from_secs(60))).build())
}

pub async fn send(account: &Account, secret: &Secret, outgoing: &Outgoing) -> Result<()> {
    let sender: Address = outgoing.sender.parse().with_context(|| format!("{} isn't an address", outgoing.sender))?;
    let recipients = outgoing
        .recipients
        .iter()
        .map(|addr| addr.parse::<Address>().with_context(|| format!("{addr} isn't an address")))
        .collect::<Result<Vec<_>>>()?;
    let envelope = Envelope::new(Some(sender), recipients)?;
    transport(account, secret)?
        .send_raw(&envelope, &outgoing.raw)
        .await
        .with_context(|| format!("{} refused the message", account.smtp.host))?;
    Ok(())
}

/// Connects and says hello, to check the server settings when an account is added.
pub async fn check(account: &Account, secret: &Secret) -> Result<()> {
    let ok = transport(account, secret)?
        .test_connection()
        .await
        .with_context(|| format!("couldn't reach {}:{}", account.smtp.host, account.smtp.port))?;
    anyhow::ensure!(ok, "{} didn't answer", account.smtp.host);
    Ok(())
}
