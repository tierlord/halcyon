//! First version of the case linker ("Verknüpfer", plan step 3).
//!
//! It is deliberately conservative: a mail only becomes part of a case on
//! strong evidence (an order or tracking number, the article name from the
//! same sender, or the reply headers of a conversation). Anything unclear is
//! left alone, following the plan's rule "lieber ein Vorschlag als eine
//! falsche Verknüpfung".
//!
//! Two kinds of cases are recognised so far:
//!
//! - **Bestellungen**: order confirmation, shipping and delivery mails,
//!   recognised from the subject (German and English) and linked by order
//!   number, tracking number or article name.
//! - **Gespräche**: reply chains in which both you and someone else wrote.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::Result;
use crate::cases::{
    CaseEventPhase, CaseKind, CaseStatus, EventDirection, NewCase, NewCaseEvent, normalize_key,
};
use crate::extract::tracking::{Evidence, find_shipments};
use crate::model::FolderRole;
use crate::store::{ScanMessage, Store};

/// Only mail from this period is looked at, so a first run does not fill
/// the board with years-old orders.
pub const SCAN_WINDOW_SECS: i64 = 120 * 86_400;

/// Open cases without news for this long are considered finished.
pub const STALE_AFTER_SECS: i64 = 21 * 86_400;

const BATCH: usize = 500;

/// What a run changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkReport {
    pub scanned: usize,
    pub linked: usize,
    pub created: usize,
    pub closed: usize,
}

impl LinkReport {
    pub fn changed(&self) -> bool {
        self.linked > 0 || self.created > 0 || self.closed > 0
    }
}

/// The step of an order a mail reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrderStep {
    Invoice,
    Placed,
    Shipped,
    Delivered,
}

impl OrderStep {
    fn kind(self) -> &'static str {
        match self {
            OrderStep::Invoice => "order.invoice",
            OrderStep::Placed => "order.placed",
            OrderStep::Shipped => "order.shipped",
            OrderStep::Delivered => "order.delivered",
        }
    }

    fn label(self) -> &'static str {
        match self {
            OrderStep::Invoice => "Rechnung",
            OrderStep::Placed => "Bestellt",
            OrderStep::Shipped => "Versandt",
            OrderStep::Delivered => "Zugestellt",
        }
    }

    fn phase(self) -> CaseEventPhase {
        match self {
            OrderStep::Placed => CaseEventPhase::Start,
            OrderStep::Invoice | OrderStep::Shipped => CaseEventPhase::Update,
            OrderStep::Delivered => CaseEventPhase::Done,
        }
    }

    fn from_kind(kind: &str) -> Option<Self> {
        Some(match kind {
            "order.invoice" => OrderStep::Invoice,
            "order.placed" => OrderStep::Placed,
            "order.shipped" => OrderStep::Shipped,
            "order.delivered" => OrderStep::Delivered,
            _ => return None,
        })
    }
}

static RE_DELIVERED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bzugestellt\b|\bdelivered\b|ablageort|wurde geliefert|ist angekommen|has arrived|abholbereit|ready for pickup",
    )
    .unwrap()
});
static RE_SHIPPED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bversendet\b|\bversandt\b|verschickt|versandbestätigung|ist unterwegs|auf dem weg zu|\bshipped\b|dispatched|out for delivery|kommt heute|kommt morgen|paketupdate|lieferstatus|sendungsstatus|aktualisierung der lieferung|(?:ihre|deine|amazon)\s+sendung|ihr (?:dhl |dpd |gls |hermes |ups )?paket|bald ist ihr",
    )
    .unwrap()
});
static RE_PLACED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^bestellt:|bestellbestätigung|bestellung\b.{0,40}\bbestätigt|bestätigung (?:ihrer|deiner) bestellung|dank für (?:ihre|deine) bestellung|auftragsbestätigung|bestelleingang|bestellung (?:ist )?eingegangen|(?:ihre|deine) bestellung bei|order confirmation|confirmation of your order|thank you for your order|thanks for your order|your order (?:has been (?:placed|received)|is confirmed)",
    )
    .unwrap()
});
static RE_INVOICE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\brechnung\b|\binvoice\b|\breceipt\b|\bquittung\b").unwrap());

