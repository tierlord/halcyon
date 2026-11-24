//! Local mail store on top of SQLite.
//!
//! All accounts live in one database, which is what makes cross-account
//! views (categories, search) cheap. Full-text search uses FTS5 as an
//! external-content index that triggers keep in sync with `messages`.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Result;
use crate::brand::{BrandLogo, LOGO_HIT_TTL_SECS, LOGO_MISS_TTL_SECS};
use crate::cases::{
    Case, CaseEvent, CaseEventPhase, CaseKey, CaseKind, CaseStatus, CaseView, EventDirection,
    NewCase, NewCaseEvent, normalize_key,
};
use crate::model::{
    Account, AuthMethod, Folder, FolderRole, MessageHeader, MessageSummary, PendingChange,
    PendingOp, Security, ServerConfig,
};
use crate::send::{Built, Draft};
use crate::sources::{
    AnomalyKind, IncomingSourceMail, MailboxEvalStats, SourceAnomaly, SourceMode, SourceRecord,
    SourceStats, detect_anomaly, suggest_source_mode_with_behavior,
};
use crate::turn::{SenderBehavior, TurnStatus, assess_turn};

/// Schema migrations, applied in order. The index + 1 is stored in
/// `PRAGMA user_version`, so new migrations must only ever be appended.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE accounts (
    id              INTEGER PRIMARY KEY,
    email           TEXT NOT NULL UNIQUE,
    display_name    TEXT,
    imap_host       TEXT NOT NULL,
    imap_port       INTEGER NOT NULL,
    imap_security   TEXT NOT NULL,
    imap_username   TEXT NOT NULL,
    imap_auth       TEXT NOT NULL,
    smtp_host       TEXT,
    smtp_port       INTEGER,
    smtp_security   TEXT,
    smtp_username   TEXT,
    smtp_auth       TEXT,
    created_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE folders (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path            TEXT NOT NULL,
    delimiter       TEXT,
    role            TEXT,
    selectable      INTEGER NOT NULL DEFAULT 1,
    uid_validity    INTEGER,
    last_seen_uid   INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, path)
);

CREATE TABLE messages (
    id                    INTEGER PRIMARY KEY,
    folder_id             INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid                   INTEGER NOT NULL,
    message_id            TEXT,
    in_reply_to           TEXT,
    refs                  TEXT NOT NULL DEFAULT '[]',
    subject               TEXT,
    from_name             TEXT,
    from_addr             TEXT,
    reply_to              TEXT,
    to_addrs              TEXT NOT NULL DEFAULT '[]',
    cc_addrs              TEXT NOT NULL DEFAULT '[]',
    date                  INTEGER,
    size                  INTEGER,
    flags                 TEXT NOT NULL DEFAULT '[]',
    list_id               TEXT,
    list_unsubscribe      TEXT NOT NULL DEFAULT '[]',
    list_unsubscribe_post INTEGER NOT NULL DEFAULT 0,
    UNIQUE (folder_id, uid)
);
CREATE INDEX messages_date ON messages(date DESC);
CREATE INDEX messages_message_id ON messages(message_id);
CREATE INDEX messages_from ON messages(from_addr);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, from_name, from_addr, to_addrs,
    content = 'messages', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, to_addrs)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.to_addrs);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, to_addrs)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.to_addrs);
END;
CREATE TRIGGER messages_au AFTER UPDATE OF subject, from_name, from_addr, to_addrs ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, to_addrs)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.to_addrs);
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, to_addrs)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.to_addrs);
END;
"#,
    // 2: message content. The raw message is kept (compressed) so features
    // added later can re-read it without downloading again. The extracted text
    // joins the full-text index.
    r#"
ALTER TABLE messages ADD COLUMN body_text TEXT;
ALTER TABLE messages ADD COLUMN preview TEXT;
ALTER TABLE messages ADD COLUMN has_attachments INTEGER NOT NULL DEFAULT 0;

CREATE TABLE message_bodies (
    message_id  INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    raw_zstd    BLOB NOT NULL,
    raw_size    INTEGER NOT NULL,
    fetched_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

DROP TRIGGER messages_ai;
DROP TRIGGER messages_ad;
DROP TRIGGER messages_au;
DROP TABLE messages_fts;
CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, from_name, from_addr, to_addrs, body_text,
    content = 'messages', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, to_addrs, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.to_addrs, new.body_text);
END;
CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, to_addrs, body_text)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.to_addrs, old.body_text);
END;
CREATE TRIGGER messages_au AFTER UPDATE OF subject, from_name, from_addr, to_addrs, body_text ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, to_addrs, body_text)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.to_addrs, old.body_text);
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, to_addrs, body_text)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.to_addrs, new.body_text);
END;
INSERT INTO messages_fts(messages_fts) VALUES ('rebuild');
"#,
    // 3: two-way sync. CONDSTORE state per folder, and a queue of local
    // changes (read, flagged, moved, deleted) that still have to reach the
    // server.
    r#"
ALTER TABLE folders ADD COLUMN highest_modseq INTEGER;

