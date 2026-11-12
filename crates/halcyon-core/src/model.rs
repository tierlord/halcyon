//! Plain data types shared by all parts of the core.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// How the connection to a server is secured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the first byte (IMAPS on 993, SMTPS on 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP on 143, submission on 587).
    StartTls,
    /// Unencrypted. Only ever offered for servers on the local machine.
    Plain,
}

impl fmt::Display for Security {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::Plain => "plain",
        })
    }
}

impl FromStr for Security {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tls" => Ok(Security::Tls),
            "starttls" => Ok(Security::StartTls),
            "plain" => Ok(Security::Plain),
            other => Err(Error::Other(format!("unknown security mode {other:?}"))),
        }
    }
}

/// How the user authenticates against a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    /// Username and password (or app password), sent over the encrypted channel.
    Password,
    /// OAuth 2.0 bearer token (SASL XOAUTH2 / OAUTHBEARER).
    OAuth2,
}

impl fmt::Display for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AuthMethod::Password => "password",
            AuthMethod::OAuth2 => "oauth2",
        })
    }
}

impl FromStr for AuthMethod {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "password" => Ok(AuthMethod::Password),
            "oauth2" => Ok(AuthMethod::OAuth2),
            other => Err(Error::Other(format!("unknown auth method {other:?}"))),
        }
    }
}

/// Connection settings for a single server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub auth: AuthMethod,
}

impl ServerConfig {
    /// Rejects settings that would expose the password: unencrypted
    /// connections are only allowed to the local machine (e.g. a local
    /// bridge such as a Proton Mail or Davmail gateway).
    ///
    /// Enforced in the core, not just the UI, so no front end – and no
    /// compromised web view – can send a password in clear text.
    pub fn validate(&self) -> crate::Result<()> {
        let host = self.host.trim();
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err(Error::Other(format!("invalid server name {:?}", self.host)));
        }
        if self.port == 0 {
            return Err(Error::Other("invalid port 0".into()));
        }
        if self.security == Security::Plain && !is_local_host(host) {
            return Err(Error::Unsupported(
                "unencrypted connections to remote servers",
            ));
        }
        Ok(())
    }

    /// Whether `other` would talk to the same server as the same user, so a
    /// stored password may be reused for it.
    pub fn same_endpoint(&self, other: &ServerConfig) -> bool {
        self.host.trim().eq_ignore_ascii_case(other.host.trim())
            && self.port == other.port
            && self.security == other.security
            && self.username == other.username
            && self.auth == other.auth
    }
}

/// `localhost`, `127.0.0.0/8` or `::1`.
pub fn is_local_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// A configured mail account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    /// Local database id; `0` for an account that has not been saved yet.
    pub id: i64,
    pub email: String,
    pub display_name: Option<String>,
    pub imap: ServerConfig,
    pub smtp: Option<ServerConfig>,
}

/// What a folder is used for, from the server's SPECIAL-USE attributes
/// (RFC 6154) or, as a fallback, from well-known folder names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    All,
    Flagged,
}

impl FolderRole {
    pub fn as_str(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Sent => "sent",
            FolderRole::Drafts => "drafts",
            FolderRole::Trash => "trash",
            FolderRole::Junk => "junk",
            FolderRole::Archive => "archive",
            FolderRole::All => "all",
            FolderRole::Flagged => "flagged",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "inbox" => FolderRole::Inbox,
            "sent" => FolderRole::Sent,
            "drafts" => FolderRole::Drafts,
            "trash" => FolderRole::Trash,
            "junk" => FolderRole::Junk,
            "archive" => FolderRole::Archive,
            "all" => FolderRole::All,
            "flagged" => FolderRole::Flagged,
            _ => return None,
        })
    }

    /// Guesses the role from a folder name for servers without SPECIAL-USE.
    pub fn guess_from_name(path: &str, delimiter: Option<&str>) -> Option<Self> {
        if path.eq_ignore_ascii_case("INBOX") {
            return Some(FolderRole::Inbox);
        }
        let leaf = match delimiter {
            Some(d) if !d.is_empty() => path.rsplit(d).next().unwrap_or(path),
            _ => path,
        };
        let leaf = leaf.to_lowercase();
        Some(match leaf.as_str() {
            "sent" | "sent items" | "sent messages" | "gesendet" | "gesendete objekte"
            | "gesendete elemente" | "postausgang" => FolderRole::Sent,
            "drafts" | "entwürfe" | "entwurf" => FolderRole::Drafts,
            "trash"
            | "deleted"
            | "deleted items"
            | "deleted messages"
            | "papierkorb"
            | "gelöscht"
            | "gelöschte objekte"
            | "gelöschte elemente" => FolderRole::Trash,
            "junk" | "spam" | "junk e-mail" | "junk-e-mail" | "spamverdacht" | "unerwünscht" => {
                FolderRole::Junk
            }
            "archive" | "archiv" => FolderRole::Archive,
            _ => return None,
        })
    }
}