/// Order numbers announced by a keyword ("Bestellnummer: 302-…",
/// "Auftragsnr.: SO1029…", "Order #A123…").
static RE_ORDER_NO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:bestell|auftrags|order)(?:ung)?[-\s]?(?:nr|nummer|number|no|id)?\.?\s*[:#]?\s*#?\s*([a-z0-9][a-z0-9-]{4,24}[a-z0-9])\b",
    )
    .unwrap()
});
/// A code of capitals and digits in an order subject, e.g. "SO10293602".
static RE_SUBJECT_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z]{0,4}\d{6,22})\b").unwrap());
/// An article name in quotes, as shops put it into the subject.
static RE_ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[„"“]([^„“”"]{4,120})[“”"]"#).unwrap());
static RE_REPLY_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(?:(?:re|aw|antw|fw|fwd|wg|wtr)\s*(?:\[\d+\])?\s*:\s*)+").unwrap()
});

/// Senders that only carry parcels. Their mails join an order that is
/// already known, but never start one, so a shop order and its parcel do
/// not become two cases.
const CARRIER_DOMAINS: &[&str] = &[
    "dhl.de",
    "dhl.com",
    "deutschepost.de",
    "dpd.de",
    "dpd.com",
    "hermesworld.com",
    "myhermes.de",
    "hermes-europe.eu",
    "gls-pakete.de",
    "gls-group.eu",
    "ups.com",
    "fedex.com",
    "usps.com",
];

/// Identifying values of one mail, normalised for lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderKeys {
    /// Order or tracking numbers: valid across senders.
    pub numbers: Vec<(String, String)>,
    /// Article name, only valid together with the sender's domain.
    pub item: Option<String>,
}

/// What the linker understood from one mail about an order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderSignal {
    pub step: OrderStep,
    pub keys: OrderKeys,
}

/// Classifies a mail as an order step, from the subject only. Bodies are
/// full of words like "Versand" and "Rechnung" in footers and adverts.
pub fn order_step(subject: &str) -> Option<OrderStep> {
    if RE_DELIVERED.is_match(subject) {
        Some(OrderStep::Delivered)
    } else if RE_SHIPPED.is_match(subject) {
        Some(OrderStep::Shipped)
    } else if RE_PLACED.is_match(subject) {
        Some(OrderStep::Placed)
    } else if RE_INVOICE.is_match(subject) {
        Some(OrderStep::Invoice)
    } else {
        None
    }
}

/// Finds order numbers, tracking numbers and the article name.
pub fn order_keys(subject: &str, body: &str) -> OrderKeys {
    let mut keys = OrderKeys::default();
    let mut seen = HashSet::new();
    let mut push = |label: &str, value: &str, keys: &mut OrderKeys| {
        let norm = normalize_key(value);
        if seen.insert(norm) {
            keys.numbers.push((label.to_string(), value.to_string()));
        }
    };
    for text in [subject, body] {
        for c in RE_ORDER_NO.captures_iter(text) {
            let v = &c[1];
            if v.bytes().filter(u8::is_ascii_digit).count() >= 4 {
                push("Bestellnr.", v, &mut keys);
            }
        }
    }
    for c in RE_SUBJECT_CODE.captures_iter(subject) {
        // Skip the digits of a number found already ("glnest-335478").
        let code = normalize_key(&c[1]);
        if !keys
            .numbers
            .iter()
            .any(|(_, v)| normalize_key(v).contains(&code))
        {
            push("Nummer", &c[1], &mut keys);
        }
    }
    for s in find_shipments(subject, body) {
        if s.evidence != Evidence::Context {
            push("Sendung", &s.tracking_number, &mut keys);
        }
    }
    keys.item = RE_ITEM
        .captures(subject)
        .map(|c| clean_item(&c[1]))
        .filter(|s| s.chars().count() >= 4);
    keys
}

fn clean_item(raw: &str) -> String {
    raw.trim()
        .trim_end_matches(['.', '…', ' ', '-', '–', '—', ',', ':', ';'])
        .trim()
        .to_string()
}

/// Subject without "Re:", "AW:", "Fwd:" and the like.
pub fn clean_subject(subject: &str) -> String {
    RE_REPLY_PREFIX.replace(subject, "").trim().to_string()
}

