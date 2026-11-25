//! Where the user put mail: "Für dich" or a bundle (see [`crate::bundles`]),
//! for one mail or for a *kind* of mail of a sender (see
//! `docs/design/relevanz.md`, task R1, and [`crate::kinds`]).
//!
//! The assignments table is the label store: nothing else records these
//! decisions. Resolution order for one mail:
//!
//! 1. Guard rails ([`Protection`]): protected mail is never put into a
//!    bundle, whatever the user assigned.
//! 2. An assignment of this very mail.
//! 3. The mail is an exception of a kind assignment: no assignment.
//! 4. The assignment of its kind.
//! 5. Nothing assigned: the rules decide, and for mail that is not "Für
//!    dich" by the rules a bundle is chosen automatically
//!    ([`crate::bundles::classify`], origin [`Origin::Auto`]).
//!
//! Effects, deliberately conservative:
//!
//! - A bundle: the turn assessment stops asking for an answer, and no
//!   notification is shown. A bundle that is switched off acts like
//!   "Weitere Mails".
//! - A bundle with `mark_read`: mail of that kind that arrives *after* the
//!   assignment (and after `mark_read` was switched on) is filed quietly
//!   (marked read) by the filing pass. Mail that existed before is never
//!   touched on the server. Automatic placement never changes anything on
//!   the server.
//! - `for_you`: notifies (unless the sender check failed); the placement
//!   lets the UI show the mail under "Für dich".
//!
//! Over the wire (Tauri) a [`Target`] is `{ "kind": "for_you" }` or
//! `{ "kind": "bundle", "bundle": "<key>" }`; in the database it is
//! `for_you` or `bundle:<key>`.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::bundles::{self, AutoInput, Bundle};
use crate::kinds::{self, Kind, KindFeatures, KindMail, OTHER_KEY, OTHER_LABEL};
use crate::sources::SourceMode;
use crate::store::{Store, TurnEnv};
use crate::turn::{TurnAssessment, TurnStatus};
use crate::{Error, Result};

/// Mail of a sender older than this does not shape its kinds or show up in a
/// preview. Matches the horizon of the invoice ledger.
pub const WINDOW_SECS: i64 = 800 * 86_400;

/// Mails listed per group in a preview; the count still says how many exist.
const MAX_IDS_PER_GROUP: usize = 200;
/// Protected mails listed per group.
const MAX_PROTECTED_PER_GROUP: usize = 50;
const EXAMPLES: usize = 3;

/// Where the user wants a mail (or a kind of mail) to go.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// Visible under "Für dich", with a notification.
    ForYou,
    /// A bundle by key (see [`crate::bundles`]).
    Bundle { bundle: String },
}

impl Target {
    pub fn bundle(key: &str) -> Self {
        Target::Bundle {
            bundle: key.to_string(),
        }
    }

    /// The form stored in the database: `for_you` or `bundle:<key>`.
    pub fn as_db(&self) -> String {
        match self {
            Target::ForYou => "for_you".into(),
            Target::Bundle { bundle } => format!("bundle:{bundle}"),
        }
    }

    /// Whether this target takes mail out of the user's way.
    pub fn hides(&self) -> bool {
        matches!(self, Target::Bundle { .. })
    }
}

impl FromStr for Target {
    type Err = Error;

    /// Parses the database form.
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "for_you" => Ok(Target::ForYou),
            other => match other.strip_prefix("bundle:") {
                Some(key) if !key.is_empty() => Ok(Target::bundle(key)),
                _ => Err(Error::Other(format!("unknown target {other:?}"))),
            },
        }
    }
}

/// Where an effective target comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Assigned to this mail itself.
    Mail,
    /// Assigned to the kind of mail.
    Kind,
    /// Chosen by Halcyon; the user assigned nothing.
    Auto,
    /// Held back by the phishing check (see [`crate::phishing`]): neither
    /// "Für dich" nor in a bundle. The UI shows such mail in a quiet line of
    /// its own.
    Suspicious,
}

/// Why a mail is never hidden. The order is the order in which reasons are
/// listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Protection {
    OneTimeCode,
    OrderCase,
    InvoiceCase,
    TrackingNumber,
    InvoiceNumber,
    OpenInvoice,
    Answered,
    NeedsYou,
}

impl Protection {
    /// Short German reason, shown next to a mail set aside in a preview.
    pub fn reason(self) -> &'static str {
        match self {
            Protection::OneTimeCode => "enthält einen Einmalcode",
            Protection::OrderCase => "gehört zu einer Bestellung",
            Protection::InvoiceCase => "gehört zu einer Rechnung",
            Protection::TrackingNumber => "enthält eine Sendungsnummer",
            Protection::InvoiceNumber => "enthält eine Rechnung",
            Protection::OpenInvoice => "Rechnung noch nicht bezahlt",
            Protection::Answered => "du hast darauf geantwortet",
            Protection::NeedsYou => "wartet auf deine Antwort",
        }
    }
}

/// Effective placement of one mail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub message_id: i64,
    /// What the user assigned and applies now; `None` if nothing was
    /// assigned, or if the mail is protected against a bundle. A bundle that
    /// is switched off shows up as `other`.
    pub target: Option<Target>,
    pub origin: Option<Origin>,
    /// When the assignment in effect was made (Unix seconds).
    pub assigned_at: Option<i64>,
    /// A bundle the user assigned that a protection holds back.
    pub held_back: Option<Target>,
    /// The bundle the mail belongs to (assigned or automatic); `None` if it
    /// stays with "Für dich" or the rules.
    pub bundle: Option<String>,
    pub bundle_name: Option<String>,
    /// Short German reason for the placement, for a "Warum?".
    pub reason: Option<String>,
    pub kind_key: String,
    pub kind_label: String,
    pub protected: bool,
    /// Short German reasons, one per protection.
    pub protection: Vec<String>,
}

/// A mail set aside in a preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedMail {
    pub message_id: i64,
    pub subject: String,
    pub date: i64,
    pub reasons: Vec<String>,
}

/// One kind of mail in a preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewGroup {
    pub kind_key: String,
    pub label: String,
    /// Machine names of the shared features (`tracking`, `invoice`, ...).
    pub features: Vec<String>,
    /// All mails of the kind, protected ones included.
    pub count: usize,
    pub protected_count: usize,
    /// Up to three subjects, newest first.
    pub examples: Vec<String>,
    pub first_date: i64,
    pub last_date: i64,
    /// Only the group of the mail the user assigned.
    pub preselected: bool,
    /// The mails an assignment of this kind would take along (not protected),
    /// newest first, at most 200.
    pub message_ids: Vec<i64>,
    /// Mails of the kind that stay as they are, with the reason. Never
    /// preselected.
    pub protected: Vec<ProtectedMail>,
    /// The assignment this kind already has, if any.
    pub assigned: Option<Target>,
}

/// What assigning a mail to a target would do, grouped by kind of mail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignmentPreview {
    pub message_id: i64,
    pub sender: String,
    pub sender_name: Option<String>,
    pub target: Target,
    /// Kind of the mail the user assigned.
    pub kind_key: String,
    /// Largest group first, "Sonstiges" last.
    pub groups: Vec<PreviewGroup>,
    /// Mails of the sender in the preview.
    pub total_mails: usize,
    pub group_count: usize,
    /// Mails an apply of the preselection takes along.
    pub selected_mails: usize,
    /// Protected mails inside the preselected groups.
    pub selected_protected: usize,
    /// Mails in the groups that are not preselected.
    pub other_mails: usize,
    pub protected_total: usize,
}

/// A stored assignment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: i64,
    /// `mail` or `kind`.
    pub scope: String,
    pub target: Target,
    pub created_at: i64,
    pub rfc_message_id: Option<String>,
    pub sender: Option<String>,
    pub kind_key: Option<String>,
    /// Label of the kind (scope `kind`), if the sender's mail still shows it.
    pub kind_label: Option<String>,
    /// Subject of the mail (scope `mail`), if it is still there.
    pub subject: Option<String>,
    /// Mails the user took out of a kind assignment.
    pub exceptions: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Mail,
    Kind,
}

struct AssignmentRow {
    id: i64,
    scope: Scope,
    rfc: Option<String>,
    sender: Option<String>,
    kind_key: Option<String>,
    target: Target,
    created_at: i64,
}

/// A stored mail as far as grouping and protection need it.
#[derive(Clone, Debug)]
struct MailRow {
    id: i64,
    rfc: Option<String>,
    from: String,
    from_name: String,
    subject: String,
    date: i64,
    flags: Vec<String>,
    is_reply: bool,
    body: Option<String>,
    /// `List-Id` or `List-Unsubscribe` present.
    list_mail: bool,
    source_id: Option<i64>,
    /// The phishing check holds the mail back: 0 not, 1 suspicious, 2
    /// phishing. A suspicious mail the user vouched for counts as 0.
    held: u8,
}

const ROW_SELECT: &str = "SELECT m.id, m.message_id, lower(trim(COALESCE(m.from_addr, ''))),
        COALESCE(m.from_name, ''), COALESCE(m.subject, ''), COALESCE(m.date, 0), m.flags,
        m.in_reply_to, m.refs, m.body_text, m.list_id, m.list_unsubscribe, m.source_id,
        CASE WHEN m.phishing_level >= 2 THEN 2
             WHEN m.phishing_level = 1 AND NOT EXISTS (
                 SELECT 1 FROM phishing_acknowledged pa
                 WHERE pa.key = COALESCE(NULLIF(m.message_id, ''), 'f' || m.folder_id || ':' || m.uid)
             ) THEN 1
             ELSE 0 END
     FROM messages m JOIN folders f ON f.id = m.folder_id";

fn mail_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MailRow> {
    let flags: Vec<String> = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
    let refs: Vec<String> = serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default();
    Ok(MailRow {
        id: r.get(0)?,
        rfc: r.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty()),
        from: r.get(2)?,
        from_name: r.get(3)?,
        subject: r.get(4)?,
        date: r.get(5)?,
        flags,
        is_reply: r.get::<_, Option<String>>(7)?.is_some() || !refs.is_empty(),
        body: r.get(9)?,
        list_mail: r
            .get::<_, Option<String>>(10)?
            .is_some_and(|s| !s.is_empty())
            || serde_json::from_str::<Vec<String>>(&r.get::<_, String>(11)?)
                .is_ok_and(|v| !v.is_empty()),
        source_id: r.get(12)?,
        held: r.get(13)?,
    })
}

/// Everything resolution needs besides the mail itself.
struct Ctx<'a> {
    env: &'a TurnEnv,
    now: i64,
    assignments: Vec<AssignmentRow>,
    mail_index: HashMap<String, usize>,
    kind_index: HashMap<(String, String), usize>,
    exceptions: HashMap<i64, HashSet<String>>,
    bundles: HashMap<String, Bundle>,
}

