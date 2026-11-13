//! Local mail store on top of SQLite.
//!
//! All accounts live in one database, which is what makes cross-account
//! views (categories, search) cheap. Full-text search uses FTS5 as an
//! external-content index that triggers keep in sync with `messages`.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Result;
use crate::brand::{BrandLogo, LOGO_HIT_TTL_SECS, LOGO_MISS_TTL_SECS};
use crate::model::{
    Account, AuthMethod, Folder, FolderRole, MessageHeader, MessageSummary, PendingChange,
    PendingOp, Security, ServerConfig,
};
use crate::send::{Built, Draft};

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
];

/// Result of looking up a domain in the persistent brand logo cache.
#[derive(Clone, Debug, PartialEq)]
pub enum CachedLogo {
    Hit(BrandLogo),
    Miss,
}

/// Failed changes are retried this many times, then kept for inspection.
const MAX_OP_ATTEMPTS: u32 = 5;

/// Compression level for stored messages: fast, and mail text compresses
/// well even at low levels.
const ZSTD_LEVEL: i32 = 3;

/// Characters of text kept as list preview.
const PREVIEW_CHARS: usize = 180;

pub struct Store {
    conn: Connection,
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
             PRAGMA synchronous = NORMAL;",
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

    /// One-time job: reads the DMARC verdict from stored originals of
    /// messages synced before it was recorded. Offline; runs once (a marker
    /// in `meta`) and returns how many messages turned out authenticated.
    pub fn backfill_dmarc_from_bodies(&self) -> Result<Option<usize>> {
        const MARKER: &str = "backfill.dmarc.v1";
        let done: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM meta WHERE key = ?1)",
            [MARKER],
            |r| r.get(0),
        )?;
        if done {
            return Ok(None);
        }
        let mut passed = 0;
        let mut last_id = 0i64;
        loop {
            // In batches, so the UI's connection is never blocked for long.
            let batch: Vec<(i64, Vec<u8>, i64)> = {
                let mut stmt = self.conn.prepare(
                    "SELECT m.id, b.raw_zstd, b.raw_size FROM messages m
                     JOIN message_bodies b ON b.message_id = m.id
                     WHERE m.dmarc_pass = 0 AND m.id > ?1 ORDER BY m.id LIMIT 200",
                )?;
                stmt.query_map([last_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<_, _>>()?
            };
            let Some(&(id, _, _)) = batch.last() else {
                break;
            };
            last_id = id;
            let tx = self.conn.unchecked_transaction()?;
            for (id, compressed, size) in batch {
                let Ok(raw) = zstd::bulk::decompress(&compressed, size.max(0) as usize) else {
                    continue;
                };
                if crate::imap::parse_header_block(&raw).dmarc_pass {
                    tx.execute("UPDATE messages SET dmarc_pass = 1 WHERE id = ?1", [id])?;
                    passed += 1;
                }
            }
            tx.commit()?;
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, unixepoch())",
            [MARKER],
        )?;
        Ok(Some(passed))
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

    /// Sets or clears a flag (`\Seen`, `\Flagged`, …) locally and queues the
    /// change for the server. Returns the number of messages changed.
    pub fn queue_flag(&mut self, message_ids: &[i64], flag: &str, on: bool) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut changed = 0;
        for &id in message_ids {
            let row: Option<(String, i64, String, u32)> = tx
                .query_row(
                    "SELECT m.flags, f.account_id, f.path, m.uid FROM messages m
                     JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((raw, account_id, path, uid)) = row else {
                continue;
            };
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
    fn dmarc_verdict_backfilled_once_from_stored_originals() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("a@example.org")).unwrap();
        let f = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        store
            .insert_messages(f.id, &[header(1, "x", "news@shop.example", 1)])
            .unwrap();
        let id = store.recent_messages(None, 1).unwrap()[0].id;
        let raw = b"Authentication-Results: mx.provider.example; dmarc=pass header.from=shop.example\r\nFrom: Shop <news@shop.example>\r\nSubject: x\r\n\r\nHallo\r\n";
        // Simulate a body stored before the column existed.
        store.store_body(id, raw).unwrap();
        store
            .conn
            .execute("UPDATE messages SET dmarc_pass = 0", [])
            .unwrap();

        assert_eq!(store.backfill_dmarc_from_bodies().unwrap(), Some(1));
        assert!(store.message(id).unwrap().unwrap().header.dmarc_pass);
        assert_eq!(
            store.backfill_dmarc_from_bodies().unwrap(),
            None,
            "runs once"
        );
    }
}