/// Registrable domain, good enough for logos and display: the last two
/// labels, or three for "co.uk"-style suffixes.
pub fn base_domain(addr_or_domain: &str) -> Option<String> {
    let domain = addr_or_domain
        .rsplit_once('@')
        .map_or(addr_or_domain, |(_, d)| d)
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return None;
    }
    let second = labels[labels.len() - 2];
    let take = if labels.len() >= 3
        && labels[labels.len() - 1].len() == 2
        && matches!(second, "co" | "com" | "org" | "net" | "gov" | "ac" | "or")
    {
        3
    } else {
        2
    };
    Some(labels[labels.len() - take..].join("."))
}

fn is_carrier(domain: Option<&str>) -> bool {
    domain.is_some_and(|d| CARRIER_DOMAINS.contains(&d))
}

/// Looks at new mail and files it into cases. Safe to call often: each
/// mail is looked at once (and once more when its text arrives).
pub fn run(store: &mut Store, now: i64) -> Result<LinkReport> {
    let own: HashSet<String> = store
        .accounts()?
        .into_iter()
        .map(|a| a.email.to_ascii_lowercase())
        .collect();
    store.in_transaction(|store| {
        let mut report = LinkReport::default();
        loop {
            let batch = store.case_scan_batch(now - SCAN_WINDOW_SECS, BATCH)?;
            if batch.is_empty() {
                break;
            }
            for m in &batch {
                link_message(store, m, &own, &mut report)?;
                store.mark_case_scanned(m.id, m.body_text.is_some())?;
                report.scanned += 1;
            }
            if batch.len() < BATCH {
                break;
            }
        }
        report.closed += store.close_stale_cases(now - STALE_AFTER_SECS)?;
        Ok(report)
    })
}

fn direction(m: &ScanMessage, own: &HashSet<String>) -> EventDirection {
    let from_me = m
        .from_addr
        .as_deref()
        .is_some_and(|a| own.contains(&a.to_ascii_lowercase()));
    if from_me || m.folder_role == Some(FolderRole::Sent) {
        EventDirection::Out
    } else {
        EventDirection::In
    }
}

fn link_message(
    store: &Store,
    m: &ScanMessage,
    own: &HashSet<String>,
    report: &mut LinkReport,
) -> Result<()> {
    let subject = m.subject.as_deref().unwrap_or("");
    let body = m.body_text.as_deref().unwrap_or("");

    // The same mail can sit in several folders (Gmail labels, copies).
    if let Some(rfc) = m.message_id.as_deref()
        && let Some(case_id) = store.case_of_rfc_message(rfc)?
    {
        // Seen before; its text may now be there with more numbers.
        if m.body_text.is_some() && order_step(subject).is_some() {
            let keys = order_keys(subject, body);
            for (label, value) in &keys.numbers {
                store.add_case_key(case_id, label, value)?;
            }
        }
        return Ok(());
    }

    let dir = direction(m, own);
    if m.list_id.is_none() && dir == EventDirection::In {
        link_order(store, m, subject, body, report)?;
    }
    Ok(())
}

fn event_for(
    m: &ScanMessage,
    kind: &str,
    label: String,
    phase: CaseEventPhase,
    dir: EventDirection,
) -> NewCaseEvent {
    NewCaseEvent {
        message_id: Some(m.id),
        occurred_at: m.date,
        kind: Some(kind.to_string()),
        label,
        phase,
        direction: Some(dir),
        confidence: 1.0,
        manual: false,
    }
}

