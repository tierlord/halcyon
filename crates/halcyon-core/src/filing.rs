//! Mail Halcyon has filed on its own, and how that travels to other devices.
//!
//! Two IMAP keywords carry Halcyon's decisions through the mail server, so
//! another client (a phone, say) does not show the same mail again:
//!
//! - [`KEYWORD_FILED`] (`$HalcyonFiled`): Halcyon filed this mail without
//!   anything left for the user to do (an order status step, a settled
//!   invoice, mail of a bundled or muted source). Such mail is also marked
//!   read.
//! - [`KEYWORD_DONE`] (`$HalcyonDone`): the user settled the item in Halcyon
//!   (dismissed a conversation card, set a case to done).
//!
//! Keywords are only stored on folders whose `PERMANENTFLAGS` allow them
//! (see `Folder::keywords`); `\Seen` is a system flag and always works.

use crate::Result;
use crate::cases::CaseStatus;
use crate::extract::otp;
use crate::model::FolderRole;
use crate::sources::SourceMode;
use crate::store::Store;

/// Halcyon filed this mail automatically.
pub const KEYWORD_FILED: &str = "$HalcyonFiled";
/// The user settled this item in Halcyon.
pub const KEYWORD_DONE: &str = "$HalcyonDone";

/// `meta` keys.
const META_SWITCH: &str = "auto_mark_read";
const META_STARTED: &str = "auto_mark_read_since";
/// First run of the ignore pass; older mail is never counted as ignored.
const META_IGNORE_SINCE: &str = "ignore_tracking_since";

/// Unread this long after arrival, an inbox mail counts as ignored.
pub const IGNORE_AFTER_SECS: i64 = 14 * 86_400;

/// Mails judged per run; newer mail than the start time is a small set.
const BATCH: usize = 1000;

/// Whether filed mail is marked read (default on).
pub fn auto_mark_read(store: &Store) -> Result<bool> {
    Ok(store.meta(META_SWITCH)?.as_deref() != Some("0"))
}

pub fn set_auto_mark_read(store: &Store, enabled: bool) -> Result<()> {
    store.set_meta(META_SWITCH, if enabled { "1" } else { "0" })
}

/// How a recurring source stands for one mail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFacts {
    pub mode: SourceMode,
    /// An anomaly needs the user's attention.
    pub needs_you: bool,
}