impl<'a> Ctx<'a> {
    fn load(store: &Store, env: &'a TurnEnv, now: i64) -> Result<Self> {
        let assignments = store.assignment_rows()?;
        let mut mail_index = HashMap::new();
        let mut kind_index = HashMap::new();
        for (i, a) in assignments.iter().enumerate() {
            match (a.scope, &a.rfc, &a.sender, &a.kind_key) {
                (Scope::Mail, Some(rfc), _, _) => {
                    mail_index.insert(rfc.clone(), i);
                }
                (Scope::Kind, _, Some(sender), Some(key)) => {
                    kind_index.insert((sender.clone(), key.clone()), i);
                }
                _ => {}
            }
        }
        let mut exceptions: HashMap<i64, HashSet<String>> = HashMap::new();
        let mut stmt = store
            .conn
            .prepare("SELECT assignment_id, rfc_message_id FROM assignment_exceptions")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, rfc) = row?;
            exceptions.entry(id).or_default().insert(rfc);
        }
        let bundles = store
            .bundles()?
            .into_iter()
            .map(|b| (b.key.clone(), b))
            .collect();
        Ok(Ctx {
            env,
            now,
            assignments,
            mail_index,
            kind_index,
            exceptions,
            bundles,
        })
    }

    /// The bundle a key stands for: itself if it exists and is switched on,
    /// otherwise "Weitere Mails".
    fn live_bundle(&self, key: &str) -> Option<&Bundle> {
        self.bundles
            .get(key)
            .filter(|b| b.enabled)
            .or_else(|| self.bundles.get(bundles::OTHER))
    }

    /// What a stored target means now: a bundle that is gone or switched off
    /// acts like "Weitere Mails".
    fn effective(&self, target: &Target) -> Target {
        match target {
            Target::Bundle { bundle } => match self.live_bundle(bundle) {
                Some(b) => Target::bundle(&b.key),
                None => Target::bundle(bundles::OTHER),
            },
            Target::ForYou => Target::ForYou,
        }
    }

    fn is_answered(&self, row: &MailRow) -> bool {
        row.flags.iter().any(|f| f == "\\Answered")
            || row
                .rfc
                .as_deref()
                .and_then(|rfc| self.env.answered.get(rfc))
                .is_some_and(|&at| at >= row.date)
    }

    /// Whether any assignment could apply to this mail; a cheap test before
    /// the costly grouping.
    fn may_apply(&self, row: &MailRow) -> bool {
        row.rfc
            .as_deref()
            .is_some_and(|rfc| self.mail_index.contains_key(rfc))
            || self
                .assignments
                .iter()
                .any(|a| a.scope == Scope::Kind && a.sender.as_deref() == Some(row.from.as_str()))
    }

    /// The assignment of this mail by precedence: its own, else its kind's
    /// unless the mail is an exception.
    fn assignment_for(&self, row: &MailRow, kind_key: &str) -> Option<(Target, Origin, i64)> {
        if let Some(&i) = row.rfc.as_deref().and_then(|rfc| self.mail_index.get(rfc)) {
            let a = &self.assignments[i];
            return Some((self.effective(&a.target), Origin::Mail, a.created_at));
        }
        let &i = self
            .kind_index
            .get(&(row.from.clone(), kind_key.to_string()))?;
        let a = &self.assignments[i];
        let excepted = row.rfc.as_deref().is_some_and(|rfc| {
            self.exceptions
                .get(&a.id)
                .is_some_and(|set| set.contains(rfc))
        });
        (!excepted).then(|| (self.effective(&a.target), Origin::Kind, a.created_at))
    }
}

/// What looking at one mail reveals.
struct Probe {
    features: KindFeatures,
    order_case: bool,
    invoice_case: bool,
    invoice_open: bool,
    answered: bool,
}

/// All mail of a sender, grouped.
struct SenderKinds {
    rows: Vec<MailRow>,
    probes: Vec<Probe>,
    kinds: Vec<Kind>,
    /// Index into `kinds` per row.
    kind_of: Vec<usize>,
    by_id: HashMap<i64, usize>,
    by_rfc: HashMap<String, usize>,
}

impl SenderKinds {
    /// Row index of this mail or of a copy of it.
    fn find(&self, row: &MailRow) -> Option<usize> {
        self.by_id
            .get(&row.id)
            .or_else(|| row.rfc.as_ref().and_then(|rfc| self.by_rfc.get(rfc)))
            .copied()
    }
}

/// What the recurring source of a mail says about it.
struct SourceInfo {
    mode: SourceMode,
    newsletter: bool,
    mostly_promo: bool,
}

impl Store {
    // ----- reading -----------------------------------------------------------

    fn assignment_rows(&self) -> Result<Vec<AssignmentRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, scope, rfc_message_id, sender, kind_key, target, created_at
             FROM assignments ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            let scope: String = r.get(1)?;
            let target: String = r.get(5)?;
            Ok(AssignmentRow {
                id: r.get(0)?,
                scope: if scope == "mail" {
                    Scope::Mail
                } else {
                    Scope::Kind
                },
                rfc: r.get(2)?,
                sender: r.get(3)?,
                kind_key: r.get(4)?,
                // Migration 22 leaves no other value; anything unreadable
                // is treated as the most conservative target.
                target: target.parse().unwrap_or(Target::ForYou),
                created_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn mail_rows_by_ids(&self, ids: &[i64]) -> Result<Vec<MailRow>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{ROW_SELECT} WHERE m.id = ?1"))?;
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(row) = stmt.query_row([id], mail_row).optional()? {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// Mail of one sender in the window, outside junk and drafts, newest first.
    fn sender_rows(&self, sender: &str, since: i64) -> Result<Vec<MailRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "{ROW_SELECT}
             WHERE lower(trim(COALESCE(m.from_addr, ''))) = ?1
               AND COALESCE(m.date, 0) >= ?2
               AND COALESCE(f.role, '') NOT IN ('junk', 'drafts')
               AND m.phishing_level = 0
             ORDER BY COALESCE(m.date, 0) DESC, m.id DESC"
        ))?;
        let rows = stmt.query_map(params![sender, since], mail_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn probe(&self, row: &MailRow, ctx: &Ctx<'_>) -> Result<Probe> {
        let mut features = KindFeatures::detect(&row.subject, row.body.as_deref());
        let links = self.case_links_of(row.id, row.rfc.as_deref())?;
        let live = |kind: &str| {
            links
                .iter()
                .any(|(k, status, _)| k == kind && status != "rejected")
        };
        let order_case = live("order");
        let invoice_case = live("invoice");
        // Placeholder rows (`from_order`) stand for an order without invoice.
        let mut stmt = self.conn.prepare_cached(
            "SELECT paid FROM invoices
             WHERE from_order = 0 AND (message_id = ?1 OR (?2 IS NOT NULL AND rfc_message_id = ?2))",
        )?;
        let paid: Vec<bool> = stmt
            .query_map(params![row.id, row.rfc], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        let answered = ctx.is_answered(row);
        features.has_invoice |= !paid.is_empty();
        features.in_order_case = order_case;
        features.is_reply = row.is_reply || answered;
        Ok(Probe {
            features,
            order_case,
            invoice_case,
            invoice_open: paid.contains(&false),
            answered,
        })
    }

    /// Groups the mail of `sender`; `extra` mails (the ones asked about) join
    /// even if they lie outside the window or in a folder left out.
    fn sender_kinds(
        &self,
        sender: &str,
        since: i64,
        extra: &[&MailRow],
        ctx: &Ctx<'_>,
    ) -> Result<SenderKinds> {
        let mut rows: Vec<MailRow> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut ids: HashSet<i64> = HashSet::new();
        for row in self.sender_rows(sender, since)? {
            // A mail moved to another folder is a new row with the same id.
            if let Some(rfc) = &row.rfc
                && !seen.insert(rfc.clone())
            {
                continue;
            }
            ids.insert(row.id);
            rows.push(row);
        }
        for &row in extra {
            if ids.contains(&row.id) || row.rfc.as_ref().is_some_and(|r| seen.contains(r)) {
                continue;
            }
            if let Some(rfc) = &row.rfc {
                seen.insert(rfc.clone());
            }
            ids.insert(row.id);
            rows.push(row.clone());
        }
        let mut probes = Vec::with_capacity(rows.len());
        for row in &rows {
            probes.push(self.probe(row, ctx)?);
        }
        let input: Vec<KindMail<'_>> = rows
            .iter()
            .zip(&probes)
            .map(|(row, p)| KindMail {
                subject: &row.subject,
                features: p.features,
            })
            .collect();
        let kinds = kinds::group_mails(&input);
        let mut kind_of = vec![0; rows.len()];
        for (k, kind) in kinds.iter().enumerate() {
            for &m in &kind.members {
                kind_of[m] = k;
            }
        }
        let by_id = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        let by_rfc = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.rfc.clone().map(|rfc| (rfc, i)))
            .collect();
        Ok(SenderKinds {
            rows,
            probes,
            kinds,
            kind_of,
            by_id,
            by_rfc,
        })
    }

    /// Whether the sender is someone the user deals with: a thread
    /// participant, a correspondent, or a sender the user has answered.
    fn is_known_sender(&self, row: &MailRow, ctx: &Ctx<'_>) -> Result<bool> {
        Ok(row.is_reply
            || ctx.env.correspondents.contains(&row.from)
            || self.sender_behavior(&row.from, ctx.now)?.is_engaged())
    }

    /// Whether the rules say a known correspondent is waiting for an answer
    /// to this mail.
    fn needs_you_from_known(&self, row: &MailRow, ctx: &Ctx<'_>) -> Result<bool> {
        if !self.is_known_sender(row, ctx)? {
            return Ok(false);
        }
        Ok(self
            .assess_turn_raw(row.id, ctx.env, ctx.now)?
            .is_some_and(|t| t.status == TurnStatus::NeedsYou))
    }

    fn protections(&self, row: &MailRow, probe: &Probe, ctx: &Ctx<'_>) -> Result<Vec<Protection>> {
        let mut out = Vec::new();
        let f = probe.features;
        if f.has_code {
            out.push(Protection::OneTimeCode);
        }
        // Unverified senders (failed or missing DMARC) are deliberately not
        // protected: bundling them is harmless, and most of them are spam.
        // What must not happen is trusting them, and that is guarded where
        // it matters: no notification for `for_you`, no server change in the
        // filing pass.
        if probe.order_case {
            out.push(Protection::OrderCase);
        }
        if probe.invoice_case {
            out.push(Protection::InvoiceCase);
        }
        if f.has_tracking {
            out.push(Protection::TrackingNumber);
        }
        if f.has_invoice {
            out.push(Protection::InvoiceNumber);
        }
        if probe.invoice_open {
            out.push(Protection::OpenInvoice);
        }
        if probe.answered || ctx.is_answered(row) {
            out.push(Protection::Answered);
        }
        if self.needs_you_from_known(row, ctx)? {
            out.push(Protection::NeedsYou);
        }
        Ok(out)
    }

    /// What the recurring source of a mail says about it; cached per call.
    fn source_info<'c>(
        &self,
        id: i64,
        cache: &'c mut HashMap<i64, Option<SourceInfo>>,
    ) -> Result<Option<&'c SourceInfo>> {
        if let Entry::Vacant(slot) = cache.entry(id) {
            let head: Option<(String, bool)> = self
                .conn
                .query_row(
                    "SELECT mode, is_newsletter FROM sources WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let info = match head {
                Some((mode, newsletter)) => {
                    let mut stmt = self.conn.prepare_cached(
                        "SELECT COALESCE(subject, '') FROM messages
                         WHERE source_id = ?1 ORDER BY COALESCE(date, 0) DESC LIMIT 20",
                    )?;
                    let subjects: Vec<String> = stmt
                        .query_map([id], |r| r.get(0))?
                        .collect::<std::result::Result<_, _>>()?;
                    Some(SourceInfo {
                        mode: mode.parse().unwrap_or(SourceMode::Show),
                        newsletter,
                        mostly_promo: bundles::mostly_promo(&subjects),
                    })
                }
                None => None,
            };
            slot.insert(info);
        }
        Ok(cache.get(&id).and_then(Option::as_ref))
    }

