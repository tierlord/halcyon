use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("IMAP error: {0}")]
    Imap(#[from] async_imap::error::Error),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("the server does not support {0}")]
    Unsupported(&'static str),

    #[error("could not store or read a secret: {0}")]
    Secret(String),

    #[error("invalid email address: {0}")]
    InvalidAddress(String),

    #[error("{0}")]
    Other(String),
}