/// Everything the decision needs to know about one mail.
#[derive(Clone, Debug)]
pub struct FilingFacts {
    pub unread: bool,
    pub folder_role: Option<FolderRole>,
    pub date: i64,
    pub dmarc_pass: bool,
    /// The mail carries a one-time code the user may still need.
    pub has_code: bool,
    /// A reply, i.e. part of a conversation with somebody.
    pub is_reply: bool,
    /// The turn assessment says the user is up (needs you / waiting).
    pub turn_open: bool,
    /// Status of the order case this mail is a status step of.
    pub order_step_of: Option<CaseStatus>,
    /// Some case of this mail needs the user.
    pub case_needs_you: bool,
    /// The ledger invoice made from this mail: settled or not.
    pub invoice_paid: Option<bool>,
    pub source: Option<SourceFacts>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FiledAs {
    OrderStep,
    SettledInvoice,
    QuietSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Disabled,
    AlreadyRead,
    NotInbox,
    BeforeStart,
    Unauthenticated,
    OneTimeCode,
    Conversation,
    TurnOpen,
    NeedsYou,
    OpenInvoice,
    /// Nothing filed it, or its source is shown as is.
    NotFiled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Mark(FiledAs),
    Skip(Skip),
}

/// Whether a mail is marked read. Conservative: any doubt keeps it unread.
/// `since` is the time the feature was first switched on; older mail is
/// never touched, so a backlog is not read in bulk.
pub fn decide(f: &FilingFacts, since: i64, enabled: bool) -> Verdict {
    use Skip::*;
    if !enabled {
        return Verdict::Skip(Disabled);
    }
    if !f.unread {
        return Verdict::Skip(AlreadyRead);
    }
    if f.folder_role != Some(FolderRole::Inbox) {
        return Verdict::Skip(NotInbox);
    }
    if f.date < since {
        return Verdict::Skip(BeforeStart);
    }
    // Forged mail must not look filed, and phishing is exactly what the
    // user should still see.
    if !f.dmarc_pass {
        return Verdict::Skip(Unauthenticated);
    }
    // The code is needed on whichever device the user is holding.
    if f.has_code {
        return Verdict::Skip(OneTimeCode);
    }
    if f.is_reply {
        return Verdict::Skip(Conversation);
    }
    if f.turn_open {
        return Verdict::Skip(TurnOpen);
    }
    if f.case_needs_you || f.source.is_some_and(|s| s.needs_you) {
        return Verdict::Skip(NeedsYou);
    }
    if f.invoice_paid == Some(false) {
        return Verdict::Skip(OpenInvoice);
    }
    if matches!(
        f.order_step_of,
        Some(CaseStatus::Waiting | CaseStatus::Done)
    ) {
        return Verdict::Mark(FiledAs::OrderStep);
    }
    if f.invoice_paid == Some(true) {
        return Verdict::Mark(FiledAs::SettledInvoice);
    }
    if f.source.is_some_and(|s| {
        matches!(
            s.mode,
            SourceMode::Bundle | SourceMode::LatestOnly | SourceMode::Mute
        )
    }) {
        return Verdict::Mark(FiledAs::QuietSource);
    }
    Verdict::Skip(NotFiled)
}

/// A new inbox mail waiting to be judged (see `Store::filing_candidates`).
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub id: i64,
    pub date: i64,
    pub unread: bool,
    pub dmarc_pass: bool,
    pub subject: String,
    pub body_text: Option<String>,
    pub is_reply: bool,
    pub rfc_message_id: Option<String>,
    pub source_id: Option<i64>,
    /// The case linker has looked at the mail with its text.
    pub linked: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilingReport {
    /// Mails judged in this run.
    pub judged: usize,
    /// Mails marked read.
    pub marked: usize,
    /// Cases closed because the user settled them on another device.
    pub settled_cases: usize,
    /// Mails newly counted as ignored (unread for [`IGNORE_AFTER_SECS`]).
    pub ignored: usize,
}

impl FilingReport {
    pub fn changed(&self) -> bool {
        self.marked > 0 || self.settled_cases > 0
    }
}

/// Counts inbox mail that is still unread [`IGNORE_AFTER_SECS`] after it
/// arrived as ignored, once per mail. Only mail dated after the first run
/// counts, so the backlog of old unread mail does not teach anything.
pub fn record_ignored(store: &Store, now: i64) -> Result<usize> {
    let since = match store
        .meta(META_IGNORE_SINCE)?
        .and_then(|v| v.parse::<i64>().ok())
    {
        Some(t) => t,
        None => {
            store.set_meta(META_IGNORE_SINCE, &now.to_string())?;
            now
        }
    };
    let ids = store.unread_inbox_ids_to_ignore(since, now - IGNORE_AFTER_SECS, BATCH)?;
    for &id in &ids {
        store.record_interaction(id, crate::store::Interaction::Ignore, now)?;
    }
    Ok(ids.len())
}

/// Marks mail Halcyon filed without anything left to do as read, and takes
/// over cases the user settled elsewhere. Run it after the case linker and
/// the invoice pass, which produce what it looks at. Each mail is judged
/// once, so a mail the user marks unread again stays unread.
pub fn run(store: &mut Store, now: i64) -> Result<FilingReport> {
    let mut report = FilingReport {
        settled_cases: store.settle_cases_done_by_user()?,
        ignored: record_ignored(store, now)?,
        ..FilingReport::default()
    };
    if !auto_mark_read(store)? {
        return Ok(report);
    }
    // Set on the first run that is allowed to mark anything, never moved.
    let since = match store
        .meta(META_STARTED)?
        .and_then(|v| v.parse::<i64>().ok())
    {
        Some(t) => t,
        None => {
            store.set_meta(META_STARTED, &now.to_string())?;
            now
        }
    };

    let mut judged: Vec<(Candidate, FilingFacts)> = Vec::new();
    for c in store.filing_candidates(since, BATCH)? {
        // Unread mail waits until the linker has seen its text; mail that
        // is read already only needs to be remembered.
        if c.unread && (c.body_text.is_none() || !c.linked) {
            continue;
        }
        let facts = facts_of(store, &c)?;
        judged.push((c, facts));
    }

    // The turn assessment is the costly signal: ask it only for mail that
    // would otherwise be marked, and in one call.
    let probe: Vec<i64> = judged
        .iter()
        .filter(|(_, f)| matches!(decide(f, since, true), Verdict::Mark(_)))
        .map(|(c, _)| c.id)
        .collect();
    let open: std::collections::HashSet<i64> = store
        .assess_messages_turns(&probe, now)?
        .into_iter()
        .filter(|(_, t)| {
            matches!(
                t.status,
                crate::turn::TurnStatus::NeedsYou | crate::turn::TurnStatus::Waiting { .. }
            )
        })
        .map(|(id, _)| id)
        .collect();

    for (c, mut facts) in judged {
        facts.turn_open = open.contains(&c.id);
        store.mark_filing_scanned(c.id)?;
        report.judged += 1;
        if let Verdict::Mark(_) = decide(&facts, since, true) {
            store.queue_flag(&[c.id], "\\Seen", true)?;
            store.queue_flag(&[c.id], KEYWORD_FILED, true)?;
            report.marked += 1;
        }
    }
    Ok(report)
}

fn facts_of(store: &Store, c: &Candidate) -> Result<FilingFacts> {
    let links = store.case_links_of(c.id, c.rfc_message_id.as_deref())?;
    let mut order_step_of = None;
    let mut case_needs_you = false;
    for (kind, status, event) in &links {
        let status: CaseStatus = status.parse().unwrap_or(CaseStatus::NeedsYou);
        case_needs_you |= status == CaseStatus::NeedsYou;
        if kind == "order" && event.starts_with("order.") {
            order_step_of = Some(status);
        }
    }
    let source = match c.source_id {
        Some(id) => store
            .source_of(id, c.id)?
            .map(|(mode, needs_you)| SourceFacts { mode, needs_you }),
        None => None,
    };
    Ok(FilingFacts {
        unread: c.unread,
        folder_role: Some(FolderRole::Inbox),
        date: c.date,
        dmarc_pass: c.dmarc_pass,
        has_code: !otp::find_codes(&c.subject, c.body_text.as_deref().unwrap_or("")).is_empty(),
        is_reply: c.is_reply,
        turn_open: false,
        order_step_of,
        case_needs_you,
        invoice_paid: store.invoice_paid_of(c.id)?,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINCE: i64 = 1_000;

    /// A mail that qualifies as an order step of a waiting case.
    fn order_mail() -> FilingFacts {
        FilingFacts {
            unread: true,
            folder_role: Some(FolderRole::Inbox),
            date: SINCE + 10,
            dmarc_pass: true,
            has_code: false,
            is_reply: false,
            turn_open: false,
            order_step_of: Some(CaseStatus::Waiting),
            case_needs_you: false,
            invoice_paid: None,
            source: None,
        }
    }

    fn source_mail(mode: SourceMode, needs_you: bool) -> FilingFacts {
        FilingFacts {
            order_step_of: None,
            source: Some(SourceFacts { mode, needs_you }),
            ..order_mail()
        }
    }

    fn verdict(f: &FilingFacts) -> Verdict {
        decide(f, SINCE, true)
    }

    #[test]
    fn allows_order_steps_of_cases_that_need_nothing() {
        assert_eq!(verdict(&order_mail()), Verdict::Mark(FiledAs::OrderStep));
        let done = FilingFacts {
            order_step_of: Some(CaseStatus::Done),
            ..order_mail()
        };
        assert_eq!(verdict(&done), Verdict::Mark(FiledAs::OrderStep));
    }

    #[test]
    fn keeps_order_mail_of_cases_that_need_the_user() {
        let f = FilingFacts {
            order_step_of: Some(CaseStatus::NeedsYou),
            case_needs_you: true,
            ..order_mail()
        };
        assert_eq!(verdict(&f), Verdict::Skip(Skip::NeedsYou));
        let rejected = FilingFacts {
            order_step_of: Some(CaseStatus::Rejected),
            ..order_mail()
        };
        assert_eq!(verdict(&rejected), Verdict::Skip(Skip::NotFiled));
    }

    #[test]
    fn allows_settled_invoices_but_not_open_ones() {
        let paid = FilingFacts {
            order_step_of: None,
            invoice_paid: Some(true),
            ..order_mail()
        };
        assert_eq!(verdict(&paid), Verdict::Mark(FiledAs::SettledInvoice));
        let open = FilingFacts {
            invoice_paid: Some(false),
            ..paid
        };
        assert_eq!(verdict(&open), Verdict::Skip(Skip::OpenInvoice));
        // An open invoice is not filed even if its sender is a quiet source.
        let open_in_quiet_source = FilingFacts {
            source: Some(SourceFacts {
                mode: SourceMode::Mute,
                needs_you: false,
            }),
            invoice_paid: Some(false),
            order_step_of: None,
            ..order_mail()
        };
        assert_eq!(
            verdict(&open_in_quiet_source),
            Verdict::Skip(Skip::OpenInvoice)
        );
    }

    #[test]
    fn allows_quiet_sources_only() {
        for mode in [SourceMode::Bundle, SourceMode::LatestOnly, SourceMode::Mute] {
            assert_eq!(
                verdict(&source_mail(mode, false)),
                Verdict::Mark(FiledAs::QuietSource)
            );
        }
        assert_eq!(
            verdict(&source_mail(SourceMode::Show, false)),
            Verdict::Skip(Skip::NotFiled)
        );
        assert_eq!(
            verdict(&source_mail(SourceMode::Mute, true)),
            Verdict::Skip(Skip::NeedsYou)
        );
    }

    #[test]
    fn never_touches_mail_that_may_be_for_the_user() {
        let cases: [(FilingFacts, Skip); 7] = [
            (
                FilingFacts {
                    dmarc_pass: false,
                    ..order_mail()
                },
                Skip::Unauthenticated,
            ),
            (
                FilingFacts {
                    has_code: true,
                    ..order_mail()
                },
                Skip::OneTimeCode,
            ),
            (
                FilingFacts {
                    is_reply: true,
                    ..order_mail()
                },
                Skip::Conversation,
            ),
            (
                FilingFacts {
                    turn_open: true,
                    ..order_mail()
                },
                Skip::TurnOpen,
            ),
            (
                FilingFacts {
                    folder_role: Some(FolderRole::Sent),
                    ..order_mail()
                },
                Skip::NotInbox,
            ),
            (
                FilingFacts {
                    folder_role: Some(FolderRole::Junk),
                    ..order_mail()
                },
                Skip::NotInbox,
            ),
            (
                FilingFacts {
                    folder_role: Some(FolderRole::Archive),
                    ..order_mail()
                },
                Skip::NotInbox,
            ),
        ];
        for (facts, why) in cases {
            assert_eq!(verdict(&facts), Verdict::Skip(why));
        }
    }

    #[test]
    fn leaves_old_and_read_mail_alone() {
        let old = FilingFacts {
            date: SINCE - 1,
            ..order_mail()
        };
        assert_eq!(verdict(&old), Verdict::Skip(Skip::BeforeStart));
        let read = FilingFacts {
            unread: false,
            ..order_mail()
        };
        assert_eq!(verdict(&read), Verdict::Skip(Skip::AlreadyRead));
    }

    #[test]
    fn the_switch_turns_everything_off() {
        assert_eq!(
            decide(&order_mail(), SINCE, false),
            Verdict::Skip(Skip::Disabled)
        );
    }

    // ----- with a store ------------------------------------------------

    use crate::model::{Account, AuthMethod, MessageHeader, PendingChange, Security, ServerConfig};

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

    /// One account with an inbox; `keywords` says whether it takes keywords.
    fn inbox_store(keywords: bool) -> (Store, i64, i64) {
        let store = Store::open_in_memory().unwrap();
        let a = store.add_account(&account("me@example.org")).unwrap();
        let inbox = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        store.set_folder_keywords(inbox.id, keywords).unwrap();
        (store, a.id, inbox.id)
    }

    /// Adds a delivered mail with text and returns its id.
    fn add_mail(store: &mut Store, folder: i64, uid: u32, date: i64, msgid: &str) -> i64 {
        store
            .insert_messages(
                folder,
                &[MessageHeader {
                    uid,
                    message_id: Some(msgid.into()),
                    subject: Some(format!("Newsletter {uid}")),
                    from_addr: Some("news@shop.example.com".into()),
                    date: Some(date),
                    dmarc_pass: true,
                    ..Default::default()
                }],
            )
            .unwrap();
        let id = store
            .conn
            .query_row(
                "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                rusqlite::params![folder, uid],
                |r| r.get(0),
            )
            .unwrap();
        store
            .store_body(
                id,
                b"Subject: x\r\nContent-Type: text/plain\r\n\r\nDas neue Angebot der Woche.\r\n",
            )
            .unwrap();
        store.mark_case_scanned(id, true).unwrap();
        store
            .conn
            .execute("UPDATE messages SET dmarc_pass = 1 WHERE id = ?1", [id])
            .unwrap();
        id
    }

    /// Puts the mail into a source with the given mode.
    fn file_in_source(store: &Store, id: i64, mode: &str) {
        store
            .conn
            .execute(
                "INSERT INTO sources (source_key, display_name, mode) VALUES (?1, 'Shop', ?1)
                 ON CONFLICT (source_key) DO NOTHING",
                [mode],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET source_id = (SELECT id FROM sources WHERE source_key = ?2)
                 WHERE id = ?1",
                rusqlite::params![id, mode],
            )
            .unwrap();
    }

    fn flags_of(store: &Store, id: i64) -> Vec<String> {
        store.message(id).unwrap().unwrap().header.flags
    }

    #[test]
    fn keywords_are_queued_only_where_the_folder_allows_them() {
        let (mut store, account_id, folder) = inbox_store(false);
        let id = add_mail(&mut store, folder, 1, 100, "<a@shop.example.com>");

        assert_eq!(store.queue_flag(&[id], "\\Seen", true).unwrap(), 1);
        assert_eq!(store.queue_flag(&[id], KEYWORD_FILED, true).unwrap(), 0);
        let ops = store.pending_ops(account_id).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].change, PendingChange::AddFlag("\\Seen".into()));
        assert!(!flags_of(&store, id).contains(&KEYWORD_FILED.to_string()));

        store.set_folder_keywords(folder, true).unwrap();
        assert_eq!(store.queue_flag(&[id], KEYWORD_FILED, true).unwrap(), 1);
        let ops = store.pending_ops(account_id).unwrap();
        assert_eq!(ops.len(), 2);
        assert!(ops[1].change.is_keyword());
        assert_eq!(ops[1].change, PendingChange::AddFlag(KEYWORD_FILED.into()));
    }

    #[test]
    fn server_keywords_are_kept_and_reported_like_other_flags() {
        let (mut store, _, folder) = inbox_store(true);
        let id = add_mail(&mut store, folder, 1, 100, "<a@shop.example.com>");
        let n = store
            .apply_server_flags(folder, &[(1, vec!["\\Seen".into(), KEYWORD_FILED.into()])])
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            flags_of(&store, id),
            vec!["$HalcyonFiled".to_string(), "\\Seen".to_string()]
        );
        // Unchanged flags are not reported again.
        let again = store
            .apply_server_flags(folder, &[(1, vec![KEYWORD_FILED.into(), "\\Seen".into()])])
            .unwrap();
        assert_eq!(again, 0);
    }

