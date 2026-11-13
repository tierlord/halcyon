//! IMAP connections: TLS / STARTTLS setup, login and header parsing.

use std::fmt::Debug;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_imap::types::{Fetch, Flag, NameAttribute};
use async_imap::{Authenticator, Client, Session};
use mail_parser::{Address, AddressList, MessageParser};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::model::{AuthMethod, FolderRole, MessageHeader, Security, ServerConfig};
use crate::{Error, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// The header fields fetched for every message. Fetching only these instead
/// of the full header keeps the initial sync of large mailboxes small.
pub const HEADER_FIELDS: &str = "FROM TO CC REPLY-TO SUBJECT DATE MESSAGE-ID IN-REPLY-TO \
     REFERENCES LIST-ID LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST AUTHENTICATION-RESULTS";

/// Any byte stream an IMAP session can run over (plain TCP or TLS).
pub trait ImapIo: AsyncRead + AsyncWrite + Unpin + Send + Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Debug> ImapIo for T {}

pub type ImapSession = Session<Box<dyn ImapIo>>;

/// Server features that change how Halcyon syncs.
#[derive(Clone, Debug, Default)]
pub struct Capabilities {
    pub idle: bool,
    pub condstore: bool,
    pub qresync: bool,
    pub special_use: bool,
    pub uidplus: bool,
    pub r#move: bool,
    /// Gmail's IMAP extensions (labels, thread ids).
    pub gmail: bool,
    pub raw: Vec<String>,
}

impl Capabilities {
    fn from_server(caps: &async_imap::types::Capabilities) -> Self {
        let raw: Vec<String> = caps
            .iter()
            .map(|c| match c {
                async_imap::types::Capability::Imap4rev1 => "IMAP4rev1".to_string(),
                async_imap::types::Capability::Auth(a) => format!("AUTH={a}"),
                async_imap::types::Capability::Atom(a) => a.to_string(),
            })
            .collect();
        let has = |name: &str| raw.iter().any(|c| c.eq_ignore_ascii_case(name));
        Capabilities {
            idle: has("IDLE"),
            condstore: has("CONDSTORE"),
            qresync: has("QRESYNC"),
            special_use: has("SPECIAL-USE"),
            uidplus: has("UIDPLUS"),
            r#move: has("MOVE"),
            gmail: has("X-GM-EXT-1"),
            raw,
        }
    }
}

/// Credentials for logging in.
pub enum Credentials {
    Password(String),
    /// An OAuth 2.0 access token.
    OAuth2(String),
}

struct XOAuth2<'a> {
    user: &'a str,
    token: &'a str,
}

impl Authenticator for XOAuth2<'_> {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

fn tls_connector() -> Result<TlsConnector> {
    static CONFIG: OnceLock<std::result::Result<Arc<ClientConfig>, String>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let mut roots = RootCertStore::empty();
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            // Single broken system certificates must not break all connections.
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            return Err(format!(
                "no usable system root certificates ({:?})",
                native.errors
            ));
        }
        Ok(Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ))
    });
    config.clone().map(TlsConnector::from).map_err(Error::Tls)
}

async fn tls_wrap(tcp: TcpStream, host: &str) -> Result<Box<dyn ImapIo>> {
    let name = ServerName::try_from(host.to_string())
        .map_err(|e| Error::Tls(format!("invalid host name {host:?}: {e}")))?;
    let tls = tls_connector()?
        .connect(name, tcp)
        .await
        .map_err(|e| Error::Tls(e.to_string()))?;
    Ok(Box::new(tls))
}

async fn read_greeting(client: &mut Client<Box<dyn ImapIo>>) -> Result<()> {
    match client.read_response().await? {
        Some(_) => Ok(()),
        None => Err(Error::Other("server closed the connection".into())),
    }
}

