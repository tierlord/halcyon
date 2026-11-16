//! The invoice ledger ("Rechnungen"): every invoice of a year in one place,
//! for the tax return.
//!
//! The case linker only follows invoices that still need an action. The
//! ledger is a separate, additional pass over the same mail: it records
//! *all* invoices (also direct debits and already paid ones), keeps a copy
//! of the proof document so it outlives the mail, and – when there is no
//! document – remembers where the sender's customer portal is.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Datelike, Local, TimeZone};
use regex::Regex;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::cases::normalize_key;
use crate::content::{self, Attachment};
use crate::extract::finance::{
    BusinessKey, BusinessKeyKind, PaymentDisposition, find_amounts, find_business_keys,
    find_due_info,
};
use crate::extract::portal::{self, Origin, PortalLink};
use crate::linker::{OrderStep, base_domain, clean_subject, order_step};
use crate::model::{AccountKind, FolderRole};
use crate::store::{SCAN_SELECT, ScanMessage, Store, scan_from_row};

/// Only mail from this period is looked at. Longer than the case window:
/// the ledger must cover the previous tax year as well.
pub const SCAN_WINDOW_SECS: i64 = 800 * 86_400;

const BATCH: usize = 500;
const ZSTD_LEVEL: i32 = 3;
/// Larger proof files are not copied (a mail with a huge unrelated
/// attachment is not worth a copy of the whole message either).
pub const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;
const MAX_EML_BYTES: usize = 8 * 1024 * 1024;
/// How many other mails of a sender are searched for a portal link.
const PORTAL_WELCOME_MAILS: usize = 8;
const PORTAL_OTHER_MAILS: usize = 15;

const NOTE_PAID: &str = "Bezahlt";
const NOTE_DEBIT: &str = "Wird per Lastschrift eingezogen";
const NOTE_INFO: &str = "Zur Information";
const NOTE_ORDER: &str = "Bestellung · Rechnung im Kundenkonto";

/// What a run changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvoiceReport {
    pub scanned: usize,
    pub created: usize,
    pub updated: usize,
    /// New or better portal links.
    pub portals: usize,
}

impl InvoiceReport {
    pub fn changed(&self) -> bool {
        self.created > 0 || self.updated > 0 || self.portals > 0
    }
}

/// What proof the ledger can offer for an invoice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InvoiceEvidence {
    /// The invoice document (PDF or XML) that came with the mail.
    Attachment,
    /// The mail itself is the receipt; a copy of it is stored.
    Mail,
    /// No document, but a link to the sender's customer portal.
    Portal,
    None,
}

impl InvoiceEvidence {
    pub fn as_str(self) -> &'static str {
        match self {
            InvoiceEvidence::Attachment => "attachment",
            InvoiceEvidence::Mail => "mail",
            InvoiceEvidence::Portal => "portal",
            InvoiceEvidence::None => "none",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "attachment" => InvoiceEvidence::Attachment,
            "mail" => InvoiceEvidence::Mail,
            "portal" => InvoiceEvidence::Portal,
            _ => InvoiceEvidence::None,
        }
    }
}

/// Who decided whether an invoice is paid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaidSource {
    Auto,
    User,
}

/// One row of the ledger as the UI shows it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvoiceRecord {
    pub id: i64,
    pub account_id: i64,
    pub account_email: String,
    /// Effective kind: the override, else the account's kind.
    pub kind: AccountKind,
    pub kind_overridden: bool,
    pub issuer: String,
    pub domain: Option<String>,
    pub title: String,
    pub invoice_number: Option<String>,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
    pub issued_at: i64,
    pub due_at: Option<i64>,
    pub paid: bool,
    pub paid_source: Option<PaidSource>,
    pub paid_note: Option<String>,
    pub evidence: InvoiceEvidence,
    /// Stored proof file, if any.
    pub file_name: Option<String>,
    pub portal_url: Option<String>,
    pub portal_label: Option<String>,
    /// `None` once the mail was deleted.
    pub message_id: Option<i64>,
    pub case_id: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountKindInfo {
    pub account_id: i64,
    pub email: String,
    pub kind: AccountKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvoiceExportReport {
    pub path: String,
    /// Invoices listed in the overview.
    pub exported: usize,
    /// Of those, how many have no stored proof file.
    pub without_proof: usize,
}

/// A stored proof file.
#[derive(Clone, Debug)]
pub struct InvoiceFile {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

// ----- detection -------------------------------------------------------------

static RE_INVOICE_SUBJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\brechnung\b|\binvoice\b|\breceipt\b|\bquittung\b").unwrap());
static RE_CONTRACT_SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:kündigungsbestätigung|kündigung\b|vertragsbestätigung|vertragsverlängerung|tarifwechsel|ihr vertrag\b|cancellation confirmed|contract confirmation)\b",
    )
    .unwrap()
});
static RE_INVOICE_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)rechnung|invoice|receipt|quittung|beleg").unwrap());
/// Documents that come with an invoice mail but are not the invoice.
static RE_OTHER_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)agb|widerruf|datenschutz|bedingungen|terms|privacy|conditions|belehrung|gebrauchsanleitung|manual",
    )
    .unwrap()
});
fn is_pdf_or_xml(a: &Attachment) -> bool {
    let name = a.name.to_ascii_lowercase();
    a.mime_type.contains("pdf")
        || a.mime_type.ends_with("/xml")
        || a.mime_type.ends_with("+xml")
        || name.ends_with(".pdf")
        || name.ends_with(".xml")
}

fn is_pdf(a: &Attachment) -> bool {
    a.mime_type.contains("pdf") || a.name.to_ascii_lowercase().ends_with(".pdf")
}

/// The attachment that is the invoice, and whether its *name* says so.
fn pick_attachment(attachments: &[Attachment], text_invoice: bool) -> Option<(&Attachment, bool)> {
    let usable = || {
        attachments
            .iter()
            .filter(|a| !a.inline && is_pdf_or_xml(a) && !RE_OTHER_FILE.is_match(&a.name))
    };
    let named = usable()
        .filter(|a| RE_INVOICE_FILE.is_match(&a.name))
        .max_by_key(|a| is_pdf(a));
    if let Some(a) = named {
        return Some((a, true));
    }
    // The mail says "invoice" but the file has an opaque name ("20260101.pdf").
    if text_invoice {
        return usable().find(|a| is_pdf(a)).map(|a| (a, false));
    }
    None
}

/// Whether a subject marks a mail the ledger (or the order linker) may need
/// the body of: an invoice subject, or any order step. Used to decide which
/// old mail is worth downloading in full.
pub fn is_ledger_candidate_subject(subject: &str) -> bool {
    RE_INVOICE_SUBJECT.is_match(subject) || order_step(subject).is_some()
}

/// Whether the text of a mail makes it an invoice: what the case linker
/// accepts, plus invoice step mails of an order.
fn is_text_invoice(subject: &str, bkeys: &[BusinessKey], has_amount_or_due: bool) -> bool {
    let has_invoice_key = bkeys
        .iter()
        .any(|k| k.kind == BusinessKeyKind::InvoiceNumber);
    let is_contract = RE_CONTRACT_SUBJECT.is_match(subject)
        || (!has_invoice_key
            && bkeys
                .iter()
                .any(|k| k.kind == BusinessKeyKind::ContractNumber));
    if is_contract {
        return false;
    }
    ((RE_INVOICE_SUBJECT.is_match(subject) || has_invoice_key)
        && (has_amount_or_due || has_invoice_key))
        || order_step(subject) == Some(OrderStep::Invoice)
}

// ----- the pass ----------------------------------------------------------------

/// Looks at new mail and records its invoices. Safe to call often: each mail
/// is looked at once (and once more when its text arrives). Call it after
/// [`crate::linker::run`] so the case of an invoice is known.
pub fn run(store: &mut Store, now: i64) -> Result<InvoiceReport> {
    let own: HashSet<String> = store
        .accounts()?
        .into_iter()
        .map(|a| a.email.to_ascii_lowercase())
        .collect();
    let mut report = InvoiceReport::default();
    let mut portal_domains: BTreeSet<String> = BTreeSet::new();
    loop {
        let batch = store.invoice_scan_batch(now - SCAN_WINDOW_SECS, BATCH)?;
        if batch.is_empty() {
            break;
        }
        store.in_transaction(|store| {
            for m in &batch {
                process(store, m, &own, &mut report, &mut portal_domains)?;
                store.mark_invoice_scanned(m.id, m.body_text.is_some())?;
                report.scanned += 1;
            }
            Ok(())
        })?;
        if batch.len() < BATCH {
            break;
        }
    }
    mirror_orders(store, now, &mut report, &mut portal_domains)?;
    for domain in &portal_domains {
        if portal_from_sender_mails(store, domain)? {
            report.portals += 1;
        }
    }
    Ok(report)
}