fn link_order(
    store: &Store,
    m: &ScanMessage,
    subject: &str,
    body: &str,
    report: &mut LinkReport,
) -> Result<()> {
    let Some(step) = order_step(subject) else {
        return Ok(());
    };
    let keys = order_keys(subject, body);
    let domain = m.from_addr.as_deref().and_then(base_domain);
    let item_key = match (&keys.item, &domain) {
        (Some(item), Some(d)) => Some(format!("{d} · {item}")),
        _ => None,
    };

    let number_norms: Vec<String> = keys.numbers.iter().map(|(_, v)| normalize_key(v)).collect();
    let mut found = store.case_for_keys(CaseKind::Order, &number_norms)?;
    // A new order of an article bought before is a new case; the article
    // name only links later steps of an order.
    if found.is_none()
        && step != OrderStep::Placed
        && let Some(k) = &item_key
    {
        found = store.case_for_keys(CaseKind::Order, &[normalize_key(k)])?;
    }

    let case_id = match found {
        Some(id) => id,
        None => {
            let starts = match step {
                OrderStep::Placed => true,
                OrderStep::Shipped => {
                    !is_carrier(domain.as_deref())
                        && (!keys.numbers.is_empty() || item_key.is_some())
                }
                OrderStep::Delivered | OrderStep::Invoice => false,
            };
            if !starts {
                return Ok(());
            }
            let counterparty = m
                .from_name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| domain.clone())
                .unwrap_or_else(|| "Unbekannt".into());
            let title = keys
                .item
                .clone()
                .or_else(|| {
                    keys.numbers
                        .first()
                        .map(|(label, v)| match (label.as_str(), step) {
                            ("Bestellnr.", _) | (_, OrderStep::Placed) => format!("Bestellung {v}"),
                            _ => format!("Sendung {v}"),
                        })
                })
                .unwrap_or_else(|| clean_subject(subject));
            let reason = match keys.numbers.first() {
                Some((label, v)) => format!("{} erkannt ({label} {v})", step.label()),
                None => format!("{} erkannt am Betreff", step.label()),
            };
            let created = store.create_case(&NewCase {
                title,
                counterparty,
                // Logos only for senders whose mail passed DMARC, so a forged
                // mail cannot dress up as a known shop.
                domain: if m.dmarc_pass { domain.clone() } else { None },
                kind: CaseKind::Order,
                status: CaseStatus::Waiting,
                next_step: String::new(),
                due_at: None,
                due_label: None,
                amount_cents: None,
                currency: None,
                reason,
            })?;
            report.created += 1;
            created.case.id
        }
    };

    for (label, value) in &keys.numbers {
        store.add_case_key(case_id, label, value)?;
    }
    if let Some(k) = &item_key {
        store.add_case_key(case_id, "Artikel", k)?;
    }
    store.add_case_event(
        case_id,
        &event_for(
            m,
            step.kind(),
            step.label().into(),
            step.phase(),
            EventDirection::In,
        ),
    )?;
    report.linked += 1;
    refresh_order(store, case_id)
}