    #[test]
    fn dismissing_a_turn_round_trips_through_the_keyword() {
        let (mut store, account_id, folder) = inbox_store(true);
        let id = add_mail(&mut store, folder, 1, 100, "<a@shop.example.com>");

        store.dismiss_turn(id).unwrap();
        assert!(flags_of(&store, id).contains(&KEYWORD_DONE.to_string()));
        assert_eq!(
            store.pending_ops(account_id).unwrap()[0].change,
            PendingChange::AddFlag(KEYWORD_DONE.into())
        );

        store.restore_turn(id).unwrap();
        assert!(!flags_of(&store, id).contains(&KEYWORD_DONE.to_string()));
        // The unsent add was replaced by nothing worse than a removal.
        let ops = store.pending_ops(account_id).unwrap();
        assert!(
            ops.iter()
                .all(|o| o.change != PendingChange::AddFlag(KEYWORD_DONE.into()))
        );
    }

    #[test]
    fn a_done_keyword_from_the_server_counts_as_dismissed() {
        let (mut store, _, folder) = inbox_store(true);
        let id = add_mail(&mut store, folder, 1, 100, "<a@shop.example.com>");
        let before = store.assess_messages_turns(&[id], 200).unwrap();
        assert_ne!(before[0].1.reason, "Von dir als erledigt entfernt");

        store
            .apply_server_flags(folder, &[(1, vec![KEYWORD_DONE.into()])])
            .unwrap();
        let after = store.assess_messages_turns(&[id], 200).unwrap();
        assert_eq!(after[0].1.status, crate::turn::TurnStatus::Done);
        assert_eq!(after[0].1.reason, "Von dir als erledigt entfernt");
    }

