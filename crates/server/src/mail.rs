//! Sending email: verification and password-reset links (design.md, Accounts;
//! Hosting and operations: Purelymail SMTP). The server queues what it sends
//! (`mail_queue`); the daily email sends through a `Mailer` itself.

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

/// Why an email didn't go. A send's failure keeps only the SMTP server's reply code,
/// since the reply's text can repeat the recipient's address.
#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("`{0}` is not an email address")]
    Address(String),
    #[error("the SMTP server answered {code}")]
    Reply { code: u16 },
    #[error("no answer from the SMTP server: {0}")]
    Unreachable(String),
    #[error("SMTP couldn't be set up: {0}")]
    Setup(#[from] lettre::transport::smtp::Error),
    #[error("the email couldn't be built: {0}")]
    Build(#[from] lettre::error::Error),
}

impl MailError {
    /// A failed send's error: the reply's code when the server answered, else what
    /// kept it from answering (connection, TLS or timeout), which names no recipient.
    fn sending(error: lettre::transport::smtp::Error) -> MailError {
        match error.status() {
            Some(code) => MailError::Reply { code: code.into() },
            None => MailError::Unreachable(error.to_string()),
        }
    }
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
        let transport = transport(settings)?.build();
        Ok(SmtpMailer { transport, from })
    }
}

/// The SMTP transport for `settings`, which the mailer and `check_login` share.
fn transport(
    settings: &SmtpSettings,
) -> Result<lettre::transport::smtp::AsyncSmtpTransportBuilder, lettre::transport::smtp::Error> {
    Ok(AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)?
        .port(settings.port)
        .credentials(Credentials::new(
            settings.user.clone(),
            settings.password.clone(),
        )))
}

/// Why `check_login` failed. Its causes, which `error::with_causes` prints, never hold
/// the password.
#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("SMTP couldn't be set up")]
    Setup(#[source] lettre::transport::smtp::Error),
    #[error("the SMTP login and NOOP failed")]
    Failed(#[source] lettre::transport::smtp::Error),
    #[error("the SMTP server refused NOOP")]
    NoopRefused,
}

/// Logs in to the SMTP server and sends NOOP, which sends no email, each step within
/// `timeout`: `check-outbound`'s test (#102).
pub async fn check_login(
    settings: &SmtpSettings,
    timeout: std::time::Duration,
) -> Result<(), LoginError> {
    let transport: AsyncSmtpTransport<Tokio1Executor> = transport(settings)
        .map_err(LoginError::Setup)?
        .timeout(Some(timeout))
        .build();
    match transport.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(LoginError::NoopRefused),
        Err(error) => Err(LoginError::Failed(error)),
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
            self.transport
                .send(message)
                .await
                .map_err(MailError::sending)?;
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