/// Opens an unauthenticated connection with the configured security.
pub async fn connect(server: &ServerConfig) -> Result<Client<Box<dyn ImapIo>>> {
    server.validate()?;
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((server.host.as_str(), server.port)),
    )
    .await
    .map_err(|_| {
        Error::Other(format!(
            "timeout connecting to {}:{}",
            server.host, server.port
        ))
    })??;

    match server.security {
        Security::Tls => {
            let mut client = Client::new(tls_wrap(tcp, &server.host).await?);
            read_greeting(&mut client).await?;
            Ok(client)
        }
        Security::StartTls => {
            let mut plain = Client::new(Box::new(tcp) as Box<dyn ImapIo>);
            read_greeting(&mut plain).await?;
            plain.run_command_and_check_ok("STARTTLS", None).await?;
            // The boxed stream is the TcpStream we put in; downcasting is
            // not possible through `dyn`, so re-wrap the raw stream instead.
            let raw = plain.into_inner();
            let tls = tls_connector()?
                .connect(
                    ServerName::try_from(server.host.clone())
                        .map_err(|e| Error::Tls(e.to_string()))?,
                    raw,
                )
                .await
                .map_err(|e| Error::Tls(e.to_string()))?;
            // No greeting after STARTTLS (RFC 3501, 6.2.1).
            Ok(Client::new(Box::new(tls) as Box<dyn ImapIo>))
        }
        Security::Plain => {
            let mut client = Client::new(Box::new(tcp) as Box<dyn ImapIo>);
            read_greeting(&mut client).await?;
            Ok(client)
        }
    }
}

/// Connects and logs in. Returns the session and the server's capabilities.
pub async fn login(
    server: &ServerConfig,
    credentials: &Credentials,
) -> Result<(ImapSession, Capabilities)> {
    let client = connect(server).await?;
    let mut session = match (server.auth, credentials) {
        (AuthMethod::Password, Credentials::Password(pw)) => client
            .login(&server.username, pw)
            .await
            .map_err(|(e, _)| Error::Auth(e.to_string()))?,
        (AuthMethod::OAuth2, Credentials::OAuth2(token)) => client
            .authenticate(
                "XOAUTH2",
                XOAuth2 {
                    user: &server.username,
                    token,
                },
            )
            .await
            .map_err(|(e, _)| Error::Auth(e.to_string()))?,
        _ => {
            return Err(Error::Auth(
                "credentials do not match the account's authentication method".into(),
            ));
        }
    };
    let caps = Capabilities::from_server(&session.capabilities().await?);
    Ok((session, caps))
}

/// Tries to log in with the given settings and logs out again.
///
/// Used to verify guessed configurations during account setup.
pub async fn probe(server: &ServerConfig, credentials: &Credentials) -> Result<Capabilities> {
    let (mut session, caps) = login(server, credentials).await?;
    let _ = session.logout().await;
    Ok(caps)
}

/// Downloads complete messages (`BODY.PEEK[]`, which leaves them unread) for
/// the given UIDs of the currently selected mailbox.
pub async fn fetch_raw(session: &mut ImapSession, uids: &str) -> Result<Vec<(u32, Vec<u8>)>> {
    use futures::TryStreamExt;

    let fetches: Vec<_> = session
        .uid_fetch(uids, "(UID BODY.PEEK[])")
        .await?
        .try_collect()
        .await?;
    Ok(fetches
        .iter()
        .filter_map(|f| Some((f.uid?, f.body()?.to_vec())))
        .collect())
}

/// A folder as reported by `LIST`.
#[derive(Clone, Debug)]
pub struct RemoteFolder {
    pub path: String,
    pub delimiter: Option<String>,
    pub role: Option<FolderRole>,
    pub selectable: bool,
}