    fn open_case_with(store: &Store, ids: &[i64]) -> i64 {
        use crate::cases::{CaseEventPhase, CaseKind, NewCase};
        let case = store
            .create_case(&NewCase {
                title: "Bestellung 1".into(),
                counterparty: "Shop".into(),
                domain: None,
                kind: CaseKind::Order,
                status: CaseStatus::Waiting,
                next_step: String::new(),
                due_at: None,
                due_label: None,
                amount_cents: None,
                currency: None,
                reason: String::new(),
            })
            .unwrap();
        for &id in ids {
            store
                .assign_message_to_case(case.case.id, id, "Versandt", CaseEventPhase::Start)
                .unwrap();
        }
        case.case.id
    }

    #[test]
    fn setting_a_case_done_tags_its_mails_and_reopening_removes_the_tag() {
        let (mut store, _, folder) = inbox_store(true);
        let a = add_mail(&mut store, folder, 1, 100, "<a@shop.example.com>");
        let b = add_mail(&mut store, folder, 2, 101, "<b@shop.example.com>");
        let case = open_case_with(&store, &[a, b]);

        store
            .set_case_status_by_user(case, CaseStatus::Done)
            .unwrap();
        for id in [a, b] {
            assert!(flags_of(&store, id).contains(&KEYWORD_DONE.to_string()));
        }
        store
            .set_case_status_by_user(case, CaseStatus::Waiting)
            .unwrap();
        for id in [a, b] {
            assert!(!flags_of(&store, id).contains(&KEYWORD_DONE.to_string()));
        }
    }