fn process(
    store: &Store,
    m: &ScanMessage,
    own: &HashSet<String>,
    report: &mut InvoiceReport,
    portal_domains: &mut BTreeSet<String>,
) -> Result<()> {
    let from_lower = m
        .from_addr
        .as_deref()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let subject = m.subject.as_deref().unwrap_or("");
    if m.list_id.is_some()
        || !m.dmarc_pass
        || m.folder_role == Some(FolderRole::Sent)
        || own.contains(&from_lower)
        // Replies are a conversation about an invoice, not the invoice.
        || m.in_reply_to.is_some()
        || clean_subject(subject) != subject.trim()
    {
        return Ok(());
    }
    let Some(domain) = base_domain(&from_lower) else {
        return Ok(());
    };
    let body = m.body_text.as_deref().unwrap_or("");
    let amounts = find_amounts(subject, body);
    let due_info = find_due_info(subject, body, m.date);
    let mut bkeys = find_business_keys(subject, body);
    // Same input, same pick: the key finder returns them in map order.
    bkeys.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.value.cmp(&b.value))
    });
    let text_invoice = is_text_invoice(subject, &bkeys, !amounts.is_empty() || due_info.is_some());
    let (account_id, has_attachments) = store.message_account(m.id)?;
    if !text_invoice && !has_attachments {
        return Ok(());
    }

    let raw = if m.body_text.is_some() {
        store.body(m.id)?
    } else {
        None
    };
    let parsed = raw.as_deref().map(content::parse);
    let attachment = parsed
        .as_ref()
        .and_then(|c| pick_attachment(&c.attachments, text_invoice));
    let named_attachment = attachment.is_some_and(|(_, named)| named);
    if !text_invoice && !named_attachment {
        return Ok(());
    }
    if RE_CONTRACT_SUBJECT.is_match(subject) {
        return Ok(());
    }

    let top_amount = amounts.first();
    let number = bkeys
        .iter()
        .find(|k| k.kind == BusinessKeyKind::InvoiceNumber)
        .map(|k| k.value.clone());
    let file = match (&raw, attachment) {
        (Some(raw), Some((a, _))) => content::attachment_data(raw, a.part)
            .filter(|d| !d.is_empty() && d.len() <= MAX_FILE_BYTES)
            .map(|data| NewFile {
                name: a.name.clone(),
                mime_type: a.mime_type.clone(),
                data,
                is_attachment: true,
            }),
        (Some(raw), None)
            if (top_amount.is_some() || number.is_some()) && raw.len() <= MAX_EML_BYTES =>
        {
            Some(NewFile {
                name: format!("{}.eml", content::safe_file_name(&clean_subject(subject))),
                mime_type: "message/rfc822".into(),
                data: raw.clone(),
                is_attachment: false,
            })
        }
        _ => None,
    };
    let issuer = m
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| domain.clone());
    let case_id = match m.message_id.as_deref() {
        Some(rfc) => store.case_of_rfc_message(rfc)?,
        None => None,
    };
    let new = NewInvoice {
        account_id,
        issuer,
        domain: domain.clone(),
        title: clean_subject(subject),
        number_norm: number.as_deref().map(normalize_key),
        number,
        amount_cents: top_amount.map(|a| a.amount_cents),
        currency: top_amount.map(|a| a.currency.clone()),
        issued_at: m.date,
        due_at: due_info.as_ref().and_then(|d| d.due_at),
        disposition: due_info.as_ref().map(|d| d.disposition),
        message_id: m.id,
        rfc_message_id: m.message_id.clone(),
        case_id,
        file,
    };
    let (invoice_id, outcome) = store.record_invoice(&new)?;
    match outcome {
        Outcome::Created => report.created += 1,
        Outcome::Updated => report.updated += 1,
        Outcome::Unchanged => {}
    }

    // No document to keep: the sender's own links in this mail may lead to
    // the invoice. The sender's other mail is searched after the batch.
    if !store.invoice_has_attachment(invoice_id)? {
        let (html, text) = match &parsed {
            Some(c) => (c.html.as_deref(), c.text.as_deref().unwrap_or("")),
            None => (None, body),
        };
        if let Some(best) =
            portal::find_portal_links(html, text, &domain, Origin::InvoiceMail).first()
            && store.offer_portal(&domain, best, Origin::InvoiceMail, Some(m.id))?
        {
            report.portals += 1;
        }
        portal_domains.insert(domain);
    }
    Ok(())
}

/// An order case without a mail of its own in the ledger: many shops never
/// mail an invoice, it has to be fetched in the customer account. The row
/// keeps the purchase listed and points there.
struct OrderCase {
    case_id: i64,
    title: String,
    counterparty: String,
    domain: Option<String>,
    amount_cents: i64,
    currency: Option<String>,
    first_at: i64,
    first_message_id: Option<i64>,
    account_id: Option<i64>,
}

fn mirror_orders(
    store: &Store,
    now: i64,
    report: &mut InvoiceReport,
    portal_domains: &mut BTreeSet<String>,
) -> Result<()> {
    store.drop_dismissed_order_rows()?;
    for order in store.order_cases()? {
        match store.order_row_state(order.case_id)? {
            // A real invoice exists for the case: it is the entry.
            (_, true) => {
                store.drop_order_row(order.case_id)?;
            }
            (Some(_), false) => {
                if store.sync_order_row(&order)? {
                    report.updated += 1;
                }
            }
            (None, false) => {
                if order.first_at < now - SCAN_WINDOW_SECS {
                    continue;
                }
                let Some(account_id) = order.account_id else {
                    continue;
                };
                store.insert_order_row(&order, account_id)?;
                report.created += 1;
                if let Some(domain) = order.domain.clone() {
                    // The order mails themselves often link to the order page.
                    if let Some(best) = store.order_mail_portal(&order, &domain)?
                        && store.offer_portal(
                            &domain,
                            &best.0,
                            Origin::InvoiceMail,
                            Some(best.1),
                        )?
                    {
                        report.portals += 1;
                    }
                    portal_domains.insert(domain);
                }
            }
        }
    }
    Ok(())
}

/// Looks through stored mail of the sender (welcome and account mails
/// first) for a customer portal link. Returns whether a link was stored.
fn portal_from_sender_mails(store: &Store, domain: &str) -> Result<bool> {
    let mails = store.sender_mails_with_body(domain, 200)?;
    let (welcome, other): (Vec<_>, Vec<_>) = mails
        .into_iter()
        .partition(|(_, subject)| portal::is_welcome_subject(subject));
    let picks = welcome
        .into_iter()
        .take(PORTAL_WELCOME_MAILS)
        .map(|(id, _)| (id, Origin::Welcome))
        .chain(
            other
                .into_iter()
                .take(PORTAL_OTHER_MAILS)
                .map(|(id, _)| (id, Origin::Other)),
        );
    let mut changed = false;
    for (id, origin) in picks {
        let Some(raw) = store.body(id)? else { continue };
        let c = content::parse(&raw);
        let links = portal::find_portal_links(
            c.html.as_deref(),
            c.text.as_deref().unwrap_or(""),
            domain,
            origin,
        );
        if let Some(best) = links.first() {
            changed |= store.offer_portal(domain, best, origin, Some(id))?;
        }
    }
    Ok(changed)
}

// ----- store: writing ----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Created,
    Updated,
    Unchanged,
}

pub(crate) struct NewFile {
    name: String,
    mime_type: String,
    data: Vec<u8>,
    /// An attachment beats a copy of the mail.
    is_attachment: bool,
}

pub(crate) struct NewInvoice {
    account_id: i64,
    issuer: String,
    domain: String,
    title: String,
    number: Option<String>,
    number_norm: Option<String>,
    amount_cents: Option<i64>,
    currency: Option<String>,
    issued_at: i64,
    due_at: Option<i64>,
    disposition: Option<PaymentDisposition>,
    message_id: i64,
    rfc_message_id: Option<String>,
    case_id: Option<i64>,
    file: Option<NewFile>,
}

/// Paid state of a new invoice. Mirrors the case logic: only an invoice
/// that needs a transfer is open.
fn paid_for(d: Option<PaymentDisposition>) -> (bool, Option<&'static str>) {
    match d {
        Some(PaymentDisposition::AlreadyPaid) => (true, Some(NOTE_PAID)),
        Some(PaymentDisposition::DirectDebit) => (true, Some(NOTE_DEBIT)),
        Some(PaymentDisposition::NeedsTransfer) => (false, None),
        None => (true, Some(NOTE_INFO)),
    }
}

impl Store {
    /// Mail since `since` the invoice pass has not looked at yet, or looked
    /// at before its text was downloaded. Only authenticated, non-list mail
    /// outside junk, trash and drafts.
    fn invoice_scan_batch(&self, since: i64, limit: usize) -> Result<Vec<ScanMessage>> {
        let sql = format!(
            "{SCAN_SELECT}
             LEFT JOIN invoice_scan s ON s.message_id = m.id
             WHERE m.date >= ?1
               AND (s.message_id IS NULL OR (s.with_body = 0 AND m.body_text IS NOT NULL))
               AND m.dmarc_pass != 0 AND m.list_id IS NULL
               AND COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts')
             ORDER BY m.date ASC, m.id ASC
             LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, limit as i64], scan_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn mark_invoice_scanned(&self, message_id: i64, with_body: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO invoice_scan (message_id, with_body) VALUES (?1, ?2)
             ON CONFLICT (message_id) DO UPDATE SET with_body = excluded.with_body",
            params![message_id, with_body],
        )?;
        Ok(())
    }