/// Recomputes an order's status from its furthest step.
fn refresh_order(store: &Store, case_id: i64) -> Result<()> {
    let Some(view) = store.get_case(case_id)? else {
        return Ok(());
    };
    let furthest = view
        .events
        .iter()
        .filter_map(|e| e.kind.as_deref().and_then(OrderStep::from_kind))
        .filter(|s| *s != OrderStep::Invoice)
        .max();
    let (status, next) = match furthest {
        Some(OrderStep::Delivered) => (CaseStatus::Done, "Zugestellt"),
        Some(OrderStep::Shipped) => (CaseStatus::Waiting, "Unterwegs · wartet auf Zustellung"),
        _ => (CaseStatus::Waiting, "Bestellt · wartet auf Versand"),
    };
    store.update_case_state(case_id, status, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_steps_from_subjects() {
        let cases = [
            ("Bestellt: „Raspberry Pi 4...“", Some(OrderStep::Placed)),
            (
                "Bestellung glnest-335478 bestätigt",
                Some(OrderStep::Placed),
            ),
            (
                "Versendet: „Intenso microSDHC 16GB...“",
                Some(OrderStep::Shipped),
            ),
            (
                "Deine Bestellung ist unterwegs SO10293602",
                Some(OrderStep::Shipped),
            ),
            (
                "Deine Bestellung wurde zugestellt SO10293602",
                Some(OrderStep::Delivered),
            ),
            (
                "Zugestellt: 2 „innomaker 1080P USB2.0...“",
                Some(OrderStep::Delivered),
            ),
            (
                "Ihre Amazon Sendung liegt am gewünschten Ablageort",
                Some(OrderStep::Delivered),
            ),
            ("Your order has shipped", Some(OrderStep::Shipped)),
            ("Thank you for your order #1001", Some(OrderStep::Placed)),
            (
                "RECHNUNG inkl. Versandbestätigung - Auftragsnr.: SO10293602",
                Some(OrderStep::Shipped),
            ),
            ("Ihre Rechnung für Oktober", Some(OrderStep::Invoice)),
            (
                "Hat deine aktuelle Amazon-Bestellung deine Erwartungen erfüllt?",
                None,
            ),
            ("20 % auf alles – versandkostenfrei!", None),
            ("Neue TV-Sendung heute Abend", None),
            ("Treffen am Dienstag", None),
        ];
        for (subject, want) in cases {
            assert_eq!(order_step(subject), want, "{subject}");
        }
    }

    #[test]
    fn order_keys_found() {
        let k = order_keys("Bestellung glnest-335478 bestätigt", "");
        assert_eq!(
            k.numbers,
            vec![("Bestellnr.".into(), "glnest-335478".into())]
        );

        let k = order_keys(
            "RECHNUNG inkl. Versandbestätigung - Auftragsnr.: SO10293602",
            "",
        );
        assert!(k.numbers.iter().any(|(_, v)| v == "SO10293602"));

        let k = order_keys(
            "Bestellt: „Raspberry Pi 4...“",
            "Bestellung #302-1234567-7654321",
        );
        assert_eq!(k.item.as_deref(), Some("Raspberry Pi 4"));
        assert_eq!(k.numbers[0].1, "302-1234567-7654321");
        let k = order_keys("Bestellt: „Google Pixel 11 Pro XL –...“", "");
        assert_eq!(k.item.as_deref(), Some("Google Pixel 11 Pro XL"));

        // Words after the keyword are not numbers.
        let k = order_keys("Deine Bestellung wurde versendet", "Order confirmation");
        assert!(k.numbers.is_empty());
    }

    #[test]
    fn domains_and_subjects() {
        assert_eq!(
            base_domain("shop@mail.shop.example").as_deref(),
            Some("shop.example")
        );
        assert_eq!(
            base_domain("a@news.shop.co.uk").as_deref(),
            Some("shop.co.uk")
        );
        assert_eq!(base_domain("localhost"), None);
        assert_eq!(clean_subject("AW: Re: Angebot"), "Angebot");
        assert_eq!(clean_subject("Fwd: WG: Termin"), "Termin");
    }

    use crate::model::{Account, AuthMethod, MessageHeader, Security, ServerConfig};

    fn store_with_inbox() -> (Store, i64, i64) {
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
        let sent = store
            .upsert_folder(a.id, "Sent", None, Some(FolderRole::Sent), true)
            .unwrap();
        (store, inbox.id, sent.id)
    }

    fn mail(uid: u32, subject: &str, from: &str, date: i64) -> MessageHeader {
        MessageHeader {
            uid,
            message_id: Some(format!("m{uid}@example.org")),
            subject: Some(subject.into()),
            from_name: Some("Shop".into()),
            from_addr: Some(from.into()),
            date: Some(date),
            dmarc_pass: true,
            ..Default::default()
        }
    }

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    #[test]
    fn order_steps_build_one_case() {
        let (mut store, inbox, _) = store_with_inbox();
        store
            .insert_messages(
                inbox,
                &[
                    mail(
                        1,
                        "Bestellt: „Raspberry Pi 4...“",
                        "order@shop.example",
                        NOW - 3 * DAY,
                    ),
                    mail(
                        2,
                        "Versendet: „Raspberry Pi 4...“",
                        "order@shop.example",
                        NOW - 2 * DAY,
                    ),
                    // Same article from another shop: not this order.
                    mail(
                        3,
                        "Versendet: „Raspberry Pi 4...“",
                        "info@other.example",
                        NOW - 2 * DAY,
                    ),
                    // A carrier alone never starts a case.
                    mail(4, "Ihr DHL Paket kommt heute", "noreply@dhl.de", NOW - DAY),
                    mail(
                        5,
                        "Newsletter: Neue Angebote",
                        "news@shop.example",
                        NOW - DAY,
                    ),
                ],
            )
            .unwrap();

        let report = run(&mut store, NOW).unwrap();
        let cases = store.list_cases(false).unwrap();
        assert_eq!(report.created, 2, "{cases:#?}");
        let pi = cases
            .iter()
            .find(|c| c.case.domain.as_deref() == Some("shop.example"))
            .unwrap();
        assert_eq!(pi.case.title, "Raspberry Pi 4");
        assert_eq!(pi.case.kind, CaseKind::Order);
        assert_eq!(pi.case.status, CaseStatus::Waiting);
        assert_eq!(pi.events.len(), 2);
        assert_eq!(pi.events[1].kind.as_deref(), Some("order.shipped"));

        // Nothing new: a second run changes nothing.
        let again = run(&mut store, NOW).unwrap();
        assert!(!again.changed(), "{again:?}");

        // Delivery closes the order.
        store
            .insert_messages(
                inbox,
                &[mail(
                    6,
                    "Zugestellt: „Raspberry Pi 4...“",
                    "order@shop.example",
                    NOW,
                )],
            )
            .unwrap();
        run(&mut store, NOW).unwrap();
        let pi = store.get_case(pi.case.id).unwrap().unwrap();
        assert_eq!(pi.case.status, CaseStatus::Done);
    }

    #[test]
    fn rejected_case_is_not_started_again() {
        let (mut store, inbox, _) = store_with_inbox();
        store
            .insert_messages(
                inbox,
                &[mail(
                    1,
                    "Bestellung A-123456 bestätigt",
                    "o@shop.example",
                    NOW - DAY,
                )],
            )
            .unwrap();
        run(&mut store, NOW).unwrap();
        let id = store.list_cases(false).unwrap()[0].case.id;
        store.dissolve_case(id).unwrap();

        store
            .insert_messages(
                inbox,
                &[mail(
                    2,
                    "Deine Bestellung A-123456 ist unterwegs",
                    "o@shop.example",
                    NOW,
                )],
            )
            .unwrap();
        let report = run(&mut store, NOW).unwrap();
        assert_eq!(report.created, 0);
        assert!(store.list_cases(false).unwrap().is_empty());
    }

    #[test]
    fn plain_conversations_do_not_create_cases() {
        let (mut store, inbox, sent) = store_with_inbox();
        let mut question = mail(
            1,
            "Tischreservierung Freitag",
            "info@restaurant.example",
            NOW - 2 * DAY,
        );
        question.from_name = Some("La Piazza".into());
        let mut answer = mail(
            2,
            "Re: Tischreservierung Freitag",
            "me@example.org",
            NOW - DAY,
        );
        answer.in_reply_to = Some("m1@example.org".into());
        answer.to = vec!["info@restaurant.example".into()];
        let mut back = mail(
            3,
            "Re: Re: Tischreservierung Freitag",
            "info@restaurant.example",
            NOW,
        );
        back.in_reply_to = Some("m2@example.org".into());
        back.references = vec!["m1@example.org".into(), "m2@example.org".into()];
        store.insert_messages(inbox, &[question, back]).unwrap();
        store.insert_messages(sent, &[answer]).unwrap();

        run(&mut store, NOW).unwrap();
        assert!(store.list_cases(false).unwrap().is_empty());
    }

    #[test]
    fn stale_cases_close_and_old_mail_is_ignored() {
        let (mut store, inbox, _) = store_with_inbox();
        store
            .insert_messages(
                inbox,
                &[
                    mail(
                        1,
                        "Bestellung B-998877 bestätigt",
                        "o@shop.example",
                        NOW - 30 * DAY,
                    ),
                    mail(
                        2,
                        "Bestellung C-112233 bestätigt",
                        "o@shop.example",
                        NOW - 400 * DAY,
                    ),
                ],
            )
            .unwrap();
        let report = run(&mut store, NOW).unwrap();
        assert_eq!(report.created, 1);
        assert_eq!(report.closed, 1);
        assert_eq!(
            store.list_cases(false).unwrap()[0].case.status,
            CaseStatus::Done
        );
    }

    #[test]
    fn sent_replies_and_one_time_prune() {
        let (mut store, inbox, sent) = store_with_inbox();
        let question = mail(1, "Tischreservierung", "info@restaurant.example", NOW - DAY);
        let mut answer = mail(2, "Re: Tischreservierung", "me@example.org", NOW);
        answer.in_reply_to = Some("m1@example.org".into());
        store.insert_messages(inbox, &[question]).unwrap();
        store.insert_messages(sent, &[answer]).unwrap();
        let replies = store.sent_replies_to(&["m1@example.org".into()]).unwrap();
        assert_eq!(replies.len(), 1);
        assert_eq!(
            replies[0].header.subject.as_deref(),
            Some("Re: Tischreservierung")
        );
    }
}