/// A folder (IMAP mailbox) of an account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub path: String,
    pub delimiter: Option<String>,
    pub role: Option<FolderRole>,
    pub selectable: bool,
    pub uid_validity: Option<u32>,
    /// Highest UID we have stored for this folder.
    pub last_seen_uid: u32,
}

/// The header-level data the core keeps for every message.
///
/// The fields are chosen for what Halcyon needs to do without downloading
/// bodies: listing, search, conversation grouping (`message_id`,
/// `in_reply_to`, `references`) and newsletter handling (`list_*`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MessageHeader {
    pub uid: u32,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub reply_to: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// `Date:` header as Unix timestamp; falls back to the server's INTERNALDATE.
    pub date: Option<i64>,
    pub size: Option<u32>,
    pub flags: Vec<String>,
    pub list_id: Option<String>,
    pub list_unsubscribe: Vec<String>,
    /// RFC 8058 one-click unsubscribe is possible.
    pub list_unsubscribe_post: bool,
}

/// A stored message as returned from the database.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: i64,
    pub account_id: i64,
    pub account_email: String,
    pub folder_id: i64,
    /// Mailbox name as the server knows it (modified UTF-7).
    pub folder_path: String,
    /// Mailbox name for display.
    pub folder_name: String,
    pub header: MessageHeader,
    /// Start of the text, once the content is downloaded.
    pub preview: Option<String>,
    pub has_attachments: bool,
    pub has_body: bool,
}

impl MessageSummary {
    pub fn is_seen(&self) -> bool {
        self.header.flags.iter().any(|f| f == "\\Seen")
    }
}

/// Splits an address into local part and domain.
pub fn split_address(email: &str) -> crate::Result<(&str, &str)> {
    let email = email.trim();
    match email.rsplit_once('@') {
        Some((local, domain)) if !local.is_empty() && domain.contains('.') => Ok((local, domain)),
        _ => Err(Error::InvalidAddress(email.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_guessing_handles_german_and_nested_names() {
        assert_eq!(
            FolderRole::guess_from_name("INBOX", Some("/")),
            Some(FolderRole::Inbox)
        );
        assert_eq!(
            FolderRole::guess_from_name("INBOX.Gesendet", Some(".")),
            Some(FolderRole::Sent)
        );
        assert_eq!(
            FolderRole::guess_from_name("Papierkorb", None),
            Some(FolderRole::Trash)
        );
        assert_eq!(FolderRole::guess_from_name("Rechnungen", Some("/")), None);
    }

    #[test]
    fn split_address_rejects_garbage() {
        assert_eq!(
            split_address("max@example.org").unwrap(),
            ("max", "example.org")
        );
        assert!(split_address("max").is_err());
        assert!(split_address("@example.org").is_err());
    }

    fn server(host: &str, security: Security) -> ServerConfig {
        ServerConfig {
            host: host.into(),
            port: 143,
            security,
            username: "max".into(),
            auth: AuthMethod::Password,
        }
    }

    #[test]
    fn plain_text_only_to_local_machine() {
        assert!(
            server("imap.example.org", Security::Plain)
                .validate()
                .is_err()
        );
        assert!(server("10.0.0.5", Security::Plain).validate().is_err());
        assert!(server("localhost", Security::Plain).validate().is_ok());
        assert!(server("127.0.0.1", Security::Plain).validate().is_ok());
        assert!(server("[::1]", Security::Plain).validate().is_ok());
        assert!(
            server("imap.example.org", Security::StartTls)
                .validate()
                .is_ok()
        );
        assert!(server("", Security::Tls).validate().is_err());
        assert!(
            server("imap example.org", Security::Tls)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn endpoint_comparison_ignores_host_case_only() {
        let a = server("imap.example.org", Security::Tls);
        let mut b = server("IMAP.example.org", Security::Tls);
        assert!(a.same_endpoint(&b));
        b.port = 993;
        assert!(!a.same_endpoint(&b));
        let c = server("evil.example.net", Security::Tls);
        assert!(!a.same_endpoint(&c));
    }
}
