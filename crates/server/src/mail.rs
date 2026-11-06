//! Sending email: verification and password-reset links (design.md, Accounts;
//! Hosting and operations: Purelymail SMTP).

use std::future::Future;
use std::pin::Pin;

use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::SmtpSettings;

/// One plain-text email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// What sending an email returns.
pub type Sending<'a> = Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + 'a>>;

/// Where email goes. The server holds one as `Arc<dyn Mailer>`; tests pass a fake that
/// keeps what it was sent.
pub trait Mailer: Send + Sync {
    fn send(&self, email: Email) -> Sending<'_>;
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("`{0}` is not an email address")]
    Address(String),
    #[error("the SMTP server refused the email: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),
    #[error("the email couldn't be built: {0}")]
    Build(#[from] lettre::error::Error),
}

/// Sends `email` without waiting for it; a failure is logged.
pub fn send_later(mailer: &std::sync::Arc<dyn Mailer>, email: Email) {
    let mailer = mailer.clone();
    tokio::spawn(async move {
        if let Err(error) = mailer.send(email).await {
            tracing::error!(%error, "an email wasn't sent");
        }
    });
}

/// Sends through an SMTP server, over implicit TLS.
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    /// An SMTP mailer for `settings`. Errors name the setting, never the password.
    pub fn new(settings: &SmtpSettings) -> Result<SmtpMailer, MailError> {
        let from: Mailbox = settings
            .from
            .parse()
            .map_err(|_| MailError::Address(settings.from.clone()))?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)?
            .port(settings.port)
            .credentials(Credentials::new(
                settings.user.clone(),
                settings.password.clone(),
            ))
            .build();
        Ok(SmtpMailer { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, email: Email) -> Sending<'_> {
        Box::pin(async move {
            let to: Mailbox = email
                .to
                .parse()
                .map_err(|_| MailError::Address(email.to.clone()))?;
            let message = Message::builder()
                .from(self.from.clone())
                .to(to)
                .subject(email.subject)
                .body(email.body)?;
            self.transport.send(message).await?;
            Ok(())
        })
    }
}

/// Writes each email to the log: for dev, and for staging before SMTP is set up.
pub struct LogMailer;

impl Mailer for LogMailer {
    fn send(&self, email: Email) -> Sending<'_> {
        tracing::info!(to = %email.to, subject = %email.subject, body = %email.body, "email (not sent: no SMTP settings)");
        Box::pin(async { Ok(()) })
    }
}
