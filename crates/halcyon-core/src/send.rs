//! Sending mail: building the message, delivering it over SMTP and keeping a
//! copy in the sent folder.
//!
//! Messages are built once, when the user presses "Senden", and the exact
//! bytes go through the outbox (see [`crate::store::Store::queue_outgoing`]).
//! That makes "undo send" possible and guarantees that what lands in the
//! sent folder is what the recipients got.

use std::time::{Duration, SystemTime};

use lettre::message::{Mailbox, header::ContentType};
use lettre::transport::smtp::authentication::Credentials as SmtpCredentials;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};

use crate::imap::{self, Credentials};
use crate::model::{Account, Security, ServerConfig};
use crate::reply::Recipient;
use crate::{Error, Result};

const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

/// What the composer hands over for sending.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Draft {
    pub account_id: i64,
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub bcc: Vec<Recipient>,
    pub subject: String,
    pub body: String,
    /// Message-ID (without `<>`) of the message this answers.
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// Local id of the message this answers, to mark it answered.
    pub reply_to_local_id: Option<i64>,
}

/// A built message, ready for the outbox.
#[derive(Clone, Debug)]
pub struct Built {
    /// The bytes sent to the server (without `Bcc`).
    pub wire: Vec<u8>,
    /// The copy for the sent folder (with `Bcc`, so the user can see it).
    pub record: Vec<u8>,
    pub envelope_from: String,
    pub envelope_to: Vec<String>,
    pub message_id: String,
}

fn mailbox(r: &Recipient) -> Result<Mailbox> {
    let address: Address = r
        .address
        .parse()
        .map_err(|_| Error::InvalidAddress(r.address.clone()))?;
    Ok(Mailbox::new(
        r.name.clone().filter(|n| !n.trim().is_empty()),
        address,
    ))
}

/// Builds the message. Fails on invalid addresses or without recipients.
pub fn build(draft: &Draft, account: &Account, date: SystemTime) -> Result<Built> {
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err(Error::Other("no recipients".into()));
    }
    let from = mailbox(&Recipient {
        name: account.display_name.clone(),
        address: account.email.clone(),
    })?;
    let domain = account
        .email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .unwrap_or_else(|| "halcyon.invalid".into());
    let message_id = format!("{}.{}@{}", unix_millis(date), random_token(), domain);

    let make = |keep_bcc: bool| -> Result<Message> {
        let mut b = Message::builder()
            .from(from.clone())
            .subject(draft.subject.clone())
            .date(date)
            .message_id(Some(format!("<{message_id}>")))
            .user_agent(concat!("Halcyon/", env!("CARGO_PKG_VERSION")).to_string())
            .header(ContentType::TEXT_PLAIN);
        for r in &draft.to {
            b = b.to(mailbox(r)?);
        }
        for r in &draft.cc {
            b = b.cc(mailbox(r)?);
        }
        for r in &draft.bcc {
            b = b.bcc(mailbox(r)?);
        }
        if let Some(id) = &draft.in_reply_to {
            b = b.in_reply_to(format!("<{id}>"));
        }
        if !draft.references.is_empty() {
            let refs: Vec<String> = draft.references.iter().map(|r| format!("<{r}>")).collect();
            b = b.references(refs.join(" "));
        }
        if keep_bcc {
            b = b.keep_bcc();
        }
        // Normalise line endings; SMTP needs CRLF, which lettre produces.
        b.body(draft.body.replace("\r\n", "\n"))
            .map_err(|e| Error::Other(e.to_string()))
    };

    let wire = make(false)?;
    let record = make(true)?;
    let envelope = wire.envelope();
    Ok(Built {
        envelope_from: envelope.from().map(|a| a.to_string()).unwrap_or_default(),
        envelope_to: envelope.to().iter().map(|a| a.to_string()).collect(),
        wire: wire.formatted(),
        record: record.formatted(),
        message_id,
    })
}

fn unix_millis(t: SystemTime) -> u128 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    // Not secret, only unique: RandomState is seeded randomly per process.
    let mut h = RandomState::new().build_hasher();
    h.write_u128(unix_millis(SystemTime::now()));
    format!("{:016x}", h.finish())
}

fn transport(
    server: &ServerConfig,
    password: Option<&str>,
) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    server.validate()?;
    let builder = match server.security {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&server.host),
        Security::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&server.host),
        // Only for the local machine; `validate` rejects anything else.
        Security::Plain => Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
            &server.host,
        )),
    }
    .map_err(|e| Error::Tls(e.to_string()))?
    .port(server.port)
    .timeout(Some(SMTP_TIMEOUT));
    let builder = match password {
        Some(pw) => builder.credentials(SmtpCredentials::new(
            server.username.clone(),
            pw.to_string(),
        )),
        None => builder,
    };
    Ok(builder.build())
}