pub async fn list_folders(session: &mut ImapSession) -> Result<Vec<RemoteFolder>> {
    use futures::TryStreamExt;

    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    Ok(names
        .iter()
        .map(|n| {
            let attrs = n.attributes();
            let selectable = !attrs.iter().any(|a| matches!(a, NameAttribute::NoSelect));
            let role = attrs
                .iter()
                .find_map(|a| match a {
                    NameAttribute::Sent => Some(FolderRole::Sent),
                    NameAttribute::Drafts => Some(FolderRole::Drafts),
                    NameAttribute::Trash => Some(FolderRole::Trash),
                    NameAttribute::Junk => Some(FolderRole::Junk),
                    NameAttribute::Archive => Some(FolderRole::Archive),
                    NameAttribute::All => Some(FolderRole::All),
                    NameAttribute::Flagged => Some(FolderRole::Flagged),
                    _ => None,
                })
                .or_else(|| FolderRole::guess_from_name(n.name(), n.delimiter()));
            RemoteFolder {
                path: n.name().to_string(),
                delimiter: n.delimiter().map(str::to_string),
                role,
                selectable,
            }
        })
        .collect())
}

/// Builds a [`MessageHeader`] from a `FETCH` response that contains
/// `UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS (...)]`.
pub fn header_from_fetch(fetch: &Fetch) -> Option<MessageHeader> {
    let uid = fetch.uid?;
    let flags = fetch.flags().filter_map(|f| flag_name(&f)).collect();
    let raw = fetch.header().unwrap_or_default();
    let mut header = parse_header_block(raw);
    header.uid = uid;
    header.flags = flags;
    header.size = fetch.size;
    if header.date.is_none() {
        header.date = fetch.internal_date().map(|d| d.timestamp());
    }
    Some(header)
}

/// The flags of a `FETCH` response as stored by Halcyon.
pub fn flag_names(fetch: &Fetch) -> Vec<String> {
    fetch.flags().filter_map(|f| flag_name(&f)).collect()
}

fn flag_name(flag: &Flag<'_>) -> Option<String> {
    Some(match flag {
        Flag::Seen => "\\Seen".into(),
        Flag::Answered => "\\Answered".into(),
        Flag::Flagged => "\\Flagged".into(),
        Flag::Deleted => "\\Deleted".into(),
        Flag::Draft => "\\Draft".into(),
        // \Recent is session-specific and meaningless once stored.
        Flag::Recent | Flag::MayCreate => return None,
        Flag::Custom(c) => c.to_string(),
    })
}