    #[test]
    fn a_case_whose_mails_are_all_tagged_is_closed_but_not_a_partly_tagged_one() {
        let (mut store, _, folder) = inbox_store(true);
        let a = add_mail(&mut store, folder, 1, 100, "<a@shop.example.com>");
        let b = add_mail(&mut store, folder, 2, 101, "<b@shop.example.com>");
        let case = open_case_with(&store, &[a, b]);

        store
            .apply_server_flags(folder, &[(1, vec![KEYWORD_DONE.into()]), (2, vec![])])
            .unwrap();
        assert_eq!(store.settle_cases_done_by_user().unwrap(), 0);
        assert_eq!(
            store.get_case(case).unwrap().unwrap().case.status,
            CaseStatus::Waiting
        );

        store
            .apply_server_flags(
                folder,
                &[
                    (1, vec![KEYWORD_DONE.into()]),
                    (2, vec![KEYWORD_DONE.into()]),
                ],
            )
            .unwrap();
        assert_eq!(store.settle_cases_done_by_user().unwrap(), 1);
        assert_eq!(
            store.get_case(case).unwrap().unwrap().case.status,
            CaseStatus::Done
        );
    }

    #[test]
    fn the_pass_marks_new_quiet_mail_once_and_leaves_the_backlog() {
        let (mut store, account_id, folder) = inbox_store(true);
        let now = 10_000;
        let old = add_mail(&mut store, folder, 1, now - 500, "<old@shop.example.com>");
        let fresh = add_mail(&mut store, folder, 2, now + 5, "<new@shop.example.com>");
        let shown = add_mail(&mut store, folder, 3, now + 6, "<show@shop.example.com>");
        for id in [old, fresh] {
            file_in_source(&store, id, "bundle");
        }
        file_in_source(&store, shown, "show");

        let report = run(&mut store, now).unwrap();
        assert_eq!(report.marked, 1);
        let seen = "\\Seen".to_string();
        assert!(flags_of(&store, fresh).contains(&seen));
        assert!(flags_of(&store, fresh).contains(&KEYWORD_FILED.to_string()));
        assert!(!flags_of(&store, old).contains(&seen));
        assert!(!flags_of(&store, shown).contains(&seen));
        assert_eq!(store.pending_ops(account_id).unwrap().len(), 2);

        // The user reads it differently: unread again. Not marked a second time.
        store.queue_flag(&[fresh], "\\Seen", false).unwrap();
        let again = run(&mut store, now + 60).unwrap();
        assert_eq!(again.marked, 0);
        assert!(!flags_of(&store, fresh).contains(&seen));
    }

