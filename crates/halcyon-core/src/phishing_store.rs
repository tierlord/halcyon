//! The phishing check as far as it touches the database: when a mail is
//! assessed, what is learnt from verified mail, the local "Ich kenne den
//! Absender", moving a mail to the junk folder, and the one-time pass over
//! existing mail. The judgement itself is [`crate::phishing`].
//!
//! Nothing here changes anything on the server by itself. The only server
//! change is [`Store::queue_move_to_junk`], and it runs only when the user
//! asks for it.

use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;
use crate::model::MessageSummary;
use crate::phishing::{self, Context, Input, NameSighting, PhishingVerdict};
use crate::store::{SUMMARY_SELECT, Store, summary_from_row, unix_now};

/// Key of the one-time pass in `meta`.
const META_BACKFILL: &str = "phishing_backfill_v1";
/// The one-time pass assesses mail of this many days.
pub const BACKFILL_DAYS: i64 = 90;
/// ... at most this many mails (newest first), so a huge mailbox does not
/// hold up the start.
const BACKFILL_MAX_MAILS: usize = 4_000;
/// Verified mail older than the assessed window still teaches names, up to
/// this age and count.
const LEARN_DAYS: i64 = 400;
const LEARN_MAX_MAILS: usize = 20_000;

/// What the verdict needs from the stored row.
struct Facts {
    /// Identity that survives moves between folders.
    key: String,
    from_name: String,
    from_addr: String,
    subject: String,
    dmarc_pass: bool,
    is_reply: bool,
    list_mail: bool,
    role: Option<String>,
}

impl Facts {
    /// Mail the user receives; sent mail and drafts say nothing about
    /// senders.
    fn incoming(&self) -> bool {
        !matches!(self.role.as_deref(), Some("sent" | "drafts"))
    }
}

fn load_facts(conn: &Connection, id: i64) -> Result<Option<Facts>> {
    Ok(conn
        .query_row(
            "SELECT m.id,
                    COALESCE(NULLIF(m.message_id, ''), 'f' || m.folder_id || ':' || m.uid),
                    COALESCE(m.from_name, ''), lower(trim(COALESCE(m.from_addr, ''))),
                    COALESCE(m.subject, ''), m.dmarc_pass,
                    m.in_reply_to, m.refs, m.list_id, m.list_unsubscribe, f.role
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ?1",
            [id],
            |r| {
                let refs: Vec<String> =
                    serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default();
                let unsub: Vec<String> =
                    serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default();
                Ok(Facts {
                    key: r.get(1)?,
                    from_name: r.get(2)?,
                    from_addr: r.get(3)?,
                    subject: r.get(4)?,
                    dmarc_pass: r.get(5)?,
                    is_reply: r.get::<_, Option<String>>(6)?.is_some() || !refs.is_empty(),
                    list_mail: r
                        .get::<_, Option<String>>(8)?
                        .is_some_and(|s| !s.is_empty())
                        || !unsub.is_empty(),
                    role: r.get(10)?,
                })
            },
        )
        .optional()?)
}