CREATE TABLE pending_ops (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    message_id  INTEGER,
    folder_path TEXT NOT NULL,
    uid         INTEGER NOT NULL,
    op          TEXT NOT NULL,
    arg         TEXT,
    attempts    INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX pending_ops_account ON pending_ops(account_id, id);
CREATE INDEX pending_ops_message ON pending_ops(message_id);
"#,
    // 4: outbox. Messages are stored fully built, so what is sent (and filed
    // as sent) is exactly what the user confirmed; `send_after` gives the
    // "undo send" window.
    r#"
CREATE TABLE outbox (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    wire           BLOB NOT NULL,
    record         BLOB NOT NULL,
    envelope_from  TEXT NOT NULL,
    envelope_to    TEXT NOT NULL,
    message_id     TEXT NOT NULL,
    subject        TEXT NOT NULL,
    draft          TEXT NOT NULL,
    send_after     INTEGER NOT NULL,
    state          TEXT NOT NULL DEFAULT 'queued',
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX outbox_due ON outbox(state, send_after);
"#,
    // 5: brand logo cache (hits and negative misses with TTL).
    r#"
CREATE TABLE brand_logos (
    domain      TEXT PRIMARY KEY,
    data_url    TEXT,
    bg          TEXT,
    padded      INTEGER NOT NULL DEFAULT 1,
    scale       REAL NOT NULL DEFAULT 1.0,
    fetched_at  INTEGER NOT NULL DEFAULT (unixepoch())
);
"#,
    // 6: DMARC authentication status per message header.
    r#"
ALTER TABLE messages ADD COLUMN dmarc_pass INTEGER NOT NULL DEFAULT 0;
"#,
    // 7: small key/value store for one-time jobs and settings.
    r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#,
    // 8–9: reserved (previous temporary DMARC IMAP backfill removed; keep
    // user_version alignment for databases that already ran 8–9).
    "",
    "",
    // 10: cases ("Vorgänge"), their chronology events, identifying keys,
    // per-message exclusions ("Gehört nicht zum Vorgang"), and learned rules.
    r#"
CREATE TABLE cases (
    id           INTEGER PRIMARY KEY,
    title        TEXT NOT NULL,
    counterparty TEXT NOT NULL,
    domain       TEXT,
    kind         TEXT NOT NULL,
    status       TEXT NOT NULL,
    next_step    TEXT NOT NULL,
    due_at       INTEGER,
    due_label    TEXT,
    amount_cents INTEGER,
    currency     TEXT,
    reason       TEXT NOT NULL DEFAULT '',
    created_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX cases_status ON cases(status, updated_at DESC);

CREATE TABLE case_events (
    id          INTEGER PRIMARY KEY,
    case_id     INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    message_id  INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    occurred_at INTEGER NOT NULL,
    kind        TEXT,
    label       TEXT NOT NULL,
    phase       TEXT NOT NULL,
    direction   TEXT,
    confidence  REAL NOT NULL DEFAULT 1.0,
    manual      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX case_events_case ON case_events(case_id, occurred_at ASC, id ASC);
CREATE INDEX case_events_message ON case_events(message_id);

CREATE TABLE case_keys (
    id         INTEGER PRIMARY KEY,
    case_id    INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    label      TEXT NOT NULL,
    value      TEXT NOT NULL,
    normalized TEXT NOT NULL,
    UNIQUE (case_id, label, normalized)
);
CREATE INDEX case_keys_lookup ON case_keys(normalized);

CREATE TABLE case_exclusions (
    case_id    INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (case_id, message_id)
);

CREATE TABLE case_rules (
    id          INTEGER PRIMARY KEY,
    sender_addr TEXT,
    domain      TEXT,
    rule_kind   TEXT NOT NULL,
    target_kind TEXT,
    pattern     TEXT,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX case_rules_domain ON case_rules(domain);

-- Messages the linker has looked at. `with_body` records whether the text
-- was available, so a message is looked at again once its body arrives.
CREATE TABLE case_scan (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    with_body  INTEGER NOT NULL
);
"#,
    // 11: senders for whom remote images are always loaded automatically.
    r#"
CREATE TABLE trusted_remote_senders (
    sender     TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
"#,
    // 12: recurring machine-mail sources ("Quellen") and normalized subject
    // patterns ("Wer ist am Zug?" concept).
    r#"
CREATE TABLE sources (
    id                   INTEGER PRIMARY KEY,
    source_key           TEXT NOT NULL UNIQUE,
    display_name         TEXT NOT NULL,
    domain               TEXT,
    mode                 TEXT NOT NULL DEFAULT 'show',
    user_mode            INTEGER NOT NULL DEFAULT 0,
    total_messages       INTEGER NOT NULL DEFAULT 0,
    read_messages        INTEGER NOT NULL DEFAULT 0,
    dmarc_pass_count     INTEGER NOT NULL DEFAULT 0,
    is_newsletter        INTEGER NOT NULL DEFAULT 0,
    first_seen_at        INTEGER NOT NULL DEFAULT 0,
    last_seen_at         INTEGER NOT NULL DEFAULT 0,
    median_interval_secs INTEGER,
    median_amount_cents  INTEGER,
    amount_sample_count  INTEGER NOT NULL DEFAULT 0,
    anomaly_kind         TEXT,
    anomaly_needs_you    INTEGER NOT NULL DEFAULT 0,
    anomaly_reason       TEXT,
    anomaly_message_id   INTEGER REFERENCES messages(id) ON DELETE SET NULL
);
CREATE INDEX sources_last_seen ON sources(last_seen_at DESC);

CREATE TABLE source_patterns (
    id            INTEGER PRIMARY KEY,
    source_id     INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    pattern       TEXT NOT NULL,
    message_count INTEGER NOT NULL DEFAULT 0,
    last_seen_at  INTEGER NOT NULL DEFAULT 0,
    UNIQUE (source_id, pattern)
);

ALTER TABLE messages ADD COLUMN source_id INTEGER REFERENCES sources(id) ON DELETE SET NULL;
ALTER TABLE messages ADD COLUMN subject_pattern TEXT;
CREATE INDEX messages_source ON messages(source_id, date DESC);
DELETE FROM case_scan;
"#,
    // 13: invoice ledger. Invoices outlive the mail they came from
    // (`message_id` is cleared, not the row), proof copies are kept in
    // `invoice_files`, and `sender_portals` remembers where a sender's
    // customer area is. `kind` on accounts splits private from business.
    r#"
ALTER TABLE accounts ADD COLUMN kind TEXT NOT NULL DEFAULT 'private';

CREATE TABLE invoices (
    id             INTEGER PRIMARY KEY,
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    issuer         TEXT NOT NULL,
    domain         TEXT,
    title          TEXT NOT NULL,
    invoice_number TEXT,
    number_norm    TEXT,
    amount_cents   INTEGER,
    currency       TEXT,
    issued_at      INTEGER NOT NULL,
    due_at         INTEGER,
    paid           INTEGER NOT NULL DEFAULT 0,
    paid_source    TEXT,
    paid_note      TEXT,
    kind_override  TEXT,
    evidence       TEXT NOT NULL DEFAULT 'none',
    message_id     INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    rfc_message_id TEXT,
    case_id        INTEGER REFERENCES cases(id) ON DELETE SET NULL,
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX invoices_issued ON invoices(issued_at DESC);
CREATE INDEX invoices_rfc ON invoices(account_id, rfc_message_id);
CREATE INDEX invoices_number ON invoices(account_id, domain, number_norm);

CREATE TABLE invoice_files (
    id         INTEGER PRIMARY KEY,
    invoice_id INTEGER NOT NULL UNIQUE REFERENCES invoices(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    mime_type  TEXT NOT NULL,
    size       INTEGER NOT NULL,
    data_zstd  BLOB NOT NULL
);

CREATE TABLE sender_portals (
    domain     TEXT PRIMARY KEY,
    url        TEXT NOT NULL,
    label      TEXT NOT NULL,
    origin     TEXT NOT NULL,
    score      INTEGER NOT NULL,
    message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Messages the invoice pass has looked at (see `case_scan`).
CREATE TABLE invoice_scan (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    with_body  INTEGER NOT NULL
);
"#,
    // 14: remember the RFC `Message-ID` on case events. A mail moved to
    // Trash is a new row (new UID) and the old row's events lose their
    // `message_id`; without this the moved copy would be counted twice.
    r#"
ALTER TABLE case_events ADD COLUMN rfc_message_id TEXT;
UPDATE case_events SET rfc_message_id =
    (SELECT m.message_id FROM messages m WHERE m.id = case_events.message_id)
 WHERE message_id IS NOT NULL;
CREATE INDEX case_events_rfc ON case_events(rfc_message_id);
"#,
    // 15: the ledger lists every order, also when no invoice mail ever
    // arrives. `from_order` marks such a placeholder row (one per order
    // case); an invoice for the same case takes it over.
    r#"
ALTER TABLE invoices ADD COLUMN from_order INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX invoices_order_case ON invoices(case_id) WHERE from_order = 1;
"#,
    // 16: "Von dir als erledigt entfernt" for conversations. Keyed by the RFC
    // `Message-ID` of the judged mail (not the row id), so the dismissal
    // survives moves between folders and copies, while a NEW mail in the
    // thread has a new id and is assessed normally. `from_addr` is kept as
    // the raw signal for learning which senders never need the user.
    r#"
CREATE TABLE turn_dismissals (
    rfc_message_id TEXT PRIMARY KEY,
    from_addr      TEXT,
    created_at     INTEGER NOT NULL DEFAULT (unixepoch())
);
"#,
    // 17: keywords on the server. `folders.keywords` is whether the folder
    // accepts client-defined keywords (PERMANENTFLAGS contains `\*`), learnt
    // when the folder is selected. `filing_scan` remembers which inbox mails
    // the auto-read pass has already judged, so it judges each one once and
    // never re-marks a mail the user marked unread again.
    r#"
ALTER TABLE folders ADD COLUMN keywords INTEGER NOT NULL DEFAULT 0;

CREATE TABLE filing_scan (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE
);
"#,
    // 18: which mails the user has answered, by RFC `Message-ID`. Kept apart
    // from the sent mail itself, so an answered question does not come back
    // as "needs you" when the Sent folder is emptied before the inbox.
    r#"
CREATE TABLE answered_mails (
    rfc_message_id TEXT PRIMARY KEY,
    answered_at    INTEGER NOT NULL
);
INSERT INTO answered_mails (rfc_message_id, answered_at)
SELECT rfc, MAX(at) FROM (
    SELECT m.in_reply_to AS rfc, COALESCE(m.date, 0) AS at
    FROM messages m JOIN folders f ON f.id = m.folder_id
    WHERE f.role = 'sent' AND m.in_reply_to IS NOT NULL
    UNION ALL
    SELECT j.value, COALESCE(m.date, 0)
    FROM messages m JOIN folders f ON f.id = m.folder_id, json_each(m.refs) j
    WHERE f.role = 'sent'
) WHERE rfc IS NOT NULL AND rfc != '' GROUP BY rfc;
"#,
    // 19: learned user behavior per sender (open, reply, delete, ignore).
    // `message_interactions` deduplicates per message; `sender_behavior`
    // retains the learned counts even after messages are deleted.
    r#"
CREATE TABLE message_interactions (
    message_id  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    action      TEXT NOT NULL CHECK (action IN ('open', 'reply', 'delete', 'ignore')),
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (message_id, action)
);

CREATE TABLE sender_behavior (
    from_addr      TEXT PRIMARY KEY,
    opened_count   INTEGER NOT NULL DEFAULT 0,
    replied_count  INTEGER NOT NULL DEFAULT 0,
    deleted_count  INTEGER NOT NULL DEFAULT 0,
    ignored_count  INTEGER NOT NULL DEFAULT 0,
    last_action_at INTEGER NOT NULL DEFAULT 0
);
"#,
];

/// Result of looking up a domain in the persistent brand logo cache.
#[derive(Clone, Debug, PartialEq)]
pub enum CachedLogo {
    Hit(BrandLogo),
    Miss,
}

/// User interaction tracked for learning sender relevance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    Open,
    Reply,
    Delete,
    Ignore,
}

impl Interaction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Reply => "reply",
            Self::Delete => "delete",
            Self::Ignore => "ignore",
        }
    }
}

impl std::str::FromStr for Interaction {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "open" => Ok(Self::Open),
            "reply" => Ok(Self::Reply),
            "delete" => Ok(Self::Delete),
            "ignore" => Ok(Self::Ignore),
            other => Err(crate::Error::Other(format!("unknown interaction: {other}"))),
        }
    }
}

fn record_interaction_conn(
    conn: &Connection,
    message_id: i64,
    action: Interaction,
    now: i64,
) -> Result<()> {
    let from: Option<String> = conn
        .query_row(
            "SELECT from_addr FROM messages WHERE id = ?1",
            [message_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(addr) = from
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO message_interactions (message_id, action, created_at)
         VALUES (?1, ?2, ?3)",
        params![message_id, action.as_str(), now],
    )?;
    if inserted == 0 {
        return Ok(());
    }
    let col = match action {
        Interaction::Open => "opened_count",
        Interaction::Reply => "replied_count",
        Interaction::Delete => "deleted_count",
        Interaction::Ignore => "ignored_count",
    };
    let sql = format!(
        "INSERT INTO sender_behavior (from_addr, {col}, last_action_at)
         VALUES (?1, 1, ?2)
         ON CONFLICT(from_addr) DO UPDATE SET
             {col} = sender_behavior.{col} + 1,
             last_action_at = MAX(sender_behavior.last_action_at, excluded.last_action_at)"
    );
    conn.execute(&sql, params![addr, now])?;
    Ok(())
}

/// Failed changes are retried this many times, then kept for inspection.
const MAX_OP_ATTEMPTS: u32 = 5;

/// Compression level for stored messages: fast, and mail text compresses
/// well even at low levels.
const ZSTD_LEVEL: i32 = 3;

/// Characters of text kept as list preview.
const PREVIEW_CHARS: usize = 180;

pub struct Store {
    /// Shared with the other store-backed modules (`invoices`).
    pub(crate) conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 15000;",
        )?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    // ----- accounts -------------------------------------------------------

    /// Inserts a new account and returns it with its id set.
    pub fn add_account(&self, account: &Account) -> Result<Account> {
        let smtp = account.smtp.as_ref();
        self.conn.execute(
            "INSERT INTO accounts (email, display_name,
                imap_host, imap_port, imap_security, imap_username, imap_auth,
                smtp_host, smtp_port, smtp_security, smtp_username, smtp_auth)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                account.email,
                account.display_name,
                account.imap.host,
                account.imap.port,
                account.imap.security.to_string(),
                account.imap.username,
                account.imap.auth.to_string(),
                smtp.map(|s| &s.host),
                smtp.map(|s| s.port),
                smtp.map(|s| s.security.to_string()),
                smtp.map(|s| &s.username),
                smtp.map(|s| s.auth.to_string()),
            ],
        )?;
        let mut saved = account.clone();
        saved.id = self.conn.last_insert_rowid();
        Ok(saved)
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare("SELECT * FROM accounts ORDER BY id")?;
        let rows = stmt.query_map([], account_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn account_by_email(&self, email: &str) -> Result<Option<Account>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM accounts WHERE email = ?1 COLLATE NOCASE",
                [email],
                account_from_row,
            )
            .optional()?)
    }

    pub fn remove_account(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Updates an existing account's display name and server settings.
    pub fn update_account(&self, account: &Account) -> Result<()> {
        let smtp = account.smtp.as_ref();
        self.conn.execute(
            "UPDATE accounts
             SET email = ?2, display_name = ?3,
                 imap_host = ?4, imap_port = ?5, imap_security = ?6,
                 imap_username = ?7, imap_auth = ?8,
                 smtp_host = ?9, smtp_port = ?10, smtp_security = ?11,
                 smtp_username = ?12, smtp_auth = ?13
             WHERE id = ?1",
            params![
                account.id,
                account.email,
                account.display_name,
                account.imap.host,
                account.imap.port,
                account.imap.security.to_string(),
                account.imap.username,
                account.imap.auth.to_string(),
                smtp.map(|s| &s.host),
                smtp.map(|s| s.port),
                smtp.map(|s| s.security.to_string()),
                smtp.map(|s| &s.username),
                smtp.map(|s| s.auth.to_string()),
            ],
        )?;
        Ok(())
    }

    // ----- folders --------------------------------------------------------

    /// Creates the folder or updates its server-side attributes. Sync state
    /// (`uid_validity`, `last_seen_uid`) is left untouched.
    pub fn upsert_folder(
        &self,
        account_id: i64,
        path: &str,
        delimiter: Option<&str>,
        role: Option<FolderRole>,
        selectable: bool,
    ) -> Result<Folder> {
        self.conn.execute(
            "INSERT INTO folders (account_id, path, delimiter, role, selectable)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (account_id, path) DO UPDATE SET
                delimiter = excluded.delimiter,
                role = excluded.role,
                selectable = excluded.selectable",
            params![
                account_id,
                path,
                delimiter,
                role.map(FolderRole::as_str),
                selectable
            ],
        )?;
        Ok(self.conn.query_row(
            "SELECT * FROM folders WHERE account_id = ?1 AND path = ?2",
            params![account_id, path],
            folder_from_row,
        )?)
    }

    /// Records whether the server accepts client-defined keywords here.
    pub fn set_folder_keywords(&self, folder_id: i64, allowed: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET keywords = ?2 WHERE id = ?1",
            params![folder_id, allowed],
        )?;
        Ok(())
    }

    pub fn folder_by_path(&self, account_id: i64, path: &str) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM folders WHERE account_id = ?1 AND path = ?2",
                params![account_id, path],
                folder_from_row,
            )
            .optional()?)
    }

    pub fn folders(&self, account_id: i64) -> Result<Vec<Folder>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM folders WHERE account_id = ?1 ORDER BY path")?;
        let rows = stmt.query_map([account_id], folder_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn folder_by_id(&self, folder_id: i64) -> Result<Option<Folder>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM folders WHERE id = ?1",
                [folder_id],
                folder_from_row,
            )
            .optional()?)
    }

    /// Removes folders that no longer exist on the server.
    pub fn retain_folders(&self, account_id: i64, existing: &[String]) -> Result<usize> {
        let mut removed = 0;
        for folder in self.folders(account_id)? {
            if !existing.contains(&folder.path) {
                removed += self
                    .conn
                    .execute("DELETE FROM folders WHERE id = ?1", [folder.id])?;
            }
        }
        Ok(removed)
    }

    /// Records a new UIDVALIDITY. If it differs from the stored one, every
    /// UID we know for that folder is meaningless, so the cached messages are
    /// dropped and the folder is synced from scratch.
    pub fn set_uid_validity(&self, folder_id: i64, uid_validity: u32) -> Result<bool> {
        let current: Option<u32> = self.conn.query_row(
            "SELECT uid_validity FROM folders WHERE id = ?1",
            [folder_id],
            |r| r.get(0),
        )?;
        if current == Some(uid_validity) {
            return Ok(false);
        }
        self.conn
            .execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
        self.conn.execute(
            "UPDATE folders SET uid_validity = ?2, last_seen_uid = 0 WHERE id = ?1",
            params![folder_id, uid_validity],
        )?;
        Ok(current.is_some())
    }

    // ----- messages -------------------------------------------------------

    /// Stores a batch of message headers. Already known messages only get
    /// their flags updated.
    ///
    /// This does not advance the folder's `last_seen_uid`; the sync calls
    /// [`Store::mark_synced_up_to`] once a folder is complete. That way the
    /// sync can fetch newest-first and still never skip messages when it is
    /// interrupted.
    pub fn insert_messages(&mut self, folder_id: i64, headers: &[MessageHeader]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO messages (folder_id, uid, message_id, in_reply_to, refs, subject,
                    from_name, from_addr, reply_to, to_addrs, cc_addrs, date, size, flags,
                    list_id, list_unsubscribe, list_unsubscribe_post, dmarc_pass)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT (folder_id, uid) DO UPDATE SET flags = excluded.flags, dmarc_pass = excluded.dmarc_pass",
            )?;
            for h in headers {
                stmt.execute(params![
                    folder_id,
                    h.uid,
                    h.message_id,
                    h.in_reply_to,
                    json(&h.references),
                    h.subject,
                    h.from_name,
                    h.from_addr,
                    h.reply_to,
                    json(&h.to),
                    json(&h.cc),
                    h.date,
                    h.size,
                    json(&h.flags),
                    h.list_id,
                    json(&h.list_unsubscribe),
                    h.list_unsubscribe_post,
                    h.dmarc_pass,
                ])?;
            }
            let sent: bool = tx.query_row(
                "SELECT COALESCE(role, '') = 'sent' FROM folders WHERE id = ?1",
                [folder_id],
                |r| r.get(0),
            )?;
            if sent {
                let mut answered = tx.prepare_cached(
                    "INSERT INTO answered_mails (rfc_message_id, answered_at) VALUES (?1, ?2)
                     ON CONFLICT (rfc_message_id) DO UPDATE SET
                        answered_at = MAX(answered_at, excluded.answered_at)",
                )?;
                for h in headers {
                    let at = h.date.unwrap_or(0);
                    for rfc in h.in_reply_to.iter().chain(&h.references) {
                        if !rfc.is_empty() {
                            answered.execute(params![rfc, at])?;
                        }
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Records that every message up to `uid` has been stored.
    pub fn mark_synced_up_to(&self, folder_id: i64, uid: u32) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET last_seen_uid = MAX(last_seen_uid, ?2) WHERE id = ?1",
            params![folder_id, uid],
        )?;
        Ok(())
    }

    /// Newest messages, optionally limited to one folder role (across all
    /// accounts) – e.g. a unified inbox.
    pub fn recent_messages(
        &self,
        role: Option<FolderRole>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>> {
        self.messages(None, role, limit)
    }

    /// Newest messages, filtered by account and/or folder role.
    pub fn messages(
        &self,
        account_id: Option<i64>,
        role: Option<FolderRole>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>> {
        let sql = format!(
            "{SUMMARY_SELECT}
             WHERE (?1 IS NULL OR f.account_id = ?1)
               AND (?2 IS NULL OR f.role = ?2)
             ORDER BY m.date DESC LIMIT ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![account_id, role.map(FolderRole::as_str), limit as i64],
            summary_from_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Messages inside a single folder, newest first.
    pub fn folder_messages(&self, folder_id: i64, limit: usize) -> Result<Vec<MessageSummary>> {
        let sql = format!(
            "{SUMMARY_SELECT}
             WHERE f.id = ?1
             ORDER BY m.date DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![folder_id, limit as i64], summary_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Number of unread messages, filtered like [`Store::messages`].
    pub fn unread_count(&self, account_id: Option<i64>, role: Option<FolderRole>) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE (?1 IS NULL OR f.account_id = ?1)
               AND (?2 IS NULL OR f.role = ?2)
               AND NOT EXISTS (SELECT 1 FROM json_each(m.flags) WHERE value = '\\Seen')",
            params![account_id, role.map(FolderRole::as_str)],
            |r| r.get(0),
        )?)
    }

    /// Unread message count keyed by `folder_id` across all folders.
    pub fn unread_counts_by_folder(&self) -> Result<std::collections::HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT folder_id, COUNT(*)
             FROM messages
             WHERE NOT EXISTS (SELECT 1 FROM json_each(flags) WHERE value = '\\Seen')
             GROUP BY folder_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (folder_id, count) = row?;
            map.insert(folder_id, count);
        }
        Ok(map)
    }

    /// Full-text search over subject, sender and recipients of all accounts.
    ///
    /// Each word of `query` must match (as prefix), so typing as you go works.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<MessageSummary>> {
        let Some(fts_query) = fts_prefix_query(query) else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "{SUMMARY_SELECT}
             JOIN messages_fts ON messages_fts.rowid = m.id
             WHERE messages_fts MATCH ?1
             ORDER BY bm25(messages_fts, 8.0, 4.0, 4.0, 2.0, 1.0), m.date DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![fts_query, limit as i64], summary_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Stores the raw message and the text extracted from it.
    pub fn store_body(&self, message_id: i64, raw: &[u8]) -> Result<()> {
        let content = crate::content::parse(raw);
        let text = content.text.unwrap_or_default();
        let preview = make_preview(&text);
        let has_attachments = content.attachments.iter().any(|a| !a.inline);
        // The full original has every header, also those the header-only
        // sync skipped or that came later (e.g. Authentication-Results).
        let dmarc_pass = crate::imap::parse_header_block(raw).dmarc_pass;
        let compressed = zstd::bulk::compress(raw, ZSTD_LEVEL)?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO message_bodies (message_id, raw_zstd, raw_size) VALUES (?1, ?2, ?3)
             ON CONFLICT (message_id) DO UPDATE SET
                raw_zstd = excluded.raw_zstd, raw_size = excluded.raw_size,
                fetched_at = unixepoch()",
            params![message_id, compressed, raw.len() as i64],
        )?;
        tx.execute(
            "UPDATE messages SET body_text = ?2, preview = ?3, has_attachments = ?4, dmarc_pass = ?5
             WHERE id = ?1",
            params![message_id, text, preview, has_attachments, dmarc_pass],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The raw message, if its content was downloaded.
    pub fn body(&self, message_id: i64) -> Result<Option<Vec<u8>>> {
        let row: Option<(Vec<u8>, i64)> = self
            .conn
            .query_row(
                "SELECT raw_zstd, raw_size FROM message_bodies WHERE message_id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((compressed, size)) => Ok(Some(zstd::bulk::decompress(
                &compressed,
                size.max(0) as usize,
            )?)),
            None => Ok(None),
        }
    }

    /// Messages of a folder without downloaded content, newest first: those
    /// dated after `since` and not larger than `max_size`.
    pub fn messages_without_body(
        &self,
        folder_id: i64,
        since: i64,
        max_size: u32,
        limit: usize,
    ) -> Result<Vec<(i64, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.uid FROM messages m
             WHERE m.folder_id = ?1 AND COALESCE(m.date, 0) >= ?2
               AND COALESCE(m.size, 0) <= ?3
               AND NOT EXISTS (SELECT 1 FROM message_bodies b WHERE b.message_id = m.id)
             ORDER BY m.date DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![folder_id, since, max_size, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Server-reported sizes of the given messages.
    pub fn message_sizes(&self, ids: &[i64]) -> Result<std::collections::HashMap<i64, u32>> {
        let mut out = std::collections::HashMap::new();
        let mut stmt = self
            .conn
            .prepare_cached("SELECT COALESCE(size, 0) FROM messages WHERE id = ?1")?;
        for id in ids {
            if let Some(size) = stmt.query_row([id], |r| r.get::<_, u32>(0)).optional()? {
                out.insert(*id, size);
            }
        }
        Ok(out)
    }

    /// One message by id.
    pub fn message(&self, message_id: i64) -> Result<Option<MessageSummary>> {
        let sql = format!("{SUMMARY_SELECT} WHERE m.id = ?1");
        Ok(self
            .conn
            .query_row(&sql, [message_id], summary_from_row)
            .optional()?)
    }

    // ----- two-way sync ---------------------------------------------------

    pub fn set_highest_modseq(&self, folder_id: i64, modseq: Option<u64>) -> Result<()> {
        self.conn.execute(
            "UPDATE folders SET highest_modseq = ?2 WHERE id = ?1",
            params![folder_id, modseq.map(|m| m as i64)],
        )?;
        Ok(())
    }

    /// Removes messages that no longer exist on the server (deleted or moved
    /// elsewhere). Only UIDs up to `up_to` are considered, so messages that
    /// arrived during the sync are not touched.
    pub fn delete_missing(
        &mut self,
        folder_id: i64,
        existing: &std::collections::HashSet<u32>,
        up_to: u32,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let local: Vec<(i64, u32)> = {
            let mut stmt =
                tx.prepare("SELECT id, uid FROM messages WHERE folder_id = ?1 AND uid <= ?2")?;
            stmt.query_map(params![folder_id, up_to], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let mut removed = 0;
        for (id, uid) in local {
            if !existing.contains(&uid) {
                removed += tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Takes over flags from the server. Messages with local changes that
    /// have not reached the server yet keep their local flags; otherwise a
    /// sync would undo what the user just did.
    pub fn apply_server_flags(
        &mut self,
        folder_id: i64,
        flags: &[(u32, Vec<String>)],
    ) -> Result<usize> {
        // While "mark all read" for this folder waits for the server, the
        // server still reports the old flags; do not take them over.
        let folder_op_pending: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pending_ops p JOIN folders f
               ON f.account_id = p.account_id AND f.path = p.folder_path
             WHERE f.id = ?1 AND p.op = 'mark_all_seen' AND p.attempts < ?2)",
            params![folder_id, MAX_OP_ATTEMPTS],
            |r| r.get(0),
        )?;
        if folder_op_pending {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE messages SET flags = ?3
                 WHERE folder_id = ?1 AND uid = ?2 AND flags <> ?3
                   AND NOT EXISTS (SELECT 1 FROM pending_ops p WHERE p.message_id = messages.id)",
            )?;
            for (uid, f) in flags {
                let mut sorted = f.clone();
                sorted.sort();
                changed += stmt.execute(params![folder_id, uid, json(&sorted)])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Sets or clears a flag (`\Seen`, `\Flagged`, …) or keyword (`$Name`)
    /// locally and queues the change for the server. Returns the number of
    /// messages changed. A keyword is skipped (neither stored nor queued) for
    /// folders that do not accept keywords.
    pub fn queue_flag(&mut self, message_ids: &[i64], flag: &str, on: bool) -> Result<usize> {
        let is_keyword = !flag.starts_with('\\');
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        for &id in message_ids {
            let row: Option<(String, i64, String, u32, bool)> = tx
                .query_row(
                    "SELECT m.flags, f.account_id, f.path, m.uid, f.keywords FROM messages m
                     JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let Some((raw, account_id, path, uid, keywords_ok)) = row else {
                continue;
            };
            if is_keyword && !keywords_ok {
                continue;
            }
            let mut flags: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if flags.iter().any(|f| f == flag) == on {
                continue;
            }
            if on {
                flags.push(flag.to_string());
            } else {
                flags.retain(|f| f != flag);
            }
            flags.sort();
            tx.execute(
                "UPDATE messages SET flags = ?2 WHERE id = ?1",
                params![id, json(&flags)],
            )?;
            // A newer change for the same flag replaces an unsent older one.
            tx.execute(
                "DELETE FROM pending_ops WHERE message_id = ?1 AND arg = ?2
                   AND op IN ('add_flag', 'remove_flag') AND attempts = 0",
                params![id, flag],
            )?;
            tx.execute(
                "INSERT INTO pending_ops (account_id, message_id, folder_path, uid, op, arg)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    account_id,
                    id,
                    path,
                    uid,
                    if on { "add_flag" } else { "remove_flag" },
                    flag
                ],
            )?;
            changed += 1;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Deletes messages: moves them to the account's trash, or removes them
    /// for good if they already are in the trash. Local copies go at once.
    /// Fails without any change if an account has no trash folder.
    pub fn queue_delete(&mut self, message_ids: &[i64]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut deleted = 0;
        for &id in message_ids {
            let row: Option<(i64, String, u32, Option<String>)> = tx
                .query_row(
                    "SELECT f.account_id, f.path, m.uid, f.role FROM messages m
                     JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((account_id, path, uid, role)) = row else {
                continue;
            };
            let (op, arg) = if role.as_deref() == Some("trash") {
                ("expunge", None)
            } else {
                let trash: Option<String> = tx
                    .query_row(
                        "SELECT path FROM folders WHERE account_id = ?1 AND role = 'trash' LIMIT 1",
                        [account_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                match trash {
                    Some(t) => ("move", Some(t)),
                    None => {
                        return Err(crate::Error::Other(
                            "this account has no trash folder".into(),
                        ));
                    }
                }
            };
            tx.execute(
                "INSERT INTO pending_ops (account_id, message_id, folder_path, uid, op, arg)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
                params![account_id, path, uid, op, arg],
            )?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            record_interaction_conn(&tx, id, Interaction::Delete, now)?;
            // Pending flag changes for this message are moot now.
            tx.execute("DELETE FROM pending_ops WHERE message_id = ?1", [id])?;
            tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            deleted += 1;
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// Marks every message of a folder read, locally at once and on the
    /// server with a single command. Returns how many were unread.
    pub fn queue_mark_folder_seen(&mut self, folder_id: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let Some((account_id, path, max_uid)) = folder_target(&tx, folder_id)? else {
            return Ok(0);
        };
        let changed = tx.execute(
            "UPDATE messages SET flags = json_insert(flags, '$[#]', '\\Seen')
             WHERE folder_id = ?1
               AND NOT EXISTS (SELECT 1 FROM json_each(messages.flags) WHERE value = '\\Seen')",
            [folder_id],
        )?;
        if changed > 0 {
            tx.execute(
                "INSERT INTO pending_ops (account_id, message_id, folder_path, uid, op)
                 VALUES (?1, NULL, ?2, ?3, 'mark_all_seen')",
                params![account_id, path, max_uid],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Empties a spam or trash folder for good: local copies go at once,
    /// the server follows with one command. Other folders are refused, so a
    /// slip of the mouse cannot wipe the inbox.
    pub fn queue_empty_folder(&mut self, folder_id: i64) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let role: Option<String> = tx
            .query_row("SELECT role FROM folders WHERE id = ?1", [folder_id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        if !matches!(role.as_deref(), Some("junk" | "trash")) {
            return Err(crate::Error::Other(
                "only spam and trash folders can be emptied".into(),
            ));
        }
        let Some((account_id, path, max_uid)) = folder_target(&tx, folder_id)? else {
            return Ok(0);
        };
        // Pending changes for these messages are moot now.
        tx.execute(
            "DELETE FROM pending_ops WHERE message_id IN (SELECT id FROM messages WHERE folder_id = ?1)",
            [folder_id],
        )?;
        let removed = tx.execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
        if removed > 0 {
            tx.execute(
                "INSERT INTO pending_ops (account_id, message_id, folder_path, uid, op)
                 VALUES (?1, NULL, ?2, ?3, 'empty_up_to')",
                params![account_id, path, max_uid],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Queued changes of an account that have not failed too often, oldest
    /// first.
    pub fn pending_ops(&self, account_id: i64) -> Result<Vec<PendingOp>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, folder_path, uid, op, arg, attempts FROM pending_ops
             WHERE account_id = ?1 AND attempts < ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![account_id, MAX_OP_ATTEMPTS], |r| {
            let op: String = r.get(4)?;
            let arg: Option<String> = r.get(5)?;
            let change = match (op.as_str(), arg) {
                ("add_flag", Some(f)) => PendingChange::AddFlag(f),
                ("remove_flag", Some(f)) => PendingChange::RemoveFlag(f),
                ("move", Some(t)) => PendingChange::MoveTo(t),
                ("mark_all_seen", _) => PendingChange::MarkAllSeen,
                ("empty_up_to", _) => PendingChange::EmptyUpTo,
                _ => PendingChange::Expunge,
            };
            Ok(PendingOp {
                id: r.get(0)?,
                account_id: r.get(1)?,
                folder_path: r.get(2)?,
                uid: r.get(3)?,
                change,
                attempts: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether any account has changes waiting for the server.
    pub fn has_pending_ops(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pending_ops WHERE attempts < ?1)",
            [MAX_OP_ATTEMPTS],
            |r| r.get(0),
        )?)
    }

    pub fn complete_op(&self, op_id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM pending_ops WHERE id = ?1", [op_id])?;
        Ok(())
    }

    pub fn fail_op(&self, op_id: i64, error: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending_ops SET attempts = attempts + 1, last_error = ?2 WHERE id = ?1",
            params![op_id, error],
        )?;
        Ok(())
    }

    // ----- outbox ---------------------------------------------------------

    /// Queues a built message for sending at `send_after` (Unix seconds).
    pub fn queue_outgoing(&self, draft: &Draft, built: &Built, send_after: i64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO outbox (account_id, wire, record, envelope_from, envelope_to,
                message_id, subject, draft, send_after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                draft.account_id,
                built.wire,
                built.record,
                built.envelope_from,
                json(&built.envelope_to),
                built.message_id,
                draft.subject,
                serde_json::to_string(draft).map_err(|e| crate::Error::Other(e.to_string()))?,
                send_after,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Takes a queued message back ("undo send"). Returns its draft, or
    /// `None` if it is already being sent or gone.
    pub fn cancel_outgoing(&self, id: i64) -> Result<Option<Draft>> {
        let draft: Option<String> = self
            .conn
            .query_row(
                "DELETE FROM outbox WHERE id = ?1 AND state = 'queued' RETURNING draft",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(draft.and_then(|d| serde_json::from_str(&d).ok()))
    }

    /// The next message due for sending, claimed atomically so it is never
    /// sent twice.
    pub fn claim_due_outgoing(&self, now: i64) -> Result<Option<OutboxItem>> {
        let has_due: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM outbox WHERE state = 'queued' AND send_after <= ?1)",
            [now],
            |r| r.get(0),
        )?;
        if !has_due {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                "UPDATE outbox SET state = 'sending', attempts = attempts + 1
                 WHERE id = (SELECT id FROM outbox WHERE state = 'queued' AND send_after <= ?1
                             ORDER BY send_after, id LIMIT 1)
                 RETURNING id, account_id, wire, record, envelope_from, envelope_to,
                           message_id, subject, draft, attempts",
                [now],
                outbox_from_row,
            )
            .optional()?)
    }

    /// When the next queued message is due, if any.
    pub fn next_outgoing_due(&self) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT MIN(send_after) FROM outbox WHERE state = 'queued'",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn finish_outgoing(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Records a failed attempt: retried at `retry_at`, or kept as failed
    /// for the user to look at (`retry_at = None`).
    pub fn fail_outgoing(&self, id: i64, error: &str, retry_at: Option<i64>) -> Result<()> {
        match retry_at {
            Some(t) => self.conn.execute(
                "UPDATE outbox SET state = 'queued', send_after = ?3, last_error = ?2 WHERE id = ?1",
                params![id, error, t],
            )?,
            None => self.conn.execute(
                "UPDATE outbox SET state = 'failed', last_error = ?2 WHERE id = ?1",
                params![id, error],
            )?,
        };
        Ok(())
    }

    /// Messages interrupted while sending (e.g. the app was closed) go back to
    /// the queue. The server may have received them already; a duplicate is
    /// better than a lost mail.
    pub fn requeue_interrupted_outgoing(&self) -> Result<usize> {
        let has_interrupted: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM outbox WHERE state = 'sending')",
            [],
            |r| r.get(0),
        )?;
        if !has_interrupted {
            return Ok(0);
        }
        Ok(self.conn.execute(
            "UPDATE outbox SET state = 'queued' WHERE state = 'sending'",
            [],
        )?)
    }

    /// A failed message back to the editor (it leaves the outbox).
    pub fn take_failed_outgoing(&self, id: i64) -> Result<Option<Draft>> {
        let draft: Option<String> = self
            .conn
            .query_row(
                "DELETE FROM outbox WHERE id = ?1 AND state = 'failed' RETURNING draft",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(draft.and_then(|d| serde_json::from_str(&d).ok()))
    }

    /// Everything in the outbox, for the UI.
    pub fn outbox(&self) -> Result<Vec<OutboxEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account_id, subject, envelope_to, send_after, state, attempts, last_error
             FROM outbox ORDER BY send_after, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OutboxEntry {
                id: r.get(0)?,
                account_id: r.get(1)?,
                subject: r.get(2)?,
                recipients: json_list(r, "envelope_to")?,
                send_after: r.get(4)?,
                state: r.get(5)?,
                attempts: r.get(6)?,
                last_error: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn message_count(&self, folder_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?)
    }

    // ----- brand logos ----------------------------------------------------

    /// Returns a cached brand logo (or negative miss) if it has not expired.
    pub fn get_brand_logo(&self, domain: &str, now_secs: i64) -> Result<Option<CachedLogo>> {
        struct Row {
            data_url: Option<String>,
            bg: Option<String>,
            padded: i64,
            scale: f64,
            fetched_at: i64,
        }
        let row = self
            .conn
            .query_row(
                "SELECT data_url, bg, padded, scale, fetched_at
                 FROM brand_logos WHERE domain = ?1",
                [domain],
                |r| {
                    Ok(Row {
                        data_url: r.get(0)?,
                        bg: r.get(1)?,
                        padded: r.get(2)?,
                        scale: r.get(3)?,
                        fetched_at: r.get(4)?,
                    })
                },
            )
            .optional()?;

        let Some(row) = row else {
            return Ok(None);
        };

        let age = now_secs.saturating_sub(row.fetched_at);
        match row.data_url {
            Some(url) if age <= LOGO_HIT_TTL_SECS => Ok(Some(CachedLogo::Hit(BrandLogo {
                data_url: url,
                bg: row.bg,
                padded: row.padded != 0,
                scale: row.scale as f32,
            }))),
            None if age <= LOGO_MISS_TTL_SECS => Ok(Some(CachedLogo::Miss)),
            _ => Ok(None),
        }
    }

    /// Stores a resolved brand logo (or negative miss when `logo` is `None`).
    pub fn save_brand_logo(
        &self,
        domain: &str,
        logo: Option<&BrandLogo>,
        now_secs: i64,
    ) -> Result<()> {
        match logo {
            Some(l) => {
                self.conn.execute(
                    "INSERT INTO brand_logos (domain, data_url, bg, padded, scale, fetched_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(domain) DO UPDATE SET
                       data_url = excluded.data_url,
                       bg = excluded.bg,
                       padded = excluded.padded,
                       scale = excluded.scale,
                       fetched_at = excluded.fetched_at",
                    params![
                        domain,
                        l.data_url,
                        l.bg,
                        l.padded as i64,
                        l.scale as f64,
                        now_secs,
                    ],
                )?;
            }
            None => {
                self.conn.execute(
                    "INSERT INTO brand_logos (domain, data_url, bg, padded, scale, fetched_at)
                     VALUES (?1, NULL, NULL, 1, 1.0, ?2)
                     ON CONFLICT(domain) DO UPDATE SET
                       data_url = NULL,
                       bg = NULL,
                       padded = 1,
                       scale = 1.0,
                       fetched_at = excluded.fetched_at",
                    params![domain, now_secs],
                )?;
            }
        }
        Ok(())
    }

    /// Clears all cached brand logos.
    pub fn clear_brand_logos(&self) -> Result<()> {
        self.conn.execute("DELETE FROM brand_logos", [])?;
        Ok(())
    }

    // ----- cases ----------------------------------------------------------

    /// Creates a new case and returns its full view.
    pub fn create_case(&self, new_case: &NewCase) -> Result<CaseView> {
        self.conn.execute(
            "INSERT INTO cases (title, counterparty, domain, kind, status, next_step,
                                due_at, due_label, amount_cents, currency, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                new_case.title,
                new_case.counterparty,
                new_case.domain,
                new_case.kind.to_string(),
                new_case.status.to_string(),
                new_case.next_step,
                new_case.due_at,
                new_case.due_label,
                new_case.amount_cents,
                new_case.currency,
                new_case.reason,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        Ok(self.get_case(id)?.expect("just-inserted case exists"))
    }

    /// Appends an event to an existing case and touches `cases.updated_at`.
    pub fn add_case_event(&self, case_id: i64, ev: &NewCaseEvent) -> Result<CaseEvent> {
        self.conn.execute(
            "INSERT INTO case_events (case_id, message_id, occurred_at, kind, label, phase,
                                      direction, confidence, manual, rfc_message_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                     (SELECT message_id FROM messages WHERE id = ?2))",
            params![
                case_id,
                ev.message_id,
                ev.occurred_at,
                ev.kind,
                ev.label,
                ev.phase.to_string(),
                ev.direction.map(|d| d.to_string()),
                ev.confidence,
                ev.manual,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.touch_case(case_id)?;
        Ok(self.conn.query_row(
            "SELECT * FROM case_events WHERE id = ?1",
            [id],
            case_event_from_row,
        )?)
    }

    /// Attaches an identifying key (order number, invoice number, thread root)
    /// to a case. A value the case already has is ignored, whatever its label.
    pub fn add_case_key(&self, case_id: i64, label: &str, value: &str) -> Result<()> {
        let normalized = normalize_key(value);
        if normalized.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO case_keys (case_id, label, value, normalized)
             SELECT ?1, ?2, ?3, ?4
             WHERE NOT EXISTS (SELECT 1 FROM case_keys WHERE case_id = ?1 AND normalized = ?4)",
            params![case_id, label, value.trim(), normalized],
        )?;
        Ok(())
    }

    /// Finds case ids that match a normalized identifying key (excluding rejected cases).
    pub fn find_cases_by_key(&self, key_value: &str) -> Result<Vec<i64>> {
        let normalized = normalize_key(key_value);
        if normalized.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT k.case_id
             FROM case_keys k
             JOIN cases c ON c.id = k.case_id
             WHERE k.normalized = ?1 AND c.status != 'rejected'
             ORDER BY c.updated_at DESC",
        )?;
        let rows = stmt.query_map([normalized], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Returns a single case with its events, keys, and linked message ids.
    pub fn get_case(&self, case_id: i64) -> Result<Option<CaseView>> {
        let Some(case) = self
            .conn
            .query_row(
                "SELECT * FROM cases WHERE id = ?1",
                [case_id],
                case_from_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        Ok(Some(self.hydrate_case(case)?))
    }

    /// Lists cases ordered by most recently updated. By default, rejected
    /// ("Dies ist kein Vorgang") cases are omitted.
    pub fn list_cases(&self, include_rejected: bool) -> Result<Vec<CaseView>> {
        let sql = if include_rejected {
            "SELECT * FROM cases ORDER BY updated_at DESC, id DESC"
        } else {
            "SELECT * FROM cases WHERE status != 'rejected' ORDER BY updated_at DESC, id DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let cases: Vec<Case> = stmt
            .query_map([], case_from_row)?
            .collect::<Result<_, _>>()?;
        cases.into_iter().map(|c| self.hydrate_case(c)).collect()
    }

    /// Updates the status of a case.
    pub fn set_case_status(&self, case_id: i64, status: CaseStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE cases SET status = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![status.to_string(), case_id],
        )?;
        Ok(())
    }

    /// Mails of a case that still exist, without the user's own replies
    /// (their Sent folder is not where the decision is read back).
    fn case_message_ids(&self, case_id: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT e.message_id FROM case_events e
             JOIN messages m ON m.id = e.message_id
             WHERE e.case_id = ?1 AND COALESCE(e.direction, 'in') != 'out'",
        )?;
        let rows = stmt.query_map([case_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Like [`Store::set_case_status`], for a decision the user made: marking
    /// a case done also tags its mails `$HalcyonDone`, any other status
    /// removes the tag, so other devices see the same state.
    pub fn set_case_status_by_user(&mut self, case_id: i64, status: CaseStatus) -> Result<()> {
        self.set_case_status(case_id, status)?;
        let ids = self.case_message_ids(case_id)?;
        self.queue_flag(
            &ids,
            crate::filing::KEYWORD_DONE,
            status == CaseStatus::Done,
        )?;
        Ok(())
    }

    /// Closes open cases whose mails all carry `$HalcyonDone`: the user
    /// settled them on another device. A new mail without the tag keeps
    /// the case open. Returns how many cases were closed.
    pub fn settle_cases_done_by_user(&self) -> Result<usize> {
        let tagged = "instr(m.flags, '\"$HalcyonDone\"') > 0";
        Ok(self.conn.execute(
            &format!(
                "UPDATE cases SET status = 'done', next_step = 'Von dir erledigt',
                                  updated_at = unixepoch()
                 WHERE status IN ('waiting', 'needs_you')
                   AND EXISTS (SELECT 1 FROM case_events e JOIN messages m ON m.id = e.message_id
                               WHERE e.case_id = cases.id AND COALESCE(e.direction, 'in') != 'out')
                   AND NOT EXISTS (SELECT 1 FROM case_events e JOIN messages m ON m.id = e.message_id
                                   WHERE e.case_id = cases.id AND COALESCE(e.direction, 'in') != 'out'
                                     AND NOT ({tagged}))"
            ),
            [],
        )?)
    }

    // ----- small settings -------------------------------------------------

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ----- filed mail -----------------------------------------------------

    /// Inbox mail dated `since` or later that the auto-read pass has not
    /// judged yet, oldest first.
    pub(crate) fn filing_candidates(
        &self,
        since: i64,
        limit: usize,
    ) -> Result<Vec<crate::filing::Candidate>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, COALESCE(m.date, 0), m.flags, m.dmarc_pass,
                    COALESCE(m.subject, ''), m.body_text, m.in_reply_to, m.refs,
                    m.message_id, m.source_id,
                    EXISTS (SELECT 1 FROM case_scan cs
                            WHERE cs.message_id = m.id AND cs.with_body = 1)
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE f.role = 'inbox' AND m.date >= ?1
               AND NOT EXISTS (SELECT 1 FROM filing_scan fs WHERE fs.message_id = m.id)
             ORDER BY m.date ASC, m.id ASC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![since, limit as i64], |r| {
            let flags: Vec<String> =
                serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default();
            let refs: Vec<String> =
                serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default();
            Ok(crate::filing::Candidate {
                id: r.get(0)?,
                date: r.get(1)?,
                unread: !flags.iter().any(|f| f == "\\Seen"),
                dmarc_pass: r.get(3)?,
                subject: r.get(4)?,
                body_text: r.get(5)?,
                is_reply: r.get::<_, Option<String>>(6)?.is_some() || !refs.is_empty(),
                rfc_message_id: r.get(8)?,
                source_id: r.get(9)?,
                linked: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub(crate) fn mark_filing_scanned(&self, message_id: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO filing_scan (message_id) VALUES (?1)",
            [message_id],
        )?;
        Ok(())
    }

    /// `(case kind, case status, event kind)` of every case event of a mail
    /// (any copy of it).
    pub(crate) fn case_links_of(
        &self,
        message_id: i64,
        rfc_message_id: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.kind, c.status, COALESCE(e.kind, '') FROM case_events e
             JOIN cases c ON c.id = e.case_id
             WHERE e.message_id = ?1 OR (?2 IS NOT NULL AND e.rfc_message_id = ?2)",
        )?;
        let rows = stmt.query_map(params![message_id, rfc_message_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether the ledger invoice made from this mail is settled, if the
    /// mail produced one.
    pub(crate) fn invoice_paid_of(&self, message_id: i64) -> Result<Option<bool>> {
        Ok(self
            .conn
            .query_row(
                "SELECT paid FROM invoices WHERE message_id = ?1 ORDER BY paid ASC LIMIT 1",
                [message_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Mode and "needs you" state of a mail's recurring source. An anomaly
    /// raised by this very mail counts as needing the user.
    pub(crate) fn source_of(
        &self,
        source_id: i64,
        message_id: i64,
    ) -> Result<Option<(SourceMode, bool)>> {
        let row = self
            .conn
            .query_row(
                "SELECT mode, anomaly_kind IS NOT NULL
                        AND (anomaly_needs_you != 0 OR anomaly_message_id = ?2)
                 FROM sources WHERE id = ?1",
                params![source_id, message_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(mode, needs_you)| Some((mode.parse().ok()?, needs_you))))
    }

    /// Marks a case as "Dies ist kein Vorgang" (`rejected`).
    pub fn dissolve_case(&self, case_id: i64) -> Result<()> {
        self.set_case_status(case_id, CaseStatus::Rejected)
    }

    /// Manually assigns a message to a case (e.g. via drag and drop), clearing
    /// any prior exclusion for `(case_id, message_id)`.
    pub fn assign_message_to_case(
        &self,
        case_id: i64,
        message_id: i64,
        label: &str,
        phase: CaseEventPhase,
    ) -> Result<CaseEvent> {
        self.conn.execute(
            "DELETE FROM case_exclusions WHERE case_id = ?1 AND message_id = ?2",
            params![case_id, message_id],
        )?;
        let occurred_at: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(date, unixepoch()) FROM messages WHERE id = ?1",
                [message_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            });
        self.add_case_event(
            case_id,
            &NewCaseEvent {
                message_id: Some(message_id),
                occurred_at,
                kind: None,
                label: label.to_string(),
                phase,
                direction: None,
                confidence: 1.0,
                manual: true,
            },
        )
    }

    /// Unlinks a message from a case ("Gehört nicht zum Vorgang") and records
    /// an exclusion so automatic linking will not re-add it.
    pub fn remove_message_from_case(&self, case_id: i64, message_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM case_events WHERE case_id = ?1 AND message_id = ?2",
            params![case_id, message_id],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO case_exclusions (case_id, message_id) VALUES (?1, ?2)",
            params![case_id, message_id],
        )?;
        self.touch_case(case_id)?;
        Ok(())
    }

    /// Whether a message was explicitly excluded from a case by the user.
    pub fn is_message_excluded(&self, case_id: i64, message_id: i64) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM case_exclusions WHERE case_id = ?1 AND message_id = ?2",
                params![case_id, message_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    // ----- linker support -------------------------------------------------

    /// Runs `f` in one transaction; rolled back if it fails.
    pub fn in_transaction<T>(&mut self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Mail since `since` the linker has not looked at yet, or looked at
    /// before its text was downloaded. Oldest first, so an order
    /// confirmation is seen before its shipping notice.
    ///
    /// Trash is included only for mail since `trash_since` whose text is
    /// there: a delivery notice deleted on another device still advances its
    /// order, but the linker must never create anything from it.
    pub fn case_scan_batch(
        &self,
        since: i64,
        trash_since: i64,
        limit: usize,
    ) -> Result<Vec<ScanMessage>> {
        let sql = format!(
            "{SCAN_SELECT}
             LEFT JOIN case_scan s ON s.message_id = m.id
             WHERE m.date >= ?1
               AND (s.message_id IS NULL OR (s.with_body = 0 AND m.body_text IS NOT NULL))
               AND COALESCE(f.role, '') NOT IN ('junk', 'drafts')
               AND (COALESCE(f.role, '') != 'trash'
                    OR (m.date >= ?3 AND m.body_text IS NOT NULL))
             ORDER BY m.date ASC, m.id ASC
             LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, limit as i64, trash_since], scan_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn mark_case_scanned(&self, message_id: i64, with_body: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO case_scan (message_id, with_body) VALUES (?1, ?2)
             ON CONFLICT (message_id) DO UPDATE SET with_body = excluded.with_body",
            params![message_id, with_body],
        )?;
        Ok(())
    }

    /// Stored mail with one of these `Message-ID`s (outside junk and trash).
    pub fn scan_messages_by_rfc_ids(&self, ids: &[String]) -> Result<Vec<ScanMessage>> {
        let sql = format!(
            "{SCAN_SELECT}
             WHERE m.message_id = ?1
               AND COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts')
             LIMIT 1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            if let Some(m) = stmt.query_row([id], scan_from_row).optional()? {
                out.push(m);
            }
        }
        Ok(out)
    }

    /// The case a mail with this `Message-ID` already belongs to, from any
    /// of its copies.
    pub fn case_of_rfc_message(&self, rfc_message_id: &str) -> Result<Option<i64>> {
        if rfc_message_id.is_empty() {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT case_id FROM case_events
                 WHERE rfc_message_id = ?1
                 LIMIT 1",
                [rfc_message_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Most recently updated case of `kind` that has one of the normalised
    /// keys. Rejected cases count too, so a mail of a case the user
    /// dismissed does not start it again.
    pub fn case_for_keys(&self, kind: CaseKind, normalized: &[String]) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT c.id FROM case_keys k JOIN cases c ON c.id = k.case_id
             WHERE k.normalized = ?1 AND c.kind = ?2
             ORDER BY c.updated_at DESC, c.id DESC LIMIT 1",
        )?;
        let kind = kind.to_string();
        for key in normalized {
            if key.is_empty() {
                continue;
            }
            if let Some(id) = stmt
                .query_row(params![key, kind], |r| r.get::<_, i64>(0))
                .optional()?
            {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Sets the derived status and next step. Leaves cases the user
    /// rejected alone.
    pub fn update_case_state(
        &self,
        case_id: i64,
        status: CaseStatus,
        next_step: &str,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE cases SET status = ?1, next_step = ?2
             WHERE id = ?3 AND status != 'rejected'",
            params![status.to_string(), next_step, case_id],
        )?;
        Ok(())
    }

    /// Updates optional financial and deadline metadata on a case without
    /// overwriting existing values with `None`.
    pub fn update_case_financials(
        &self,
        case_id: i64,
        amount_cents: Option<i64>,
        currency: Option<&str>,
        due_at: Option<i64>,
        due_label: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE cases
             SET amount_cents = COALESCE(?1, amount_cents),
                 currency     = COALESCE(?2, currency),
                 due_at       = COALESCE(?3, due_at),
                 due_label    = COALESCE(?4, due_label),
                 updated_at   = unixepoch()
             WHERE id = ?5 AND status != 'rejected'",
            params![amount_cents, currency, due_at, due_label, case_id],
        )?;
        Ok(())
    }

    /// Closes open cases whose last event is older than `before`.
    pub fn close_stale_cases(&self, before: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE cases SET status = 'done', next_step = 'Ohne Neuigkeiten abgeschlossen',
                              updated_at = unixepoch()
             WHERE status IN ('waiting', 'needs_you')
               AND COALESCE((SELECT max(occurred_at) FROM case_events e WHERE e.case_id = cases.id),
                            cases.created_at) < ?1",
            [before],
        )?)
    }

    /// Your own replies (in a Sent folder) to any of these `Message-ID`s, so
    /// a list of incoming mail can show whole threads.
    pub fn sent_replies_to(&self, rfc_ids: &[String]) -> Result<Vec<MessageSummary>> {
        let mut out = Vec::new();
        for chunk in rfc_ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "{SUMMARY_SELECT}
                 WHERE f.role = 'sent' AND m.in_reply_to IN ({marks})
                 ORDER BY m.date ASC"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk), summary_from_row)?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }

    /// Lowercased recipient addresses of messages sent by the user, used to
    /// distinguish real human conversation partners from automated sources.
    pub fn sent_recipient_addresses(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.to_addrs, m.cc_addrs
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE f.role = 'sent'",
        )?;
        let mut out = std::collections::HashSet::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (to_raw, cc_raw) = row?;
            let to_vec: Vec<String> = serde_json::from_str(&to_raw).unwrap_or_default();
            let cc_vec: Vec<String> = serde_json::from_str(&cc_raw).unwrap_or_default();
            for addr in to_vec.into_iter().chain(cc_vec) {
                let lower = addr.trim().to_ascii_lowercase();
                if !lower.is_empty() {
                    out.insert(lower);
                }
            }
        }
        Ok(out)
    }

    fn touch_case(&self, case_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE cases SET updated_at = unixepoch() WHERE id = ?1",
            [case_id],
        )?;
        Ok(())
    }

    fn hydrate_case(&self, case: Case) -> Result<CaseView> {
        let mut ev_stmt = self.conn.prepare(
            "SELECT * FROM case_events WHERE case_id = ?1 ORDER BY occurred_at ASC, id ASC",
        )?;
        let events: Vec<CaseEvent> = ev_stmt
            .query_map([case.id], case_event_from_row)?
            .collect::<Result<_, _>>()?;

        let mut key_stmt = self.conn.prepare(
            "SELECT label, value, normalized FROM case_keys WHERE case_id = ?1 ORDER BY id ASC",
        )?;
        let keys: Vec<CaseKey> = key_stmt
            .query_map([case.id], case_key_from_row)?
            .collect::<Result<_, _>>()?;

        let mut message_ids = Vec::new();
        for ev in &events {
            if let Some(mid) = ev.message_id
                && !message_ids.contains(&mid)
            {
                message_ids.push(mid);
            }
        }

        Ok(CaseView {
            case,
            events,
            keys,
            message_ids,
        })
    }

    // ----- remote image trust ---------------------------------------------

    /// Whether remote images should be loaded automatically for `sender`.
    pub fn is_remote_sender_trusted(&self, sender: &str) -> Result<bool> {
        let norm = sender.trim().to_ascii_lowercase();
        if norm.is_empty() {
            return Ok(false);
        }
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM trusted_remote_senders WHERE sender = ?1)",
            [norm],
            |r| r.get(0),
        )?)
    }

    /// Always loads remote images for `sender` in the future.
    pub fn trust_remote_sender(&self, sender: &str) -> Result<()> {
        let norm = sender.trim().to_ascii_lowercase();
        if norm.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO trusted_remote_senders (sender) VALUES (?1)",
            [norm],
        )?;
        Ok(())
    }

    /// Revokes automatic remote image loading for `sender`.
    pub fn untrust_remote_sender(&self, sender: &str) -> Result<()> {
        let norm = sender.trim().to_ascii_lowercase();
        self.conn.execute(
            "DELETE FROM trusted_remote_senders WHERE sender = ?1",
            [norm],
        )?;
        Ok(())
    }

    // ----- sources ("Quellen") & evaluation -------------------------------

    /// Records a message under its recurring source (`source_key`) and
    /// normalized subject `pattern`, updating source statistics, suggested
    /// display mode, and anomaly detection ("Anders als sonst").
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    pub fn record_source_message(
        &self,
        m: &ScanMessage,
        source_key: &str,
        display_name: &str,
        domain: Option<&str>,
        pattern: &str,
        amount_cents: Option<i64>,
        currency: Option<&str>,
        is_status_update: bool,
    ) -> Result<i64> {
        let is_newsletter = m.list_id.is_some() || m.has_unsubscribe;
        // Check if this message was already counted for a source (e.g. body arrived later).
        let existing_source_id: Option<i64> = self
            .conn
            .query_row(
                "SELECT source_id FROM messages WHERE id = ?1",
                [m.id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();

        if let Some(sid) = existing_source_id {
            // If an amount was newly extracted from the body, update running amount stats
            // (for non-newsletter, non-order-status recurring sources only).
            if !is_newsletter
                && !is_status_update
                && let Some(cents) = amount_cents
                && cents > 0
            {
                self.update_source_amount(sid, m, pattern, cents, currency)?;
            }
            return Ok(sid);
        }

        self.conn.execute(
            "INSERT INTO sources (source_key, display_name, domain, is_newsletter,
                                  first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT (source_key) DO UPDATE SET
                 display_name  = CASE WHEN sources.display_name = '' THEN excluded.display_name ELSE sources.display_name END,
                 domain        = COALESCE(sources.domain, excluded.domain),
                 is_newsletter = MAX(sources.is_newsletter, excluded.is_newsletter)",
            params![source_key, display_name, domain, is_newsletter as i64, m.date],
        )?;

        let (
            source_id,
            total_messages,
            read_messages,
            dmarc_pass_count,
            user_mode,
            last_seen_at,
            median_interval_secs,
            median_amount_cents,
            amount_sample_count,
            stored_newsletter,
        ): (
            i64,
            u32,
            u32,
            u32,
            bool,
            i64,
            Option<i64>,
            Option<i64>,
            u32,
            bool,
        ) = self.conn.query_row(
            "SELECT id, total_messages, read_messages, dmarc_pass_count, user_mode != 0,
                    last_seen_at, median_interval_secs, median_amount_cents,
                    amount_sample_count, is_newsletter != 0
             FROM sources WHERE source_key = ?1",
            [source_key],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )?;

        let known_patterns: Vec<String> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT pattern FROM source_patterns WHERE source_id = ?1 ORDER BY message_count DESC LIMIT 10",
            )?;
            stmt.query_map([source_id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };

        let recent_hour_count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE source_id = ?1 AND date >= ?2 AND date <= ?3",
            params![source_id, m.date - 3600, m.date],
            |r| r.get(0),
        )?;

        let effective_amount = if stored_newsletter || is_newsletter || is_status_update {
            None
        } else {
            amount_cents.filter(|&c| c > 0)
        };

        let stats = SourceStats {
            total_messages,
            dmarc_pass_count,
            known_patterns,
            median_interval_secs,
            last_seen_at: (total_messages > 0).then_some(last_seen_at),
            recent_hour_count: recent_hour_count + 1,
            median_amount_cents,
            amount_sample_count,
        };
        let incoming = IncomingSourceMail {
            occurred_at: m.date,
            normalized_pattern: pattern.to_string(),
            dmarc_pass: m.dmarc_pass,
            amount_cents: effective_amount,
            currency: currency.map(str::to_string),
        };
        let anomaly = detect_anomaly(&stats, &incoming);

        let new_total = total_messages + 1;
        let new_read = read_messages + u32::from(m.is_seen);
        let new_dmarc = dmarc_pass_count + u32::from(m.dmarc_pass);
        let new_interval = if total_messages > 0 && m.date > last_seen_at {
            let delta = m.date - last_seen_at;
            Some(match median_interval_secs {
                Some(prev) => (prev * 3 + delta) / 4,
                None => delta,
            })
        } else {
            median_interval_secs
        };
        let (new_median_amount, new_amount_samples) = match effective_amount {
            Some(c) => (
                Some(match median_amount_cents {
                    Some(prev) => (prev * 3 + c) / 4,
                    None => c,
                }),
                amount_sample_count + 1,
            ),
            None => (median_amount_cents, amount_sample_count),
        };
        let is_newest = total_messages == 0 || m.date >= last_seen_at;
        let new_last_seen = m.date.max(last_seen_at);
        let behavior = self
            .sender_behavior(m.from_addr.as_deref().unwrap_or(""))
            .unwrap_or_default();
        let suggested = suggest_source_mode_with_behavior(
            new_total,
            new_read,
            stored_newsletter || is_newsletter,
            is_status_update,
            behavior,
        );

        self.conn.execute(
            "UPDATE sources
             SET total_messages       = ?1,
                 read_messages        = ?2,
                 dmarc_pass_count     = ?3,
                 last_seen_at         = ?4,
                 median_interval_secs = ?5,
                 median_amount_cents  = ?6,
                 amount_sample_count  = ?7,
                 mode                 = CASE WHEN user_mode = 0 THEN ?8 ELSE mode END,
                 anomaly_kind         = CASE WHEN ?14 != 0 THEN ?9 ELSE COALESCE(?9, anomaly_kind) END,
                 anomaly_needs_you    = CASE WHEN ?14 != 0 THEN ?10 WHEN ?9 IS NOT NULL THEN ?10 ELSE anomaly_needs_you END,
                 anomaly_reason       = CASE WHEN ?14 != 0 THEN ?11 ELSE COALESCE(?11, anomaly_reason) END,
                 anomaly_message_id   = CASE WHEN ?14 != 0 THEN (CASE WHEN ?9 IS NOT NULL THEN ?12 ELSE NULL END) WHEN ?9 IS NOT NULL THEN ?12 ELSE anomaly_message_id END
             WHERE id = ?13",
            params![
                new_total,
                new_read,
                new_dmarc,
                new_last_seen,
                new_interval,
                new_median_amount,
                new_amount_samples,
                suggested.to_string(),
                anomaly.as_ref().map(|a| a.kind.to_string()),
                anomaly.as_ref().is_some_and(|a| a.needs_you) as i64,
                anomaly.as_ref().map(|a| a.reason.as_str()),
                m.id,
                source_id,
                is_newest as i64,
            ],
        )?;
        let _ = user_mode;

        self.conn.execute(
            "INSERT INTO source_patterns (source_id, pattern, message_count, last_seen_at)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (source_id, pattern) DO UPDATE SET
                 message_count = source_patterns.message_count + 1,
                 last_seen_at  = MAX(source_patterns.last_seen_at, excluded.last_seen_at)",
            params![source_id, pattern, m.date],
        )?;

        self.conn.execute(
            "UPDATE messages SET source_id = ?1, subject_pattern = ?2 WHERE id = ?3",
            params![source_id, pattern, m.id],
        )?;

        Ok(source_id)
    }

    fn update_source_amount(
        &self,
        source_id: i64,
        m: &ScanMessage,
        pattern: &str,
        cents: i64,
        currency: Option<&str>,
    ) -> Result<()> {
        let (total_messages, dmarc_pass_count, median_amount_cents, amount_sample_count): (
            u32,
            u32,
            Option<i64>,
            u32,
        ) = self.conn.query_row(
            "SELECT total_messages, dmarc_pass_count, median_amount_cents, amount_sample_count
             FROM sources WHERE id = ?1",
            [source_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let stats = SourceStats {
            total_messages,
            dmarc_pass_count,
            median_amount_cents,
            amount_sample_count,
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: m.date,
            normalized_pattern: pattern.to_string(),
            dmarc_pass: m.dmarc_pass,
            amount_cents: Some(cents),
            currency: currency.map(str::to_string),
        };
        let anomaly = detect_anomaly(&stats, &incoming);
        let new_median = match median_amount_cents {
            Some(prev) => (prev * 3 + cents) / 4,
            None => cents,
        };
        self.conn.execute(
            "UPDATE sources
             SET median_amount_cents = ?1,
                 amount_sample_count = amount_sample_count + 1,
                 anomaly_kind        = COALESCE(?2, anomaly_kind),
                 anomaly_needs_you   = CASE WHEN ?2 IS NOT NULL THEN ?3 ELSE anomaly_needs_you END,
                 anomaly_reason      = COALESCE(?4, anomaly_reason),
                 anomaly_message_id  = CASE WHEN ?2 IS NOT NULL THEN ?5 ELSE anomaly_message_id END
             WHERE id = ?6",
            params![
                new_median,
                anomaly.as_ref().map(|a| a.kind.to_string()),
                anomaly.as_ref().is_some_and(|a| a.needs_you) as i64,
                anomaly.as_ref().map(|a| a.reason.as_str()),
                m.id,
                source_id,
            ],
        )?;
        Ok(())
    }

    /// Lists recurring sources with DMARC pass ordered by recency descending.
    pub fn list_sources(&self, limit: usize) -> Result<Vec<SourceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_key, display_name, domain, mode, user_mode != 0,
                    total_messages, read_messages, dmarc_pass_count, is_newsletter != 0,
                    first_seen_at, last_seen_at, median_interval_secs, median_amount_cents,
                    amount_sample_count, anomaly_kind, anomaly_needs_you != 0, anomaly_reason,
                    (SELECT m.id FROM messages m WHERE m.source_id = sources.id ORDER BY m.date DESC LIMIT 1)
             FROM sources
             WHERE dmarc_pass_count > 0
             ORDER BY last_seen_at DESC, total_messages DESC
             LIMIT ?1",
        )?;
        let mut pat_stmt = self.conn.prepare(
            "SELECT pattern FROM source_patterns
             WHERE source_id = ?1
             ORDER BY message_count DESC, last_seen_at DESC
             LIMIT 5",
        )?;
        let max_date: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(date), 0) FROM messages", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let anomaly_cutoff = (max_date - 14 * 86_400).max(0);
        let rows = stmt.query_map([limit as i64], |r| {
            let id: i64 = r.get(0)?;
            let last_seen_at: i64 = r.get(11)?;
            let anomaly_kind: Option<String> = r.get(15)?;
            let anomaly_needs_you: bool = r.get(16)?;
            let anomaly_reason: Option<String> = r.get(17)?;
            let latest_message_id: Option<i64> = r.get(18)?;
            let anomaly = if last_seen_at >= anomaly_cutoff {
                match (anomaly_kind.as_deref(), anomaly_reason) {
                    (Some(k), Some(reason)) => {
                        k.parse::<AnomalyKind>().ok().map(|kind| SourceAnomaly {
                            kind,
                            needs_you: anomaly_needs_you,
                            reason,
                        })
                    }
                    _ => None,
                }
            } else {
                None
            };
            Ok(SourceRecord {
                id,
                source_key: r.get(1)?,
                display_name: r.get(2)?,
                domain: r.get(3)?,
                mode: parse_col::<SourceMode>(r, "mode")?,
                user_mode: r.get(5)?,
                total_messages: r.get(6)?,
                read_messages: r.get(7)?,
                dmarc_pass_count: r.get(8)?,
                is_newsletter: r.get(9)?,
                first_seen_at: r.get(10)?,
                last_seen_at: r.get(11)?,
                median_interval_secs: r.get(12)?,
                median_amount_cents: r.get(13)?,
                amount_sample_count: r.get(14)?,
                anomaly,
                patterns: Vec::new(),
                latest_message_id,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            let mut rec = row?;
            rec.patterns = pat_stmt
                .query_map([rec.id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            out.push(rec);
        }
        Ok(out)
    }

    /// Mode of the source with this key and whether it has a recent anomaly
    /// that needs you. Used right after sync, before the linker has filed
    /// the new mail, so only what is known from earlier mail counts.
    pub fn source_notify_state(
        &self,
        source_key: &str,
        now: i64,
    ) -> Result<Option<(SourceMode, bool)>> {
        // Same 14-day horizon as `list_sources`, so an old unresolved
        // anomaly does not keep a quiet source loud forever.
        let row = self
            .conn
            .query_row(
                "SELECT mode, anomaly_kind IS NOT NULL AND anomaly_needs_you != 0
                        AND last_seen_at >= ?2
                 FROM sources WHERE source_key = ?1",
                params![source_key, now - 14 * 86_400],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(mode, needs_you)| Some((mode.parse().ok()?, needs_you))))
    }

    /// Sets the user's explicit display mode for a source (`user_mode = 1`).
    pub fn set_source_mode(&self, source_id: i64, mode: SourceMode) -> Result<()> {
        self.conn.execute(
            "UPDATE sources SET mode = ?1, user_mode = 1 WHERE id = ?2",
            params![mode.to_string(), source_id],
        )?;
        Ok(())
    }

    /// Clears an active anomaly banner for a source after user acknowledgement.
    pub fn dismiss_source_anomaly(&self, source_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sources SET anomaly_kind = NULL, anomaly_needs_you = 0, anomaly_reason = NULL WHERE id = ?1",
            [source_id],
        )?;
        Ok(())
    }

    /// Removes a recurring source and moves its associated messages to the given `role`
    /// (e.g. `FolderRole::Trash` or `FolderRole::Junk`), creating sync pending ops.
    pub fn remove_source_and_messages(
        &mut self,
        source_id: i64,
        role: FolderRole,
    ) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;

        let source_key: Option<String> = tx
            .query_row(
                "SELECT source_key FROM sources WHERE id = ?1",
                [source_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(source_key) = source_key else {
            return Ok(Vec::new());
        };

        let mut id_set = std::collections::HashSet::new();

        {
            let mut stmt = tx.prepare("SELECT id FROM messages WHERE source_id = ?1")?;
            let rows = stmt.query_map([source_id], |r| r.get(0))?;
            for id in rows {
                id_set.insert(id?);
            }
        }

        if let Some(addr) = source_key.strip_prefix("addr:") {
            let addr_lower = addr.to_ascii_lowercase();
            let mut stmt = tx.prepare(
                "SELECT m.id FROM messages m
                 JOIN folders f ON f.id = m.folder_id
                 WHERE (f.role IS NULL OR f.role NOT IN ('trash', 'junk'))
                   AND LOWER(m.from_addr) = ?1",
            )?;
            let rows = stmt.query_map([addr_lower], |r| r.get(0))?;
            for id in rows {
                id_set.insert(id?);
            }
        } else if let Some(list_id) = source_key.strip_prefix("list:") {
            let list_lower = list_id.to_ascii_lowercase();
            let bracketed = format!("<{list_lower}>");
            let mut stmt = tx.prepare(
                "SELECT m.id FROM messages m
                 JOIN folders f ON f.id = m.folder_id
                 WHERE (f.role IS NULL OR f.role NOT IN ('trash', 'junk'))
                   AND (LOWER(m.list_id) = ?1 OR LOWER(m.list_id) = ?2)",
            )?;
            let rows = stmt.query_map(params![list_lower, bracketed], |r| r.get(0))?;
            for id in rows {
                id_set.insert(id?);
            }
        }

        let mut message_ids: Vec<i64> = id_set.into_iter().collect();
        message_ids.sort_unstable();

        let mut affected = Vec::new();
        for id in message_ids {
            let row: Option<(i64, String, u32, Option<String>)> = tx
                .query_row(
                    "SELECT f.account_id, f.path, m.uid, f.role FROM messages m
                     JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((account_id, path, uid, current_role)) = row else {
                continue;
            };

            let (op, arg) = if current_role.as_deref() == Some(role.as_str()) {
                ("expunge", None)
            } else {
                let target_folder: Option<String> = tx
                    .query_row(
                        "SELECT path FROM folders WHERE account_id = ?1 AND role = ?2 LIMIT 1",
                        params![account_id, role.as_str()],
                        |r| r.get(0),
                    )
                    .optional()?;
                let target_folder = match target_folder {
                    Some(p) => Some(p),
                    None if role == FolderRole::Junk => tx
                        .query_row(
                            "SELECT path FROM folders WHERE account_id = ?1 AND role = 'trash' LIMIT 1",
                            [account_id],
                            |r| r.get(0),
                        )
                        .optional()?,
                    None => None,
                };
                match target_folder {
                    Some(t) => ("move", Some(t)),
                    None => {
                        return Err(crate::Error::Other(format!(
                            "this account has no {} folder",
                            role.as_str()
                        )));
                    }
                }
            };

            tx.execute(
                "INSERT INTO pending_ops (account_id, message_id, folder_path, uid, op, arg)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
                params![account_id, path, uid, op, arg],
            )?;
            tx.execute("DELETE FROM pending_ops WHERE message_id = ?1", [id])?;
            tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            affected.push(id);
        }

        tx.execute(
            "DELETE FROM source_patterns WHERE source_id = ?1",
            [source_id],
        )?;
        tx.execute(
            "UPDATE messages SET source_id = NULL WHERE source_id = ?1",
            [source_id],
        )?;
        tx.execute("DELETE FROM sources WHERE id = ?1", [source_id])?;

        tx.commit()?;
        Ok(affected)
    }

    fn is_turn_dismissed(&self, rfc_message_id: Option<&str>) -> Result<bool> {
        let Some(rfc) = rfc_message_id.filter(|s| !s.is_empty()) else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM turn_dismissals WHERE rfc_message_id = ?1",
                [rfc],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Marks the conversation of this mail as settled ("Heute" and the turn
    /// badges then show it as done). Only this exact mail is dismissed.
    ///
    /// The decision also travels to other devices as the `$HalcyonDone`
    /// keyword (where the folder allows keywords).
    pub fn dismiss_turn(&mut self, message_id: i64) -> Result<()> {
        let (rfc, from): (Option<String>, Option<String>) = self
            .conn
            .query_row(
                "SELECT message_id, from_addr FROM messages WHERE id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| crate::Error::Other("message not found".into()))?;
        let rfc = rfc
            .filter(|s| !s.is_empty())
            .ok_or_else(|| crate::Error::Other("message has no Message-ID".into()))?;
        self.conn.execute(
            "INSERT OR IGNORE INTO turn_dismissals (rfc_message_id, from_addr) VALUES (?1, ?2)",
            params![rfc, from.map(|f| f.to_ascii_lowercase())],
        )?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        record_interaction_conn(&self.conn, message_id, Interaction::Ignore, now)?;
        self.queue_flag(&[message_id], crate::filing::KEYWORD_DONE, true)?;
        Ok(())
    }

    /// Undoes [`Store::dismiss_turn`].
    pub fn restore_turn(&mut self, message_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM turn_dismissals WHERE rfc_message_id =
                 (SELECT message_id FROM messages WHERE id = ?1)",
            [message_id],
        )?;
        self.queue_flag(&[message_id], crate::filing::KEYWORD_DONE, false)?;
        Ok(())
    }

    /// Evaluates conversation turn status ("Wer ist am Zug?") for a batch of message IDs.
    pub fn assess_messages_turns(
        &self,
        message_ids: &[i64],
        now: i64,
    ) -> Result<Vec<(i64, crate::turn::TurnAssessment)>> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }
        let own: Vec<String> = self
            .accounts()?
            .into_iter()
            .map(|a| a.email.to_ascii_lowercase())
            .collect();
        // Once per call: the sent folder can hold thousands of recipients.
        let correspondents = self.sent_recipient_addresses()?;
        let answered = self.answered_message_ids()?;
        let mut out = Vec::with_capacity(message_ids.len());
        for &id in message_ids.iter().take(500) {
            if let Some(msg) = self.message(id)? {
                // A mail is judged on its own, so without this an answered
                // question would keep asking for an answer.
                let from_me = msg
                    .header
                    .from_addr
                    .as_deref()
                    .is_some_and(|a| own.contains(&a.trim().to_ascii_lowercase()));
                let answered_at = msg
                    .header
                    .message_id
                    .as_deref()
                    .and_then(|rfc| answered.get(rfc).copied());
                // `\\Answered` is set by the replying client on the original,
                // so it also covers replies whose sent copy never reached us.
                let flagged = msg.header.flags.iter().any(|f| f == "\\Answered");
                if !from_me
                    && msg.folder_role != Some(FolderRole::Sent)
                    && (flagged || answered_at.is_some_and(|at| at >= msg.header.date.unwrap_or(0)))
                {
                    out.push((
                        id,
                        crate::turn::TurnAssessment {
                            status: crate::turn::TurnStatus::Done,
                            confidence: 0.95,
                            reason: "Du hast schon geantwortet".into(),
                        },
                    ));
                    continue;
                }
                // `$HalcyonDone` from the server is the same decision made on
                // another device.
                if msg
                    .header
                    .flags
                    .iter()
                    .any(|f| f == crate::filing::KEYWORD_DONE)
                    || self.is_turn_dismissed(msg.header.message_id.as_deref())?
                {
                    out.push((
                        id,
                        crate::turn::TurnAssessment {
                            status: crate::turn::TurnStatus::Done,
                            confidence: 1.0,
                            reason: "Von dir als erledigt entfernt".into(),
                        },
                    ));
                    continue;
                }
                let (body, source_total): (String, Option<i64>) = self
                    .conn
                    .query_row(
                        "SELECT COALESCE(m.body_text, m.preview, ''), s.total_messages
                         FROM messages m LEFT JOIN sources s ON s.id = m.source_id
                         WHERE m.id = ?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap_or_default();
                let from = msg
                    .header
                    .from_addr
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let is_reply =
                    msg.header.in_reply_to.is_some() || !msg.header.references.is_empty();
                let behavior = self.sender_behavior(&from).unwrap_or_default();
                let context = crate::turn::TurnContext {
                    known_correspondent: is_reply || correspondents.contains(&from),
                    recurring_source: source_total
                        .is_some_and(|n| n >= crate::turn::RECURRING_SOURCE_MIN_MESSAGES),
                    behavior,
                };
                let turn = crate::turn::assess_turn_with_context(
                    &msg.header,
                    &body,
                    &own,
                    now,
                    Some(context),
                );
                out.push((id, turn));
            }
        }
        Ok(out)
    }

    /// `Message-ID`s the user has replied to, with the date of the latest
    /// reply. Recorded when a sent mail is stored and kept after that mail
    /// is gone; every mail of the thread up to the reply counts as answered.
    fn answered_message_ids(&self) -> Result<std::collections::HashMap<String, i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT rfc_message_id, answered_at FROM answered_mails")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Records a user interaction (open, reply, delete, ignore) for learning.
    /// Deduplicated per `(message_id, action)` so re-opening the same mail
    /// does not skew sender statistics.
    pub fn record_interaction(&self, message_id: i64, action: Interaction, now: i64) -> Result<()> {
        record_interaction_conn(&self.conn, message_id, action, now)
    }

    /// Learned interaction counts for `from_addr`.
    pub fn sender_behavior(&self, from_addr: &str) -> Result<SenderBehavior> {
        let addr = from_addr.trim().to_ascii_lowercase();
        if addr.is_empty() {
            return Ok(SenderBehavior::default());
        }
        let row = self
            .conn
            .query_row(
                "SELECT opened_count, replied_count, deleted_count, ignored_count
                 FROM sender_behavior WHERE from_addr = ?1",
                [&addr],
                |r| {
                    Ok(SenderBehavior {
                        opened: r.get::<_, i64>(0)?.max(0) as u32,
                        replied: r.get::<_, i64>(1)?.max(0) as u32,
                        deleted: r.get::<_, i64>(2)?.max(0) as u32,
                        ignored: r.get::<_, i64>(3)?.max(0) as u32,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    /// Computes privacy-preserving numerical metrics for "Wer ist am Zug?"
    /// (sources, cases, human conversations, and "Heute" compression ratio).
    pub fn evaluate_mailbox_stats(&self, now: i64) -> Result<MailboxEvalStats> {
        let total_messages = self.conn.query_row(
            "SELECT COUNT(*) FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts')",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;

        let unique_sources = self
            .conn
            .query_row("SELECT COUNT(*) FROM sources", [], |r| r.get::<_, i64>(0))?
            as usize;
        let unique_patterns =
            self.conn
                .query_row("SELECT COUNT(*) FROM source_patterns", [], |r| {
                    r.get::<_, i64>(0)
                })? as usize;
        let recurring_pattern_messages = self.conn.query_row(
            "SELECT COALESCE(SUM(message_count), 0) FROM source_patterns WHERE message_count >= 2",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let top_100_patterns_messages = self.conn.query_row(
            "SELECT COALESCE(SUM(cnt), 0) FROM (
                 SELECT message_count AS cnt FROM source_patterns
                 ORDER BY message_count DESC LIMIT 100
             )",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;

        let mut stats = MailboxEvalStats {
            total_messages,
            unique_sources,
            unique_patterns,
            recurring_pattern_messages,
            top_100_patterns_messages,
            ..Default::default()
        };

        let mut mode_stmt = self
            .conn
            .prepare("SELECT mode, COUNT(*) FROM sources GROUP BY mode")?;
        for row in mode_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (mode, count) = row?;
            match mode.as_str() {
                "show" => stats.sources_show = count as usize,
                "bundle" => stats.sources_bundle = count as usize,
                "latest_only" => stats.sources_latest_only = count as usize,
                "mute" => stats.sources_mute = count as usize,
                _ => {}
            }
        }

        stats.active_anomalies = self.conn.query_row(
            "SELECT COUNT(*) FROM sources WHERE anomaly_kind IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;

        for c in self.list_cases(false)? {
            match c.case.status {
                CaseStatus::NeedsYou => stats.cases_needs_you += 1,
                CaseStatus::Waiting => stats.cases_waiting += 1,
                CaseStatus::Done => stats.cases_done += 1,
                _ => {}
            }
            match c.case.kind {
                CaseKind::Order => stats.cases_order += 1,
                CaseKind::Invoice => stats.cases_invoice += 1,
                CaseKind::Contract => stats.cases_contract += 1,
                _ => {}
            }
        }

        // Evaluate human conversation threads (messages with in_reply_to or sent replies).
        let own: Vec<String> = self
            .accounts()?
            .into_iter()
            .map(|a| a.email.to_ascii_lowercase())
            .collect();
        let mut thread_stmt = self.conn.prepare(&format!(
            "{SUMMARY_SELECT}
             WHERE COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts')
               AND m.in_reply_to IS NOT NULL AND m.in_reply_to != ''
             ORDER BY m.date DESC
             LIMIT 500"
        ))?;
        let reply_msgs: Vec<MessageSummary> = thread_stmt
            .query_map([], summary_from_row)?
            .collect::<std::result::Result<_, _>>()?;
        let mut seen_threads = std::collections::HashSet::new();
        for m in reply_msgs {
            let root = m
                .header
                .references
                .first()
                .or(m.header.in_reply_to.as_ref())
                .cloned()
                .unwrap_or_default();
            if !root.is_empty() && !seen_threads.insert(root) {
                continue;
            }
            let body: String = self
                .conn
                .query_row(
                    "SELECT COALESCE(body_text, preview, '') FROM messages WHERE id = ?1",
                    [m.id],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            let turn = assess_turn(&m.header, &body, &own, now);
            stats.human_threads_total += 1;
            match turn.status {
                TurnStatus::NeedsYou => stats.human_needs_you += 1,
                TurnStatus::Waiting { follow_up_due } => {
                    stats.human_waiting += 1;
                    if follow_up_due {
                        stats.human_follow_up_due += 1;
                    }
                }
                TurnStatus::ReadingAlong => stats.human_reading_along += 1,
                TurnStatus::Done => stats.human_done += 1,
            }
        }

        let seven_days_ago = now - 7 * 86_400;
        stats.recent_7d_incoming = self.conn.query_row(
            "SELECT COUNT(*) FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.date >= ?1
               AND COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts', 'sent')",
            [seven_days_ago],
            |r| r.get::<_, i64>(0),
        )? as usize;

        // "Heute" rows: active cases + human conversations (NeedsYou + Waiting) + active anomalies + 1 summary line for bundled/muted sources + unbundled individual messages in 7d.
        let unbundled_7d: usize = self.conn.query_row(
            "SELECT COUNT(DISTINCT COALESCE(m.source_id, -m.id))
             FROM messages m
             JOIN folders f ON f.id = m.folder_id
             LEFT JOIN sources s ON s.id = m.source_id
             WHERE m.date >= ?1
               AND COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts', 'sent')
               AND COALESCE(s.mode, 'show') IN ('show', 'latest_only')
               AND NOT EXISTS (SELECT 1 FROM case_events ce WHERE ce.message_id = m.id)",
            [seven_days_ago],
            |r| r.get::<_, i64>(0),
        )? as usize;
        stats.recent_7d_today_rows = stats.cases_needs_you
            + stats.cases_waiting
            + stats.human_needs_you
            + stats.human_waiting
            + stats.active_anomalies
            + unbundled_7d
            + usize::from(stats.sources_bundle + stats.sources_mute > 0);

        Ok(stats)
    }
}

// Lists the columns instead of `m.*` so lists never load the full body text.
const SUMMARY_SELECT: &str = "
    SELECT m.id, m.folder_id, m.uid, m.message_id, m.in_reply_to, m.refs, m.subject,
           m.from_name, m.from_addr, m.reply_to, m.to_addrs, m.cc_addrs, m.date, m.size,
           m.flags, m.list_id, m.list_unsubscribe, m.list_unsubscribe_post, m.dmarc_pass,
           m.preview, m.has_attachments,
           EXISTS (SELECT 1 FROM message_bodies b WHERE b.message_id = m.id) AS has_body,
           f.account_id AS account_id, f.path AS folder_path, f.role AS folder_role,
           a.email AS account_email
    FROM messages m
    JOIN folders f ON f.id = m.folder_id
    JOIN accounts a ON a.id = f.account_id";

/// Turns free user input into an FTS5 query where every word is a quoted
/// prefix term. Quoting keeps FTS5 syntax characters in the input harmless.
fn fts_prefix_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Account, server path and highest stored UID of a folder.
fn folder_target(conn: &Connection, folder_id: i64) -> Result<Option<(i64, String, u32)>> {
    Ok(conn
        .query_row(
            "SELECT f.account_id, f.path,
                    MAX(f.last_seen_uid, COALESCE((SELECT MAX(uid) FROM messages WHERE folder_id = f.id), 0))
             FROM folders f WHERE f.id = ?1",
            [folder_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}

/// A message claimed for sending.
#[derive(Debug)]
pub struct OutboxItem {
    pub id: i64,
    pub account_id: i64,
    pub built: Built,
    pub subject: String,
    pub draft: Draft,
    pub attempts: u32,
}

/// Outbox state for the UI.
#[derive(Clone, Debug, serde::Serialize)]
pub struct OutboxEntry {
    pub id: i64,
    pub account_id: i64,
    pub subject: String,
    pub recipients: Vec<String>,
    pub send_after: i64,
    /// `queued`, `sending` or `failed`.
    pub state: String,
    pub attempts: u32,
    pub last_error: Option<String>,
}

fn outbox_from_row(r: &Row<'_>) -> rusqlite::Result<OutboxItem> {
    let draft: String = r.get("draft")?;
    Ok(OutboxItem {
        id: r.get("id")?,
        account_id: r.get("account_id")?,
        built: Built {
            wire: r.get("wire")?,
            record: r.get("record")?,
            envelope_from: r.get("envelope_from")?,
            envelope_to: json_list(r, "envelope_to")?,
            message_id: r.get("message_id")?,
        },
        subject: r.get("subject")?,
        draft: serde_json::from_str(&draft).unwrap_or_default(),
        attempts: r.get("attempts")?,
    })
}

/// First characters of the text with whitespace collapsed and quoted lines
/// and signatures left out, for the message list.
fn make_preview(text: &str) -> Option<String> {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line == "--" || line == "-- " {
            break;
        }
        if line.is_empty() || line.starts_with('>') {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&line.split_whitespace().collect::<Vec<_>>().join(" "));
        if out.chars().count() >= PREVIEW_CHARS {
            break;
        }
    }
    let preview: String = out.chars().take(PREVIEW_CHARS).collect();
    (!preview.is_empty()).then_some(preview)
}

fn json(v: &[String]) -> String {
    serde_json::to_string(v).expect("a string list always serialises")
}

fn json_list(row: &Row<'_>, col: &str) -> rusqlite::Result<Vec<String>> {
    let raw: String = row.get(col)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn parse_col<T: std::str::FromStr>(row: &Row<'_>, col: &str) -> rusqlite::Result<T> {
    let raw: String = row.get(col)?;
    raw.parse().map_err(|_| {
        rusqlite::Error::InvalidColumnType(0, col.to_string(), rusqlite::types::Type::Text)
    })
}

fn account_from_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    let smtp_host: Option<String> = row.get("smtp_host")?;
    let smtp = match smtp_host {
        Some(host) => Some(ServerConfig {
            host,
            port: row.get("smtp_port")?,
            security: parse_col::<Security>(row, "smtp_security")?,
            username: row.get("smtp_username")?,
            auth: parse_col::<AuthMethod>(row, "smtp_auth")?,
        }),
        None => None,
    };
    Ok(Account {
        id: row.get("id")?,
        email: row.get("email")?,
        display_name: row.get("display_name")?,
        imap: ServerConfig {
            host: row.get("imap_host")?,
            port: row.get("imap_port")?,
            security: parse_col(row, "imap_security")?,
            username: row.get("imap_username")?,
            auth: parse_col(row, "imap_auth")?,
        },
        smtp,
    })
}

fn folder_from_row(row: &Row<'_>) -> rusqlite::Result<Folder> {
    let role: Option<String> = row.get("role")?;
    Ok(Folder {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        path: row.get("path")?,
        delimiter: row.get("delimiter")?,
        role: role.as_deref().and_then(FolderRole::parse),
        selectable: row.get("selectable")?,
        uid_validity: row.get("uid_validity")?,
        last_seen_uid: row.get("last_seen_uid")?,
        highest_modseq: row
            .get::<_, Option<i64>>("highest_modseq")?
            .map(|v| v as u64),
        keywords: row.get("keywords")?,
    })
}

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        account_email: row.get("account_email")?,
        folder_id: row.get("folder_id")?,
        folder_name: crate::utf7::decode(&row.get::<_, String>("folder_path")?),
        folder_path: row.get("folder_path")?,
        folder_role: row
            .get::<_, Option<String>>("folder_role")?
            .as_deref()
            .and_then(FolderRole::parse),
        preview: row.get("preview")?,
        has_attachments: row.get("has_attachments")?,
        has_body: row.get("has_body")?,
        header: MessageHeader {
            uid: row.get("uid")?,
            message_id: row.get("message_id")?,
            in_reply_to: row.get("in_reply_to")?,
            references: json_list(row, "refs")?,
            subject: row.get("subject")?,
            from_name: row.get("from_name")?,
            from_addr: row.get("from_addr")?,
            reply_to: row.get("reply_to")?,
            to: json_list(row, "to_addrs")?,
            cc: json_list(row, "cc_addrs")?,
            date: row.get("date")?,
            size: row.get("size")?,
            flags: json_list(row, "flags")?,
            list_id: row.get("list_id")?,
            list_unsubscribe: json_list(row, "list_unsubscribe")?,
            list_unsubscribe_post: row.get("list_unsubscribe_post")?,
            dmarc_pass: row.get("dmarc_pass")?,
        },
    })
}

/// What the linker needs from a stored message.
#[derive(Clone, Debug)]
pub struct ScanMessage {
    pub id: i64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: i64,
    pub list_id: Option<String>,
    pub has_unsubscribe: bool,
    pub is_seen: bool,
    pub dmarc_pass: bool,
    pub body_text: Option<String>,
    pub folder_role: Option<FolderRole>,
}

pub(crate) const SCAN_SELECT: &str = "
    SELECT m.id, m.message_id, m.in_reply_to, m.refs, m.subject, m.from_name,
           m.from_addr, m.to_addrs, m.cc_addrs, COALESCE(m.date, 0) AS date, m.list_id,
           m.list_unsubscribe != '[]' OR m.list_unsubscribe_post != 0 AS has_unsubscribe,
           instr(m.flags, '\\Seen') > 0 AS is_seen,
           m.dmarc_pass, m.body_text, f.role AS folder_role
    FROM messages m
    JOIN folders f ON f.id = m.folder_id";

pub(crate) fn scan_from_row(row: &Row<'_>) -> rusqlite::Result<ScanMessage> {
    Ok(ScanMessage {
        id: row.get("id")?,
        message_id: row.get("message_id")?,
        in_reply_to: row.get("in_reply_to")?,
        references: json_list(row, "refs")?,
        subject: row.get("subject")?,
        from_name: row.get("from_name")?,
        from_addr: row.get("from_addr")?,
        to: json_list(row, "to_addrs")?,
        cc: json_list(row, "cc_addrs")?,
        date: row.get("date")?,
        list_id: row.get("list_id")?,
        has_unsubscribe: row.get("has_unsubscribe")?,
        is_seen: row.get("is_seen")?,
        dmarc_pass: row.get("dmarc_pass")?,
        body_text: row.get("body_text")?,
        folder_role: row
            .get::<_, Option<String>>("folder_role")?
            .as_deref()
            .and_then(FolderRole::parse),
    })
}

fn case_from_row(row: &Row<'_>) -> rusqlite::Result<Case> {
    Ok(Case {
        id: row.get("id")?,
        title: row.get("title")?,
        counterparty: row.get("counterparty")?,
        domain: row.get("domain")?,
        kind: parse_col::<CaseKind>(row, "kind")?,
        status: parse_col::<CaseStatus>(row, "status")?,
        next_step: row.get("next_step")?,
        due_at: row.get("due_at")?,
        due_label: row.get("due_label")?,
        amount_cents: row.get("amount_cents")?,
        currency: row.get("currency")?,
        reason: row.get("reason")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn case_event_from_row(row: &Row<'_>) -> rusqlite::Result<CaseEvent> {
    let dir: Option<String> = row.get("direction")?;
    let direction = match dir.as_deref() {
        Some(s) => Some(s.parse::<EventDirection>().map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                0,
                "direction".to_string(),
                rusqlite::types::Type::Text,
            )
        })?),
        None => None,
    };
    Ok(CaseEvent {
        id: row.get("id")?,
        case_id: row.get("case_id")?,
        message_id: row.get("message_id")?,
        occurred_at: row.get("occurred_at")?,
        kind: row.get("kind")?,
        label: row.get("label")?,
        phase: parse_col::<CaseEventPhase>(row, "phase")?,
        direction,
        confidence: row.get("confidence")?,
        manual: row.get("manual")?,
    })
}

fn case_key_from_row(row: &Row<'_>) -> rusqlite::Result<CaseKey> {
    Ok(CaseKey {
        label: row.get("label")?,
        value: row.get("value")?,
        normalized: row.get("normalized")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: &str) -> Account {
        Account {
            id: 0,
            email: email.into(),
            display_name: None,
            imap: ServerConfig {
                host: "imap.example.org".into(),
                port: 993,
                security: Security::Tls,
                username: email.into(),
                auth: AuthMethod::Password,
            },
            smtp: None,
        }
    }

    fn header(uid: u32, subject: &str, from: &str, date: i64) -> MessageHeader {
        MessageHeader {
            uid,
            subject: Some(subject.into()),
            from_addr: Some(from.into()),
            date: Some(date),
            ..Default::default()
        }
    }

    /// Inbox with the user's account and a sent mail to `anna@example.com`.
    fn turn_store() -> (Store, i64, i64) {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("me@example.org")).unwrap();
        let inbox = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        let sent = store
            .upsert_folder(a.id, "Sent", None, Some(FolderRole::Sent), true)
            .unwrap();
        store
            .insert_messages(
                sent.id,
                &[MessageHeader {
                    to: vec!["Anna@Example.com".into()],
                    ..header(1, "Termin", "me@example.org", 50)
                }],
            )
            .unwrap();
        (store, inbox.id, a.id)
    }

    fn turn_of(store: &Store, folder: i64, uid: u32) -> crate::turn::TurnAssessment {
        let id = store
            .folder_messages(folder, 100)
            .unwrap()
            .iter()
            .find(|m| m.header.uid == uid)
            .unwrap()
            .id;
        store
            .assess_messages_turns(&[id], 1000)
            .unwrap()
            .remove(0)
            .1
    }

    fn question_mail(uid: u32, rfc_id: &str) -> MessageHeader {
        MessageHeader {
            to: vec!["me@example.org".into()],
            message_id: Some(rfc_id.into()),
            ..header(uid, "Frage", "bob@example.net", 100 + uid as i64)
        }
    }

    fn set_preview(store: &Store, uid: u32) {
        store
            .conn
            .execute(
                "UPDATE messages SET preview = 'Hast du Zeit für einen Anruf?' WHERE uid = ?1",
                [uid],
            )
            .unwrap();
    }

    fn id_of(store: &Store, folder: i64, uid: u32) -> i64 {
        store
            .folder_messages(folder, 100)
            .unwrap()
            .iter()
            .find(|m| m.header.uid == uid)
            .unwrap()
            .id
    }

    #[test]
    fn dismissed_turn_is_done_until_restored() {
        let (mut store, inbox, _) = turn_store();
        store
            .insert_messages(inbox, &[question_mail(20, "<q1@example.net>")])
            .unwrap();
        set_preview(&store, 20);
        assert_eq!(turn_of(&store, inbox, 20).status, TurnStatus::NeedsYou);

        let id = id_of(&store, inbox, 20);
        store.dismiss_turn(id).unwrap();
        let a = turn_of(&store, inbox, 20);
        assert_eq!(a.status, TurnStatus::Done);
        assert_eq!(a.reason, "Von dir als erledigt entfernt");

        // Dismissing twice is harmless.
        store.dismiss_turn(id).unwrap();
        store.restore_turn(id).unwrap();
        assert_eq!(turn_of(&store, inbox, 20).status, TurnStatus::NeedsYou);
    }

    #[test]
    fn answered_mail_no_longer_needs_you() {
        let (mut store, inbox, a) = turn_store();
        let sent = store
            .upsert_folder(a, "Sent", None, Some(FolderRole::Sent), true)
            .unwrap();
        store
            .insert_messages(
                inbox,
                &[
                    question_mail(20, "<q1@example.net>"),
                    MessageHeader {
                        in_reply_to: Some("<q1@example.net>".into()),
                        references: vec!["<q1@example.net>".into()],
                        ..question_mail(21, "<q2@example.net>")
                    },
                    // Arrives after the reply: the other side is waiting again.
                    question_mail(40, "<q3@example.net>"),
                ],
            )
            .unwrap();
        for uid in [20, 21, 40] {
            set_preview(&store, uid);
        }
        assert_eq!(turn_of(&store, inbox, 21).status, TurnStatus::NeedsYou);

        // The reply answers the forwarded mail and, through References, the
        // first one as well.
        store
            .insert_messages(
                sent.id,
                &[MessageHeader {
                    to: vec!["bob@example.net".into()],
                    in_reply_to: Some("<q2@example.net>".into()),
                    references: vec!["<q1@example.net>".into(), "<q2@example.net>".into()],
                    ..header(2, "Re: Frage", "me@example.org", 130)
                }],
            )
            .unwrap();
        for uid in [20, 21] {
            let t = turn_of(&store, inbox, uid);
            assert_eq!(t.status, TurnStatus::Done);
            assert_eq!(t.reason, "Du hast schon geantwortet");
        }
        assert_eq!(turn_of(&store, inbox, 40).status, TurnStatus::NeedsYou);

        // Emptying the Sent folder must not bring the answered mails back.
        store
            .conn
            .execute("DELETE FROM messages WHERE folder_id = ?1", [sent.id])
            .unwrap();
        assert_eq!(turn_of(&store, inbox, 21).status, TurnStatus::Done);
    }

    #[test]
    fn answered_flag_counts_without_a_sent_copy() {
        let (mut store, inbox, _) = turn_store();
        store
            .insert_messages(
                inbox,
                &[MessageHeader {
                    flags: vec!["\\Answered".into(), "\\Seen".into()],
                    ..question_mail(20, "<q1@example.net>")
                }],
            )
            .unwrap();
        set_preview(&store, 20);
        assert_eq!(turn_of(&store, inbox, 20).status, TurnStatus::Done);
    }

    #[test]
    fn new_mail_in_dismissed_thread_is_assessed_normally() {
        let (mut store, inbox, _) = turn_store();
        store
            .insert_messages(
                inbox,
                &[
                    question_mail(20, "<q1@example.net>"),
                    question_mail(21, "<q2@example.net>"),
                ],
            )
            .unwrap();
        set_preview(&store, 20);
        set_preview(&store, 21);
        store.dismiss_turn(id_of(&store, inbox, 20)).unwrap();
        assert_eq!(turn_of(&store, inbox, 20).status, TurnStatus::Done);
        assert_eq!(turn_of(&store, inbox, 21).status, TurnStatus::NeedsYou);
    }

    #[test]
    fn dismissal_follows_the_message_id_to_other_copies() {
        let (mut store, inbox, a) = turn_store();
        let archive = store
            .upsert_folder(a, "Archive", None, Some(FolderRole::Archive), true)
            .unwrap();
        store
            .insert_messages(inbox, &[question_mail(20, "<q1@example.net>")])
            .unwrap();
        store
            .insert_messages(archive.id, &[question_mail(7, "<q1@example.net>")])
            .unwrap();
        set_preview(&store, 7);
        store.dismiss_turn(id_of(&store, inbox, 20)).unwrap();
        assert_eq!(turn_of(&store, archive.id, 7).status, TurnStatus::Done);
    }

    #[test]
    fn marketing_mail_without_list_id_is_not_needs_you() {
        let (mut store, inbox, _) = turn_store();
        // The real-world shape: unsubscribe header, news@ sender, anonymous
        // greeting, several earlier mails, the user never wrote to it.
        let mut headers = Vec::new();
        for uid in 10..16 {
            headers.push(MessageHeader {
                to: vec!["me@example.org".into()],
                list_unsubscribe: vec!["<mailto:u@example.net>".into()],
                ..header(
                    uid,
                    "Angebote der Woche",
                    "news@shop.example.com",
                    100 + uid as i64,
                )
            });
        }
        store.insert_messages(inbox, &headers).unwrap();
        let a = turn_of(&store, inbox, 15);
        assert_eq!(a.status, TurnStatus::ReadingAlong, "{a:?}");
    }

    #[test]
    fn recurring_unknown_sender_without_machine_headers_is_reading_along() {
        let (mut store, inbox, _) = turn_store();
        let headers: Vec<_> = (10..14)
            .map(|uid| MessageHeader {
                to: vec!["me@example.org".into()],
                ..header(uid, "Hallo", "team@shop.example.com", 100 + uid as i64)
            })
            .collect();
        store.insert_messages(inbox, &headers).unwrap();
        // Before the sender is known as a source: unknown, no question.
        assert_eq!(turn_of(&store, inbox, 13).status, TurnStatus::ReadingAlong);
        // Pretend the source pass counted them; a question does not help then.
        store
            .conn
            .execute(
                "INSERT INTO sources (source_key, display_name, total_messages)
                 VALUES ('team@shop.example.com', 'Shop', 4)",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET source_id = (SELECT id FROM sources)",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET preview = 'Kannst du uns kurz antworten?' WHERE uid = 13",
                [],
            )
            .unwrap();
        let a = turn_of(&store, inbox, 13);
        assert_eq!(a.status, TurnStatus::ReadingAlong, "{a:?}");
    }

    #[test]
    fn unknown_sender_with_a_question_still_needs_you() {
        let (mut store, inbox, _) = turn_store();
        store
            .insert_messages(
                inbox,
                &[MessageHeader {
                    to: vec!["me@example.org".into()],
                    ..header(20, "Frage", "bob@example.net", 100)
                }],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET preview = 'Hast du Zeit für einen Anruf?' WHERE uid = 20",
                [],
            )
            .unwrap();
        let a = turn_of(&store, inbox, 20);
        assert_eq!(a.status, TurnStatus::NeedsYou);
        assert!(a.confidence < 0.7);
    }

    #[test]
    fn reply_from_someone_the_user_wrote_to_needs_you() {
        let (mut store, inbox, _) = turn_store();
        store
            .insert_messages(
                inbox,
                &[MessageHeader {
                    to: vec!["me@example.org".into()],
                    ..header(30, "Re: Termin", "anna@example.com", 100)
                }],
            )
            .unwrap();
        // Known by the sent folder, even without a question.
        let a = turn_of(&store, inbox, 30);
        assert_eq!(a.status, TurnStatus::NeedsYou, "{a:?}");
    }

    #[test]
    fn reply_headers_mark_a_known_correspondent() {
        let (mut store, inbox, _) = turn_store();
        store
            .insert_messages(
                inbox,
                &[MessageHeader {
                    to: vec!["me@example.org".into()],
                    in_reply_to: Some("<mine@example.org>".into()),
                    ..header(31, "Re: Angebot", "carl@example.net", 100)
                }],
            )
            .unwrap();
        assert_eq!(turn_of(&store, inbox, 31).status, TurnStatus::NeedsYou);
    }

    #[test]
    fn accounts_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let saved = store.add_account(&account("a@example.org")).unwrap();
        assert!(saved.id > 0);
        let loaded = store.account_by_email("A@example.org").unwrap().unwrap();
        assert_eq!(loaded.imap, saved.imap);
        assert_eq!(store.accounts().unwrap().len(), 1);
    }

    #[test]
    fn search_spans_accounts_and_matches_prefixes() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let b = store.add_account(&account("b@example.net")).unwrap();
        let fa = store
            .upsert_folder(a.id, "INBOX", Some("/"), Some(FolderRole::Inbox), true)
            .unwrap();
        let fb = store
            .upsert_folder(b.id, "INBOX", Some("."), Some(FolderRole::Inbox), true)
            .unwrap();
        store
            .insert_messages(
                fa.id,
                &[header(1, "Ihre Rechnung März", "billing@telekom.de", 10)],
            )
            .unwrap();
        store
            .insert_messages(
                fb.id,
                &[
                    header(7, "Rechnung Nr. 42", "shop@example.com", 20),
                    header(8, "Willkommen", "hello@example.com", 30),
                ],
            )
            .unwrap();

        let hits = store.search("rech", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|m| m.account_email == "a@example.org"));
        assert!(hits.iter().any(|m| m.account_email == "b@example.net"));

        // Diacritics are folded, operators in user input are harmless.
        assert_eq!(store.search("marz", 10).unwrap().len(), 1);
        assert!(store.search("\"AND OR(", 10).unwrap().is_empty());

        assert_eq!(
            store.unread_count(None, Some(FolderRole::Inbox)).unwrap(),
            3
        );
        assert_eq!(store.unread_count(Some(a.id), None).unwrap(), 1);
        store
            .insert_messages(
                fa.id,
                &[MessageHeader {
                    flags: vec!["\\Seen".into()],
                    ..header(1, "Ihre Rechnung März", "billing@telekom.de", 10)
                }],
            )
            .unwrap();
        assert_eq!(store.unread_count(Some(a.id), None).unwrap(), 0);
        assert_eq!(store.messages(Some(b.id), None, 10).unwrap().len(), 2);

        let inbox = store.recent_messages(Some(FolderRole::Inbox), 10).unwrap();
        assert_eq!(
            inbox.first().unwrap().header.subject.as_deref(),
            Some("Willkommen")
        );
        assert_eq!(store.folders(b.id).unwrap()[0].last_seen_uid, 0);
        store.mark_synced_up_to(fb.id, 8).unwrap();
        store.mark_synced_up_to(fb.id, 3).unwrap();
        assert_eq!(store.folders(b.id).unwrap()[0].last_seen_uid, 8);
    }

    #[test]
    fn changed_uid_validity_resets_folder() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "INBOX", None, None, true)
            .unwrap();
        assert!(!store.set_uid_validity(f.id, 100).unwrap());
        store
            .insert_messages(f.id, &[header(5, "x", "y@z.de", 1)])
            .unwrap();
        store.mark_synced_up_to(f.id, 5).unwrap();
        assert!(!store.set_uid_validity(f.id, 100).unwrap());
        assert_eq!(store.message_count(f.id).unwrap(), 1);
        assert!(store.set_uid_validity(f.id, 200).unwrap());
        assert_eq!(store.message_count(f.id).unwrap(), 0);
        assert_eq!(store.folders(a.id).unwrap()[0].last_seen_uid, 0);
    }

    #[test]
    fn deleting_account_cascades() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "INBOX", None, None, true)
            .unwrap();
        store
            .insert_messages(f.id, &[header(1, "Hallo", "x@y.de", 1)])
            .unwrap();
        store.remove_account(a.id).unwrap();
        assert!(store.search("hallo", 10).unwrap().is_empty());
    }

    #[test]
    fn update_account_persists_changes() {
        let store = Store::open_in_memory().unwrap();
        let mut a = store.add_account(&account("a@example.org")).unwrap();
        a.display_name = Some("Max Muster".into());
        a.imap.host = "mail.example.org".into();
        a.imap.port = 143;
        store.update_account(&a).unwrap();

        let loaded = store.account_by_email("a@example.org").unwrap().unwrap();
        assert_eq!(loaded.display_name.as_deref(), Some("Max Muster"));
        assert_eq!(loaded.imap.host, "mail.example.org");
        assert_eq!(loaded.imap.port, 143);
    }

    #[test]
    fn bodies_are_stored_compressed_and_searchable() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        store
            .insert_messages(f.id, &[header(1, "Hallo", "x@example.org", 100)])
            .unwrap();
        let id = store.recent_messages(None, 1).unwrap()[0].id;
        assert!(!store.message(id).unwrap().unwrap().has_body);
        assert_eq!(
            store.messages_without_body(f.id, 0, 10_000, 10).unwrap(),
            vec![(id, 1)]
        );

        let raw = b"Subject: Hallo\r\nContent-Type: text/plain\r\n\r\nDie Lieferung kommt Donnerstag.\r\n> altes Zitat\r\n-- \r\nSignatur\r\n";
        store.store_body(id, raw).unwrap();
        assert_eq!(store.body(id).unwrap().unwrap(), raw.to_vec());
        let m = store.message(id).unwrap().unwrap();
        assert!(m.has_body);
        assert_eq!(
            m.preview.as_deref(),
            Some("Die Lieferung kommt Donnerstag.")
        );
        assert!(
            store
                .messages_without_body(f.id, 0, 10_000, 10)
                .unwrap()
                .is_empty()
        );
        // Body text is searchable, prefix and diacritics as for headers.
        assert_eq!(store.search("lieferu donnerst", 10).unwrap().len(), 1);
    }

    #[test]
    fn folder_names_are_decoded_for_display() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "Entw&APw-rfe", None, None, true)
            .unwrap();
        store
            .insert_messages(f.id, &[header(1, "x", "y@example.org", 1)])
            .unwrap();
        let m = &store.recent_messages(None, 1).unwrap()[0];
        assert_eq!(m.folder_path, "Entw&APw-rfe");
        assert_eq!(m.folder_name, "Entwürfe");
    }

    #[test]
    fn local_changes_win_until_sent_and_deletes_go_to_trash() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let inbox = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        store
            .insert_messages(
                inbox.id,
                &[
                    header(1, "x", "y@example.org", 1),
                    header(2, "z", "y@example.org", 2),
                ],
            )
            .unwrap();
        let ids: Vec<i64> = store
            .messages(None, None, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();

        // Mark read locally: queued, and a server flag sync does not undo it.
        assert_eq!(store.queue_flag(&[ids[0]], "\\Seen", true).unwrap(), 1);
        assert_eq!(store.queue_flag(&[ids[0]], "\\Seen", true).unwrap(), 0);
        store
            .apply_server_flags(inbox.id, &[(2, vec![]), (1, vec![])])
            .unwrap();
        assert!(store.message(ids[0]).unwrap().unwrap().is_seen());
        let ops = store.pending_ops(a.id).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].change, PendingChange::AddFlag("\\Seen".into()));

        // Once sent, the server is authoritative again.
        store.complete_op(ops[0].id).unwrap();
        store
            .apply_server_flags(inbox.id, &[(2, vec![]), (1, vec![])])
            .unwrap();
        assert!(!store.message(ids[0]).unwrap().unwrap().is_seen());

        // Delete without trash folder: refused, nothing changed.
        assert!(store.queue_delete(&[ids[1]]).is_err());
        assert!(store.message(ids[1]).unwrap().is_some());
        store
            .upsert_folder(a.id, "Trash", None, Some(FolderRole::Trash), true)
            .unwrap();
        assert_eq!(store.queue_delete(&[ids[1]]).unwrap(), 1);
        assert!(store.message(ids[1]).unwrap().is_none());
        let ops = store.pending_ops(a.id).unwrap();
        assert_eq!(
            ops.last().unwrap().change,
            PendingChange::MoveTo("Trash".into())
        );

        // Failing ops are retried a limited number of times.
        for _ in 0..MAX_OP_ATTEMPTS {
            store.fail_op(ops[0].id, "boom").unwrap();
        }
        assert!(store.pending_ops(a.id).unwrap().is_empty());
        assert!(!store.has_pending_ops().unwrap());
    }

    #[test]
    fn messages_gone_from_server_are_removed() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "INBOX", None, None, true)
            .unwrap();
        store
            .insert_messages(
                f.id,
                &[
                    header(1, "a", "x@example.org", 1),
                    header(2, "b", "x@example.org", 2),
                    header(9, "c", "x@example.org", 3),
                ],
            )
            .unwrap();
        let existing: std::collections::HashSet<u32> = [2].into_iter().collect();
        // UID 9 is above `up_to` and must survive.
        assert_eq!(store.delete_missing(f.id, &existing, 5).unwrap(), 1);
        assert_eq!(store.message_count(f.id).unwrap(), 2);
    }

    #[test]
    fn outbox_supports_undo_and_never_sends_twice() {
        let store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let draft = Draft {
            account_id: a.id,
            to: vec![crate::reply::Recipient {
                name: None,
                address: "b@example.org".into(),
            }],
            subject: "Hallo".into(),
            body: "x".into(),
            ..Default::default()
        };
        let built = crate::send::build(&draft, &a, std::time::SystemTime::now()).unwrap();

        // Undo within the window returns the draft.
        let id = store.queue_outgoing(&draft, &built, 100).unwrap();
        assert!(store.claim_due_outgoing(99).unwrap().is_none());
        assert_eq!(store.cancel_outgoing(id).unwrap().unwrap().subject, "Hallo");
        assert!(store.outbox().unwrap().is_empty());

        // Once claimed, it cannot be cancelled or claimed again.
        let id = store.queue_outgoing(&draft, &built, 100).unwrap();
        assert_eq!(store.next_outgoing_due().unwrap(), Some(100));
        let item = store.claim_due_outgoing(100).unwrap().unwrap();
        assert_eq!(item.id, id);
        assert_eq!(item.built.wire, built.wire);
        assert!(store.cancel_outgoing(id).unwrap().is_none());
        assert!(store.claim_due_outgoing(1000).unwrap().is_none());

        // Temporary failure: back in the queue at the retry time.
        store.fail_outgoing(id, "timeout", Some(200)).unwrap();
        assert!(store.claim_due_outgoing(150).unwrap().is_none());
        let item = store.claim_due_outgoing(200).unwrap().unwrap();
        assert_eq!(item.attempts, 2);

        // Interrupted sends are requeued at start-up.
        assert_eq!(store.requeue_interrupted_outgoing().unwrap(), 1);
        let item = store.claim_due_outgoing(200).unwrap().unwrap();

        // Permanent failure stays visible until the user takes it back.
        store.fail_outgoing(item.id, "550 rejected", None).unwrap();
        assert_eq!(store.outbox().unwrap()[0].state, "failed");
        assert!(store.claim_due_outgoing(10_000).unwrap().is_none());
        assert!(store.take_failed_outgoing(item.id).unwrap().is_some());
        assert!(store.outbox().unwrap().is_empty());
    }

    #[test]
    fn folder_messages_and_unread_counts_by_folder() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let inbox = store
            .upsert_folder(a.id, "INBOX", Some("/"), Some(FolderRole::Inbox), true)
            .unwrap();
        let custom = store
            .upsert_folder(a.id, "Rechnungen", Some("/"), None, true)
            .unwrap();

        let mut h1 = header(1, "Willkommen", "hello@example.org", 10);
        h1.flags = vec!["\\Seen".into()];
        let h2 = header(2, "Rechnung Januar", "billing@example.org", 20);
        let h3 = header(3, "Rechnung Februar", "billing@example.org", 30);

        store.insert_messages(inbox.id, &[h1]).unwrap();
        store.insert_messages(custom.id, &[h2, h3]).unwrap();

        let custom_msgs = store.folder_messages(custom.id, 10).unwrap();
        assert_eq!(custom_msgs.len(), 2);

        let counts = store.unread_counts_by_folder().unwrap();
        assert_eq!(counts.get(&inbox.id).copied().unwrap_or(0), 0);
        assert_eq!(counts.get(&custom.id).copied().unwrap_or(0), 2);
    }

    #[test]
    fn whole_folder_mark_read_and_empty() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let junk = store
            .upsert_folder(a.id, "Spam", None, Some(FolderRole::Junk), true)
            .unwrap();
        let inbox = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        let mut seen = header(1, "a", "x@example.org", 1);
        seen.flags = vec!["\\Seen".into()];
        store
            .insert_messages(
                junk.id,
                &[
                    seen,
                    header(2, "b", "x@example.org", 2),
                    header(7, "c", "x@example.org", 3),
                ],
            )
            .unwrap();
        store
            .insert_messages(inbox.id, &[header(3, "d", "x@example.org", 4)])
            .unwrap();

        assert_eq!(store.queue_mark_folder_seen(junk.id).unwrap(), 2);
        assert_eq!(
            store
                .unread_count(Some(a.id), Some(FolderRole::Junk))
                .unwrap(),
            0
        );
        let ops = store.pending_ops(a.id).unwrap();
        assert_eq!(ops.len(), 1, "one op for the whole folder");
        assert_eq!(ops[0].change, PendingChange::MarkAllSeen);
        assert_eq!(ops[0].uid, 7);
        // The server still says "unread" until the op is sent: ignored.
        assert_eq!(
            store.apply_server_flags(junk.id, &[(2, vec![])]).unwrap(),
            0
        );
        assert_eq!(store.queue_mark_folder_seen(junk.id).unwrap(), 0);

        // The inbox can never be emptied this way.
        assert!(store.queue_empty_folder(inbox.id).is_err());
        assert_eq!(store.message_count(inbox.id).unwrap(), 1);

        assert_eq!(store.queue_empty_folder(junk.id).unwrap(), 3);
        assert_eq!(store.message_count(junk.id).unwrap(), 0);
        let last = store.pending_ops(a.id).unwrap().pop().unwrap();
        assert_eq!(last.change, PendingChange::EmptyUpTo);
        assert_eq!(last.uid, 7);
    }

    #[test]
    fn brand_logo_cache_respects_ttl_and_clears() {
        let store = Store::open_in_memory().unwrap();
        let logo = BrandLogo {
            data_url: "data:image/svg+xml;base64,PHN2Zz4=".into(),
            bg: Some("rgb(42, 53, 24)".into()),
            padded: true,
            scale: 1.25,
        };
        store
            .save_brand_logo("kleinanzeigen.example", Some(&logo), 1_000)
            .unwrap();
        assert_eq!(
            store
                .get_brand_logo("kleinanzeigen.example", 2_000)
                .unwrap(),
            Some(CachedLogo::Hit(logo))
        );
        // Expired after 30 days
        assert_eq!(
            store
                .get_brand_logo("kleinanzeigen.example", 1_000 + LOGO_HIT_TTL_SECS + 1)
                .unwrap(),
            None
        );

        store
            .save_brand_logo("missing.example", None, 1_000)
            .unwrap();
        assert_eq!(
            store.get_brand_logo("missing.example", 2_000).unwrap(),
            Some(CachedLogo::Miss)
        );
        assert_eq!(
            store
                .get_brand_logo("missing.example", 1_000 + LOGO_MISS_TTL_SECS + 1)
                .unwrap(),
            None
        );

        store.clear_brand_logos().unwrap();
        assert_eq!(
            store
                .get_brand_logo("kleinanzeigen.example", 2_000)
                .unwrap(),
            None
        );
    }

    #[test]
    fn cases_crud_keys_and_dissolve_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let created = store
            .create_case(&NewCase {
                title: "Bestellung #A-123".into(),
                counterparty: "Shop".into(),
                domain: Some("shop.example".into()),
                kind: CaseKind::Order,
                status: CaseStatus::Waiting,
                next_step: "Wartet auf Versand".into(),
                due_at: None,
                due_label: Some("bis Fr".into()),
                amount_cents: Some(4990),
                currency: Some("EUR".into()),
                reason: "Bestellnummer erkannt".into(),
            })
            .unwrap();

        store
            .add_case_key(created.case.id, "Bestellnr.", "  A-123 ")
            .unwrap();
        store
            .add_case_key(created.case.id, "Bestellnr.", "a-123")
            .unwrap();
        assert_eq!(
            store.find_cases_by_key("a-123").unwrap(),
            vec![created.case.id]
        );

        store
            .add_case_event(
                created.case.id,
                &NewCaseEvent {
                    message_id: None,
                    occurred_at: 1_000,
                    kind: Some("order.placed".into()),
                    label: "Bestellt".into(),
                    phase: CaseEventPhase::Start,
                    direction: Some(EventDirection::In),
                    confidence: 0.95,
                    manual: false,
                },
            )
            .unwrap();

        let listed = store.list_cases(false).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].keys.len(), 1);
        assert_eq!(listed[0].events.len(), 1);

        store.dissolve_case(created.case.id).unwrap();
        assert!(store.list_cases(false).unwrap().is_empty());
        assert!(store.find_cases_by_key("a-123").unwrap().is_empty());
        assert_eq!(store.list_cases(true).unwrap().len(), 1);
    }

    #[test]
    fn case_assign_and_exclude_message() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        store
            .insert_messages(
                f.id,
                &[
                    header(1, "Bestellung 1", "shop@shop.example", 1_000),
                    header(2, "Werbung", "news@shop.example", 2_000),
                ],
            )
            .unwrap();
        let msgs = store.recent_messages(None, 10).unwrap();
        let m1 = msgs[1].id;
        let m2 = msgs[0].id;

        let c = store
            .create_case(&NewCase {
                title: "Bestellung".into(),
                counterparty: "Shop".into(),
                domain: Some("shop.example".into()),
                kind: CaseKind::Order,
                status: CaseStatus::Waiting,
                next_step: "Unterwegs".into(),
                due_at: None,
                due_label: None,
                amount_cents: None,
                currency: None,
                reason: "Test".into(),
            })
            .unwrap();

        store
            .assign_message_to_case(c.case.id, m1, "Bestellt", CaseEventPhase::Start)
            .unwrap();
        store
            .assign_message_to_case(c.case.id, m2, "Manuell", CaseEventPhase::Update)
            .unwrap();
        assert_eq!(
            store.get_case(c.case.id).unwrap().unwrap().message_ids,
            vec![m1, m2]
        );

        store.remove_message_from_case(c.case.id, m2).unwrap();
        assert!(store.is_message_excluded(c.case.id, m2).unwrap());
        assert_eq!(
            store.get_case(c.case.id).unwrap().unwrap().message_ids,
            vec![m1]
        );

        // Manual re-assign clears the exclusion.
        store
            .assign_message_to_case(c.case.id, m2, "Doch dazu", CaseEventPhase::Update)
            .unwrap();
        assert!(!store.is_message_excluded(c.case.id, m2).unwrap());
        assert_eq!(
            store.get_case(c.case.id).unwrap().unwrap().message_ids,
            vec![m1, m2]
        );
    }

    #[test]
    fn trusted_remote_senders_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.is_remote_sender_trusted("News@Shop.Example").unwrap());
        store.trust_remote_sender("  News@Shop.Example ").unwrap();
        assert!(store.is_remote_sender_trusted("news@shop.example").unwrap());
        store.untrust_remote_sender("NEWS@shop.example").unwrap();
        assert!(!store.is_remote_sender_trusted("news@shop.example").unwrap());
    }

    #[test]
    fn sender_behavior_learns_from_interactions_and_survives_delete() {
        let mut store = Store::open_in_memory().unwrap();
        let acc = store.add_account(&account("me@example.org")).unwrap();
        let inbox = store
            .upsert_folder(acc.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap()
            .id;
        store
            .upsert_folder(acc.id, "Trash", None, Some(FolderRole::Trash), true)
            .unwrap();

        let headers: Vec<MessageHeader> = (1..=4)
            .map(|uid| MessageHeader {
                uid,
                message_id: Some(format!("<msg-{uid}@sender.example>")),
                from_addr: Some("noisy@sender.example".into()),
                to: vec!["me@example.org".into()],
                subject: Some("Kannst du das kurz prüfen?".into()),
                date: Some(1_700_000_000 + uid as i64 * 100),
                ..Default::default()
            })
            .collect();
        store.insert_messages(inbox, &headers).unwrap();
        let id1 = id_of(&store, inbox, 1);
        let id2 = id_of(&store, inbox, 2);
        let id3 = id_of(&store, inbox, 3);
        let id4 = id_of(&store, inbox, 4);

        // Opening the same message twice only counts once.
        store
            .record_interaction(id1, Interaction::Open, 100)
            .unwrap();
        store
            .record_interaction(id1, Interaction::Open, 200)
            .unwrap();
        assert_eq!(
            store
                .sender_behavior("NOISY@sender.example")
                .unwrap()
                .opened,
            1
        );

        // Dismissing and deleting 3 messages deprioritizes the 4th.
        store.dismiss_turn(id1).unwrap();
        store.queue_delete(&[id2, id3]).unwrap();

        let b = store.sender_behavior("noisy@sender.example").unwrap();
        assert_eq!(b.ignored, 1);
        assert_eq!(b.deleted, 2);
        assert!(b.should_deprioritize());

        let turns = store.assess_messages_turns(&[id4], 1_700_001_000).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].1.status, TurnStatus::ReadingAlong);
    }
}