    /// The bundle Halcyon picks for a mail nobody assigned; `None` if the
    /// mail stays with the rules. Mail that is meant for the user (their own,
    /// from people they deal with, waiting for an answer) is never bundled.
    fn auto_bundle(
        &self,
        row: &MailRow,
        probe: &Probe,
        ctx: &Ctx<'_>,
        sources: &mut HashMap<i64, Option<SourceInfo>>,
    ) -> Result<Option<bundles::AutoChoice>> {
        if ctx.env.own.contains(&row.from) {
            return Ok(None);
        }
        // List mail is sent to many; the question "does this want me?" is
        // only worth asking for the rest.
        if !row.list_mail {
            if self.is_known_sender(row, ctx)? {
                return Ok(None);
            }
            let wants_answer = self
                .assess_turn_raw(row.id, ctx.env, ctx.now)?
                .is_some_and(|t| t.status == TurnStatus::NeedsYou);
            if wants_answer {
                return Ok(None);
            }
        }
        let source = match row.source_id {
            Some(id) => self.source_info(id, sources)?,
            None => None,
        };
        Ok(bundles::classify(&AutoInput {
            subject: &row.subject,
            body: row.body.as_deref().unwrap_or(""),
            list_mail: row.list_mail,
            source_mode: source.map(|s| s.mode),
            source_newsletter: source.is_some_and(|s| s.newsletter),
            source_mostly_promo: source.is_some_and(|s| s.mostly_promo),
            has_code: probe.features.has_code,
        }))
    }

    /// The placement of mail the phishing check holds back, or `None` for
    /// everything else. Such mail is neither "Für dich" nor in a bundle, so
    /// the UI can keep it in its own place. An assignment never pulls a
    /// phishing mail into "Für dich"; a merely suspicious one may be put
    /// there by the user for this very mail (and only then), because that
    /// is a deliberate act and the check can be wrong.
    fn held_back_placement(&self, row: &MailRow, ctx: &Ctx<'_>) -> Option<Placement> {
        if row.held == 0 {
            return None;
        }
        let candidate = ctx.assignment_for(row, OTHER_KEY);
        if row.held == 1
            && let Some((Target::ForYou, Origin::Mail, _)) = candidate
        {
            return None;
        }
        Some(Placement {
            message_id: row.id,
            target: None,
            origin: Some(Origin::Suspicious),
            assigned_at: None,
            held_back: candidate.map(|(t, _, _)| t),
            bundle: None,
            bundle_name: None,
            reason: Some(
                if row.held == 2 {
                    "Sieht nach Phishing aus; zurückgehalten"
                } else {
                    "Wirkt verdächtig; zurückgehalten"
                }
                .to_string(),
            ),
            kind_key: OTHER_KEY.into(),
            kind_label: OTHER_LABEL.into(),
            protected: false,
            protection: Vec::new(),
        })
    }