fn is_own_address(conn: &Connection, addr: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM accounts WHERE lower(email) = ?1",
            [addr],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Whether the user has written to this address. The database narrows the
/// candidates by text, the exact comparison is made here; a full scan of
/// the sent mail per header would slow the first sync down, so the caller
/// only asks for mail that would be flagged anyway.
fn is_correspondent(conn: &Connection, addr: &str) -> Result<bool> {
    if addr.is_empty() {
        return Ok(false);
    }
    let escaped = addr
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let mut stmt = conn.prepare_cached(
        "SELECT m.to_addrs, m.cc_addrs FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE f.role = 'sent'
           AND (lower(m.to_addrs) LIKE ?1 ESCAPE '\\' OR lower(m.cc_addrs) LIKE ?1 ESCAPE '\\')
         LIMIT 200",
    )?;
    let rows = stmt.query_map([pattern], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (to, cc) = row?;
        for raw in [to, cc] {
            let list: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if list.iter().any(|a| {
                let a = a.trim().to_ascii_lowercase();
                a == addr || a.ends_with(&format!("<{addr}>"))
            }) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn name_history(
    conn: &Connection,
    name: &str,
) -> Result<std::collections::HashMap<String, Vec<NameSighting>>> {
    let mut out = std::collections::HashMap::new();
    let mut stmt =
        conn.prepare_cached("SELECT base_domain, count FROM name_history WHERE token = ?1")?;
    for token in phishing::name_tokens(name) {
        let seen: Vec<NameSighting> = stmt
            .query_map([&token], |r| {
                Ok(NameSighting {
                    base_domain: r.get(0)?,
                    count: r.get::<_, i64>(1)?.max(0) as u32,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        if !seen.is_empty() {
            out.insert(token, seen);
        }
    }
    Ok(out)
}

fn judge(
    conn: &Connection,
    f: &Facts,
    text: Option<&str>,
    html: Option<&str>,
    dmarc_pass: bool,
) -> Result<PhishingVerdict> {
    let mut ctx = Context {
        known_correspondent: false,
        is_reply: f.is_reply,
        list_mail: f.list_mail,
        name_history: name_history(conn, &f.from_name)?,
    };
    let input = Input {
        display_name: &f.from_name,
        from_addr: &f.from_addr,
        dmarc_pass,
        subject: &f.subject,
        text,
        html,
    };
    let mut verdict = phishing::assess(&input, &ctx);
    // Knowing the correspondent only ever lowers the verdict, so the
    // lookup is only worth it for mail that would be flagged.
    if verdict.level.is_flagged() && is_correspondent(conn, &f.from_addr)? {
        ctx.known_correspondent = true;
        verdict = phishing::assess(&input, &ctx);
    }
    Ok(verdict)
}

fn save(conn: &Connection, id: i64, v: &PhishingVerdict) -> Result<()> {
    let reasons = (!v.reasons.is_empty())
        .then(|| serde_json::to_string(&v.reasons))
        .transpose()
        .map_err(|e| crate::Error::Other(e.to_string()))?;
    conn.execute(
        "UPDATE messages SET phishing_level = ?2, phishing_score = ?3, phishing_reasons = ?4
         WHERE id = ?1",
        params![id, v.level.as_db(), v.score, reasons],
    )?;
    Ok(())
}

/// Counts the brand-like display-name words of a verified sender, once per
/// mail. Called only for mail that is not suspicious itself, so a forged
/// mail cannot teach that its sender may use the name.
fn learn_names(conn: &Connection, f: &Facts) -> Result<()> {
    if !f.dmarc_pass || !f.incoming() || f.from_name.trim().is_empty() {
        return Ok(());
    }
    // Mail one moved to the trash or junk says nothing about who may use a name.
    if matches!(f.role.as_deref(), Some("trash" | "junk")) {
        return Ok(());
    }
    let tokens = phishing::learnable_name_tokens(&f.from_name, &f.from_addr);
    let Some(base) = crate::linker::base_domain(&f.from_addr) else {
        return Ok(());
    };
    if tokens.is_empty() {
        return Ok(());
    }
    let fresh = conn.execute(
        "INSERT OR IGNORE INTO name_learned (key) VALUES (?1)",
        [&f.key],
    )?;
    if fresh == 0 {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO name_history (token, base_domain, count) VALUES (?1, ?2, 1)
         ON CONFLICT (token, base_domain) DO UPDATE SET count = count + 1",
    )?;
    for token in tokens {
        stmt.execute(params![token, base])?;
    }
    Ok(())
}

/// Assesses a freshly stored row from its headers (the body is not there
/// yet) and learns from it if it is clean and verified.
pub(crate) fn assess_headers(conn: &Connection, id: i64) -> Result<()> {
    let Some(f) = load_facts(conn, id)? else {
        return Ok(());
    };
    if !f.incoming() || is_own_address(conn, &f.from_addr)? {
        return Ok(());
    }
    let verdict = judge(conn, &f, None, None, f.dmarc_pass)?;
    if verdict.level.is_flagged() {
        save(conn, id, &verdict)?;
    } else {
        learn_names(conn, &f)?;
    }
    Ok(())
}

/// Assesses a row again with its body. Replaces the verdict of the headers.
pub(crate) fn assess_with_body(
    conn: &Connection,
    id: i64,
    text: Option<&str>,
    html: Option<&str>,
) -> Result<()> {
    let Some(f) = load_facts(conn, id)? else {
        return Ok(());
    };
    if !f.incoming() || is_own_address(conn, &f.from_addr)? {
        return Ok(());
    }
    let verdict = judge(conn, &f, text, html, f.dmarc_pass)?;
    save(conn, id, &verdict)
}

/// What the one-time pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Mails assessed.
    pub assessed: usize,
    /// ... of which flagged.
    pub flagged: usize,
    /// Older verified mails that only taught names.
    pub learned_from: usize,
}

impl Store {
    /// Whether the one-time pass has run.
    pub fn phishing_backfill_done(&self) -> Result<bool> {
        Ok(self.meta(META_BACKFILL)?.is_some())
    }

    /// One-time pass over existing mail: learns display names from older
    /// verified mail, then assesses the mail of the last [`BACKFILL_DAYS`]
    /// days (with the stored body where there is one). Bounded, run once
    /// (`meta`), and local: it only fills this store's own columns.
    pub fn phishing_backfill(&mut self, now: i64) -> Result<Option<BackfillReport>> {
        self.phishing_backfill_with(now, BACKFILL_MAX_MAILS, LEARN_MAX_MAILS)
    }

    fn phishing_backfill_with(
        &mut self,
        now: i64,
        max_recent: usize,
        max_older: usize,
    ) -> Result<Option<BackfillReport>> {
        if self.phishing_backfill_done()? {
            return Ok(None);
        }
        let recent_since = now - BACKFILL_DAYS * 86_400;
        let report = self.in_transaction(|store| {
            let conn = &store.conn;
            let mut report = BackfillReport::default();

            // Older verified mail first: header-only, to know names before
            // the recent mail is judged. Oldest first, so history grows the
            // way it did in reality.
            let older: Vec<i64> = {
                let mut stmt = conn.prepare(
                    "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
                     WHERE m.dmarc_pass = 1 AND COALESCE(m.date, 0) >= ?1 AND COALESCE(m.date, 0) < ?2
                       AND COALESCE(f.role, '') NOT IN ('sent', 'drafts', 'trash', 'junk')
                     ORDER BY COALESCE(m.date, 0) ASC LIMIT ?3",
                )?;
                stmt.query_map(
                    params![now - LEARN_DAYS * 86_400, recent_since, max_older as i64],
                    |r| r.get(0),
                )?
                .collect::<std::result::Result<_, _>>()?
            };
            for id in older {
                if let Some(f) = load_facts(conn, id)?
                    && f.incoming()
                    && !is_own_address(conn, &f.from_addr)?
                {
                    let v = judge(conn, &f, None, None, f.dmarc_pass)?;
                    if !v.level.is_flagged() {
                        learn_names(conn, &f)?;
                        report.learned_from += 1;
                    }
                }
            }

            // Newest mails matter most when the cap bites: take them, then
            // walk oldest first like history did.
            let mut recent: Vec<i64> = {
                let mut stmt = conn.prepare(
                    "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
                     WHERE COALESCE(m.date, 0) >= ?1
                       AND COALESCE(f.role, '') NOT IN ('sent', 'drafts')
                     ORDER BY COALESCE(m.date, 0) DESC LIMIT ?2",
                )?;
                stmt.query_map(params![recent_since, max_recent as i64], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?
            };
            recent.reverse();
            for id in recent {
                let Some(f) = load_facts(conn, id)? else {
                    continue;
                };
                if !f.incoming() || is_own_address(conn, &f.from_addr)? {
                    continue;
                }
                let (text, html) = match store.body(id)? {
                    Some(raw) => {
                        let c = crate::content::parse(&raw);
                        (c.text, c.html)
                    }
                    None => (
                        conn.query_row("SELECT body_text FROM messages WHERE id = ?1", [id], |r| {
                            r.get::<_, Option<String>>(0)
                        })?,
                        None,
                    ),
                };
                let v = judge(conn, &f, text.as_deref(), html.as_deref(), f.dmarc_pass)?;
                save(conn, id, &v)?;
                report.assessed += 1;
                if v.level.is_flagged() {
                    report.flagged += 1;
                } else {
                    learn_names(conn, &f)?;
                }
            }
            store.set_meta(META_BACKFILL, &now.to_string())?;
            Ok(report)
        })?;
        Ok(Some(report))
    }

    /// Mail flagged since `since` (Unix seconds), newest first.
    pub fn phishing_flagged(&self, since: i64, limit: usize) -> Result<Vec<MessageSummary>> {
        let sql = format!(
            "{SUMMARY_SELECT}
             WHERE m.phishing_level > 0 AND COALESCE(m.date, 0) >= ?1
             ORDER BY COALESCE(m.date, 0) DESC, m.id DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, limit as i64], summary_from_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// "Ich kenne den Absender": a local note, never sent to the server. A
    /// suspicious mail is then no longer held back; a mail called phishing
    /// stays held back (see [`phishing::PhishingInfo::holds_back`]).
    pub fn set_phishing_acknowledged(&self, message_id: i64, on: bool) -> Result<()> {
        let Some(f) = load_facts(&self.conn, message_id)? else {
            return Err(crate::Error::Other("message not found".into()));
        };
        if on {
            self.conn.execute(
                "INSERT OR IGNORE INTO phishing_acknowledged (key, created_at) VALUES (?1, ?2)",
                params![f.key, unix_now()],
            )?;
        } else {
            self.conn
                .execute("DELETE FROM phishing_acknowledged WHERE key = ?1", [&f.key])?;
        }
        Ok(())
    }

    /// Moves messages to the account's junk folder (locally at once, on the
    /// server with the next flush). Fails without any change if an account
    /// has no junk folder: unlike removing a source, this never falls back
    /// to the trash, because the user asked for "Spam".
    pub fn queue_move_to_junk(&mut self, message_ids: &[i64]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut moved = 0;
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
            if role.as_deref() == Some("junk") {
                continue;
            }
            let junk: Option<String> = tx
                .query_row(
                    "SELECT path FROM folders WHERE account_id = ?1 AND role = 'junk' LIMIT 1",
                    [account_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(junk) = junk else {
                return Err(crate::Error::Other(
                    "this account has no junk folder".into(),
                ));
            };
            tx.execute(
                "INSERT INTO pending_ops (account_id, message_id, folder_path, uid, op, arg)
                 VALUES (?1, NULL, ?2, ?3, 'move', ?4)",
                params![account_id, path, uid, junk],
            )?;
            tx.execute("DELETE FROM pending_ops WHERE message_id = ?1", [id])?;
            tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
            moved += 1;
        }
        tx.commit()?;
        Ok(moved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assignments::{Origin, Target};
    use crate::model::{
        Account, AuthMethod, FolderRole, MessageHeader, PendingChange, Security, ServerConfig,
    };
    use crate::notify::{NotifyInput, should_notify};
    use crate::phishing::Level;
    use crate::turn::TurnStatus;

    const ME: &str = "me@example.org";
    const NOW: i64 = 100_000_000;
    const ZW: &str = "\u{200B}";

    struct Fx {
        store: Store,
        inbox: i64,
        junk: i64,
        sent: i64,
        account: i64,
        uid: u32,
    }

    impl Fx {
        fn new() -> Fx {
            let store = Store::open_in_memory().unwrap();
            let a = store
                .add_account(&Account {
                    id: 0,
                    email: ME.into(),
                    display_name: None,
                    imap: ServerConfig {
                        host: "imap.example.org".into(),
                        port: 993,
                        security: Security::Tls,
                        username: ME.into(),
                        auth: AuthMethod::Password,
                    },
                    smtp: None,
                })
                .unwrap();
            let folder = |path: &str, role| {
                store
                    .upsert_folder(a.id, path, None, Some(role), true)
                    .unwrap()
                    .id
            };
            let inbox = folder("INBOX", FolderRole::Inbox);
            let junk = folder("Junk", FolderRole::Junk);
            let sent = folder("Sent", FolderRole::Sent);
            Fx {
                store,
                inbox,
                junk,
                sent,
                account: a.id,
                uid: 0,
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn add_to(
            &mut self,
            folder: i64,
            rfc: &str,
            name: &str,
            from: &str,
            subject: &str,
            verified: bool,
            date: i64,
        ) -> i64 {
            self.uid += 1;
            let uid = self.uid;
            self.store
                .insert_messages(
                    folder,
                    &[MessageHeader {
                        uid,
                        message_id: Some(rfc.into()),
                        subject: Some(subject.into()),
                        from_name: Some(name.into()),
                        from_addr: Some(from.into()),
                        to: vec![ME.into()],
                        date: Some(date),
                        dmarc_pass: verified,
                        ..Default::default()
                    }],
                )
                .unwrap();
            self.store
                .conn
                .query_row(
                    "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                    params![folder, uid],
                    |r| r.get(0),
                )
                .unwrap()
        }

        fn add(&mut self, rfc: &str, name: &str, from: &str, subject: &str, verified: bool) -> i64 {
            self.add_to(self.inbox, rfc, name, from, subject, verified, NOW - 1_000)
        }

        /// Stores an HTML body. The raw message repeats the receiving
        /// server's verdict, as the real one does, because `store_body`
        /// reads it again from there.
        fn html_body(&self, id: i64, html: &str) {
            let (verified, from): (bool, String) = self
                .store
                .conn
                .query_row(
                    "SELECT dmarc_pass, from_addr FROM messages WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            let domain = from.rsplit('@').next().unwrap_or("");
            let auth = if verified {
                format!(
                    "Authentication-Results: mx.example.org; dmarc=pass header.from={domain}\r\n"
                )
            } else {
                String::new()
            };
            let raw = format!(
                "From: x@{domain}\r\n{auth}Subject: x\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
            );
            self.store.store_body(id, raw.as_bytes()).unwrap();
        }

        fn msg(&self, id: i64) -> MessageSummary {
            self.store.message(id).unwrap().unwrap()
        }

        fn history(&self, token: &str) -> Vec<(String, i64)> {
            let mut stmt = self
                .store
                .conn
                .prepare("SELECT base_domain, count FROM name_history WHERE token = ?1 ORDER BY 1")
                .unwrap();
            stmt.query_map([token], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        }

        /// Three verified mails of a genuine sender whose name is a brand.
        fn teach_the_authority(&mut self) {
            for i in 0..3 {
                self.add(
                    &format!("<real{i}@beispielamt.example>"),
                    "Beispielamt Portal",
                    "post@beispielamt.example",
                    "Bescheid zur Steuer",
                    true,
                );
            }
        }
    }

    fn split(word: &str) -> String {
        word.chars().map(|c| format!("{c}{ZW}")).collect()
    }

    /// Body of a lure built from invented parts.
    fn lure_html() -> String {
        format!(
            "<p>{}</p><a href=\"https://exa%6dple.%65xample.invalid/p/1\">Jetzt pruefen</a>",
            split(
                "Ihre Rueckerstattung wartet, bestaetigen Sie sofort Ihre Angaben vor Ablauf der Frist."
            )
        )
    }

    #[test]
    fn verdict_is_stored_when_the_body_is_stored() {
        let mut fx = Fx::new();
        let id = fx.add(
            "<l1@webmail.example.net>",
            "Beispielamt Erstattungsportal",
            "k7x2m9q4vb81@webmail.example.net",
            "Statusabgleich",
            true,
        );
        // Headers alone: an official-sounding name on a random address is
        // not enough on its own.
        assert_eq!(
            fx.msg(id).phishing.level,
            Level::None,
            "{:?}",
            fx.msg(id).phishing
        );
        fx.html_body(id, &lure_html());
        let m = fx.msg(id);
        assert!(m.phishing.level >= Level::Suspicious, "{:?}", m.phishing);
        assert!(!m.phishing.reasons.is_empty());
        assert!(
            m.phishing
                .reasons
                .iter()
                .any(|r| r.code == "link_host_encoded")
        );
        let listed = fx.store.phishing_flagged(0, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);

        // A clean body replaces the verdict.
        fx.html_body(id, "<p>Hallo, anbei die Unterlagen.</p>");
        assert_eq!(
            fx.msg(id).phishing.level,
            Level::None,
            "{:?}",
            fx.msg(id).phishing
        );
        assert!(fx.store.phishing_flagged(0, 10).unwrap().is_empty());
    }

    #[test]
    fn headers_alone_flag_hidden_characters() {
        let mut fx = Fx::new();
        let id = fx.add(
            "<h1@webmail.example.net>",
            &format!("Anna{ZW}Beispiel"),
            "anna@webmail.example.net",
            &format!("Rech{ZW}nung"),
            true,
        );
        assert!(fx.msg(id).phishing.level >= Level::Suspicious);
    }

    #[test]
    fn names_are_learned_only_from_verified_incoming_mail_once() {
        let mut fx = Fx::new();
        fx.add(
            "<a@x.example>",
            "Beispielamt",
            "post@beispielamt.example",
            "Hallo",
            true,
        );
        // Not verified, in the junk folder, sent: no lesson.
        fx.add(
            "<b@x.example>",
            "Beispielamt",
            "post@beispielamt.example",
            "Hallo",
            false,
        );
        fx.add_to(
            fx.junk,
            "<c@x.example>",
            "Beispielamt",
            "post@beispielamt.example",
            "Hallo",
            true,
            NOW,
        );
        fx.add_to(
            fx.sent,
            "<d@x.example>",
            "Beispielamt",
            "post@beispielamt.example",
            "Hallo",
            true,
            NOW,
        );
        assert_eq!(
            fx.history("beispielamt"),
            [("beispielamt.example".to_string(), 1)]
        );

        // The same mail again (new row after a move) counts once.
        let archive = fx
            .store
            .upsert_folder(fx.account, "Archive", None, Some(FolderRole::Archive), true)
            .unwrap()
            .id;
        fx.add_to(
            archive,
            "<a@x.example>",
            "Beispielamt",
            "post@beispielamt.example",
            "Hallo",
            true,
            NOW,
        );
        assert_eq!(fx.history("beispielamt")[0].1, 1);
        // Plain first names are no brand.
        fx.add(
            "<e@x.example>",
            "Anna Beispiel",
            "anna@example.org",
            "Hallo",
            true,
        );
        assert!(fx.history("anna").is_empty());
    }

    #[test]
    fn a_flagged_mail_cannot_teach_its_sender_a_name() {
        let mut fx = Fx::new();
        fx.teach_the_authority();
        let before = fx.history("beispielamt");
        // Verified for the webmail domain, but hidden characters give it away.
        let id = fx.add(
            "<f@webmail.example.net>",
            &format!("Beispiel{ZW}amt Portal"),
            "q8x2k7m4pz91@webmail.example.net",
            "Hallo",
            true,
        );
        assert!(fx.msg(id).phishing.level.is_flagged());
        assert_eq!(fx.history("beispielamt"), before);
    }

    #[test]
    fn a_known_name_from_another_domain_is_held_back_from_the_headers() {
        let mut fx = Fx::new();
        fx.teach_the_authority();
        let genuine = fx.add(
            "<g@beispielamt.example>",
            "Beispielamt Portal",
            "post@beispielamt.example",
            "Neuer Bescheid",
            true,
        );
        assert_eq!(fx.msg(genuine).phishing.level, Level::None);
        let forged = fx.add(
            "<h@webmail.example.net>",
            "Beispielamt Portal",
            "k7x2m9q4vb81@webmail.example.net",
            "Neuer Bescheid",
            true,
        );
        let m = fx.msg(forged);
        assert_eq!(m.phishing.level, Level::Suspicious);
        assert_eq!(m.phishing.reasons[0].code, "name_other_org");
    }

    #[test]
    fn people_you_write_to_keep_their_name() {
        let mut fx = Fx::new();
        fx.teach_the_authority();
        // The user once wrote to this address.
        fx.store
            .insert_messages(
                fx.sent,
                &[MessageHeader {
                    uid: 900,
                    message_id: Some("<out@example.org>".into()),
                    to: vec!["freund@webmail.example.net".into()],
                    date: Some(NOW - 5_000),
                    ..Default::default()
                }],
            )
            .unwrap();
        let id = fx.add(
            "<k@webmail.example.net>",
            "Beispielamt Portal",
            "freund@webmail.example.net",
            "Hallo",
            true,
        );
        assert_eq!(fx.msg(id).phishing.level, Level::None);
    }

    #[test]
    fn flagged_mail_does_not_notify() {
        let mut fx = Fx::new();
        let ok = fx.add(
            "<n1@example.com>",
            "Anna",
            "anna@example.com",
            "Hallo",
            true,
        );
        let bad = fx.add(
            "<n2@example.com>",
            &format!("Ann{ZW}a"),
            "anna@example.com",
            "Hallo",
            true,
        );
        let input = |m: &MessageSummary| NotifyInput {
            dmarc_pass: m.header.dmarc_pass,
            suspicious: m.phishing.level.is_flagged(),
            ..NotifyInput::default()
        };
        assert!(should_notify(&input(&fx.msg(ok))));
        assert!(!should_notify(&input(&fx.msg(bad))));
    }

    #[test]
    fn placement_holds_flagged_mail_back() {
        let mut fx = Fx::new();
        let clean = fx.add(
            "<p1@example.com>",
            "Anna",
            "anna@example.com",
            "Hallo",
            true,
        );
        let bad = fx.add(
            "<p2@example.com>",
            &format!("Ann{ZW}a"),
            "anna@example.com",
            "Hallo",
            true,
        );
        let clean_p = fx.store.placements(&[clean], NOW).unwrap().remove(0);
        assert_ne!(clean_p.origin, Some(Origin::Suspicious));
        let p = fx.store.placements(&[bad], NOW).unwrap().remove(0);
        assert_eq!(p.origin, Some(Origin::Suspicious));
        assert_eq!(p.target, None);
        assert_eq!(p.bundle, None);

        // The user puts it in a bundle: still held back, the wish is noted.
        fx.store
            .apply_assignment(bad, Target::bundle("promo"), &[], &[], NOW)
            .unwrap();
        let p = fx.store.placements(&[bad], NOW).unwrap().remove(0);
        assert_eq!(p.origin, Some(Origin::Suspicious));
        assert_eq!(p.held_back, Some(Target::bundle("promo")));
        assert_eq!(p.bundle, None);
    }

    #[test]
    fn assignments_pull_only_suspicious_mail_into_for_you_and_never_phishing() {
        let mut fx = Fx::new();
        let suspicious = fx.add(
            "<s1@example.com>",
            &format!("Ann{ZW}a"),
            "anna@example.com",
            "Hallo",
            true,
        );
        let phishing = fx.add(
            "<s2@example.com>",
            &format!("Ann{ZW}a"),
            "anna@example.com",
            "Hallo",
            true,
        );
        fx.store
            .conn
            .execute(
                "UPDATE messages SET phishing_level = 2 WHERE id = ?1",
                [phishing],
            )
            .unwrap();
        for id in [suspicious, phishing] {
            fx.store
                .apply_assignment(id, Target::ForYou, &[], &[], NOW)
                .unwrap();
        }
        let s = fx.store.placements(&[suspicious], NOW).unwrap().remove(0);
        assert_eq!(s.target, Some(Target::ForYou));
        assert_ne!(s.origin, Some(Origin::Suspicious));
        let p = fx.store.placements(&[phishing], NOW).unwrap().remove(0);
        assert_eq!(p.target, None);
        assert_eq!(p.origin, Some(Origin::Suspicious));
        assert_eq!(p.held_back, Some(Target::ForYou));
    }

    #[test]
    fn turn_assessment_never_asks_for_an_answer_to_flagged_mail() {
        let mut fx = Fx::new();
        let text = "Bitte bestätigen Sie bis morgen Ihre Angaben. Können Sie uns kurz antworten?";
        let ask = |fx: &mut Fx, rfc: &str, name: &str| {
            let id = fx.add(rfc, name, "anna@example.com", "Rückfrage", true);
            fx.html_body(id, &format!("<p>{text}</p>"));
            id
        };
        let clean = ask(&mut fx, "<t1@example.com>", "Anna");
        let flagged = ask(&mut fx, "<t2@example.com>", &format!("Ann{ZW}a"));
        let status = |id| {
            fx.store.assess_messages_turns(&[id], NOW).unwrap()[0]
                .1
                .status
        };
        assert_eq!(status(clean), TurnStatus::NeedsYou);
        assert_eq!(status(flagged), TurnStatus::ReadingAlong);
    }

    #[test]
    fn acknowledging_releases_suspicious_but_not_phishing_mail() {
        let mut fx = Fx::new();
        let id = fx.add(
            "<a1@example.com>",
            &format!("Ann{ZW}a"),
            "anna@example.com",
            "Hallo",
            true,
        );
        assert!(fx.msg(id).phishing.holds_back());
        fx.store.set_phishing_acknowledged(id, true).unwrap();
        let m = fx.msg(id);
        assert!(m.phishing.acknowledged);
        assert!(!m.phishing.holds_back());
        // The placement and the turn follow.
        let p = fx.store.placements(&[id], NOW).unwrap().remove(0);
        assert_ne!(p.origin, Some(Origin::Suspicious));
        // Phishing stays held back.
        fx.store
            .conn
            .execute("UPDATE messages SET phishing_level = 2 WHERE id = ?1", [id])
            .unwrap();
        assert!(fx.msg(id).phishing.holds_back());
        // It can be taken back.
        fx.store.set_phishing_acknowledged(id, false).unwrap();
        assert!(!fx.msg(id).phishing.acknowledged);
    }

    #[test]
    fn backfill_is_bounded_and_runs_once() {
        let mut fx = Fx::new();
        let day = 86_400;
        // Old verified mail teaches names; it is not assessed itself.
        for i in 0..3 {
            fx.add_to(
                fx.inbox,
                &format!("<o{i}@x.example>"),
                "Beispielamt",
                "post@beispielamt.example",
                "Alt",
                true,
                NOW - 200 * day + i,
            );
        }
        // Recent mails with hidden characters, inserted before the pass by
        // writing the columns' defaults (as an old database would have them).
        let mut recent = Vec::new();
        for i in 0..5 {
            let id = fx.add_to(
                fx.inbox,
                &format!("<r{i}@webmail.example.net>"),
                "Anna",
                "anna@webmail.example.net",
                &format!("Rech{ZW}nung {i}"),
                false,
                NOW - 1_000 * (10 - i),
            );
            recent.push(id);
        }
        fx.store
            .conn
            .execute("UPDATE messages SET phishing_level = 0, phishing_score = 0, phishing_reasons = NULL", [])
            .unwrap();
        fx.store
            .conn
            .execute("DELETE FROM name_history", [])
            .unwrap();
        fx.store
            .conn
            .execute("DELETE FROM name_learned", [])
            .unwrap();

        assert!(!fx.store.phishing_backfill_done().unwrap());
        let report = fx
            .store
            .phishing_backfill_with(NOW, 2, 10)
            .unwrap()
            .unwrap();
        assert_eq!(report.assessed, 2, "{report:?}");
        assert_eq!(report.flagged, 2);
        assert_eq!(report.learned_from, 3);
        assert_eq!(fx.history("beispielamt")[0].1, 3);
        // The two newest got a verdict, the others did not.
        let flagged: Vec<i64> = fx
            .store
            .phishing_flagged(0, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(flagged.len(), 2);
        assert!(flagged.contains(&recent[4]) && flagged.contains(&recent[3]));
        // Second run does nothing.
        assert!(
            fx.store
                .phishing_backfill_with(NOW, 100, 100)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn move_to_junk_queues_a_move_and_needs_a_junk_folder() {
        let mut fx = Fx::new();
        let id = fx.add(
            "<j1@example.com>",
            "Anna",
            "anna@example.com",
            "Hallo",
            true,
        );
        assert_eq!(fx.store.queue_move_to_junk(&[id]).unwrap(), 1);
        assert!(fx.store.message(id).unwrap().is_none());
        let ops = fx.store.pending_ops(fx.account).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].change, PendingChange::MoveTo("Junk".into()));

        // Already in junk: nothing to do. No junk folder: refused, no change.
        let mut other = Fx::new();
        let a = other.add(
            "<j2@example.com>",
            "Anna",
            "anna@example.com",
            "Hallo",
            true,
        );
        other
            .store
            .conn
            .execute("UPDATE folders SET role = NULL WHERE id = ?1", [other.junk])
            .unwrap();
        let err = other.store.queue_move_to_junk(&[a]).unwrap_err();
        assert!(err.to_string().contains("junk"));
        assert!(other.store.message(a).unwrap().is_some());
        assert!(other.store.pending_ops(other.account).unwrap().is_empty());
    }
}