    /// Account of a message and whether it has attachments (known once the
    /// body is stored).
    fn message_account(&self, message_id: i64) -> Result<(i64, bool)> {
        Ok(self.conn.query_row(
            "SELECT f.account_id, m.has_attachments FROM messages m
             JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
            [message_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// Stored mail of a sender domain with downloaded body, newest first:
    /// `(id, subject)`. Authenticated mail only, and only from the sender's
    /// own domain (checked again here, the SQL match is a pre-filter).
    fn sender_mails_with_body(&self, domain: &str, limit: usize) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, COALESCE(m.subject, ''), m.from_addr FROM messages m
             JOIN folders f ON f.id = m.folder_id
             WHERE m.dmarc_pass != 0 AND m.body_text IS NOT NULL
               AND COALESCE(f.role, '') NOT IN ('junk', 'trash', 'drafts', 'sent')
               AND (lower(m.from_addr) LIKE '%@' || ?1 OR lower(m.from_addr) LIKE '%.' || ?1)
             ORDER BY m.date DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![domain, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, subject, from) = row?;
            if from.as_deref().and_then(base_domain).as_deref() == Some(domain) {
                out.push((id, subject));
            }
        }
        Ok(out)
    }

    /// Order cases that count as purchases: not dismissed, not cancelled and
    /// with an amount.
    fn order_cases(&self) -> Result<Vec<OrderCase>> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.title, c.counterparty, c.domain, c.amount_cents, c.currency,
                    (SELECT MIN(e.occurred_at) FROM case_events e WHERE e.case_id = c.id),
                    (SELECT e.message_id FROM case_events e
                       WHERE e.case_id = c.id AND e.message_id IS NOT NULL
                       ORDER BY e.occurred_at ASC, e.id ASC LIMIT 1),
                    (SELECT f.account_id FROM case_events e
                       JOIN messages m ON m.id = e.message_id
                       JOIN folders f ON f.id = m.folder_id
                       WHERE e.case_id = c.id
                       ORDER BY e.occurred_at ASC, e.id ASC LIMIT 1)
             FROM cases c
             WHERE c.kind = 'order' AND c.status != 'rejected'
               AND c.amount_cents IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM case_events e
                               WHERE e.case_id = c.id AND e.kind = 'order.cancelled')",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(OrderCase {
                case_id: r.get(0)?,
                title: r.get(1)?,
                counterparty: r.get(2)?,
                domain: r.get(3)?,
                amount_cents: r.get(4)?,
                currency: r.get(5)?,
                first_at: r.get::<_, Option<i64>>(6)?.unwrap_or(0),
                first_message_id: r.get(7)?,
                account_id: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `(id of the order row, whether a real invoice row exists)` for a case.
    fn order_row_state(&self, case_id: i64) -> Result<(Option<i64>, bool)> {
        let order = self
            .conn
            .query_row(
                "SELECT id FROM invoices WHERE case_id = ?1 AND from_order = 1",
                [case_id],
                |r| r.get(0),
            )
            .optional()?;
        let invoice: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM invoices WHERE case_id = ?1 AND from_order = 0)",
            [case_id],
            |r| r.get(0),
        )?;
        Ok((order, invoice))
    }

    fn insert_order_row(&self, o: &OrderCase, account_id: i64) -> Result<()> {
        let t = o.title.trim();
        let title = if t.is_empty() {
            "Bestellung".to_string()
        } else if t.starts_with("Bestellung") {
            t.to_string()
        } else {
            format!("Bestellung {t}")
        };
        self.conn.execute(
            "INSERT INTO invoices (account_id, issuer, domain, title, amount_cents, currency,
                issued_at, paid, paid_source, paid_note, evidence, message_id, case_id, from_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, 'auto', ?8,
                CASE WHEN EXISTS (SELECT 1 FROM sender_portals p WHERE p.domain = ?3)
                     THEN 'portal' ELSE 'none' END,
                ?9, ?10, 1)",
            params![
                account_id,
                o.counterparty,
                o.domain,
                title,
                o.amount_cents,
                o.currency,
                o.first_at,
                NOTE_ORDER,
                o.first_message_id,
                o.case_id,
            ],
        )?;
        Ok(())
    }

    /// Keeps the amount of an order row in step with its case.
    fn sync_order_row(&self, o: &OrderCase) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE invoices SET amount_cents = ?2, currency = ?3
             WHERE case_id = ?1 AND from_order = 1
               AND (amount_cents IS NOT ?2 OR currency IS NOT ?3)",
            params![o.case_id, o.amount_cents, o.currency],
        )?;
        Ok(n > 0)
    }

    fn drop_order_row(&self, case_id: i64) -> Result<()> {
        // A user decision on the placeholder row is kept rather than lost.
        self.conn.execute(
            "DELETE FROM invoices WHERE case_id = ?1 AND from_order = 1
               AND paid_source IS NOT 'user' AND kind_override IS NULL",
            [case_id],
        )?;
        Ok(())
    }

    /// Removes order rows of cases the user dismissed or that were cancelled.
    fn drop_dismissed_order_rows(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM invoices
             WHERE from_order = 1 AND paid_source IS NOT 'user' AND kind_override IS NULL
               AND case_id IN (
                   SELECT c.id FROM cases c
                   WHERE c.status = 'rejected'
                      OR EXISTS (SELECT 1 FROM case_events e
                                 WHERE e.case_id = c.id AND e.kind = 'order.cancelled'))",
            [],
        )?;
        Ok(())
    }

    /// A link to the order page found in the mails of the order itself:
    /// `(link, message id)`.
    fn order_mail_portal(&self, o: &OrderCase, domain: &str) -> Result<Option<(PortalLink, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.message_id FROM case_events e
             JOIN messages m ON m.id = e.message_id
             WHERE e.case_id = ?1 AND m.dmarc_pass != 0 AND m.body_text IS NOT NULL
             ORDER BY e.occurred_at ASC, e.id ASC LIMIT 5",
        )?;
        let ids: Vec<i64> = stmt
            .query_map([o.case_id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut best: Option<(PortalLink, i64)> = None;
        for id in ids {
            let Some(raw) = self.body(id)? else { continue };
            let c = content::parse(&raw);
            let links = portal::find_portal_links(
                c.html.as_deref(),
                c.text.as_deref().unwrap_or(""),
                domain,
                Origin::InvoiceMail,
            );
            if let Some(link) = links.into_iter().next()
                && best.as_ref().is_none_or(|(b, _)| link.score > b.score)
            {
                best = Some((link, id));
            }
        }
        Ok(best)
    }

    /// Mail without downloaded content that may carry an invoice or order
    /// step, newest first. The subject test is done here, in Rust, because it
    /// is a set of regexes; the query bounds what it has to look at to mail
    /// inside the ledger window, under the size cap and still without body.
    pub fn ledger_body_candidates(
        &self,
        folder_id: i64,
        since: i64,
        max_size: u32,
        limit: usize,
    ) -> Result<Vec<(i64, u32)>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.uid, m.subject FROM messages m
             WHERE m.folder_id = ?1 AND COALESCE(m.date, 0) >= ?2
               AND COALESCE(m.size, 0) <= ?3 AND m.subject IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM message_bodies b WHERE b.message_id = m.id)
             ORDER BY m.date DESC",
        )?;
        let mut rows = stmt.query(params![folder_id, since, max_size])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let subject: String = row.get(2)?;
            if is_ledger_candidate_subject(&subject) {
                out.push((row.get(0)?, row.get(1)?));
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn invoice_has_attachment(&self, invoice_id: i64) -> Result<bool> {
        let evidence: String = self.conn.query_row(
            "SELECT evidence FROM invoices WHERE id = ?1",
            [invoice_id],
            |r| r.get(0),
        )?;
        Ok(evidence == "attachment")
    }

    /// Stores `link` for the sender domain when it beats what is known.
    /// Returns whether anything changed.
    pub(crate) fn offer_portal(
        &self,
        domain: &str,
        link: &PortalLink,
        origin: Origin,
        message_id: Option<i64>,
    ) -> Result<bool> {
        let known: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT score, url FROM sender_portals WHERE domain = ?1",
                [domain],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((score, _)) = known
            && i64::from(link.score) <= score
        {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO sender_portals (domain, url, label, origin, score, message_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch())
             ON CONFLICT (domain) DO UPDATE SET
                url = excluded.url, label = excluded.label, origin = excluded.origin,
                score = excluded.score, message_id = excluded.message_id,
                updated_at = excluded.updated_at",
            params![
                domain,
                link.url,
                link.label,
                origin.as_str(),
                link.score,
                message_id
            ],
        )?;
        // Invoices that had nothing to show now have a place to go to.
        self.conn.execute(
            "UPDATE invoices SET evidence = 'portal' WHERE domain = ?1 AND evidence = 'none'",
            [domain],
        )?;
        Ok(true)
    }

    /// Records an invoice or, when it is known already (same `Message-ID`,
    /// or same sender and invoice number), merges the new mail into it. A
    /// decision of the user about "paid" is never touched.
    pub(crate) fn record_invoice(&self, n: &NewInvoice) -> Result<(i64, Outcome)> {
        let mut existing: Option<i64> = None;
        if let Some(rfc) = n.rfc_message_id.as_deref() {
            existing = self
                .conn
                .query_row(
                    "SELECT id FROM invoices WHERE account_id = ?1 AND rfc_message_id = ?2
                     ORDER BY id LIMIT 1",
                    params![n.account_id, rfc],
                    |r| r.get(0),
                )
                .optional()?;
        }
        if existing.is_none()
            && let Some(norm) = n.number_norm.as_deref().filter(|s| !s.is_empty())
        {
            existing = self
                .conn
                .query_row(
                    "SELECT id FROM invoices
                     WHERE account_id = ?1 AND domain = ?2 AND number_norm = ?3
                     ORDER BY id LIMIT 1",
                    params![n.account_id, n.domain, norm],
                    |r| r.get(0),
                )
                .optional()?;
        }
        // An invoice for an order that so far only has its placeholder row
        // takes that row over instead of listing the purchase twice.
        let mut upgraded = false;
        if existing.is_none()
            && let Some(case_id) = n.case_id
        {
            existing = self
                .conn
                .query_row(
                    "SELECT id FROM invoices WHERE case_id = ?1 AND from_order = 1",
                    [case_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                self.upgrade_order_row(id, n)?;
                upgraded = true;
            }
        }
        let (id, mut outcome) = match existing {
            Some(id) => {
                let merged = self.merge_invoice(id, n)?;
                (id, if upgraded { Outcome::Updated } else { merged })
            }
            None => {
                let (paid, note) = paid_for(n.disposition);
                self.conn.execute(
                    "INSERT INTO invoices (account_id, issuer, domain, title, invoice_number,
                        number_norm, amount_cents, currency, issued_at, due_at, paid, paid_source,
                        paid_note, message_id, rfc_message_id, case_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![
                        n.account_id,
                        n.issuer,
                        n.domain,
                        n.title,
                        n.number,
                        n.number_norm,
                        n.amount_cents,
                        n.currency,
                        n.issued_at,
                        n.due_at,
                        paid,
                        paid.then_some("auto"),
                        note,
                        n.message_id,
                        n.rfc_message_id,
                        n.case_id,
                    ],
                )?;
                (self.conn.last_insert_rowid(), Outcome::Created)
            }
        };
        if let Some(file) = &n.file
            && self.store_invoice_file(id, file)?
            && outcome == Outcome::Unchanged
        {
            outcome = Outcome::Updated;
        }
        self.refresh_evidence(id)?;
        Ok((id, outcome))
    }

    /// Turns the placeholder of an order into an ordinary invoice row. The
    /// invoice mail knows better than the order mail (what was paid, when),
    /// except for what the user decided.
    fn upgrade_order_row(&self, id: i64, n: &NewInvoice) -> Result<()> {
        let (paid, note) = paid_for(n.disposition);
        self.conn.execute(
            "UPDATE invoices SET from_order = 0, title = ?2, issued_at = ?3,
                amount_cents = COALESCE(?4, amount_cents),
                currency = COALESCE(?5, currency),
                message_id = ?6, rfc_message_id = ?7,
                evidence = 'none',
                paid = CASE WHEN paid_source IS 'user' THEN paid ELSE ?8 END,
                paid_note = CASE WHEN paid_source IS 'user' THEN paid_note ELSE ?9 END,
                paid_source = CASE WHEN paid_source IS 'user' THEN paid_source
                                   WHEN ?8 THEN 'auto' ELSE NULL END
             WHERE id = ?1",
            params![
                id,
                n.title,
                n.issued_at,
                n.amount_cents,
                n.currency,
                n.message_id,
                n.rfc_message_id,
                paid,
                note,
            ],
        )?;
        Ok(())
    }

    fn merge_invoice(&self, id: i64, n: &NewInvoice) -> Result<Outcome> {
        #[allow(clippy::type_complexity)]
        let (amount, currency, due_at, number, message_id, case_id, paid, source, note): (
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            bool,
            Option<String>,
            Option<String>,
        ) = self.conn.query_row(
            "SELECT amount_cents, currency, due_at, invoice_number, message_id, case_id,
                    paid, paid_source, paid_note
             FROM invoices WHERE id = ?1",
            [id],
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
                ))
            },
        )?;
        let mut changed = false;
        // Fill gaps only: the first mail's values stand.
        let amount_new = amount.or(n.amount_cents);
        let currency_new = currency.clone().or_else(|| n.currency.clone());
        let due_new = due_at.or(n.due_at);
        let number_new = number.clone().or_else(|| n.number.clone());
        let message_new = message_id.or(Some(n.message_id));
        let case_new = case_id.or(n.case_id);
        if (
            amount_new,
            &currency_new,
            due_new,
            &number_new,
            message_new,
            case_new,
        ) != (amount, &currency, due_at, &number, message_id, case_id)
        {
            changed = true;
            self.conn.execute(
                "UPDATE invoices SET amount_cents = ?2, currency = ?3, due_at = ?4,
                    invoice_number = ?5,
                    number_norm = COALESCE(number_norm, ?6),
                    message_id = ?7, case_id = ?8,
                    rfc_message_id = COALESCE(rfc_message_id, ?9)
                 WHERE id = ?1",
                params![
                    id,
                    amount_new,
                    currency_new,
                    due_new,
                    number_new,
                    n.number_norm,
                    message_new,
                    case_new,
                    n.rfc_message_id,
                ],
            )?;
        }
        if source.as_deref() != Some("user") {
            let new_state = match n.disposition {
                Some(PaymentDisposition::AlreadyPaid) => Some((true, Some(NOTE_PAID))),
                Some(PaymentDisposition::DirectDebit) => Some((true, Some(NOTE_DEBIT))),
                // A reminder reopens an invoice that was only "for information".
                Some(PaymentDisposition::NeedsTransfer)
                    if paid && note.as_deref() == Some(NOTE_INFO) =>
                {
                    Some((false, None))
                }
                _ => None,
            };
            if let Some((p, nt)) = new_state
                && (p != paid || note.as_deref() != nt)
            {
                changed = true;
                self.conn.execute(
                    "UPDATE invoices SET paid = ?2, paid_source = ?3, paid_note = ?4 WHERE id = ?1",
                    params![id, p, p.then_some("auto"), nt],
                )?;
            }
        }
        Ok(if changed {
            Outcome::Updated
        } else {
            Outcome::Unchanged
        })
    }

    /// Keeps `file` as the proof unless a better one is stored already.
    /// Returns whether it was written.
    fn store_invoice_file(&self, invoice_id: i64, file: &NewFile) -> Result<bool> {
        let evidence: String = self.conn.query_row(
            "SELECT evidence FROM invoices WHERE id = ?1",
            [invoice_id],
            |r| r.get(0),
        )?;
        let better = match evidence.as_str() {
            "attachment" => false,
            "mail" => file.is_attachment,
            _ => true,
        };
        if !better {
            return Ok(false);
        }
        let compressed = zstd::bulk::compress(&file.data, ZSTD_LEVEL)?;
        self.conn.execute(
            "INSERT INTO invoice_files (invoice_id, name, mime_type, size, data_zstd)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (invoice_id) DO UPDATE SET
                name = excluded.name, mime_type = excluded.mime_type,
                size = excluded.size, data_zstd = excluded.data_zstd",
            params![
                invoice_id,
                file.name,
                file.mime_type,
                file.data.len() as i64,
                compressed
            ],
        )?;
        let kind = if file.is_attachment {
            InvoiceEvidence::Attachment
        } else {
            InvoiceEvidence::Mail
        };
        self.conn.execute(
            "UPDATE invoices SET evidence = ?2 WHERE id = ?1",
            params![invoice_id, kind.as_str()],
        )?;
        Ok(true)
    }

    /// Without a stored file the evidence is the portal link, if the sender
    /// has one.
    fn refresh_evidence(&self, invoice_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE invoices SET evidence = CASE WHEN EXISTS
                    (SELECT 1 FROM sender_portals p WHERE p.domain = invoices.domain)
                 THEN 'portal' ELSE 'none' END
             WHERE id = ?1 AND evidence IN ('none', 'portal')",
            [invoice_id],
        )?;
        Ok(())
    }

    // ----- store: reading and user decisions -----------------------------------

    /// All invoices, newest first.
    fn all_invoices(&self) -> Result<Vec<InvoiceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.account_id, a.email, a.kind, i.kind_override, i.issuer, i.domain,
                    i.title, i.invoice_number, i.amount_cents, i.currency, i.issued_at, i.due_at,
                    i.paid, i.paid_source, i.paid_note, i.evidence, f.name, p.url, p.label,
                    i.message_id, i.case_id
             FROM invoices i
             JOIN accounts a ON a.id = i.account_id
             LEFT JOIN invoice_files f ON f.invoice_id = i.id
             LEFT JOIN sender_portals p ON p.domain = i.domain
             ORDER BY i.issued_at DESC, i.id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let account_kind: String = r.get(3)?;
            let override_kind: Option<String> = r.get(4)?;
            let source: Option<String> = r.get(14)?;
            let evidence: String = r.get(16)?;
            let overridden = override_kind.is_some();
            Ok(InvoiceRecord {
                id: r.get(0)?,
                account_id: r.get(1)?,
                account_email: r.get(2)?,
                kind: override_kind
                    .as_deref()
                    .unwrap_or(&account_kind)
                    .parse()
                    .unwrap_or_default(),
                kind_overridden: overridden,
                issuer: r.get(5)?,
                domain: r.get(6)?,
                title: r.get(7)?,
                invoice_number: r.get(8)?,
                amount_cents: r.get(9)?,
                currency: r.get(10)?,
                issued_at: r.get(11)?,
                due_at: r.get(12)?,
                paid: r.get(13)?,
                paid_source: match source.as_deref() {
                    Some("user") => Some(PaidSource::User),
                    Some(_) => Some(PaidSource::Auto),
                    None => None,
                },
                paid_note: r.get(15)?,
                evidence: InvoiceEvidence::parse(&evidence),
                file_name: r.get(17)?,
                portal_url: r.get(18)?,
                portal_label: r.get(19)?,
                message_id: r.get(20)?,
                case_id: r.get(21)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Invoices issued in the calendar year `year` (local time), or all of
    /// them, newest first.
    pub fn invoices(&self, year: Option<i32>) -> Result<Vec<InvoiceRecord>> {
        let mut all = self.all_invoices()?;
        if let Some(y) = year {
            all.retain(|i| local_year(i.issued_at) == y);
        }
        Ok(all)
    }

    /// Years that have invoices, newest first.
    pub fn invoice_years(&self) -> Result<Vec<i32>> {
        let mut stmt = self.conn.prepare("SELECT issued_at FROM invoices")?;
        let years: BTreeSet<i32> = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(local_year)
            .collect();
        Ok(years.into_iter().rev().collect())
    }

    /// The user decides whether an invoice is paid; the pass never undoes it.
    pub fn set_invoice_paid(&self, id: i64, paid: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE invoices SET paid = ?2, paid_source = 'user',
                paid_note = ?3 WHERE id = ?1",
            params![
                id,
                paid,
                if paid {
                    "Manuell als bezahlt markiert"
                } else {
                    "Manuell als offen markiert"
                }
            ],
        )?;
        Ok(())
    }

    /// Overrides the kind of one invoice; `None` follows the account again.
    pub fn set_invoice_kind(&self, id: i64, kind: Option<AccountKind>) -> Result<()> {
        self.conn.execute(
            "UPDATE invoices SET kind_override = ?2 WHERE id = ?1",
            params![id, kind.map(|k| k.to_string())],
        )?;
        Ok(())
    }

    pub fn account_kinds(&self) -> Result<Vec<AccountKindInfo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, email, kind FROM accounts ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            let kind: String = r.get(2)?;
            Ok(AccountKindInfo {
                account_id: r.get(0)?,
                email: r.get(1)?,
                kind: kind.parse().unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_account_kind(&self, account_id: i64, kind: AccountKind) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET kind = ?2 WHERE id = ?1",
            params![account_id, kind.to_string()],
        )?;
        Ok(())
    }

    /// The stored proof file of an invoice.
    pub fn invoice_file(&self, id: i64) -> Result<Option<InvoiceFile>> {
        let row: Option<(String, String, i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT name, mime_type, size, data_zstd FROM invoice_files WHERE invoice_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        match row {
            Some((name, mime_type, size, compressed)) => Ok(Some(InvoiceFile {
                name,
                mime_type,
                data: zstd::bulk::decompress(&compressed, size.max(0) as usize)?,
            })),
            None => Ok(None),
        }
    }
}

// ----- export ------------------------------------------------------------------

fn local_year(ts: i64) -> i32 {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map_or(1970, |d| d.year())
}

fn local_date(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// `49,90` for 4990 cents.
fn format_amount(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    format!("{sign}{},{:02}", abs / 100, abs % 100)
}

/// One CSV field. Text from mail could start a formula in a spreadsheet, so
/// a leading `=`, `+`, `-` or `@` is defused.
fn csv_field(value: &str) -> String {
    let mut v = value.replace(['\r', '\n'], " ");
    if v.starts_with(['=', '+', '-', '@']) {
        v.insert(0, '\'');
    }
    if v.contains([';', '"']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

fn unique_dir(base: &Path, name: &str) -> PathBuf {
    let first = base.join(name);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| base.join(format!("{name} ({n})")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// Exports the invoices of `year` below `base`: a folder
/// `Halcyon-Rechnungen-<year>` with the proof files sorted into `privat`
/// and `geschaeftlich`, and `uebersicht.csv` listing every invoice.
pub fn export_year(store: &Store, year: i32, base: &Path) -> Result<InvoiceExportReport> {
    let root = unique_dir(base, &format!("Halcyon-Rechnungen-{year}"));
    let private_dir = root.join("privat");
    let business_dir = root.join("geschaeftlich");
    std::fs::create_dir_all(&private_dir)?;
    std::fs::create_dir_all(&business_dir)?;

    let mut csv = String::from("\u{feff}");
    csv.push_str(
        "Datum;Absender;Rechnungsnummer;Betrag;Währung;Bezahlt;Art;Nachweis;Datei;Portal\r\n",
    );
    let mut without_proof = 0;
    let records = store.invoices(Some(year))?;
    // Oldest first reads like a ledger.
    for inv in records.iter().rev() {
        let (dir, dir_name, kind_label) = match inv.kind {
            AccountKind::Private => (&private_dir, "privat", "Privat"),
            AccountKind::Business => (&business_dir, "geschaeftlich", "Geschäftlich"),
        };
        let mut file_rel = String::new();
        if let Some(file) = store.invoice_file(inv.id)? {
            let ext = file
                .name
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .filter(|e| {
                    !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric())
                })
                .unwrap_or_else(|| "bin".into());
            let issuer: String = inv
                .issuer
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("_")
                .chars()
                .take(40)
                .collect();
            let amount = inv
                .amount_cents
                .map(|c| {
                    format!(
                        "{}{}",
                        format_amount(c),
                        inv.currency.as_deref().unwrap_or("")
                    )
                })
                .unwrap_or_else(|| "ohne-Betrag".into());
            let stem = content::safe_file_name(&format!(
                "{}_{}_{}",
                local_date(inv.issued_at),
                issuer,
                amount
            ));
            let mut name = format!("{stem}.{ext}");
            let mut n = 2;
            while dir.join(&name).exists() {
                name = format!("{stem}_{n}.{ext}");
                n += 1;
            }
            std::fs::write(dir.join(&name), &file.data)?;
            file_rel = format!("{dir_name}/{name}");
        } else {
            without_proof += 1;
        }
        let evidence = match inv.evidence {
            InvoiceEvidence::Attachment => "Anhang",
            InvoiceEvidence::Mail => "E-Mail",
            InvoiceEvidence::Portal => "Portal",
            InvoiceEvidence::None => "Fehlt",
        };
        let fields = [
            local_date(inv.issued_at),
            csv_field(&inv.issuer),
            csv_field(inv.invoice_number.as_deref().unwrap_or("")),
            inv.amount_cents.map(format_amount).unwrap_or_default(),
            inv.currency.clone().unwrap_or_default(),
            if inv.paid { "Ja" } else { "Nein" }.to_string(),
            kind_label.to_string(),
            evidence.to_string(),
            csv_field(&file_rel),
            csv_field(inv.portal_url.as_deref().unwrap_or("")),
        ];
        csv.push_str(&fields.join(";"));
        csv.push_str("\r\n");
    }
    std::fs::write(root.join("uebersicht.csv"), csv)?;
    Ok(InvoiceExportReport {
        path: root.display().to_string(),
        exported: records.len(),
        without_proof,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Account, AuthMethod, MessageHeader, Security, ServerConfig};

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn setup() -> (Store, i64, i64) {
        let store = Store::open_in_memory().unwrap();
        let server = ServerConfig {
            host: "imap.example.org".into(),
            port: 993,
            security: Security::Tls,
            username: "me@example.org".into(),
            auth: AuthMethod::Password,
        };
        let a = store
            .add_account(&Account {
                id: 0,
                email: "me@example.org".into(),
                display_name: None,
                imap: server,
                smtp: None,
            })
            .unwrap();
        let inbox = store
            .upsert_folder(a.id, "INBOX", None, Some(FolderRole::Inbox), true)
            .unwrap();
        (store, a.id, inbox.id)
    }

    fn header(uid: u32, subject: &str, from: &str, date: i64) -> MessageHeader {
        MessageHeader {
            uid,
            message_id: Some(format!("m{uid}@shop.example.org")),
            subject: Some(subject.into()),
            from_name: Some("Beispiel Shop".into()),
            from_addr: Some(from.into()),
            date: Some(date),
            dmarc_pass: true,
            ..Default::default()
        }
    }

    /// A raw mail the store accepts as DMARC-authenticated.
    fn raw_mail(from: &str, subject: &str, body_html: &str, pdf: Option<(&str, &[u8])>) -> Vec<u8> {
        use base64::Engine;
        let domain = from.rsplit('@').next().unwrap();
        let mut s = format!(
            "Authentication-Results: mx.example.org; dmarc=pass header.from={domain}\r\n\
             From: Beispiel Shop <{from}>\r\nTo: me@example.org\r\nSubject: {subject}\r\n\
             MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
             --b1\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{body_html}\r\n"
        );
        if let Some((name, data)) = pdf {
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            s.push_str(&format!(
                "--b1\r\nContent-Type: application/pdf; name=\"{name}\"\r\n\
                 Content-Disposition: attachment; filename=\"{name}\"\r\n\
                 Content-Transfer-Encoding: base64\r\n\r\n{b64}\r\n"
            ));
        }
        s.push_str("--b1--\r\n");
        s.into_bytes()
    }

    fn add_mail(
        store: &mut Store,
        folder: i64,
        uid: u32,
        subject: &str,
        from: &str,
        date: i64,
        raw: Option<Vec<u8>>,
    ) -> i64 {
        store
            .insert_messages(folder, &[header(uid, subject, from, date)])
            .unwrap();
        let id = store
            .folder_messages(folder, 1000)
            .unwrap()
            .iter()
            .find(|m| m.header.uid == uid)
            .unwrap()
            .id;
        if let Some(raw) = raw {
            store.store_body(id, &raw).unwrap();
        }
        id
    }

    const FROM: &str = "billing@shop.example.org";
    const PDF: &[u8] = b"%PDF-1.4 synthetic";

    fn invoice_mail(subject: &str, body: &str, with_pdf: bool) -> Vec<u8> {
        raw_mail(
            FROM,
            subject,
            body,
            with_pdf.then_some(("Rechnung_RE-2026-441.pdf", PDF)),
        )
    }

    #[test]
    fn invoice_with_pdf_has_attachment_evidence_and_stored_file() {
        let (mut store, _, inbox) = setup();
        let subject = "Ihre Rechnung RE-2026-441 über 49,90 €";
        add_mail(
            &mut store,
            inbox,
            1,
            subject,
            FROM,
            NOW - 10 * DAY,
            Some(invoice_mail(
                subject,
                "<p>Betrag 49,90 € per Lastschrift.</p>",
                true,
            )),
        );
        let report = run(&mut store, NOW).unwrap();
        assert_eq!(report.created, 1);
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1);
        let inv = &list[0];
        assert_eq!(inv.evidence, InvoiceEvidence::Attachment);
        assert_eq!(inv.amount_cents, Some(4990));
        assert_eq!(inv.invoice_number.as_deref(), Some("RE-2026-441"));
        assert_eq!(inv.file_name.as_deref(), Some("Rechnung_RE-2026-441.pdf"));
        assert_eq!(inv.domain.as_deref(), Some("example.org"));
        let file = store.invoice_file(inv.id).unwrap().unwrap();
        assert_eq!(file.data, PDF);
        // A second run changes nothing.
        assert!(!run(&mut store, NOW).unwrap().changed());
    }

    #[test]
    fn paid_state_follows_the_payment_disposition() {
        let (mut store, _, inbox) = setup();
        let debit = "Ihre Rechnung RE-2026-100 über 9,99 € wird per SEPA-Lastschrift eingezogen";
        let transfer = "Ihre Rechnung RE-2026-200 über 49,90 € – bitte überweisen";
        let info = "Ihre Rechnung RE-2026-300 über 5,00 €";
        for (uid, s) in [(1, debit), (2, transfer), (3, info)] {
            add_mail(
                &mut store,
                inbox,
                uid,
                s,
                FROM,
                NOW - DAY * uid as i64,
                None,
            );
        }
        run(&mut store, NOW).unwrap();
        let by_number = |n: &str| {
            store
                .invoices(None)
                .unwrap()
                .into_iter()
                .find(|i| i.invoice_number.as_deref() == Some(n))
                .unwrap()
        };
        let d = by_number("RE-2026-100");
        assert!(d.paid);
        assert_eq!(d.paid_source, Some(PaidSource::Auto));
        assert_eq!(
            d.paid_note.as_deref(),
            Some("Wird per Lastschrift eingezogen")
        );
        let t = by_number("RE-2026-200");
        assert!(!t.paid);
        assert_eq!(t.paid_source, None);
        let i = by_number("RE-2026-300");
        assert!(i.paid);
        assert_eq!(i.paid_note.as_deref(), Some("Zur Information"));
    }

    #[test]
    fn user_paid_flag_survives_later_mail_and_rescan() {
        let (mut store, _, inbox) = setup();
        let subject = "Ihre Rechnung RE-2026-200 über 49,90 € – bitte überweisen";
        add_mail(&mut store, inbox, 1, subject, FROM, NOW - 10 * DAY, None);
        run(&mut store, NOW).unwrap();
        let id = store.invoices(None).unwrap()[0].id;
        assert!(!store.invoices(None).unwrap()[0].paid);

        store.set_invoice_paid(id, true).unwrap();
        // The payment confirmation says the opposite of the user's decision.
        let reopen = "Zahlungserinnerung zur Rechnung RE-2026-200 über 49,90 € – bitte überweisen";
        add_mail(&mut store, inbox, 2, reopen, FROM, NOW - 2 * DAY, None);
        // And the original mail is looked at once more when its body arrives.
        let first = store.folder_messages(inbox, 10).unwrap();
        let first_id = first.iter().find(|m| m.header.uid == 1).unwrap().id;
        store
            .store_body(
                first_id,
                &invoice_mail(subject, "<p>Betrag 49,90 €</p>", false),
            )
            .unwrap();
        run(&mut store, NOW).unwrap();

        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].paid);
        assert_eq!(list[0].paid_source, Some(PaidSource::User));
    }

    #[test]
    fn payment_confirmation_updates_instead_of_duplicating() {
        let (mut store, _, inbox) = setup();
        add_mail(
            &mut store,
            inbox,
            1,
            "Ihre Rechnung RE-2026-200 über 49,90 € – bitte überweisen",
            FROM,
            NOW - 10 * DAY,
            None,
        );
        run(&mut store, NOW).unwrap();
        assert!(!store.invoices(None).unwrap()[0].paid);
        add_mail(
            &mut store,
            inbox,
            2,
            "Rechnung RE-2026-200 – vielen Dank für Ihre Zahlung, bezahlt per PayPal",
            FROM,
            NOW - 3 * DAY,
            None,
        );
        let report = run(&mut store, NOW).unwrap();
        assert_eq!(report.updated, 1);
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].paid);
        assert_eq!(list[0].paid_note.as_deref(), Some("Bezahlt"));
        // Issue date stays that of the invoice mail.
        assert_eq!(list[0].issued_at, NOW - 10 * DAY);
    }

    #[test]
    fn deleting_the_mail_keeps_invoice_and_file() {
        let (mut store, _, inbox) = setup();
        let subject = "Ihre Rechnung RE-2026-441 über 49,90 €";
        let id = add_mail(
            &mut store,
            inbox,
            1,
            subject,
            FROM,
            NOW - 10 * DAY,
            Some(invoice_mail(subject, "<p>49,90 €</p>", true)),
        );
        run(&mut store, NOW).unwrap();
        assert!(store.invoices(None).unwrap()[0].message_id.is_some());
        store
            .conn
            .execute("DELETE FROM messages WHERE id = ?1", [id])
            .unwrap();
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message_id, None);
        assert_eq!(store.invoice_file(list[0].id).unwrap().unwrap().data, PDF);
    }

    #[test]
    fn same_mail_in_two_folders_is_one_invoice() {
        let (mut store, account, inbox) = setup();
        let archive = store
            .upsert_folder(account, "Archiv", None, Some(FolderRole::Archive), true)
            .unwrap()
            .id;
        let subject = "Ihre Rechnung RE-2026-441 über 49,90 €";
        add_mail(&mut store, inbox, 1, subject, FROM, NOW - 10 * DAY, None);
        // Same Message-ID, other folder.
        let mut copy = header(1, subject, FROM, NOW - 10 * DAY);
        copy.message_id = Some("m1@shop.example.org".into());
        store.insert_messages(archive, &[copy]).unwrap();
        // Another message id but the same invoice number (resent invoice).
        add_mail(
            &mut store,
            inbox,
            2,
            "Erneut: Rechnung RE-2026-441 über 49,90 €",
            FROM,
            NOW - 5 * DAY,
            None,
        );
        run(&mut store, NOW).unwrap();
        assert_eq!(store.invoices(None).unwrap().len(), 1);
    }

    #[test]
    fn mail_without_attachment_is_stored_as_receipt() {
        let (mut store, _, inbox) = setup();
        let subject = "Ihre Rechnung RE-2026-555 über 12,00 €";
        add_mail(
            &mut store,
            inbox,
            1,
            subject,
            FROM,
            NOW - 10 * DAY,
            Some(invoice_mail(subject, "<p>Summe 12,00 €</p>", false)),
        );
        run(&mut store, NOW).unwrap();
        let inv = &store.invoices(None).unwrap()[0];
        assert_eq!(inv.evidence, InvoiceEvidence::Mail);
        let file = store.invoice_file(inv.id).unwrap().unwrap();
        assert!(file.name.ends_with(".eml"));
        assert_eq!(file.mime_type, "message/rfc822");
        assert!(String::from_utf8_lossy(&file.data).contains("Summe 12,00"));
    }

    #[test]
    fn later_attachment_replaces_receipt_copy() {
        let (mut store, _, inbox) = setup();
        let subject = "Ihre Rechnung RE-2026-441 über 49,90 €";
        add_mail(
            &mut store,
            inbox,
            1,
            subject,
            FROM,
            NOW - 10 * DAY,
            Some(invoice_mail(subject, "<p>49,90 €</p>", false)),
        );
        run(&mut store, NOW).unwrap();
        assert_eq!(
            store.invoices(None).unwrap()[0].evidence,
            InvoiceEvidence::Mail
        );
        add_mail(
            &mut store,
            inbox,
            2,
            "Rechnung RE-2026-441 als PDF",
            FROM,
            NOW - 9 * DAY,
            Some(invoice_mail(
                "Rechnung RE-2026-441 als PDF",
                "<p>anbei</p>",
                true,
            )),
        );
        run(&mut store, NOW).unwrap();
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].evidence, InvoiceEvidence::Attachment);
    }

    #[test]
    fn newsletters_replies_contracts_and_unauthenticated_mail_are_skipped() {
        let (mut store, _, inbox) = setup();
        let mut list_mail = header(1, "Rechnung RE-2026-001 über 5,00 €", FROM, NOW - DAY);
        list_mail.list_id = Some("<news.shop.example.org>".into());
        let mut spoof = header(2, "Rechnung RE-2026-002 über 5,00 €", FROM, NOW - DAY);
        spoof.dmarc_pass = false;
        let mut reply = header(3, "Re: Rechnung RE-2026-003 über 5,00 €", FROM, NOW - DAY);
        reply.in_reply_to = Some("<x@example.org>".into());
        let contract = header(
            4,
            "Kündigung Ihres Vertrags V-998877 eingegangen",
            FROM,
            NOW - DAY,
        );
        let own = header(
            5,
            "Rechnung RE-2026-005 über 5,00 €",
            "me@example.org",
            NOW - DAY,
        );
        store
            .insert_messages(inbox, &[list_mail, spoof, reply, contract, own])
            .unwrap();
        let report = run(&mut store, NOW).unwrap();
        assert_eq!(report.created, 0);
        assert!(store.invoices(None).unwrap().is_empty());
    }

    #[test]
    fn portal_link_from_invoice_mail_is_offered_without_attachment() {
        let (mut store, _, inbox) = setup();
        let subject = "Ihre Rechnung RE-2026-777 über 20,00 €";
        let html = r#"<p>Rechnung 20,00 €</p>
            <a href="https://kundencenter.shop.example.org/rechnungen?t=SECRET#x">Rechnung online abrufen</a>
            <a href="https://track.example.net/c?u=1">Mein Konto</a>
            <a href="http://www.shop.example.org/login">Login</a>
            <a href="https://www.shop.example.org/unsubscribe">Abmelden</a>"#;
        add_mail(
            &mut store,
            inbox,
            1,
            subject,
            FROM,
            NOW - DAY,
            Some(invoice_mail(subject, html, false)),
        );
        run(&mut store, NOW).unwrap();
        let inv = &store.invoices(None).unwrap()[0];
        // The mail copy is the proof; the portal is offered on top.
        assert_eq!(inv.evidence, InvoiceEvidence::Mail);
        assert_eq!(
            inv.portal_url.as_deref(),
            Some("https://kundencenter.shop.example.org/rechnungen")
        );
        assert_eq!(inv.portal_label.as_deref(), Some("Rechnung online abrufen"));
    }

    #[test]
    fn portal_evidence_when_only_the_subject_is_known_and_welcome_mail_exists() {
        let (mut store, _, inbox) = setup();
        // Welcome mail of the same sender with the customer center link.
        let welcome = "Willkommen bei Beispiel Shop";
        add_mail(
            &mut store,
            inbox,
            1,
            welcome,
            "service@mail.shop.example.org",
            NOW - 100 * DAY,
            Some(raw_mail(
                "service@mail.shop.example.org",
                welcome,
                r#"<a href="https://www.shop.example.org/kundencenter/login?x=1">Zum Kundencenter</a>"#,
                None,
            )),
        );
        // The invoice mail is known by header only, body still missing.
        add_mail(
            &mut store,
            inbox,
            2,
            "Ihre Rechnung RE-2026-888 über 30,00 €",
            FROM,
            NOW - DAY,
            None,
        );
        run(&mut store, NOW).unwrap();
        let inv = &store.invoices(None).unwrap()[0];
        assert_eq!(inv.evidence, InvoiceEvidence::Portal);
        assert_eq!(
            inv.portal_url.as_deref(),
            Some("https://www.shop.example.org/kundencenter/login")
        );
        assert_eq!(inv.portal_label.as_deref(), Some("Zum Kundencenter"));
    }

    #[test]
    fn foreign_domain_links_in_other_mail_are_not_offered() {
        let (mut store, _, inbox) = setup();
        add_mail(
            &mut store,
            inbox,
            1,
            "Neuigkeiten",
            "service@shop.example.org",
            NOW - 100 * DAY,
            Some(raw_mail(
                "service@shop.example.org",
                "Neuigkeiten",
                r#"<a href="https://evil.example.net/kundencenter">Zum Kundencenter</a>"#,
                None,
            )),
        );
        add_mail(
            &mut store,
            inbox,
            2,
            "Ihre Rechnung RE-2026-888 über 30,00 €",
            FROM,
            NOW - DAY,
            None,
        );
        run(&mut store, NOW).unwrap();
        let inv = &store.invoices(None).unwrap()[0];
        assert_eq!(inv.evidence, InvoiceEvidence::None);
        assert_eq!(inv.portal_url, None);
    }

    #[test]
    fn account_kind_and_override_decide_the_effective_kind() {
        let (mut store, account, inbox) = setup();
        add_mail(
            &mut store,
            inbox,
            1,
            "Ihre Rechnung RE-2026-441 über 49,90 €",
            FROM,
            NOW - DAY,
            None,
        );
        run(&mut store, NOW).unwrap();
        let id = store.invoices(None).unwrap()[0].id;
        assert_eq!(store.invoices(None).unwrap()[0].kind, AccountKind::Private);
        assert_eq!(store.account_kinds().unwrap()[0].kind, AccountKind::Private);

        store
            .set_account_kind(account, AccountKind::Business)
            .unwrap();
        let inv = store.invoices(None).unwrap().remove(0);
        assert_eq!(inv.kind, AccountKind::Business);
        assert!(!inv.kind_overridden);

        store
            .set_invoice_kind(id, Some(AccountKind::Private))
            .unwrap();
        let inv = store.invoices(None).unwrap().remove(0);
        assert_eq!(inv.kind, AccountKind::Private);
        assert!(inv.kind_overridden);

        store.set_invoice_kind(id, None).unwrap();
        assert_eq!(store.invoices(None).unwrap()[0].kind, AccountKind::Business);
    }

    #[test]
    fn years_filter_and_list() {
        let (mut store, _, inbox) = setup();
        // Mid-year dates stay in their year in every time zone.
        let y2026 = 1_782_000_000; // June 2026
        let y2025 = y2026 - 365 * DAY;
        add_mail(
            &mut store,
            inbox,
            1,
            "Rechnung RE-2026-001 über 5,00 €",
            FROM,
            y2026,
            None,
        );
        add_mail(
            &mut store,
            inbox,
            2,
            "Rechnung RE-2025-002 über 6,00 €",
            FROM,
            y2025,
            None,
        );
        run(&mut store, y2026 + DAY).unwrap();
        assert_eq!(store.invoice_years().unwrap(), vec![2026, 2025]);
        assert_eq!(store.invoices(Some(2025)).unwrap().len(), 1);
        assert_eq!(
            store.invoices(Some(2026)).unwrap()[0].amount_cents,
            Some(500)
        );
        assert_eq!(store.invoices(None).unwrap().len(), 2);
    }

    #[test]
    fn export_writes_folders_files_and_csv() {
        let (mut store, account, inbox) = setup();
        let y2026 = 1_782_000_000;
        let subject = "Ihre Rechnung RE-2026-441 über 49,90 €";
        add_mail(
            &mut store,
            inbox,
            1,
            subject,
            FROM,
            y2026,
            Some(invoice_mail(subject, "<p>49,90 €</p>", true)),
        );
        add_mail(
            &mut store,
            inbox,
            2,
            "Rechnung RE-2026-900 über 7,00 €",
            FROM,
            y2026 + DAY,
            None,
        );
        run(&mut store, y2026 + 2 * DAY).unwrap();
        store
            .set_account_kind(account, AccountKind::Business)
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let report = export_year(&store, 2026, tmp.path()).unwrap();
        assert_eq!(report.exported, 2);
        assert_eq!(report.without_proof, 1);
        let root = PathBuf::from(&report.path);
        assert!(root.ends_with("Halcyon-Rechnungen-2026"));
        assert!(root.join("privat").is_dir());
        let biz: Vec<_> = std::fs::read_dir(root.join("geschaeftlich"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(biz.len(), 1);
        assert!(biz[0].contains("Beispiel_Shop_49,90EUR"));
        assert!(biz[0].ends_with(".pdf"));
        let csv = std::fs::read_to_string(root.join("uebersicht.csv")).unwrap();
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains(
            "Datum;Absender;Rechnungsnummer;Betrag;Währung;Bezahlt;Art;Nachweis;Datei;Portal"
        ));
        assert!(csv.contains("RE-2026-441;49,90;EUR"));
        assert!(csv.contains("Geschäftlich;Anhang;geschaeftlich/"));
        assert!(csv.contains("RE-2026-900;7,00;EUR"));

        // A second export never overwrites the first.
        let again = export_year(&store, 2026, tmp.path()).unwrap();
        assert_ne!(again.path, report.path);
    }

    const SHOP: &str = "orders@shop.example.org";

    /// An order confirmation mail with an amount in the subject.
    fn order_mail(store: &mut Store, folder: i64, uid: u32, subject: &str, date: i64) -> i64 {
        add_mail(
            store,
            folder,
            uid,
            subject,
            SHOP,
            date,
            Some(raw_mail(
                SHOP,
                subject,
                "<p>Danke für deine Bestellung.</p>",
                None,
            )),
        )
    }

    fn run_all(store: &mut Store) -> InvoiceReport {
        crate::linker::run(store, NOW).unwrap();
        run(store, NOW).unwrap()
    }

    const ORDER_SUBJECT: &str = "Bestellung glnest-335478 bestätigt · Summe 59,90 €";

    #[test]
    fn order_case_is_mirrored_once_with_a_hint_for_the_account() {
        let (mut store, _, inbox) = setup();
        order_mail(&mut store, inbox, 1, ORDER_SUBJECT, NOW - 5 * DAY);
        let report = run_all(&mut store);
        assert_eq!(report.created, 1, "{:#?}", store.list_cases(false).unwrap());
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1);
        let row = &list[0];
        assert!(row.title.starts_with("Bestellung"), "{}", row.title);
        assert_eq!(row.amount_cents, Some(5990));
        assert_eq!(row.currency.as_deref(), Some("EUR"));
        assert_eq!(row.issued_at, NOW - 5 * DAY);
        assert!(row.paid);
        assert_eq!(row.paid_source, Some(PaidSource::Auto));
        assert_eq!(
            row.paid_note.as_deref(),
            Some("Bestellung · Rechnung im Kundenkonto")
        );
        // Not "mail": an order confirmation is no invoice.
        assert_eq!(row.evidence, InvoiceEvidence::None);
        assert!(row.case_id.is_some());
        // No duplicate on later runs.
        assert!(!run_all(&mut store).changed());
        assert_eq!(store.invoices(None).unwrap().len(), 1);
    }

    #[test]
    fn order_row_gets_portal_evidence_when_a_link_is_known() {
        let (mut store, _, inbox) = setup();
        add_mail(
            &mut store,
            inbox,
            1,
            ORDER_SUBJECT,
            SHOP,
            NOW - 5 * DAY,
            Some(raw_mail(
                SHOP,
                ORDER_SUBJECT,
                r#"<a href="https://www.shop.example.org/konto/bestellungen?id=1">Bestellung anzeigen</a>"#,
                None,
            )),
        );
        run_all(&mut store);
        let row = &store.invoices(None).unwrap()[0];
        assert_eq!(row.evidence, InvoiceEvidence::Portal);
        assert_eq!(
            row.portal_url.as_deref(),
            Some("https://www.shop.example.org/konto/bestellungen")
        );
        assert_eq!(
            row.portal_label.as_deref(),
            Some("Zur Bestellung im Kundenkonto")
        );
    }

    #[test]
    fn order_amount_follows_the_case_and_user_decisions_stay() {
        let (mut store, _, inbox) = setup();
        order_mail(&mut store, inbox, 1, ORDER_SUBJECT, NOW - 5 * DAY);
        run_all(&mut store);
        let id = store.invoices(None).unwrap()[0].id;
        store.set_invoice_paid(id, false).unwrap();
        store
            .set_invoice_kind(id, Some(AccountKind::Business))
            .unwrap();
        let case_id = store.invoices(None).unwrap()[0].case_id.unwrap();
        store
            .update_case_financials(case_id, Some(6490), Some("EUR"), None, None)
            .unwrap();
        run_all(&mut store);
        let row = &store.invoices(None).unwrap()[0];
        assert_eq!(store.invoices(None).unwrap().len(), 1);
        assert_eq!(row.amount_cents, Some(6490));
        assert!(!row.paid);
        assert_eq!(row.paid_source, Some(PaidSource::User));
        assert_eq!(row.kind, AccountKind::Business);
    }

    #[test]
    fn invoice_mail_for_the_order_takes_over_the_row() {
        let (mut store, _, inbox) = setup();
        order_mail(&mut store, inbox, 1, ORDER_SUBJECT, NOW - 5 * DAY);
        run_all(&mut store);
        let before = store.invoices(None).unwrap();
        assert_eq!(before.len(), 1);
        let case_id = before[0].case_id.unwrap();

        let subject = "Ihre Rechnung RE-2026-441 zu Bestellung glnest-335478 über 59,90 €";
        add_mail(
            &mut store,
            inbox,
            2,
            subject,
            SHOP,
            NOW - 2 * DAY,
            Some(raw_mail(
                SHOP,
                subject,
                "<p>Bereits bezahlt.</p>",
                Some(("Rechnung_RE-2026-441.pdf", PDF)),
            )),
        );
        run_all(&mut store);
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 1, "{list:#?}");
        let row = &list[0];
        assert_eq!(row.id, before[0].id);
        assert_eq!(row.case_id, Some(case_id));
        assert_eq!(row.evidence, InvoiceEvidence::Attachment);
        assert_eq!(row.invoice_number.as_deref(), Some("RE-2026-441"));
        assert_eq!(row.issued_at, NOW - 2 * DAY);
        assert_ne!(
            row.paid_note.as_deref(),
            Some("Bestellung · Rechnung im Kundenkonto")
        );
        assert!(store.invoice_file(row.id).unwrap().is_some());
        // And the order row is not created again.
        run_all(&mut store);
        assert_eq!(store.invoices(None).unwrap().len(), 1);
    }

    #[test]
    fn cancelled_order_is_not_listed() {
        let (mut store, _, inbox) = setup();
        order_mail(&mut store, inbox, 1, ORDER_SUBJECT, NOW - 5 * DAY);
        run_all(&mut store);
        assert_eq!(store.invoices(None).unwrap().len(), 1);
        add_mail(
            &mut store,
            inbox,
            2,
            "Bestellung storniert",
            SHOP,
            NOW - 4 * DAY,
            Some(raw_mail(
                SHOP,
                "Bestellung storniert",
                "<p>Bestellnr. glnest-335478 wurde storniert.</p>",
                None,
            )),
        );
        run_all(&mut store);
        assert!(store.invoices(None).unwrap().is_empty());
    }

    #[test]
    fn candidate_subjects_cover_invoices_and_order_steps() {
        assert!(is_ledger_candidate_subject("Ihre Rechnung RE-1"));
        assert!(is_ledger_candidate_subject("Your receipt"));
        assert!(is_ledger_candidate_subject("Bestellt: „Kopfhörer“"));
        assert!(is_ledger_candidate_subject(
            "Deine Bestellung wurde zugestellt"
        ));
        assert!(!is_ledger_candidate_subject("Neue Angebote für dich"));
    }

    #[test]
    fn backfill_selects_subject_window_size_and_bound() {
        let (mut store, _, inbox) = setup();
        let junk = store
            .upsert_folder(1, "Junk", None, Some(FolderRole::Junk), true)
            .unwrap()
            .id;
        let hdr = |uid: u32, subject: &str, date: i64, size: u32| MessageHeader {
            size: Some(size),
            ..header(uid, subject, SHOP, date)
        };
        let old = NOW - 400 * DAY;
        store
            .insert_messages(
                inbox,
                &[
                    hdr(1, "Ihre Rechnung 1", old, 10_000),
                    hdr(2, "Bestellt: „Kopfhörer“", old - DAY, 10_000),
                    hdr(3, "Newsletter Angebote", old, 10_000),
                    hdr(4, "Ihre Rechnung 4", NOW - 900 * DAY, 10_000),
                    hdr(5, "Ihre Rechnung 5", old, MAX_FILE_BYTES as u32 + 1),
                    hdr(6, "Ihre Rechnung 6", old - 2 * DAY, MAX_FILE_BYTES as u32),
                ],
            )
            .unwrap();
        store
            .insert_messages(junk, &[hdr(7, "Ihre Rechnung 7", old, 10_000)])
            .unwrap();
        let since = NOW - SCAN_WINDOW_SECS;
        let max = MAX_FILE_BYTES as u32;
        let uids: Vec<u32> = store
            .ledger_body_candidates(inbox, since, max, 100)
            .unwrap()
            .into_iter()
            .map(|(_, u)| u)
            .collect();
        // Newest first; not the newsletter, the too old or the too large mail.
        assert_eq!(uids, vec![1, 2, 6]);
        // Per-run bound.
        assert_eq!(
            store
                .ledger_body_candidates(inbox, since, max, 2)
                .unwrap()
                .len(),
            2
        );
        // Fetched mail drops out, so the next run continues further.
        let first = store.ledger_body_candidates(inbox, since, max, 1).unwrap()[0].0;
        store
            .store_body(first, &raw_mail(SHOP, "Ihre Rechnung 1", "x", None))
            .unwrap();
        let rest: Vec<u32> = store
            .ledger_body_candidates(inbox, since, max, 100)
            .unwrap()
            .into_iter()
            .map(|(_, u)| u)
            .collect();
        assert_eq!(rest, vec![2, 6]);
        // The sync leaves junk out by role; the query itself is per folder.
        assert_eq!(
            store
                .ledger_body_candidates(junk, since, max, 100)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn mail_scanned_without_body_is_processed_once_the_body_arrives() {
        let (mut store, _, inbox) = setup();
        // Header-only sync of an old mail: no DMARC result, no text.
        let subject = "Ihre Rechnung RE-2026-441 über 49,90 €";
        store
            .insert_messages(
                inbox,
                &[MessageHeader {
                    dmarc_pass: false,
                    ..header(1, subject, FROM, NOW - 300 * DAY)
                }],
            )
            .unwrap();
        let order = "Bestellt: „Kopfhörer Modell X“ für 59,90 €";
        store
            .insert_messages(
                inbox,
                &[MessageHeader {
                    dmarc_pass: false,
                    ..header(2, order, SHOP, NOW - 300 * DAY)
                }],
            )
            .unwrap();
        assert!(!run_all(&mut store).changed());
        assert!(store.list_cases(false).unwrap().is_empty());
        // The backfill downloads both; store_body sets text and DMARC.
        let ids: Vec<(i64, u32)> = store
            .folder_messages(inbox, 10)
            .unwrap()
            .iter()
            .map(|m| (m.id, m.header.uid))
            .collect();
        for (id, uid) in ids {
            let (from, subj) = if uid == 1 {
                (FROM, subject)
            } else {
                (SHOP, order)
            };
            store
                .store_body(id, &raw_mail(from, subj, "<p>Betrag</p>", None))
                .unwrap();
        }
        run_all(&mut store);
        assert_eq!(store.list_cases(false).unwrap().len(), 1);
        let list = store.invoices(None).unwrap();
        assert_eq!(list.len(), 2, "{list:#?}");
    }

    #[test]
    fn csv_fields_defuse_formulas_and_quote_separators() {
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_field("Müller; Söhne"), "\"Müller; Söhne\"");
        assert_eq!(format_amount(4990), "49,90");
        assert_eq!(format_amount(5), "0,05");
    }
}