/// Parses a raw RFC 5322 header block into the fields Halcyon keeps.
pub fn parse_header_block(raw: &[u8]) -> MessageHeader {
    let Some(msg) = MessageParser::new().parse_headers(raw) else {
        return MessageHeader::default();
    };
    let text_list = |list: Option<mail_parser::TextList<'_>>| -> Vec<String> {
        list.map(|l| {
            (0..l.len())
                .filter_map(|i| l.get(i))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
    };
    let headers = msg.headers();
    let raw_header = |name: &str| {
        headers
            .get(name)
            .map(|h| String::from_utf8_lossy(h.raw_value()).into_owned())
    };
    let from = msg.from().and_then(|l| l.first());

    MessageHeader {
        uid: 0,
        message_id: msg.message_id().map(str::to_string),
        in_reply_to: text_list(msg.in_reply_to()).into_iter().next(),
        references: text_list(msg.references()),
        subject: msg.subject().map(str::to_string),
        from_name: from.as_ref().and_then(|m| m.name()).map(str::to_string),
        from_addr: from
            .as_ref()
            .and_then(|m| m.address())
            .map(str::to_lowercase),
        reply_to: msg
            .reply_to()
            .and_then(|l| l.first())
            .and_then(|m| m.address().map(str::to_lowercase)),
        to: addresses(msg.to()),
        cc: addresses(msg.cc()),
        date: msg.date().map(|d| d.to_timestamp()),
        size: None,
        flags: Vec::new(),
        list_id: raw_header("List-ID").and_then(|v| angle_items(&v).into_iter().next()),
        list_unsubscribe: raw_header("List-Unsubscribe")
            .map(|v| angle_items(&v))
            .unwrap_or_default(),
        list_unsubscribe_post: raw_header("List-Unsubscribe-Post")
            .is_some_and(|v| v.contains("List-Unsubscribe=One-Click")),
        dmarc_pass: has_dmarc_pass(
            headers,
            from.as_ref()
                .and_then(|m| m.address())
                .and_then(|a| a.rsplit_once('@').map(|(_, d)| d)),
        ),
    }
}

/// Whether the receiving server attested `dmarc=pass` for the sender.
///
/// Only the **topmost** `Authentication-Results` header counts: it is the
/// one the user's provider added on receipt (RFC 8601, section 5). Anything
/// below it came with the message and can be forged by the sender – a
/// phishing mail could simply include `dmarc=pass` itself. The verdict must
/// also be about the domain shown in `From`.
fn has_dmarc_pass(headers: mail_parser::Headers<'_>, from_domain: Option<&str>) -> bool {
    let Some(topmost) = headers.iter().find(|h| {
        h.name()
            .as_str()
            .eq_ignore_ascii_case("Authentication-Results")
    }) else {
        return false;
    };
    let value = String::from_utf8_lossy(topmost.raw_value());
    parse_dmarc_pass(&value) && header_from_matches(&value, from_domain)
}

/// `header.from=` of the dmarc clause, if given, must name the From domain.
fn header_from_matches(value: &str, from_domain: Option<&str>) -> bool {
    let lower = value.to_ascii_lowercase();
    let Some(clause) = lower
        .split(';')
        .map(str::trim)
        .find(|c| c.starts_with("dmarc"))
    else {
        return false;
    };
    match clause.split("header.from=").nth(1) {
        Some(rest) => {
            let claimed = rest
                .split(|c: char| c.is_whitespace() || c == ';' || c == ')')
                .next()
                .unwrap_or("");
            from_domain.is_some_and(|d| d.eq_ignore_ascii_case(claimed))
        }
        // No header.from given: the receiving server checked the From
        // domain by definition of DMARC.
        None => true,
    }
}

fn parse_dmarc_pass(value: &str) -> bool {
    value.split(';').skip(1).any(|clause| {
        let lower = clause.trim().to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("dmarc") else {
            return false;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            return false;
        };
        let rest = rest.trim_start();
        let Some(after) = rest.strip_prefix("pass") else {
            return false;
        };
        after.is_empty() || after.starts_with(|c: char| c.is_ascii_whitespace() || c == '(')
    })
}

fn addresses(list: Option<AddressList<'_>>) -> Vec<String> {
    let Some(list) = list else {
        return Vec::new();
    };
    list.iter()
        .flat_map(|a| match a {
            Address::Mailbox(m) => vec![m],
            Address::Group(g) => g.mailboxes().collect(),
        })
        .filter_map(|m| {
            let addr = m.address()?.to_lowercase();
            Some(match m.name() {
                Some(name) if !name.is_empty() => format!("{name} <{addr}>"),
                _ => addr,
            })
        })
        .collect()
}

/// Extracts the `<...>` items of headers like List-Unsubscribe and List-ID.
fn angle_items(value: &str) -> Vec<String> {
    let unfolded = value.replace(['\r', '\n'], "");
    unfolded
        .split('<')
        .skip(1)
        .filter_map(|part| {
            part.split_once('>')
                .map(|(item, _)| item.trim().to_string())
        })
        .filter(|item| !item.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_newsletter_headers() {
        let raw = b"From: =?UTF-8?Q?M=C3=BCller_Shop?= <News@Shop.Example>\r\n\
To: Max <max@example.org>, team: a@example.org, b@example.org;\r\n\
Subject: =?UTF-8?B?RGVpbmUgV29jaGUg4piA?=\r\n\
Date: Tue, 10 Nov 2026 08:15:00 +0100\r\n\
Message-ID: <abc@shop.example>\r\n\
References: <r1@x> <r2@x>\r\n\
In-Reply-To: <r2@x>\r\n\
List-ID: Shop News <news.shop.example>\r\n\
List-Unsubscribe: <mailto:unsub@shop.example?subject=stop>,\r\n <https://shop.example/u/123>\r\n\
List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\n";
        let h = parse_header_block(raw);
        assert_eq!(h.from_name.as_deref(), Some("Müller Shop"));
        assert_eq!(h.from_addr.as_deref(), Some("news@shop.example"));
        assert_eq!(h.subject.as_deref(), Some("Deine Woche ☀"));
        assert_eq!(
            h.to,
            ["Max <max@example.org>", "a@example.org", "b@example.org"]
        );
        assert_eq!(h.message_id.as_deref(), Some("abc@shop.example"));
        assert_eq!(h.references, ["r1@x", "r2@x"]);
        assert_eq!(h.in_reply_to.as_deref(), Some("r2@x"));
        assert_eq!(h.date, Some(1_794_294_900));
        assert_eq!(h.list_id.as_deref(), Some("news.shop.example"));
        assert_eq!(
            h.list_unsubscribe,
            [
                "mailto:unsub@shop.example?subject=stop",
                "https://shop.example/u/123"
            ]
        );
        assert!(h.list_unsubscribe_post);
        assert!(!h.dmarc_pass);
    }

    #[test]
    fn forged_authentication_results_below_the_providers_do_not_count() {
        // The provider's header (topmost) says fail; the sender added a
        // fake "pass" further down.
        let raw = b"Authentication-Results: mx.provider.example; dmarc=fail header.from=paypal.example\r\n\
From: Service <service@paypal.example>\r\n\
Authentication-Results: mx.provider.example; dmarc=pass header.from=paypal.example\r\n\r\n";
        assert!(!parse_header_block(raw).dmarc_pass);

        // A pass for a different domain does not vouch for the From domain.
        let raw =
            b"Authentication-Results: mx.provider.example; dmarc=pass header.from=evil.example\r\n\
From: Service <service@paypal.example>\r\n\r\n";
        assert!(!parse_header_block(raw).dmarc_pass);

        let raw = b"Authentication-Results: mx.provider.example; dkim=pass; dmarc=pass (p=reject) header.from=paypal.example\r\n\
From: Service <service@PayPal.example>\r\n\r\n";
        assert!(parse_header_block(raw).dmarc_pass);
    }

    #[test]
    fn detects_dmarc_pass_only_on_valid_clauses() {
        let raw_pass = b"From: Info <info@shop.example>\r\n\
Authentication-Results: mx.example.org; dkim=pass; spf=pass; dmarc=pass (p=REJECT) header.from=shop.example\r\n\r\n";
        assert!(parse_header_block(raw_pass).dmarc_pass);

        let raw_fail = b"From: Info <info@shop.example>\r\n\
Authentication-Results: mx.example.org; dkim=fail; dmarc=fail header.from=shop.example\r\n\r\n";
        assert!(!parse_header_block(raw_fail).dmarc_pass);

        let raw_spoofed_prefix = b"From: Info <info@shop.example>\r\n\
Authentication-Results: mx.example.org; x-dmarc=pass; dmarc=bestguesspass\r\n\r\n";
        assert!(!parse_header_block(raw_spoofed_prefix).dmarc_pass);
    }

    #[test]
    fn tolerates_empty_or_broken_headers() {
        let h = parse_header_block(b"");
        assert!(h.subject.is_none());
        let h = parse_header_block(b"Subject\r\nFrom: <<>>\r\n\r\n");
        // Garbage in must not panic; whatever comes out is never an address.
        assert!(
            h.from_addr.as_deref().is_none_or(|a| !a.contains('@')),
            "{:?}",
            h.from_addr
        );
    }
}