    fn placements_for_rows(
        &self,
        rows: &[MailRow],
        ctx: &Ctx<'_>,
        with_auto: bool,
    ) -> Result<Vec<Placement>> {
        let mut by_sender: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, row) in rows.iter().enumerate() {
            by_sender.entry(&row.from).or_default().push(i);
        }
        let since = ctx.now - WINDOW_SECS;
        let mut sources: HashMap<i64, Option<SourceInfo>> = HashMap::new();
        let mut out: Vec<Option<Placement>> = vec![None; rows.len()];
        for (sender, idxs) in by_sender {
            let asked: Vec<&MailRow> = idxs.iter().map(|&i| &rows[i]).collect();
            // Without a sender there is nothing to compare with.
            let kinds = if sender.is_empty() {
                None
            } else {
                Some(self.sender_kinds(sender, since, &asked, ctx)?)
            };
            for &i in &idxs {
                let row = &rows[i];
                if let Some(p) = self.held_back_placement(row, ctx) {
                    out[i] = Some(p);
                    continue;
                }
                let found = kinds
                    .as_ref()
                    .and_then(|sk| sk.find(row).map(|at| (sk, at)));
                let fresh;
                let (kind_key, kind_label, probe): (String, String, &Probe) = match found {
                    Some((sk, at)) => {
                        let kind = &sk.kinds[sk.kind_of[at]];
                        (kind.key.clone(), kind.label.clone(), &sk.probes[at])
                    }
                    None => {
                        fresh = self.probe(row, ctx)?;
                        (OTHER_KEY.into(), OTHER_LABEL.into(), &fresh)
                    }
                };
                let protections = self.protections(row, probe, ctx)?;
                let candidate = ctx.assignment_for(row, &kind_key);
                let protected = !protections.is_empty();
                let (target, held_back) = match &candidate {
                    Some((t, _, _)) if t.hides() && protected => (None, Some(t.clone())),
                    Some((t, _, _)) => (Some(t.clone()), None),
                    None => (None, None),
                };
                let assigned = candidate.as_ref().filter(|_| target.is_some());
                let mut origin = assigned.map(|(_, o, _)| *o);
                let mut reason = assigned.map(|(_, o, _)| {
                    match o {
                        Origin::Mail => "Von dir für diese Mail eingeordnet",
                        _ => "Von dir für diese Art von Mails eingeordnet",
                    }
                    .to_string()
                });
                let mut bundle_key = match &target {
                    Some(Target::Bundle { bundle }) => Some(bundle.clone()),
                    _ => None,
                };
                let auto = if with_auto && candidate.is_none() && !protected {
                    self.auto_bundle(row, probe, ctx, &mut sources)?
                } else {
                    None
                };
                if let Some(choice) = auto
                    && let Some(live) = ctx.live_bundle(choice.bundle)
                {
                    reason = Some(if live.key == choice.bundle {
                        choice.reason
                    } else {
                        // Switched off: said so, instead of a reason that no
                        // longer leads to the bundle.
                        let wanted = ctx
                            .bundles
                            .get(choice.bundle)
                            .map_or(choice.bundle, |b| b.name.as_str());
                        format!(
                            "„{wanted}“ ist ausgeschaltet; die Mail liegt in „{}“",
                            live.name
                        )
                    });
                    origin = Some(Origin::Auto);
                    bundle_key = Some(live.key.clone());
                }
                let bundle_name = bundle_key
                    .as_ref()
                    .and_then(|k| ctx.bundles.get(k))
                    .map(|b| b.name.clone());
                out[i] = Some(Placement {
                    message_id: row.id,
                    target,
                    origin,
                    assigned_at: assigned.map(|(_, _, at)| *at),
                    held_back,
                    bundle: bundle_key,
                    bundle_name,
                    reason,
                    kind_key,
                    kind_label,
                    protected,
                    protection: protections.iter().map(|p| p.reason().to_string()).collect(),
                });
            }
        }
        Ok(out.into_iter().flatten().collect())
    }

    // ----- placements, effects ----------------------------------------------

    /// The effective placement of each mail (see the module documentation for
    /// the order of precedence). Unknown ids are skipped.
    pub fn placements(&self, message_ids: &[i64], now: i64) -> Result<Vec<Placement>> {
        let rows = self.mail_rows_by_ids(message_ids)?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let env = self.turn_env()?;
        let ctx = Ctx::load(self, &env, now)?;
        self.placements_for_rows(&rows, &ctx, true)
    }

    /// Applies the user's assignments to turn assessments: mail in a bundle
    /// no longer needs or awaits an answer. Protected mail keeps the
    /// assessment of the rules.
    pub(crate) fn apply_assignments_to_turns(
        &self,
        turns: &mut [(i64, TurnAssessment)],
        env: &TurnEnv,
        now: i64,
    ) -> Result<()> {
        let open: Vec<i64> = turns
            .iter()
            .filter(|(_, t)| matches!(t.status, TurnStatus::NeedsYou | TurnStatus::Waiting { .. }))
            .map(|(id, _)| *id)
            .collect();
        if open.is_empty() {
            return Ok(());
        }
        let ctx = Ctx::load(self, env, now)?;
        if !ctx.assignments.iter().any(|a| a.target.hides()) {
            return Ok(());
        }
        let rows: Vec<MailRow> = self
            .mail_rows_by_ids(&open)?
            .into_iter()
            .filter(|r| ctx.may_apply(r))
            .collect();
        if rows.is_empty() {
            return Ok(());
        }
        let placements: HashMap<i64, String> = self
            .placements_for_rows(&rows, &ctx, false)?
            .into_iter()
            .filter(|p| p.target.as_ref().is_some_and(Target::hides))
            .filter_map(|p| p.bundle_name.map(|n| (p.message_id, n)))
            .collect();
        for (id, turn) in turns.iter_mut() {
            let Some(name) = placements.get(id) else {
                continue;
            };
            *turn = TurnAssessment {
                status: TurnStatus::ReadingAlong,
                confidence: 1.0,
                reason: format!("Von dir in ‚{name}' eingeordnet"),
            };
        }
        Ok(())
    }

    /// For mail that would be filed quietly because the user put its kind
    /// into a bundle that marks mail read: the time from which mail counts,
    /// the later of the assignment and the moment `mark_read` was switched
    /// on. The filing pass compares it with the date of the mail.
    pub(crate) fn mark_read_assignments(
        &self,
        message_ids: &[i64],
        now: i64,
    ) -> Result<HashMap<i64, i64>> {
        let any: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM bundles WHERE mark_read = 1 AND enabled = 1",
            [],
            |r| r.get(0),
        )?;
        if any == 0 || message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let env = self.turn_env()?;
        let ctx = Ctx::load(self, &env, now)?;
        let rows: Vec<MailRow> = self
            .mail_rows_by_ids(message_ids)?
            .into_iter()
            .filter(|r| ctx.may_apply(r))
            .collect();
        let mut out = HashMap::new();
        for p in self.placements_for_rows(&rows, &ctx, false)? {
            let (Some(Target::Bundle { bundle }), Some(at)) = (&p.target, p.assigned_at) else {
                continue;
            };
            if let Some(b) = ctx.bundles.get(bundle).filter(|b| b.mark_read) {
                out.insert(p.message_id, at.max(b.mark_read_since));
            }
        }
        Ok(out)
    }

    // ----- preview -----------------------------------------------------------

    /// All mail of the sender of `message_id`, grouped by kind, for the scene
    /// that asks "should this apply to the others, too?".
    pub fn assignment_preview(
        &self,
        message_id: i64,
        target: Target,
        now: i64,
    ) -> Result<AssignmentPreview> {
        let row = self
            .mail_rows_by_ids(&[message_id])?
            .pop()
            .ok_or_else(|| Error::Other("message not found".into()))?;
        let env = self.turn_env()?;
        let ctx = Ctx::load(self, &env, now)?;
        let sk = self.sender_kinds(&row.from, now - WINDOW_SECS, &[&row], &ctx)?;
        let own = sk
            .find(&row)
            .map(|at| sk.kind_of[at])
            .ok_or_else(|| Error::Other("message not found".into()))?;

        // Newest first inside a group.
        let mut protected_total = 0;
        let mut groups = Vec::with_capacity(sk.kinds.len());
        for (k, kind) in sk.kinds.iter().enumerate() {
            let mut members = kind.members.clone();
            members.sort_by(|&a, &b| {
                sk.rows[b]
                    .date
                    .cmp(&sk.rows[a].date)
                    .then(sk.rows[b].id.cmp(&sk.rows[a].id))
            });
            let mut message_ids = Vec::new();
            let mut protected = Vec::new();
            let mut protected_count = 0;
            for &m in &members {
                let r = &sk.rows[m];
                let reasons = self.protections(r, &sk.probes[m], &ctx)?;
                if reasons.is_empty() {
                    if message_ids.len() < MAX_IDS_PER_GROUP {
                        message_ids.push(r.id);
                    }
                } else {
                    protected_count += 1;
                    if protected.len() < MAX_PROTECTED_PER_GROUP {
                        protected.push(ProtectedMail {
                            message_id: r.id,
                            subject: r.subject.clone(),
                            date: r.date,
                            reasons: reasons.iter().map(|p| p.reason().to_string()).collect(),
                        });
                    }
                }
            }
            protected_total += protected_count;
            let mut examples: Vec<String> = Vec::new();
            for &m in &members {
                let subject = sk.rows[m].subject.trim();
                if !subject.is_empty() && !examples.iter().any(|e| e == subject) {
                    examples.push(subject.to_string());
                    if examples.len() == EXAMPLES {
                        break;
                    }
                }
            }
            let dates = members.iter().map(|&m| sk.rows[m].date);
            groups.push(PreviewGroup {
                kind_key: kind.key.clone(),
                label: kind.label.clone(),
                features: kind.features.iter().map(|f| f.to_string()).collect(),
                count: members.len(),
                protected_count,
                examples,
                first_date: dates.clone().min().unwrap_or(0),
                last_date: dates.max().unwrap_or(0),
                preselected: k == own,
                message_ids,
                protected,
                assigned: ctx
                    .kind_index
                    .get(&(row.from.clone(), kind.key.clone()))
                    .map(|&i| ctx.assignments[i].target.clone()),
            });
        }
        let total_mails = sk.rows.len();
        let selected = &groups[own];
        let selected_mails = selected.count - selected.protected_count;
        let selected_protected = selected.protected_count;
        Ok(AssignmentPreview {
            message_id,
            sender: row.from.clone(),
            sender_name: Some(row.from_name.clone()).filter(|n| !n.is_empty()),
            target,
            kind_key: sk.kinds[own].key.clone(),
            group_count: groups.len(),
            other_mails: total_mails - selected.count,
            selected_mails,
            selected_protected,
            protected_total,
            total_mails,
            groups,
        })
    }

    // ----- writing -----------------------------------------------------------

    /// Records an assignment. Without `kind_keys` it is for this mail alone;
    /// otherwise one assignment per kind of the mail's sender, with
    /// `excluded_message_ids` taken out as exceptions. Assigning again
    /// replaces the earlier assignment of the same mail or kind. Returns the
    /// ids of the assignments, for [`Store::remove_assignment`] (undo).
    ///
    /// Nothing is queued for the server: only what arrives later is acted on.
    pub fn apply_assignment(
        &self,
        message_id: i64,
        target: Target,
        kind_keys: &[String],
        excluded_message_ids: &[i64],
        now: i64,
    ) -> Result<Vec<i64>> {
        let row = self
            .mail_rows_by_ids(&[message_id])?
            .pop()
            .ok_or_else(|| Error::Other("message not found".into()))?;
        if let Target::Bundle { bundle } = &target
            && !self.bundles()?.iter().any(|b| &b.key == bundle)
        {
            return Err(Error::Other(format!("unknown bundle {bundle:?}")));
        }
        let target_db = target.as_db();
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = Vec::new();
        if kind_keys.is_empty() {
            let rfc = row
                .rfc
                .as_deref()
                .ok_or_else(|| Error::Other("message has no Message-ID".into()))?;
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM assignments WHERE scope = 'mail' AND rfc_message_id = ?1",
                    [rfc],
                    |r| r.get(0),
                )
                .optional()?;
            ids.push(match existing {
                Some(id) => {
                    tx.execute(
                        "UPDATE assignments SET target = ?2, created_at = ?3 WHERE id = ?1",
                        params![id, target_db, now],
                    )?;
                    id
                }
                None => {
                    tx.execute(
                        "INSERT INTO assignments (scope, rfc_message_id, target, created_at)
                         VALUES ('mail', ?1, ?2, ?3)",
                        params![rfc, target_db, now],
                    )?;
                    tx.last_insert_rowid()
                }
            });
        } else {
            if row.from.is_empty() {
                return Err(Error::Other("message has no sender".into()));
            }
            let env = self.turn_env()?;
            let ctx = Ctx::load(self, &env, now)?;
            let sk = self.sender_kinds(&row.from, now - WINDOW_SECS, &[&row], &ctx)?;
            let excluded = self.mail_rows_by_ids(excluded_message_ids)?;
            for key in kind_keys {
                let Some(kind) = sk.kinds.iter().find(|k| &k.key == key) else {
                    return Err(Error::Other(format!("unknown kind {key:?}")));
                };
                let existing: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM assignments
                         WHERE scope = 'kind' AND sender = ?1 AND kind_key = ?2",
                        params![row.from, key],
                        |r| r.get(0),
                    )
                    .optional()?;
                let id = match existing {
                    Some(id) => {
                        tx.execute(
                            "UPDATE assignments SET target = ?2, created_at = ?3 WHERE id = ?1",
                            params![id, target_db, now],
                        )?;
                        // Exceptions belonged to the earlier decision.
                        tx.execute(
                            "DELETE FROM assignment_exceptions WHERE assignment_id = ?1",
                            [id],
                        )?;
                        id
                    }
                    None => {
                        tx.execute(
                            "INSERT INTO assignments (scope, sender, kind_key, target, created_at)
                             VALUES ('kind', ?1, ?2, ?3, ?4)",
                            params![row.from, key, target_db, now],
                        )?;
                        tx.last_insert_rowid()
                    }
                };
                // Only mails that really belong to this kind count.
                for ex in &excluded {
                    let Some(rfc) = &ex.rfc else { continue };
                    let in_kind = sk.find(ex).is_some_and(|at| kind.members.contains(&at));
                    if in_kind {
                        tx.execute(
                            "INSERT OR IGNORE INTO assignment_exceptions
                             (assignment_id, rfc_message_id) VALUES (?1, ?2)",
                            params![id, rfc],
                        )?;
                    }
                }
                ids.push(id);
            }
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Takes an assignment back (undo). An assignment it replaced is not
    /// restored.
    pub fn remove_assignment(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM assignments WHERE id = ?1", [id])?;
        Ok(())
    }

    /// All assignments, newest first.
    pub fn list_assignments(&self) -> Result<Vec<Assignment>> {
        self.list_assignments_at(crate::store::unix_now())
    }

    fn list_assignments_at(&self, now: i64) -> Result<Vec<Assignment>> {
        let env = self.turn_env()?;
        let ctx = Ctx::load(self, &env, now)?;
        let mut labels: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut out = Vec::new();
        for a in ctx.assignments.iter().rev() {
            let (mut kind_label, mut subject) = (None, None);
            match a.scope {
                Scope::Mail => {
                    subject = a
                        .rfc
                        .as_deref()
                        .map(|rfc| {
                            self.conn
                                .query_row(
                                    "SELECT subject FROM messages WHERE message_id = ?1 LIMIT 1",
                                    [rfc],
                                    |r| r.get::<_, Option<String>>(0),
                                )
                                .optional()
                        })
                        .transpose()?
                        .flatten()
                        .flatten();
                }
                Scope::Kind => {
                    if let (Some(sender), Some(key)) = (&a.sender, &a.kind_key) {
                        if !labels.contains_key(sender) {
                            let sk = self.sender_kinds(sender, now - WINDOW_SECS, &[], &ctx)?;
                            labels.insert(
                                sender.clone(),
                                sk.kinds
                                    .iter()
                                    .map(|k| (k.key.clone(), k.label.clone()))
                                    .collect(),
                            );
                        }
                        kind_label = labels.get(sender).and_then(|m| m.get(key)).cloned();
                    }
                }
            }
            out.push(Assignment {
                id: a.id,
                scope: if a.scope == Scope::Mail {
                    "mail"
                } else {
                    "kind"
                }
                .into(),
                target: a.target.clone(),
                created_at: a.created_at,
                rfc_message_id: a.rfc.clone(),
                sender: a.sender.clone(),
                kind_key: a.kind_key.clone(),
                kind_label,
                subject,
                exceptions: ctx.exceptions.get(&a.id).map_or(0, HashSet::len),
            });
        }
        Ok(out)
    }

    // ----- measuring ---------------------------------------------------------

    /// How well the grouping separates kinds of mail in this mailbox. Reads
    /// only; nothing is written. `top` limits the survey to the senders with
    /// the most mail, `sender` to one sender.
    pub fn survey_kinds(&self, now: i64, scope: &SurveyScope) -> Result<KindSurvey> {
        let env = self.turn_env()?;
        let ctx = Ctx::load(self, &env, now)?;
        let since = now - WINDOW_SECS;
        let mut survey = KindSurvey::default();
        let tally = |sk: &SenderKinds, survey: &mut KindSurvey| -> Result<()> {
            let mut groups = Vec::with_capacity(sk.kinds.len());
            let mut protected_mails = 0;
            for kind in &sk.kinds {
                let mut protected = 0;
                for &m in &kind.members {
                    if !self
                        .protections(&sk.rows[m], &sk.probes[m], &ctx)?
                        .is_empty()
                    {
                        protected += 1;
                    }
                }
                protected_mails += protected;
                let mut members = kind.members.clone();
                members.sort_by_key(|&m| std::cmp::Reverse(sk.rows[m].date));
                let examples = if scope.examples {
                    let mut e: Vec<String> = Vec::new();
                    for &m in &members {
                        let s = sk.rows[m].subject.trim();
                        if !s.is_empty() && !e.iter().any(|x| x == s) {
                            e.push(s.to_string());
                        }
                        if e.len() == 2 {
                            break;
                        }
                    }
                    e
                } else {
                    Vec::new()
                };
                groups.push(GroupSurvey {
                    label: kind.label.clone(),
                    mails: kind.members.len(),
                    features: kind.features.iter().map(|f| f.to_string()).collect(),
                    protected,
                    examples,
                });
            }
            let st = &mut survey.stats;
            st.senders += 1;
            st.mails += sk.rows.len();
            *st.groups_per_sender.entry(groups.len()).or_default() += 1;
            st.other_mails += groups
                .iter()
                .zip(&sk.kinds)
                .filter(|(_, k)| k.key == OTHER_KEY)
                .map(|(g, _)| g.mails)
                .sum::<usize>();
            if groups.len() > 1 {
                st.multi_group_senders += 1;
            }
            if sk.rows.len() >= 10 {
                st.busy_senders += 1;
                if groups.len() > 1 {
                    st.busy_multi_group_senders += 1;
                }
            }
            st.protected_mails += protected_mails;
            if scope.examples {
                survey.senders.push(SenderSurvey {
                    sender: sk.rows.first().map(|r| r.from.clone()).unwrap_or_default(),
                    mails: sk.rows.len(),
                    groups,
                });
            }
            Ok(())
        };

        let senders = self.survey_senders(&env, since, scope)?;
        for sender in senders {
            let sk = self.sender_kinds(&sender, since, &[], &ctx)?;
            if !sk.rows.is_empty() {
                tally(&sk, &mut survey)?;
            }
        }
        Ok(survey)
    }

    /// Where the mail of the last `days` days in the inbox would go on its
    /// own. Reads only; nothing is written. With `examples`, up to three
    /// subjects per bundle (the user's own content, newest first).
    pub fn survey_bundles(&self, now: i64, days: i64, examples: bool) -> Result<BundleSurvey> {
        let since = now - days * 86_400;
        let ids: Vec<i64> = {
            let mut stmt = self.conn.prepare(
                "SELECT m.id FROM messages m JOIN folders f ON f.id = m.folder_id
                 WHERE f.role = 'inbox' AND COALESCE(m.date, 0) >= ?1
                 ORDER BY COALESCE(m.date, 0) DESC, m.id DESC",
            )?;
            stmt.query_map([since], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let rows = self.mail_rows_by_ids(&ids)?;
        let env = self.turn_env()?;
        let ctx = Ctx::load(self, &env, now)?;
        let placements = self.placements_for_rows(&rows, &ctx, true)?;

        // Senders none of whose mail ever passed the sender check.
        let unverified: HashSet<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT lower(trim(COALESCE(from_addr, ''))) FROM messages
                 GROUP BY 1 HAVING MAX(dmarc_pass) = 0",
            )?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        let rows_by_id: HashMap<i64, &MailRow> = rows.iter().map(|r| (r.id, r)).collect();

        let mut survey = BundleSurvey {
            total: placements.len(),
            ..Default::default()
        };
        let mut tallies: Vec<BundleTally> = self
            .bundles()?
            .into_iter()
            .map(|b| BundleTally {
                key: b.key,
                name: b.name,
                enabled: b.enabled,
                mails: 0,
                examples: Vec::new(),
            })
            .collect();
        // `placements_for_rows` does not keep the order of `rows`; the
        // examples want newest first.
        let mut ordered: Vec<&Placement> = placements.iter().collect();
        ordered.sort_by_key(|p| {
            std::cmp::Reverse(
                rows_by_id
                    .get(&p.message_id)
                    .map_or((0, 0), |r| (r.date, r.id)),
            )
        });
        for p in ordered {
            let row = rows_by_id.get(&p.message_id).copied();
            match (&p.origin, &p.bundle) {
                (Some(Origin::Auto), Some(key)) => {
                    let Some(t) = tallies.iter_mut().find(|t| &t.key == key) else {
                        continue;
                    };
                    t.mails += 1;
                    if let Some(row) = row {
                        if unverified.contains(&row.from) {
                            survey.unverified += 1;
                        }
                        let subject = row.subject.trim();
                        if examples
                            && t.examples.len() < EXAMPLES
                            && !subject.is_empty()
                            && !t.examples.iter().any(|e| e == subject)
                        {
                            t.examples.push(subject.to_string());
                        }
                    }
                }
                (Some(Origin::Mail | Origin::Kind), _) => survey.assigned += 1,
                _ => survey.stays_visible += 1,
            }
        }
        survey.bundles = tallies;
        Ok(survey)
    }

    /// Senders to survey, most mail first (ties by address).
    fn survey_senders(
        &self,
        env: &TurnEnv,
        since: i64,
        scope: &SurveyScope,
    ) -> Result<Vec<String>> {
        if let Some(sender) = &scope.sender {
            return Ok(vec![sender.trim().to_ascii_lowercase()]);
        }
        let mut stmt = self.conn.prepare(
            "SELECT lower(trim(m.from_addr)), COUNT(DISTINCT COALESCE(m.message_id, m.id))
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE COALESCE(m.date, 0) >= ?1
               AND COALESCE(f.role, '') NOT IN ('junk', 'drafts')
               AND trim(COALESCE(m.from_addr, '')) != ''
             GROUP BY lower(trim(m.from_addr))
             HAVING ?2 OR MAX(m.dmarc_pass) != 0
             ORDER BY 2 DESC, 1 ASC",
        )?;
        let mut senders: Vec<String> = stmt
            .query_map(params![since, scope.include_unverified], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<std::result::Result<_, _>>()?;
        senders.retain(|s| !env.own.contains(s));
        if let Some(top) = scope.top {
            senders.truncate(top);
        }
        Ok(senders)
    }
}

/// One bundle in [`BundleSurvey`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleTally {
    pub key: String,
    pub name: String,
    pub enabled: bool,
    /// Mails that fall into the bundle on their own.
    pub mails: usize,
    /// Up to three subjects, newest first; empty unless asked for.
    pub examples: Vec<String>,
}

