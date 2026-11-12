//! Local mail store on top of SQLite.
//!
//! All accounts live in one database, which is what makes cross-account
//! views (categories, search) cheap. Full-text search uses FTS5 as an
//! external-content index that triggers keep in sync with `messages`.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::Result;
use crate::model::{
    Account, AuthMethod, Folder, FolderRole, MessageHeader, MessageSummary, Security, ServerConfig,
};

/// Schema migrations, applied in order. The index + 1 is stored in
/// `PRAGMA user_version`, so new migrations must only ever be appended.
const MIGRATIONS: &[&str] = &[r#"
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
"#];

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
                    list_id, list_unsubscribe, list_unsubscribe_post)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                 ON CONFLICT (folder_id, uid) DO UPDATE SET flags = excluded.flags",
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
             ORDER BY bm25(messages_fts), m.date DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![fts_query, limit as i64], summary_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn message_count(&self, folder_id: i64) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?)
    }
}

const SUMMARY_SELECT: &str = "
    SELECT m.*, f.account_id AS account_id, f.path AS folder_path, a.email AS account_email
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
    })
}

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        account_email: row.get("account_email")?,
        folder_id: row.get("folder_id")?,
        folder_path: row.get("folder_path")?,
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
}