    #[test]
    fn the_pass_waits_for_the_text_and_respects_the_switch() {
        let (mut store, _, folder) = inbox_store(true);
        let now = 10_000;
        let id = add_mail(&mut store, folder, 1, now + 5, "<new@shop.example.com>");
        file_in_source(&store, id, "mute");
        set_auto_mark_read(&store, false).unwrap();
        assert!(!auto_mark_read(&store).unwrap());
        assert_eq!(run(&mut store, now).unwrap().marked, 0);
        // A switched-off pass does not even start the clock.
        assert_eq!(store.meta(META_STARTED).unwrap(), None);

        set_auto_mark_read(&store, true).unwrap();
        // Mail that arrived while it was off is older than the new start.
        assert_eq!(run(&mut store, now + 100).unwrap().marked, 0);
        let later = add_mail(&mut store, folder, 2, now + 200, "<later@shop.example.com>");
        file_in_source(&store, later, "mute");
        store
            .conn
            .execute("DELETE FROM case_scan WHERE message_id = ?1", [later])
            .unwrap();
        assert_eq!(run(&mut store, now + 210).unwrap().marked, 0);
        store.mark_case_scanned(later, true).unwrap();
        assert_eq!(run(&mut store, now + 220).unwrap().marked, 1);
    }

    #[test]
    fn the_pass_keeps_mail_with_codes_and_open_invoices_unread() {
        let (mut store, _, folder) = inbox_store(true);
        let now = 10_000;
        let code = add_mail(&mut store, folder, 1, now + 5, "<code@shop.example.com>");
        store
            .store_body(
                code,
                b"Subject: x\r\nContent-Type: text/plain\r\n\r\nDein Bestaetigungscode lautet 482913.\r\n",
            )
            .unwrap();
        let open = add_mail(&mut store, folder, 2, now + 6, "<inv@shop.example.com>");
        for id in [code, open] {
            file_in_source(&store, id, "mute");
        }
        store
            .conn
            .execute(
                "INSERT INTO invoices (account_id, issuer, title, issued_at, paid, message_id)
                 SELECT f.account_id, 'Shop', 'Rechnung', 1, 0, ?1
                 FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                [open],
            )
            .unwrap();
        assert_eq!(run(&mut store, now).unwrap().marked, 0);
        assert_eq!(run(&mut store, now + 1).unwrap().judged, 0);
    }

