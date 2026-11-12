//! Account secrets in the operating system's keychain (Secret Service on
//! Linux, Credential Manager on Windows). Secrets never touch the database.

use crate::{Error, Result};

const SERVICE: &str = "halcyon";

/// Which secret of an account an entry holds.
#[derive(Clone, Copy, Debug)]
pub enum SecretKind {
    ImapPassword,
    SmtpPassword,
    OAuthRefreshToken,
}

impl SecretKind {
    fn suffix(self) -> &'static str {
        match self {
            SecretKind::ImapPassword => "imap",
            SecretKind::SmtpPassword => "smtp",
            SecretKind::OAuthRefreshToken => "oauth-refresh",
        }
    }
}

fn entry(email: &str, kind: SecretKind) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("{email}:{}", kind.suffix()))
        .map_err(|e| Error::Secret(e.to_string()))
}

pub fn store(email: &str, kind: SecretKind, secret: &str) -> Result<()> {
    entry(email, kind)?
        .set_password(secret)
        .map_err(|e| Error::Secret(e.to_string()))
}

/// Returns `None` if no secret is stored.
pub fn load(email: &str, kind: SecretKind) -> Result<Option<String>> {
    match entry(email, kind)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Secret(e.to_string())),
    }
}

pub fn delete(email: &str, kind: SecretKind) -> Result<()> {
    match entry(email, kind)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Secret(e.to_string())),
    }
}