/// Checks that the SMTP server is reachable and its TLS is valid, without
/// logging in.
pub async fn check_smtp(server: &ServerConfig) -> Result<()> {
    let ok = transport(server, None)?
        .test_connection()
        .await
        .map_err(|e| Error::Other(e.to_string()))?;
    if ok {
        Ok(())
    } else {
        Err(Error::Other(format!("{} antwortet nicht", server.host)))
    }
}

/// Why delivery failed, and whether trying again can help.
#[derive(Debug)]
pub struct DeliveryError {
    pub message: String,
    /// The server rejected the message for good (5xx): retrying is useless.
    pub permanent: bool,
}

/// Hands the message to the SMTP server.
pub async fn deliver(
    server: &ServerConfig,
    password: &str,
    built: &Built,
) -> std::result::Result<(), DeliveryError> {
    let fail = |message: String, permanent| DeliveryError { message, permanent };
    let transport = transport(server, Some(password)).map_err(|e| fail(e.to_string(), true))?;
    let from: Address = built
        .envelope_from
        .parse()
        .map_err(|_| fail("invalid sender".into(), true))?;
    let to: Vec<Address> = built
        .envelope_to
        .iter()
        .filter_map(|a| a.parse().ok())
        .collect();
    let envelope =
        lettre::address::Envelope::new(Some(from), to).map_err(|e| fail(e.to_string(), true))?;
    transport
        .send_raw(&envelope, &built.wire)
        .await
        .map(|_| ())
        .map_err(|e| fail(e.to_string(), e.is_permanent()))
}

/// Puts the sent copy into the sent folder. Gmail files sent mail itself, so
/// it is skipped there (it would appear twice).
pub async fn file_sent_copy(
    account: &Account,
    imap_credentials: &Credentials,
    sent_folder: &str,
    record: &[u8],
) -> Result<bool> {
    let (mut session, caps) = imap::login(&account.imap, imap_credentials).await?;
    if caps.gmail {
        let _ = session.logout().await;
        return Ok(false);
    }
    session
        .append(sent_folder, Some("(\\Seen)"), None, record)
        .await?;
    let _ = session.logout().await;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AuthMethod, ServerConfig};

    fn account() -> Account {
        Account {
            id: 1,
            email: "max@mail.example".into(),
            display_name: Some("Max Muster".into()),
            imap: ServerConfig {
                host: "imap.mail.example".into(),
                port: 993,
                security: Security::Tls,
                username: "max@mail.example".into(),
                auth: AuthMethod::Password,
            },
            smtp: None,
        }
    }

    fn rcpt(name: Option<&str>, address: &str) -> Recipient {
        Recipient {
            name: name.map(str::to_string),
            address: address.into(),
        }
    }

    #[test]
    fn builds_a_reply_with_threading_and_hidden_bcc() {
        let draft = Draft {
            account_id: 1,
            to: vec![rcpt(Some("Kranz, Sabine"), "kranz@zulieferer.example")],
            cc: vec![rcpt(None, "jonas@werkstatt.example")],
            bcc: vec![rcpt(None, "archiv@mail.example")],
            subject: "Re: Angebot – Rückfrage".into(),
            body: "Hallo Sabine,\r\npasst.\n\n> alt".into(),
            in_reply_to: Some("m1@zulieferer.example".into()),
            references: vec![
                "r0@zulieferer.example".into(),
                "m1@zulieferer.example".into(),
            ],
            reply_to_local_id: Some(7),
        };
        let built = build(
            &draft,
            &account(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_794_294_900),
        )
        .unwrap();
        let wire = String::from_utf8_lossy(&built.wire);
        let record = String::from_utf8_lossy(&built.record);

        assert!(
            wire.contains("In-Reply-To: <m1@zulieferer.example>"),
            "{wire}"
        );
        assert!(wire.contains("References: <r0@zulieferer.example> <m1@zulieferer.example>"));
        assert!(
            wire.contains("From: \"Max Muster\" <max@mail.example>")
                || wire.contains("From: Max Muster <max@mail.example>"),
            "{wire}"
        );
        assert!(wire.contains(&format!("Message-ID: <{}>", built.message_id)));
        // Bcc never reaches the recipients, but the sent copy keeps it.
        assert!(!wire.contains("archiv@mail.example"));
        assert!(record.contains("archiv@mail.example"));
        assert!(
            built
                .envelope_to
                .contains(&"archiv@mail.example".to_string())
        );
        assert_eq!(built.envelope_to.len(), 3);
        // Same message: identical Message-ID in both copies.
        assert!(record.contains(&format!("<{}>", built.message_id)));
    }

    #[test]
    fn refuses_bad_drafts() {
        let mut draft = Draft {
            subject: "x".into(),
            ..Default::default()
        };
        assert!(build(&draft, &account(), SystemTime::now()).is_err());
        draft.to = vec![rcpt(None, "kein-at-zeichen")];
        assert!(build(&draft, &account(), SystemTime::now()).is_err());
    }
}