/// Where inbox mail goes without any decision of the user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BundleSurvey {
    /// Inbox mails looked at.
    pub total: usize,
    /// All bundles in display order, empty ones included.
    pub bundles: Vec<BundleTally>,
    /// Mails the user put somewhere.
    pub assigned: usize,
    /// Mails the rules leave visible (for the user, protected, own).
    pub stays_visible: usize,
    /// Automatically bundled mails from senders that never passed the sender
    /// check; included in `bundles`.
    pub unverified: usize,
}

/// Which senders [`Store::survey_kinds`] looks at.
#[derive(Clone, Debug, Default)]
pub struct SurveyScope {
    /// Only the senders with the most mail.
    pub top: Option<usize>,
    /// Only this sender.
    pub sender: Option<String>,
    /// Keep example subjects (they are the user's own content).
    pub examples: bool,
    /// Also senders that never passed DMARC. Off by default: in a real
    /// mailbox the busiest of them are spam campaigns, which would push
    /// every real sender out of the listing.
    pub include_unverified: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSurvey {
    pub label: String,
    pub mails: usize,
    pub features: Vec<String>,
    pub protected: usize,
    /// Up to two subjects, newest first; empty unless asked for.
    pub examples: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SenderSurvey {
    pub sender: String,
    pub mails: usize,
    pub groups: Vec<GroupSurvey>,
}

/// Numbers only: nothing in here reveals a subject or an address.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurveyStats {
    pub senders: usize,
    pub mails: usize,
    /// Number of groups -> number of senders with that many.
    pub groups_per_sender: BTreeMap<usize, usize>,
    pub other_mails: usize,
    pub multi_group_senders: usize,
    /// Senders with at least ten mails: the ones where grouping can matter.
    pub busy_senders: usize,
    pub busy_multi_group_senders: usize,
    pub protected_mails: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KindSurvey {
    /// Filled when examples were asked for.
    pub senders: Vec<SenderSurvey>,
    pub stats: SurveyStats,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cases::{CaseEventPhase, CaseKind, CaseStatus, NewCase, NewCaseEvent};
    use crate::model::{Account, AuthMethod, FolderRole, MessageHeader, Security, ServerConfig};

    const ME: &str = "me@example.org";
    const SHOP: &str = "shop@example.com";
    /// Assignments are made at this time; fixtures sit before or after it.
    const ASSIGNED_AT: i64 = 2_000_000;
    const NOW: i64 = 3_000_000;

    struct Fx {
        store: Store,
        inbox: i64,
        account: i64,
        next_uid: u32,
    }

    impl Fx {
        fn new() -> Self {
            let mut store = Store::open_in_memory().unwrap();
            let account = store
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
            let inbox = store
                .upsert_folder(account.id, "INBOX", None, Some(FolderRole::Inbox), true)
                .unwrap();
            store.set_folder_keywords(inbox.id, true).unwrap();
            let _ = &mut store;
            Fx {
                store,
                inbox: inbox.id,
                account: account.id,
                next_uid: 1,
            }
        }

        /// A plain, verified mail without text.
        fn add(&mut self, from: &str, subject: &str, date: i64) -> i64 {
            self.add_with(from, subject, date, |_| {})
        }

        fn add_with(
            &mut self,
            from: &str,
            subject: &str,
            date: i64,
            tweak: impl FnOnce(&mut MessageHeader),
        ) -> i64 {
            let uid = self.next_uid;
            self.next_uid += 1;
            let mut h = MessageHeader {
                uid,
                message_id: Some(format!("<m{uid}@example.net>")),
                subject: Some(subject.into()),
                from_addr: Some(from.into()),
                to: vec![ME.into()],
                date: Some(date),
                dmarc_pass: true,
                ..Default::default()
            };
            tweak(&mut h);
            self.store.insert_messages(self.inbox, &[h]).unwrap();
            self.store
                .folder_messages(self.inbox, 1000)
                .unwrap()
                .iter()
                .find(|m| m.header.uid == uid)
                .unwrap()
                .id
        }

        fn body(&self, id: i64, text: &str) {
            self.store
                .conn
                .execute(
                    "UPDATE messages SET body_text = ?2 WHERE id = ?1",
                    params![id, text],
                )
                .unwrap();
        }

        fn rfc(&self, id: i64) -> String {
            self.store
                .message(id)
                .unwrap()
                .unwrap()
                .header
                .message_id
                .unwrap()
        }

        fn placement(&self, id: i64) -> Placement {
            self.store.placements(&[id], NOW).unwrap().remove(0)
        }

        /// Offers of the shop that merge into one kind, and a shipping notice
        /// with a tracking number.
        fn shop(&mut self) -> (Vec<i64>, i64) {
            let offers = vec![
                self.add(SHOP, "Angebot: rotes Fahrrad 28 Zoll", 1_000_000),
                self.add(SHOP, "Angebot: Kinderwagen", 1_100_000),
                self.add(SHOP, "Angebot: Gartenstuhl", 1_200_000),
            ];
            let shipping = self.add(SHOP, "Versandbestätigung für Bestellung 4711", 1_300_000);
            self.body(shipping, &format!("Sendungsnummer: {}", ups_number()));
            (offers, shipping)
        }
    }

    /// A syntactically valid UPS tracking number (checksum included).
    fn ups_number() -> String {
        let payload = "999AA1012345678";
        let sum: u32 = payload
            .bytes()
            .enumerate()
            .map(|(i, b)| {
                let v = match b {
                    b'0'..=b'9' => (b - b'0') as u32,
                    _ => ((b - b'A' + 2) % 10) as u32,
                };
                v * if i % 2 == 0 { 1 } else { 2 }
            })
            .sum();
        format!("1Z{payload}{}", (10 - sum % 10) % 10)
    }

    fn promo() -> Target {
        Target::bundle("promo")
    }

    fn misc() -> Target {
        Target::bundle("other")
    }

    fn kind_keys(preview: &AssignmentPreview) -> Vec<String> {
        preview
            .groups
            .iter()
            .filter(|g| g.preselected)
            .map(|g| g.kind_key.clone())
            .collect()
    }

    #[test]
    fn placement_reports_kind_and_nothing_assigned() {
        let mut fx = Fx::new();
        let (offers, shipping) = fx.shop();
        let p = fx.placement(offers[0]);
        assert_eq!(p.target, None);
        // Nothing assigned: the bundle comes from the signals.
        assert_eq!(p.origin, Some(Origin::Auto));
        assert_eq!(p.bundle.as_deref(), Some("other"));
        assert_eq!(p.kind_label, "Angebot: …");
        assert!(!p.protected);
        assert_eq!(p.kind_key, fx.placement(offers[2]).kind_key);
        let s = fx.placement(shipping);
        assert_ne!(s.kind_key, p.kind_key);
        assert!(s.protected);
    }

    #[test]
    fn precedence_mail_over_exception_over_kind() {
        let mut fx = Fx::new();
        let (offers, _) = fx.shop();
        let preview = fx
            .store
            .assignment_preview(offers[0], promo(), NOW)
            .unwrap();
        // The user takes the third offer out and puts the second on "want".
        fx.store
            .apply_assignment(
                offers[0],
                promo(),
                &kind_keys(&preview),
                &[offers[2]],
                ASSIGNED_AT,
            )
            .unwrap();
        fx.store
            .apply_assignment(offers[1], Target::ForYou, &[], &[], ASSIGNED_AT)
            .unwrap();

        let first = fx.placement(offers[0]);
        assert_eq!(first.target, Some(promo()));
        assert_eq!(first.origin, Some(Origin::Kind));
        assert_eq!(first.assigned_at, Some(ASSIGNED_AT));
        let second = fx.placement(offers[1]);
        assert_eq!(second.target, Some(Target::ForYou));
        assert_eq!(second.origin, Some(Origin::Mail));
        let third = fx.placement(offers[2]);
        assert_eq!(third.target, None);
        assert_eq!(third.origin, Some(Origin::Auto));
    }

    #[test]
    fn every_protection_has_a_reason_and_holds_hiding_back() {
        let mut fx = Fx::new();
        let sender = "service@example.com";
        let code = fx.add(sender, "Dein Code", 1_000_000);
        fx.body(code, "Dein Bestätigungscode lautet 482913.");
        let forged = fx.add_with(sender, "Hinweis", 1_000_001, |h| h.dmarc_pass = false);
        let tracking = fx.add(sender, "Paket unterwegs", 1_000_002);
        fx.body(tracking, &format!("Sendungsnummer: {}", ups_number()));
        let invoice_no = fx.add(sender, "Rechnung", 1_000_003);
        fx.body(invoice_no, "Rechnungsnummer: RE-2026-999");
        let answered = fx.add_with(sender, "Frage", 1_000_004, |h| {
            h.flags = vec!["\\Answered".into()];
        });
        let in_order = fx.add(sender, "Bestellung eingegangen", 1_000_005);
        let case = fx
            .store
            .create_case(&NewCase {
                title: "Bestellung".into(),
                counterparty: "Service".into(),
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
        fx.store
            .add_case_event(
                case.case.id,
                &NewCaseEvent {
                    message_id: Some(in_order),
                    occurred_at: 1_000_005,
                    kind: Some("order.placed".into()),
                    label: "Bestellt".into(),
                    phase: CaseEventPhase::Start,
                    direction: None,
                    confidence: 1.0,
                    manual: false,
                },
            )
            .unwrap();
        let open_invoice = fx.add(sender, "Zahlungserinnerung", 1_000_006);
        fx.store
            .conn
            .execute(
                "INSERT INTO invoices (account_id, issuer, title, issued_at, paid, message_id)
                 VALUES (?1, 'Service', 'Rechnung', 1000006, 0, ?2)",
                params![fx.account, open_invoice],
            )
            .unwrap();
        let plain = fx.add(sender, "Newsletter", 1_000_007);

        let expect = |id, reason: &str| {
            let p = fx.placement(id);
            assert!(p.protected, "{reason}");
            assert!(p.protection.iter().any(|r| r == reason), "{reason}: {p:?}");
        };
        expect(code, "enthält einen Einmalcode");
        assert!(
            !fx.placement(forged).protected,
            "an unverified sender may be hidden"
        );
        expect(tracking, "enthält eine Sendungsnummer");
        expect(invoice_no, "enthält eine Rechnung");
        expect(answered, "du hast darauf geantwortet");
        expect(in_order, "gehört zu einer Bestellung");
        expect(open_invoice, "Rechnung noch nicht bezahlt");
        assert!(!fx.placement(plain).protected);

        // A bundle for a protected mail is held back, "Für dich" is not.
        fx.store
            .apply_assignment(code, promo(), &[], &[], ASSIGNED_AT)
            .unwrap();
        fx.store
            .apply_assignment(tracking, Target::ForYou, &[], &[], ASSIGNED_AT)
            .unwrap();
        let held = fx.placement(code);
        assert_eq!(held.target, None);
        assert_eq!(held.held_back, Some(promo()));
        assert_eq!(fx.placement(tracking).target, Some(Target::ForYou));
    }

    #[test]
    fn needs_you_from_a_known_correspondent_is_protected() {
        let mut fx = Fx::new();
        // The user has written to Anna before.
        let sent = fx
            .store
            .upsert_folder(fx.account, "Sent", None, Some(FolderRole::Sent), true)
            .unwrap();
        fx.store
            .insert_messages(
                sent.id,
                &[MessageHeader {
                    uid: 900,
                    subject: Some("Termin".into()),
                    from_addr: Some(ME.into()),
                    to: vec!["anna@example.com".into()],
                    date: Some(50),
                    ..Default::default()
                }],
            )
            .unwrap();
        let known = fx.add("anna@example.com", "Frage", 2_900_000);
        fx.body(known, "Hast du Zeit für einen Anruf?");
        let stranger = fx.add("stranger@example.net", "Frage", 2_900_001);
        fx.body(stranger, "Hast du Zeit für einen Anruf?");

        let p = fx.placement(known);
        assert!(
            p.protection
                .contains(&"wartet auf deine Antwort".to_string())
        );
        assert!(!fx.placement(stranger).protected);
    }

    #[test]
    fn a_bundle_quiets_the_turn_assessment_unless_protected() {
        let mut fx = Fx::new();
        let sent = fx
            .store
            .upsert_folder(fx.account, "Sent", None, Some(FolderRole::Sent), true)
            .unwrap();
        fx.store
            .insert_messages(
                sent.id,
                &[MessageHeader {
                    uid: 900,
                    subject: Some("Termin".into()),
                    from_addr: Some(ME.into()),
                    to: vec!["anna@example.com".into()],
                    date: Some(50),
                    ..Default::default()
                }],
            )
            .unwrap();
        let known = fx.add("anna@example.com", "Frage", 2_900_000);
        fx.body(known, "Hast du Zeit für einen Anruf?");
        let stranger = fx.add("stranger@example.net", "Frage", 2_900_001);
        fx.body(stranger, "Hast du Zeit für einen Anruf?");
        let other = fx.add("stranger@example.net", "Frage", 2_900_002);
        fx.body(other, "Hast du Zeit für einen Anruf?");

        let before = |fx: &Fx, id| {
            fx.store
                .assess_messages_turns(&[id], NOW)
                .unwrap()
                .remove(0)
                .1
        };
        assert_eq!(before(&fx, stranger).status, TurnStatus::NeedsYou);

        fx.store
            .apply_assignment(stranger, misc(), &[], &[], ASSIGNED_AT)
            .unwrap();
        fx.store
            .apply_assignment(known, promo(), &[], &[], ASSIGNED_AT)
            .unwrap();
        let t = before(&fx, stranger);
        assert_eq!(t.status, TurnStatus::ReadingAlong);
        assert_eq!(t.reason, "Von dir in ‚Weitere Mails' eingeordnet");
        // Anna is a known correspondent waiting for an answer: protected.
        assert_eq!(before(&fx, known).status, TurnStatus::NeedsYou);
        // Another mail of the same sender, not assigned: as the rules say.
        assert_eq!(before(&fx, other).status, TurnStatus::NeedsYou);

        fx.store
            .apply_assignment(stranger, promo(), &[], &[], ASSIGNED_AT)
            .unwrap();
        assert_eq!(
            before(&fx, stranger).reason,
            "Von dir in ‚Werbung' eingeordnet"
        );
        // "Für dich" does not touch the assessment.
        fx.store
            .apply_assignment(stranger, Target::ForYou, &[], &[], ASSIGNED_AT)
            .unwrap();
        assert_eq!(before(&fx, stranger).status, TurnStatus::NeedsYou);
    }

    #[test]
    fn mark_read_files_only_mail_that_arrives_after_the_assignment() {
        let mut fx = Fx::new();
        let sender = "promo@example.com";
        // Starts the filing pass: its start time is the first run.
        crate::filing::run(&mut fx.store, 1_000).unwrap();
        let old = fx.add(sender, "Angebot: Lampe", 1_500_000);
        let reference = fx.add(sender, "Angebot: Sofa", 1_600_000);
        for id in [old, reference] {
            fx.body(id, "Nur heute.");
        }
        fx.store
            .apply_assignment(
                reference,
                promo(),
                &kind_keys(
                    &fx.store
                        .assignment_preview(reference, promo(), NOW)
                        .unwrap(),
                ),
                &[],
                ASSIGNED_AT,
            )
            .unwrap();
        // Arrives after the assignment, same kind; and a different kind.
        let fresh = fx.add(sender, "Angebot: Teppich", ASSIGNED_AT + 500);
        let other_kind = fx.add(sender, "Ihr Konto wurde angelegt", ASSIGNED_AT + 600);
        for id in [fresh, other_kind] {
            fx.body(id, "Nur heute.");
        }
        for id in [old, reference, fresh, other_kind] {
            fx.store
                .conn
                .execute(
                    "INSERT INTO case_scan (message_id, with_body) VALUES (?1, 1)",
                    [id],
                )
                .unwrap();
        }

        let report = crate::filing::run(&mut fx.store, NOW).unwrap();
        assert_eq!(report.marked, 1, "{report:?}");
        let flags = |id| fx.store.message(id).unwrap().unwrap().header.flags;
        assert!(flags(fresh).contains(&"\\Seen".to_string()));
        assert!(flags(fresh).contains(&crate::filing::KEYWORD_FILED.to_string()));
        for untouched in [old, reference, other_kind] {
            assert!(!flags(untouched).contains(&"\\Seen".to_string()));
        }
    }

    #[test]
    fn mark_read_respects_the_auto_mark_read_switch() {
        let mut fx = Fx::new();
        crate::filing::set_auto_mark_read(&fx.store, false).unwrap();
        let reference = fx.add("promo@example.com", "Angebot: Sofa", 1_600_000);
        fx.store
            .apply_assignment(reference, promo(), &[], &[], ASSIGNED_AT)
            .unwrap();
        let fresh = fx.add("promo@example.com", "Angebot: Sofa", ASSIGNED_AT + 500);
        fx.body(fresh, "Nur heute.");
        fx.store
            .conn
            .execute(
                "INSERT INTO case_scan (message_id, with_body) VALUES (?1, 1)",
                [fresh],
            )
            .unwrap();
        // Mail-level only: the fresh mail has another Message-ID, nothing to do.
        let report = crate::filing::run(&mut fx.store, NOW).unwrap();
        assert_eq!(report.marked, 0);
    }

    #[test]
    fn preview_preselects_only_the_group_of_the_mail_and_sets_protected_aside() {
        let mut fx = Fx::new();
        let (offers, shipping) = fx.shop();
        // An offer that carries a tracking number is protected.
        let odd = fx.add(SHOP, "Angebot: Lampe", 1_400_000);
        fx.body(odd, &format!("Sendungsnummer: {}", ups_number()));
        let preview = fx
            .store
            .assignment_preview(offers[0], promo(), NOW)
            .unwrap();
        assert_eq!(preview.sender, SHOP);
        assert_eq!(preview.total_mails, 5);
        let selected: Vec<_> = preview.groups.iter().filter(|g| g.preselected).collect();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].message_ids.len(), 3);
        assert_eq!(selected[0].count, 3);
        assert_eq!(selected[0].examples.len(), 3);
        assert!(selected[0].first_date <= selected[0].last_date);
        for g in &preview.groups {
            if !g.preselected {
                assert!(
                    g.message_ids
                        .iter()
                        .all(|id| !selected[0].message_ids.contains(id))
                );
            }
            // A protected mail is never among the ids an apply would take.
            for p in &g.protected {
                assert!(!g.message_ids.contains(&p.message_id));
                assert!(!p.reasons.is_empty());
            }
        }
        // The shipping notice and the offer with a number are set aside.
        let protected: Vec<i64> = preview
            .groups
            .iter()
            .flat_map(|g| g.protected.iter().map(|p| p.message_id))
            .collect();
        assert!(protected.contains(&shipping));
        assert!(protected.contains(&odd));
        assert_eq!(preview.protected_total, 2);
        assert_eq!(preview.selected_mails, 3);
        assert_eq!(preview.other_mails, 2);
        assert_eq!(preview.kind_key, selected[0].kind_key);
    }

    #[test]
    fn preview_leaves_out_junk_and_old_mail_and_deduplicates_copies() {
        let mut fx = Fx::new();
        let first = fx.add(SHOP, "Angebot: Lampe", 2_000_000);
        // A copy of the same mail in another folder.
        let archive = fx
            .store
            .upsert_folder(fx.account, "Archive", None, Some(FolderRole::Archive), true)
            .unwrap();
        let rfc = fx.rfc(first);
        fx.store
            .insert_messages(
                archive.id,
                &[MessageHeader {
                    uid: 77,
                    message_id: Some(rfc),
                    subject: Some("Angebot: Lampe".into()),
                    from_addr: Some(SHOP.into()),
                    date: Some(2_000_000),
                    dmarc_pass: true,
                    ..Default::default()
                }],
            )
            .unwrap();
        let junk = fx
            .store
            .upsert_folder(fx.account, "Junk", None, Some(FolderRole::Junk), true)
            .unwrap();
        fx.store
            .insert_messages(
                junk.id,
                &[MessageHeader {
                    uid: 78,
                    message_id: Some("<junk@example.net>".into()),
                    subject: Some("Angebot: Spam".into()),
                    from_addr: Some(SHOP.into()),
                    date: Some(2_000_000),
                    ..Default::default()
                }],
            )
            .unwrap();
        // Older than 800 days before NOW.
        let too_old = NOW - WINDOW_SECS - 10;
        fx.add(SHOP, "Angebot: Uralt", too_old);
        let preview = fx.store.assignment_preview(first, misc(), NOW).unwrap();
        assert_eq!(preview.total_mails, 1);
    }

    #[test]
    fn apply_and_remove_round_trip() {
        let mut fx = Fx::new();
        let (offers, _) = fx.shop();
        let preview = fx.store.assignment_preview(offers[0], misc(), NOW).unwrap();
        let ids = fx
            .store
            .apply_assignment(
                offers[0],
                misc(),
                &kind_keys(&preview),
                &[offers[1]],
                ASSIGNED_AT,
            )
            .unwrap();
        assert_eq!(ids.len(), 1);
        let listed = fx.store.list_assignments_at(NOW).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].scope, "kind");
        assert_eq!(listed[0].target, misc());
        assert_eq!(listed[0].sender.as_deref(), Some(SHOP));
        assert_eq!(listed[0].kind_label.as_deref(), Some("Angebot: …"));
        assert_eq!(listed[0].exceptions, 1);
        assert_eq!(fx.placement(offers[0]).target, Some(misc()));
        assert_eq!(fx.placement(offers[1]).target, None);

        // Assigning again replaces it (and its exceptions) instead of piling up.
        let again = fx
            .store
            .apply_assignment(
                offers[0],
                promo(),
                &kind_keys(&preview),
                &[],
                ASSIGNED_AT + 1,
            )
            .unwrap();
        assert_eq!(again, ids);
        assert_eq!(fx.placement(offers[1]).target, Some(promo()));
        assert_eq!(fx.store.list_assignments_at(NOW).unwrap()[0].exceptions, 0);

        // Mail level, then undo of both.
        let mail = fx
            .store
            .apply_assignment(offers[2], Target::ForYou, &[], &[], ASSIGNED_AT)
            .unwrap();
        let listed = fx.store.list_assignments_at(NOW).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].scope, "mail");
        assert_eq!(listed[0].subject.as_deref(), Some("Angebot: Gartenstuhl"));
        for id in again.iter().chain(&mail) {
            fx.store.remove_assignment(*id).unwrap();
        }
        assert!(fx.store.list_assignments_at(NOW).unwrap().is_empty());
        assert_eq!(fx.placement(offers[0]).target, None);
        let left: i64 = fx
            .store
            .conn
            .query_row("SELECT COUNT(*) FROM assignment_exceptions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn apply_rejects_unknown_kinds_and_mails_without_message_id() {
        let mut fx = Fx::new();
        let (offers, _) = fx.shop();
        let err = fx
            .store
            .apply_assignment(offers[0], misc(), &["nope".into()], &[], ASSIGNED_AT)
            .unwrap_err();
        assert!(err.to_string().contains("unknown kind"));
        let anonymous = fx.add_with(SHOP, "Ohne Id", 1_000_000, |h| h.message_id = None);
        assert!(
            fx.store
                .apply_assignment(anonymous, misc(), &[], &[], ASSIGNED_AT)
                .is_err()
        );
        assert!(fx.store.assignment_preview(9999, misc(), NOW).is_err());
    }

    #[test]
    fn survey_leaves_out_senders_that_never_passed_dmarc() {
        let mut fx = Fx::new();
        for i in 0..30 {
            fx.add_with("promo@spam.invalid", "Gewinn", 1_000_000 + i, |h| {
                h.dmarc_pass = false
            });
        }
        fx.add("news@example.net", "Wochenbrief", 1_000_100);
        let scope = |include_unverified| SurveyScope {
            top: Some(5),
            sender: None,
            examples: true,
            include_unverified,
        };
        let verified = fx.store.survey_kinds(NOW, &scope(false)).unwrap();
        let senders: Vec<_> = verified.senders.iter().map(|s| s.sender.as_str()).collect();
        assert_eq!(senders, vec!["news@example.net"]);
        let all = fx.store.survey_kinds(NOW, &scope(true)).unwrap();
        assert_eq!(all.senders[0].sender, "promo@spam.invalid");
        // Unverified mail is not protected: it may be hidden.
        assert_eq!(all.senders[0].groups[0].protected, 0);
    }

    #[test]
    fn survey_counts_without_writing() {
        let mut fx = Fx::new();
        fx.shop();
        for i in 0..12 {
            fx.add("news@example.net", "Wochenbrief", 1_000_000 + i);
        }
        let before: i64 = fx
            .store
            .conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let survey = fx
            .store
            .survey_kinds(
                NOW,
                &SurveyScope {
                    top: None,
                    sender: None,
                    examples: false,
                    include_unverified: false,
                },
            )
            .unwrap();
        let after: i64 = fx
            .store
            .conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "the survey must not write");
        let s = survey.stats;
        assert_eq!(s.senders, 2);
        assert_eq!(s.mails, 16);
        assert_eq!(s.multi_group_senders, 1);
        assert_eq!(s.busy_senders, 1);
        assert_eq!(s.busy_multi_group_senders, 0);
        assert_eq!(s.protected_mails, 1);
        assert!(survey.senders.is_empty());
        let detailed = fx
            .store
            .survey_kinds(
                NOW,
                &SurveyScope {
                    top: Some(1),
                    sender: None,
                    examples: true,
                    include_unverified: false,
                },
            )
            .unwrap();
        assert_eq!(detailed.senders[0].sender, "news@example.net");
        assert_eq!(detailed.senders[0].groups[0].examples, vec!["Wochenbrief"]);
    }

    // ----- bundles --------------------------------------------------------------

    /// A mail with an unsubscribe link.
    fn add_list(fx: &mut Fx, from: &str, subject: &str, date: i64) -> i64 {
        fx.add_with(from, subject, date, |h| {
            h.list_unsubscribe = vec!["<mailto:unsubscribe@example.net>".into()];
        })
    }

    #[test]
    fn mail_without_assignment_gets_an_automatic_bundle() {
        let mut fx = Fx::new();
        let news = add_list(&mut fx, "news@example.net", "Neues im Dezember", 1_000_000);
        let promo = add_list(
            &mut fx,
            "ads@example.com",
            "20% Rabatt auf alles",
            1_000_001,
        );
        let account = fx.add(
            "service@example.com",
            "Dein Passwort wurde geändert",
            1_000_002,
        );
        let statement = fx.add("bank@example.org", "Kontoauszug November", 1_000_003);
        let plain = fx.add("club@example.org", "Hallo zusammen", 1_000_004);
        let forged = fx.add_with("ads@spam.invalid", "Gutschein für dich", 1_000_005, |h| {
            h.dmarc_pass = false;
        });

        let expect = |id, bundle: &str| {
            let p = fx.placement(id);
            assert_eq!(p.bundle.as_deref(), Some(bundle), "{p:?}");
            assert_eq!(p.origin, Some(Origin::Auto));
            assert_eq!(p.target, None, "nothing was assigned");
            assert_eq!(p.assigned_at, None);
            assert!(p.reason.as_deref().is_some_and(|r| !r.is_empty()));
            assert!(p.bundle_name.is_some());
        };
        expect(news, "newsletter");
        expect(promo, "promo");
        expect(account, "account");
        expect(statement, "overviews");
        expect(plain, "other");
        // Unverified senders are bundled, too (and still never notify).
        expect(forged, "promo");
        assert_eq!(
            fx.placement(statement).bundle_name.as_deref(),
            Some("Übersichten")
        );
    }

    #[test]
    fn automatic_bundle_leaves_mail_for_the_user_alone() {
        let mut fx = Fx::new();
        // A one-time code is never account mail, whatever it says.
        let code = fx.add(
            "service@example.com",
            "Dein Passwort wurde geändert",
            1_000_000,
        );
        fx.body(code, "Dein Bestätigungscode lautet 482913.");
        let p = fx.placement(code);
        assert!(p.protected);
        assert_eq!((p.bundle, p.origin, p.reason), (None, None, None));

        // A stranger asking a question, and a mail of the user's own.
        let question = fx.add("stranger@example.net", "Frage", 2_900_001);
        fx.body(question, "Hast du Zeit für einen Anruf?");
        assert_eq!(fx.placement(question).bundle, None);
        let own = fx.add(ME, "Notiz", 1_000_002);
        assert_eq!(fx.placement(own).bundle, None);

        // Mail with a tracking number is protected, so not bundled.
        let tracking = fx.add("shop@example.com", "Paket unterwegs", 1_000_003);
        fx.body(tracking, &format!("Sendungsnummer: {}", ups_number()));
        assert_eq!(fx.placement(tracking).bundle, None);

        // Assigned wins over the automatic choice; "Für dich" is no bundle.
        let news = add_list(&mut fx, "news@example.net", "Neues", 1_000_004);
        fx.store
            .apply_assignment(news, Target::ForYou, &[], &[], ASSIGNED_AT)
            .unwrap();
        let p = fx.placement(news);
        assert_eq!(p.target, Some(Target::ForYou));
        assert_eq!(p.origin, Some(Origin::Mail));
        assert_eq!(p.bundle, None);
    }

    #[test]
    fn source_signals_pick_bundles() {
        let mut fx = Fx::new();
        let mk_source = |fx: &Fx, key: &str, mode: &str, newsletter: bool, ids: &[i64]| {
            fx.store
                .conn
                .execute(
                    "INSERT INTO sources (source_key, display_name, mode, is_newsletter)
                     VALUES (?1, ?1, ?2, ?3)",
                    params![key, mode, newsletter],
                )
                .unwrap();
            let id = fx.store.conn.last_insert_rowid();
            for m in ids {
                fx.store
                    .conn
                    .execute(
                        "UPDATE messages SET source_id = ?1 WHERE id = ?2",
                        params![id, m],
                    )
                    .unwrap();
            }
        };
        let status = fx.add("meter@example.org", "Stand heute", 1_000_000);
        mk_source(&fx, "meter", "latest_only", false, &[status]);
        assert_eq!(fx.placement(status).bundle.as_deref(), Some("overviews"));

        let ads: Vec<i64> = ["20% Rabatt", "Gutschein", "Sale", "Neu im Shop"]
            .iter()
            .enumerate()
            .map(|(i, s)| fx.add("shop@example.net", s, 1_000_000 + i as i64))
            .collect();
        mk_source(&fx, "shop", "show", true, &ads);
        // Its subject says nothing, the history does.
        assert_eq!(fx.placement(ads[3]).bundle.as_deref(), Some("promo"));

        let letters: Vec<i64> = ["Neues aus dem Verein", "Protokoll", "Einladung"]
            .iter()
            .enumerate()
            .map(|(i, s)| fx.add("verein@example.org", s, 1_000_000 + i as i64))
            .collect();
        mk_source(&fx, "verein", "show", true, &letters);
        assert_eq!(
            fx.placement(letters[0]).bundle.as_deref(),
            Some("newsletter")
        );
    }

    #[test]
    fn automatic_bundle_falls_back_to_other_when_switched_off() {
        let mut fx = Fx::new();
        let promo = add_list(&mut fx, "ads@example.com", "Gutschein", 1_000_000);
        fx.store
            .update_bundle(
                "promo",
                &crate::bundles::BundlePatch {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        let p = fx.placement(promo);
        assert_eq!(p.bundle.as_deref(), Some("other"));
        assert_eq!(p.origin, Some(Origin::Auto));
        assert!(p.reason.unwrap().contains("ausgeschaltet"));
    }

    #[test]
    fn deleting_a_custom_bundle_moves_its_mail_to_other() {
        let mut fx = Fx::new();
        let (offers, _) = fx.shop();
        fx.store.create_bundle("Verein").unwrap();
        let ids = fx
            .store
            .apply_assignment(offers[0], Target::bundle("verein"), &[], &[], ASSIGNED_AT)
            .unwrap();
        let p = fx.placement(offers[0]);
        assert_eq!(p.target, Some(Target::bundle("verein")));
        assert_eq!(p.bundle_name.as_deref(), Some("Verein"));
        assert_eq!(p.origin, Some(Origin::Mail));

        fx.store.delete_bundle("verein").unwrap();
        let p = fx.placement(offers[0]);
        assert_eq!(p.target, Some(misc()));
        assert_eq!(p.bundle.as_deref(), Some("other"));
        let listed = fx.store.list_assignments_at(NOW).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, ids[0]);
        assert_eq!(listed[0].target, misc());

        // Unknown bundles cannot be assigned.
        assert!(
            fx.store
                .apply_assignment(offers[1], Target::bundle("verein"), &[], &[], ASSIGNED_AT)
                .is_err()
        );
    }

    /// Puts an offer of a shop kind into "Werbung", lets `setup` change the
    /// bundles, then lets mail arrive at `arrival` and runs the filing pass.
    /// Returns whether that mail was marked read.
    fn files_after(fx: &mut Fx, arrival: i64, setup: impl FnOnce(&Store)) -> bool {
        crate::filing::run(&mut fx.store, 1_000).unwrap();
        let sender = "promo@example.com";
        let reference = fx.add(sender, "Angebot: Sofa", 1_600_000);
        let preview = fx
            .store
            .assignment_preview(reference, promo(), NOW)
            .unwrap();
        fx.store
            .apply_assignment(reference, promo(), &kind_keys(&preview), &[], ASSIGNED_AT)
            .unwrap();
        setup(&fx.store);
        let fresh = fx.add(sender, "Angebot: Teppich", arrival);
        fx.body(fresh, "Nur heute.");
        fx.store
            .conn
            .execute(
                "INSERT INTO case_scan (message_id, with_body) VALUES (?1, 1)",
                [fresh],
            )
            .unwrap();
        crate::filing::run(&mut fx.store, NOW).unwrap();
        fx.store
            .message(fresh)
            .unwrap()
            .unwrap()
            .header
            .flags
            .contains(&"\\Seen".to_string())
    }

    #[test]
    fn a_switched_off_bundle_acts_like_other() {
        let on = |fx: &mut Fx| files_after(fx, ASSIGNED_AT + 500, |_| {});
        assert!(on(&mut Fx::new()), "promo marks read by default");

        let mut off = Fx::new();
        let marked = files_after(&mut off, ASSIGNED_AT + 500, |s| {
            s.update_bundle(
                "promo",
                &crate::bundles::BundlePatch {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        });
        assert!(!marked, "\"Weitere Mails\" never marks read");

        // The same goes for the turn assessment and the notification input.
        let known = off.add("stranger@example.net", "Frage", 2_900_000);
        off.body(known, "Hast du Zeit für einen Anruf?");
        off.store
            .apply_assignment(known, promo(), &[], &[], ASSIGNED_AT)
            .unwrap();
        let t = off
            .store
            .assess_messages_turns(&[known], NOW)
            .unwrap()
            .remove(0)
            .1;
        assert_eq!(t.reason, "Von dir in ‚Weitere Mails' eingeordnet");
        assert_eq!(off.placement(known).target, Some(misc()));
    }

    #[test]
    fn switching_mark_read_on_does_not_reach_back() {
        // Mail arrives while "Werbung" does not mark read; the switch comes
        // later, so that mail stays unread on the server.
        let mut fx = Fx::new();
        let marked = files_after(&mut fx, ASSIGNED_AT + 500, |s| {
            let off = crate::bundles::BundlePatch {
                mark_read: Some(false),
                ..Default::default()
            };
            s.update_bundle_at("promo", &off, ASSIGNED_AT + 100)
                .unwrap();
            let on = crate::bundles::BundlePatch {
                mark_read: Some(true),
                ..Default::default()
            };
            // Switched on after the mail arrived (its date is before).
            s.update_bundle_at("promo", &on, ASSIGNED_AT + 1_000)
                .unwrap();
        });
        assert!(!marked);

        // Mail that arrives after the switch is marked.
        let mut later = Fx::new();
        let marked = files_after(&mut later, ASSIGNED_AT + 2_000, |s| {
            let on = crate::bundles::BundlePatch {
                mark_read: Some(true),
                ..Default::default()
            };
            s.update_bundle_at("promo", &on, ASSIGNED_AT + 1_000)
                .unwrap();
        });
        assert!(marked);
    }

    #[test]
    fn automatic_placement_never_marks_mail_read() {
        let mut fx = Fx::new();
        crate::filing::run(&mut fx.store, 1_000).unwrap();
        let ad = add_list(&mut fx, "ads@example.com", "20% Rabatt", ASSIGNED_AT + 500);
        fx.body(ad, "Nur heute.");
        fx.store
            .conn
            .execute(
                "INSERT INTO case_scan (message_id, with_body) VALUES (?1, 1)",
                [ad],
            )
            .unwrap();
        assert_eq!(fx.placement(ad).bundle.as_deref(), Some("promo"));
        let report = crate::filing::run(&mut fx.store, NOW).unwrap();
        assert_eq!(report.marked, 0, "{report:?}");
    }

    #[test]
    fn bundle_survey_counts_inbox_mail_without_writing() {
        let mut fx = Fx::new();
        let day = 86_400;
        let now = NOW + 200 * day;
        let recent = now - 10 * day;
        add_list(&mut fx, "news@example.net", "Neues im Dezember", recent);
        add_list(&mut fx, "news@example.net", "Neues im November", recent + 1);
        add_list(&mut fx, "ads@example.com", "20% Rabatt", recent + 2);
        fx.add_with("ads@spam.invalid", "Gutschein", recent + 3, |h| {
            h.dmarc_pass = false;
        });
        fx.add("bank@example.org", "Kontoauszug November", recent + 4);
        let assigned = fx.add("club@example.org", "Hallo", recent + 5);
        fx.store
            .apply_assignment(assigned, Target::ForYou, &[], &[], recent)
            .unwrap();
        let code = fx.add("service@example.com", "Dein Code", recent + 6);
        fx.body(code, "Dein Bestätigungscode lautet 482913.");
        // Too old, and outside the inbox.
        fx.add("old@example.net", "Uralt", now - 100 * day);
        let junk = fx
            .store
            .upsert_folder(fx.account, "Junk", None, Some(FolderRole::Junk), true)
            .unwrap();
        fx.store
            .insert_messages(
                junk.id,
                &[MessageHeader {
                    uid: 500,
                    message_id: Some("<junk1@example.net>".into()),
                    subject: Some("Spam".into()),
                    from_addr: Some("junk@example.net".into()),
                    date: Some(recent),
                    ..Default::default()
                }],
            )
            .unwrap();

        let before: i64 = fx
            .store
            .conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        let survey = fx.store.survey_bundles(now, 90, true).unwrap();
        let after: i64 = fx
            .store
            .conn
            .query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "the survey must not write");

        assert_eq!(survey.total, 7);
        let by = |k: &str| survey.bundles.iter().find(|b| b.key == k).unwrap();
        assert_eq!(by("newsletter").mails, 2);
        assert_eq!(
            by("newsletter").examples,
            vec!["Neues im November", "Neues im Dezember"]
        );
        assert_eq!(by("promo").mails, 2);
        assert_eq!(by("overviews").mails, 1);
        assert_eq!(by("other").mails, 0);
        assert_eq!(survey.unverified, 1);
        assert_eq!(survey.assigned, 1);
        assert_eq!(survey.stays_visible, 1, "the mail with a code");
        // Order of the bundles is the display order.
        assert_eq!(survey.bundles.last().unwrap().key, "other");

        let numbers_only = fx.store.survey_bundles(now, 90, false).unwrap();
        assert!(numbers_only.bundles.iter().all(|b| b.examples.is_empty()));
        assert_eq!(numbers_only.bundles[1].mails, by("account").mails);
    }
}
