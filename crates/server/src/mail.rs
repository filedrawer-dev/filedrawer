//! Sending email: verification and password-reset links (design.md, Accounts;
//! Hosting and operations: Purelymail SMTP).

use std::sync::{Arc, Mutex};

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

/// Emails a test can read back.
#[derive(Debug, Clone, Default)]
pub struct Outbox(Arc<Mutex<Vec<Email>>>);

impl Outbox {
    /// Every email sent so far, oldest first.
    pub fn sent(&self) -> Vec<Email> {
        self.0
            .lock()
            .expect("no test panics while holding the outbox")
            .clone()
    }
}

/// Where email goes.
#[derive(Clone)]
pub enum Mailer {
    /// Through an SMTP server, over implicit TLS.
    Smtp {
        transport: Box<AsyncSmtpTransport<Tokio1Executor>>,
        from: Mailbox,
    },
    /// To the log, for dev and for staging before SMTP is set up.
    Log,
    /// Into an outbox the tests read.
    Memory(Outbox),
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

impl Mailer {
    /// An SMTP mailer for `settings`. Errors name the setting, never the password.
    pub fn smtp(settings: &SmtpSettings) -> Result<Mailer, MailError> {
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
        Ok(Mailer::Smtp {
            transport: Box::new(transport),
            from,
        })
    }

    pub async fn send(&self, email: Email) -> Result<(), MailError> {
        match self {
            Mailer::Smtp { transport, from } => {
                let to: Mailbox = email
                    .to
                    .parse()
                    .map_err(|_| MailError::Address(email.to.clone()))?;
                let message = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(email.subject)
                    .body(email.body)?;
                transport.send(message).await?;
            }
            Mailer::Log => {
                tracing::info!(to = %email.to, subject = %email.subject, body = %email.body, "email (not sent: no SMTP settings)");
            }
            Mailer::Memory(outbox) => outbox
                .0
                .lock()
                .expect("no test panics while holding the outbox")
                .push(email),
        }
        Ok(())
    }
}