    #[test]
    fn the_pass_files_order_steps_unless_the_case_needs_the_user() {
        use crate::cases::{CaseEventPhase, NewCaseEvent};
        let (mut store, _, folder) = inbox_store(true);
        let now = 10_000;
        let waiting = add_mail(&mut store, folder, 1, now + 5, "<w@shop.example.com>");
        let urgent = add_mail(&mut store, folder, 2, now + 6, "<u@shop.example.com>");
        let w_case = open_case_with(&store, &[]);
        let u_case = open_case_with(&store, &[]);
        for (case, id) in [(w_case, waiting), (u_case, urgent)] {
            store
                .add_case_event(
                    case,
                    &NewCaseEvent {
                        message_id: Some(id),
                        occurred_at: now,
                        kind: Some("order.shipped".into()),
                        label: "Versandt".into(),
                        phase: CaseEventPhase::Start,
                        direction: None,
                        confidence: 1.0,
                        manual: false,
                    },
                )
                .unwrap();
        }
        store.set_case_status(u_case, CaseStatus::NeedsYou).unwrap();

        assert_eq!(run(&mut store, now).unwrap().marked, 1);
        assert!(flags_of(&store, waiting).contains(&"\\Seen".to_string()));
        assert!(!flags_of(&store, urgent).contains(&"\\Seen".to_string()));
    }
}
