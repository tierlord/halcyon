//! Extractors for invoice amounts, due dates, payment dispositions, and
//! business reference numbers (invoice, contract, customer, and ticket IDs).

use chrono::NaiveDate;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

/// A monetary amount found in a message, expressed in minor units (cents).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoneyAmount {
    /// Amount in minor units (e.g. `4990` for `49,90 €`).
    pub amount_cents: i64,
    /// ISO 4217 currency code (`"EUR"`, `"USD"`, `"CHF"`, `"GBP"`).
    pub currency: String,
    /// Confidence score in `0.0..=1.0`.
    pub confidence: f32,
}

/// How an invoice or payment request is settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentDisposition {
    /// The recipient needs to transfer or pay manually ("Braucht dich").
    NeedsTransfer,
    /// Collected automatically via direct debit / SEPA ("Wartet").
    DirectDebit,
    /// Already paid via card, PayPal, or prepayment ("Erledigt").
    AlreadyPaid,
}

/// Extracted payment disposition and optional due timestamp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DueInfo {
    pub disposition: PaymentDisposition,
    /// UTC Unix timestamp (seconds) of the due date, if stated.
    pub due_at: Option<i64>,
    /// Confidence score in `0.0..=1.0`.
    pub confidence: f32,
}

/// Kind of business identifier found in a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessKeyKind {
    InvoiceNumber,
    ContractNumber,
    CustomerNumber,
    TicketNumber,
}

impl BusinessKeyKind {
    /// German display label used on case cards.
    pub fn label(self) -> &'static str {
        match self {
            BusinessKeyKind::InvoiceNumber => "Rechnungsnr.",
            BusinessKeyKind::ContractNumber => "Vertragsnr.",
            BusinessKeyKind::CustomerNumber => "Kundennr.",
            BusinessKeyKind::TicketNumber => "Ticketnr.",
        }
    }
}

/// A business identifier extracted from subject or body text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BusinessKey {
    pub kind: BusinessKeyKind,
    pub value: String,
    pub confidence: f32,
}

static PROMO_KEYWORDS: &[&str] = &[
    "versandkostenfrei",
    "ab 29",
    "gutschein",
    "rabatt",
    "sparen",
    "bis zu",
    "voucher",
    "discount",
    "free shipping",
];

static TOTAL_KEYWORDS: &[&str] = &[
    "rechnungsbetrag",
    "gesamtbetrag",
    "zu zahlen",
    "zahlbetrag",
    "offener betrag",
    "summe",
    "endbetrag",
    "abbuchung",
    "rechnung über",
    "total",
    "amount due",
    "invoice total",
    "balance due",
];

/// Extracts monetary amounts from `subject` and `body_text`, sorted by
/// descending confidence and deduplicated by `(amount_cents, currency)`.
pub fn find_amounts(subject: &str, body_text: &str) -> Vec<MoneyAmount> {
    let mut results: HashMap<(i64, String), f32> = HashMap::new();

    fn process_text(text: &str, is_subject: bool, results: &mut HashMap<(i64, String), f32>) {
        for line in text.lines() {
            let line_lower = line.to_lowercase();
            let is_promo = PROMO_KEYWORDS.iter().any(|kw| line_lower.contains(kw));
            if is_promo {
                continue;
            }

            let has_total = TOTAL_KEYWORDS.iter().any(|kw| line_lower.contains(kw));
            let base_confidence = if is_subject {
                if has_total { 0.95 } else { 0.9 }
            } else {
                if has_total { 0.85 } else { 0.55 }
            };

            let matches = parse_line_amounts(line);
            for (cents, currency) in matches {
                if cents <= 0 {
                    continue;
                }
                let conf = base_confidence;
                let key = (cents, currency);
                results
                    .entry(key)
                    .and_modify(|c| {
                        if conf > *c {
                            *c = conf;
                        }
                    })
                    .or_insert(conf);
            }
        }
    }

    process_text(subject, true, &mut results);
    process_text(body_text, false, &mut results);

    let mut list: Vec<MoneyAmount> = results
        .into_iter()
        .map(|((amount_cents, currency), confidence)| MoneyAmount {
            amount_cents,
            currency,
            confidence,
        })
        .collect();

    list.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.amount_cents.cmp(&a.amount_cents))
    });

    list
}

fn parse_line_amounts(line: &str) -> Vec<(i64, String)> {
    let mut found = Vec::new();

    let normalize_currency = |s: &str| -> Option<String> {
        let u = s.to_uppercase();
        if u.contains('€') || u.contains("EUR") || u.contains("EURO") {
            Some("EUR".to_string())
        } else if u.contains('$') || u.contains("USD") || u.contains("US$") {
            Some("USD".to_string())
        } else if u.contains("CHF") || u.contains("FR") {
            Some("CHF".to_string())
        } else if u.contains('£') || u.contains("GBP") {
            Some("GBP".to_string())
        } else {
            None
        }
    };

    static AMT_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?x)
            (?i)
            (?:(?P<pre_cur>€|EUR|Euro|\$|USD|US\$|CHF|Fr\.?|£|GBP)\s*)?
            (?P<num>
                [0-9]+[\.,]\s*[-–—]
                |
                [0-9]{1,3}(?:[,\.][0-9]{3})*(?:[\.,][0-9]{1,2})?
                |
                [0-9]+(?:[\.,][0-9]{1,2})?
            )
            \s*
            (?:(?P<suf_cur>€|EUR|Euro|\$|USD|US\$|CHF|Fr\.?|£|GBP))?
        "#,
        )
        .unwrap()
    });

    for cap in AMT_RE.captures_iter(line) {
        let pre = cap.name("pre_cur").map(|m| m.as_str());
        let suf = cap.name("suf_cur").map(|m| m.as_str());
        let num_str = cap.name("num").unwrap().as_str();

        let currency = match (pre, suf) {
            (Some(p), _) => normalize_currency(p),
            (None, Some(s)) => normalize_currency(s),
            (None, None) => None,
        };

        if let Some(curr) = currency {
            let cleaned = num_str.replace(|c: char| c.is_whitespace() || c == '\u{00a0}', "");
            if let Some(cents) = parse_amount_str(&cleaned)
                && cents > 0
            {
                found.push((cents, curr));
            }
        }
    }

    found
}

fn parse_amount_str(s: &str) -> Option<i64> {
    for dash in &[",-", ",–", ",—", ".-", ".–", ".—", ".,-"] {
        if let Some(base) = s.strip_suffix(dash) {
            let int_part = parse_integer_part(base)?;
            return Some(int_part * 100);
        }
    }
    if let Some(base) = s.strip_suffix("-,") {
        let int_part = parse_integer_part(base)?;
        return Some(int_part * 100);
    }

    let last_comma = s.rfind(',');
    let last_dot = s.rfind('.');

    match (last_comma, last_dot) {
        (Some(c), Some(d)) => {
            if c > d {
                let int_part_str = &s[..c];
                let dec_part_str = &s[c + 1..];
                let int_val = parse_integer_part(&int_part_str.replace('.', ""))?;
                let dec_val = parse_decimal_part(dec_part_str)?;
                Some(int_val * 100 + dec_val)
            } else {
                let int_part_str = &s[..d];
                let dec_part_str = &s[d + 1..];
                let int_val = parse_integer_part(&int_part_str.replace(',', ""))?;
                let dec_val = parse_decimal_part(dec_part_str)?;
                Some(int_val * 100 + dec_val)
            }
        }
        (Some(c), None) => {
            let dec_part_str = &s[c + 1..];
            if dec_part_str.len() == 2 && !dec_part_str.contains(',') {
                let int_part_str = &s[..c];
                let int_val = parse_integer_part(&int_part_str.replace('.', ""))?;
                let dec_val = parse_decimal_part(dec_part_str)?;
                Some(int_val * 100 + dec_val)
            } else {
                let int_val = parse_integer_part(&s.replace(',', ""))?;
                Some(int_val * 100)
            }
        }
        (None, Some(d)) => {
            let dec_part_str = &s[d + 1..];
            if dec_part_str.len() == 2 && !dec_part_str.contains('.') {
                let int_part_str = &s[..d];
                let int_val = parse_integer_part(&int_part_str.replace(',', ""))?;
                let dec_val = parse_decimal_part(dec_part_str)?;
                Some(int_val * 100 + dec_val)
            } else {
                let int_val = parse_integer_part(&s.replace('.', ""))?;
                Some(int_val * 100)
            }
        }
        (None, None) => {
            let int_val = parse_integer_part(s)?;
            Some(int_val * 100)
        }
    }
}

fn parse_integer_part(s: &str) -> Option<i64> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.is_empty() {
        return Some(0);
    }
    cleaned.parse().ok()
}

fn parse_decimal_part(s: &str) -> Option<i64> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if cleaned.is_empty() {
        return Some(0);
    }
    if cleaned.len() == 1 {
        cleaned.parse::<i64>().ok().map(|v| v * 10)
    } else if cleaned.len() >= 2 {
        cleaned[..2].parse::<i64>().ok()
    } else {
        Some(0)
    }
}

pub fn find_due_info(subject: &str, body_text: &str, reference_date: i64) -> Option<DueInfo> {
    let full_text = format!("{}\n{}", subject, body_text);
    let full_lower = full_text.to_lowercase();

    let paid_signals = [
        "bereits bezahlt",
        "wurde bezahlt",
        "per paypal bezahlt",
        "dank für ihre zahlung",
        "zahlung erhalten",
        "kreditkarte belastet",
        "already paid",
        "payment received",
        "paid in full",
    ];

    let debit_signals = [
        "sepa-lastschrift",
        "lastschrift",
        "buchen wir",
        "wird abgebucht",
        "wird eingezogen",
        "bankeinzug",
        "mandatsreferenz",
        "direct debit",
        "auto-pay",
        "automatically charged",
    ];

    let transfer_signals = [
        "bitte überweisen",
        "überweisen sie",
        "zahlbar bis",
        "fällig am",
        "fällig bis",
        "fälligkeit",
        "zahlungsziel",
        "zahlbar innerhalb",
        "offener betrag",
        "mahnung",
        "zahlungserinnerung",
        "payment due",
        "due date",
        "please pay by",
        "payable within",
    ];

    let is_paid = paid_signals.iter().any(|s| full_lower.contains(s));
    let is_debit = debit_signals.iter().any(|s| full_lower.contains(s));
    let is_transfer = transfer_signals.iter().any(|s| full_lower.contains(s));

    let disposition = if is_paid {
        PaymentDisposition::AlreadyPaid
    } else if is_debit {
        PaymentDisposition::DirectDebit
    } else if is_transfer {
        PaymentDisposition::NeedsTransfer
    } else {
        if full_lower.contains("fällig") || full_lower.contains("due") {
            PaymentDisposition::NeedsTransfer
        } else {
            return None;
        }
    };

    let due_at = extract_due_date(&full_text, reference_date);
    let confidence = if due_at.is_some() { 0.9 } else { 0.75 };

    Some(DueInfo {
        disposition,
        due_at,
        confidence,
    })
}

fn extract_due_date(text: &str, reference_date: i64) -> Option<i64> {
    static RELATIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:innerhalb\s+von|binnen|in|within)\s+(\d{1,2})\s+(?:tagen|days)"#)
            .unwrap()
    });

    if let Some(cap) = RELATIVE_RE.captures(text)
        && let Ok(days) = cap.get(1).unwrap().as_str().parse::<i64>()
    {
        return Some(reference_date + days * 86_400);
    }

    static DE_DATE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\b([0-9]{1,2})\.([0-9]{1,2})\.([0-9]{2,4})\b"#).unwrap());

    for cap in DE_DATE_RE.captures_iter(text) {
        let Some(day) = cap.get(1).unwrap().as_str().parse::<u32>().ok() else {
            continue;
        };
        let Some(month) = cap.get(2).unwrap().as_str().parse::<u32>().ok() else {
            continue;
        };
        let Some(year_raw) = cap.get(3).unwrap().as_str().parse::<i32>().ok() else {
            continue;
        };
        let year = if year_raw < 100 {
            2000 + year_raw
        } else {
            year_raw
        };

        if let Some(naive_date) = NaiveDate::from_ymd_opt(year, month, day)
            && let Some(dt) = naive_date.and_hms_opt(0, 0, 0)
        {
            return Some(dt.and_utc().timestamp());
        }
    }

    static ISO_DATE_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\b([0-9]{4})-([0-9]{2})-([0-9]{2})\b"#).unwrap());

    for cap in ISO_DATE_RE.captures_iter(text) {
        let Some(year) = cap.get(1).unwrap().as_str().parse::<i32>().ok() else {
            continue;
        };
        let Some(month) = cap.get(2).unwrap().as_str().parse::<u32>().ok() else {
            continue;
        };
        let Some(day) = cap.get(3).unwrap().as_str().parse::<u32>().ok() else {
            continue;
        };

        if let Some(naive_date) = NaiveDate::from_ymd_opt(year, month, day)
            && let Some(dt) = naive_date.and_hms_opt(0, 0, 0)
        {
            return Some(dt.and_utc().timestamp());
        }
    }

    None
}

/// Extracts invoice, contract, customer, and support ticket numbers.
pub fn find_business_keys(subject: &str, body_text: &str) -> Vec<BusinessKey> {
    static RE_INVOICE_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:rechnungsnummer|rechnungs-nr\.?|rechnungsnr\.?|rechnung(?:\s+nr\.?)?|belegnummer|invoice\s*(?:number|no\.?|id|#)?)\s*[:#]?\s*([A-Za-z0-9\-/]{4,28})"#).unwrap()
    });
    static RE_CONTRACT_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:vertragsnummer|vertrags-nr\.?|vertragsnr\.?|vertragskonto|policennummer|vertrags\b|vertrag\b|contract\s*(?:number|no\.?|id|#)?)\s*[:#]?\s*([A-Za-z0-9\-/]{4,28})"#).unwrap()
    });
    static RE_CUSTOMER_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:kundennummer|kunden-nr\.?|kundennr\.?|mitgliedsnummer|customer\s*(?:number|no\.?|id|#))\s*[:#]?\s*([A-Za-z0-9\-/]{4,28})"#).unwrap()
    });
    static RE_TICKET_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:ticketnummer|ticket-nr\.?|ticket-id|vorgangsnummer|aktenzeichen|ticket\s*#?|case\s*#?|incident\s*#?)\s*[:#]?\s*([A-Za-z0-9\-/]{4,28})"#).unwrap()
    });

    let mut results: HashMap<(BusinessKeyKind, String), (String, f32)> = HashMap::new();

    fn process_text(
        text: &str,
        is_subject: bool,
        results: &mut HashMap<(BusinessKeyKind, String), (String, f32)>,
    ) {
        let confidence = if is_subject { 0.9 } else { 0.8 };

        let patterns: [(BusinessKeyKind, &Regex); 4] = [
            (BusinessKeyKind::InvoiceNumber, &RE_INVOICE_KEY),
            (BusinessKeyKind::ContractNumber, &RE_CONTRACT_KEY),
            (BusinessKeyKind::CustomerNumber, &RE_CUSTOMER_KEY),
            (BusinessKeyKind::TicketNumber, &RE_TICKET_KEY),
        ];

        for (kind, re) in patterns {
            for cap in re.captures_iter(text) {
                if let Some(val_match) = cap.get(1) {
                    let val = val_match
                        .as_str()
                        .trim_matches(|c| c == '.' || c == ',' || c == ':' || c == '-');
                    if val.len() >= 4 && val.len() <= 28 {
                        let digit_count = val.chars().filter(|c| c.is_ascii_digit()).count();
                        if digit_count >= 3 {
                            let key = (kind, val.to_ascii_lowercase());
                            results
                                .entry(key)
                                .and_modify(|(_, conf)| {
                                    if confidence > *conf {
                                        *conf = confidence;
                                    }
                                })
                                .or_insert((val.to_string(), confidence));
                        }
                    }
                }
            }
        }
    }

    process_text(subject, true, &mut results);
    process_text(body_text, false, &mut results);

    results
        .into_iter()
        .map(|((kind, _), (value, confidence))| BusinessKey {
            kind,
            value,
            confidence,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_amounts_german_and_english() {
        let subject = "Rechnung über 49,90 €";
        let body = "USD 1,234.56 or 12,- €.";
        let amounts = find_amounts(subject, body);
        assert!(!amounts.is_empty());
        assert!(
            amounts
                .iter()
                .any(|a| a.amount_cents == 4990 && a.currency == "EUR")
        );
        assert!(
            amounts
                .iter()
                .any(|a| a.amount_cents == 123456 && a.currency == "USD")
        );
        assert!(
            amounts
                .iter()
                .any(|a| a.amount_cents == 1200 && a.currency == "EUR")
        );
    }

    #[test]
    fn test_find_amounts_dash_decimal_variants() {
        let amounts = find_amounts("Summe", "Betrag: 48,– €");
        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts[0].amount_cents, 4800);
        assert_eq!(amounts[0].currency, "EUR");
    }

    #[test]
    fn test_find_amounts_promo_rejection() {
        let body = "Versandkostenfrei ab 29 € Einkaufswert! Gutschein Rabatt sparen bis zu 50 €.";
        let amounts = find_amounts("Angebote", body);
        assert!(amounts.is_empty());
    }

    #[test]
    fn test_find_amounts_confidence_ranking() {
        let subject = "Rechnungsbetrag: 150,00 EUR";
        let body = "Weitere Position: 10,00 EUR";
        let amounts = find_amounts(subject, body);
        assert!(amounts.len() >= 2);
        assert!(amounts[0].confidence > amounts[1].confidence);
    }

    #[test]
    fn test_find_due_info_already_paid() {
        let due = find_due_info(
            "Zahlung",
            "Vielen Dank für Ihre Zahlung, per PayPal bezahlt.",
            1000000,
        );
        assert!(due.is_some());
        let info = due.unwrap();
        assert_eq!(info.disposition, PaymentDisposition::AlreadyPaid);
    }

    #[test]
    fn test_find_due_info_direct_debit() {
        let due = find_due_info(
            "SEPA",
            "Wir buchen wir am 01.12.2026 ab via SEPA-Lastschrift.",
            1000000,
        );
        assert!(due.is_some());
        let info = due.unwrap();
        assert_eq!(info.disposition, PaymentDisposition::DirectDebit);
        assert!(info.due_at.is_some());
    }

    #[test]
    fn test_find_due_info_needs_transfer_absolute() {
        let due = find_due_info(
            "Mahnung",
            "Bitte überweisen Sie fällig bis 28.11.2026.",
            1000000,
        );
        assert!(due.is_some());
        let info = due.unwrap();
        assert_eq!(info.disposition, PaymentDisposition::NeedsTransfer);
        assert!(info.due_at.is_some());
    }

    #[test]
    fn test_find_due_info_relative() {
        let ref_date = 1700000000;
        let due = find_due_info(
            "Zahlungsziel",
            "Bitte zahlen innerhalb von 14 Tagen.",
            ref_date,
        );
        assert!(due.is_some());
        let info = due.unwrap();
        assert_eq!(info.disposition, PaymentDisposition::NeedsTransfer);
        assert_eq!(info.due_at, Some(ref_date + 14 * 86_400));
    }

    #[test]
    fn test_find_business_keys_invoice() {
        let keys = find_business_keys("Rechnung", "Rechnungsnummer: RE-2026-999");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].kind, BusinessKeyKind::InvoiceNumber);
        assert_eq!(keys[0].value, "RE-2026-999");
    }

    #[test]
    fn test_find_business_keys_contract() {
        let keys = find_business_keys("Vertrag", "Vertragsnummer V-12345");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].kind, BusinessKeyKind::ContractNumber);
        assert_eq!(keys[0].value, "V-12345");
    }

    #[test]
    fn test_find_business_keys_customer() {
        let keys = find_business_keys("Konto", "Kundennummer 987654");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].kind, BusinessKeyKind::CustomerNumber);
        assert_eq!(keys[0].value, "987654");
    }

    #[test]
    fn test_find_business_keys_ticket() {
        let keys = find_business_keys("Support", "Ticketnummer TCK-555");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].kind, BusinessKeyKind::TicketNumber);
        assert_eq!(keys[0].value, "TCK-555");
    }

    #[test]
    fn test_business_keys_negative_digit_count() {
        let keys = find_business_keys("Info", "Rechnungsnummer RE-AB");
        assert!(keys.is_empty());
    }

    #[test]
    fn test_additional_units_chf_gbp() {
        let amounts = find_amounts("Total", "CHF 120.00 and £9.99 and 45.00 USD");
        assert!(
            amounts
                .iter()
                .any(|a| a.currency == "CHF" && a.amount_cents == 12000)
        );
        assert!(
            amounts
                .iter()
                .any(|a| a.currency == "GBP" && a.amount_cents == 999)
        );
        assert!(
            amounts
                .iter()
                .any(|a| a.currency == "USD" && a.amount_cents == 4500)
        );
    }

    #[test]
    fn test_iso_date_due() {
        let due = find_due_info("Due", "Please pay by 2026-11-28", 1000000);
        assert!(due.is_some());
        assert!(due.unwrap().due_at.is_some());
    }

    #[test]
    fn test_two_digit_year_due() {
        let due = find_due_info("Fällig", "Zahlungsziel: 15.12.26", 1000000);
        assert!(due.is_some());
        assert!(due.unwrap().due_at.is_some());
    }

    #[test]
    fn test_already_paid_phrases() {
        assert_eq!(
            find_due_info("Paid", "Payment received thanks.", 1000000)
                .unwrap()
                .disposition,
            PaymentDisposition::AlreadyPaid
        );
        assert_eq!(
            find_due_info("Paid", "Paid in full.", 1000000)
                .unwrap()
                .disposition,
            PaymentDisposition::AlreadyPaid
        );
    }

    #[test]
    fn test_direct_debit_phrases() {
        assert_eq!(
            find_due_info("Debit", "Direct debit auto-pay.", 1000000)
                .unwrap()
                .disposition,
            PaymentDisposition::DirectDebit
        );
        assert_eq!(
            find_due_info("Debit", "Bankeinzug Mandatsreferenz.", 1000000)
                .unwrap()
                .disposition,
            PaymentDisposition::DirectDebit
        );
    }

    #[test]
    fn test_business_key_labels() {
        assert_eq!(BusinessKeyKind::InvoiceNumber.label(), "Rechnungsnr.");
        assert_eq!(BusinessKeyKind::ContractNumber.label(), "Vertragsnr.");
        assert_eq!(BusinessKeyKind::CustomerNumber.label(), "Kundennr.");
        assert_eq!(BusinessKeyKind::TicketNumber.label(), "Ticketnr.");
    }

    #[test]
    fn test_multiple_business_keys() {
        let keys = find_business_keys("Info", "Rechnung Nr. INV-001, Kundennummer KD-999");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_zero_amount_ignored() {
        let amounts = find_amounts("Free", "0,00 €");
        assert!(amounts.is_empty());
    }

    #[test]
    fn test_negative_amount_ignored() {
        let amounts = find_amounts("Credit", "Balance: 10,00 EUR");
        assert!(!amounts.is_empty());
    }

    #[test]
    fn test_due_info_none() {
        let due = find_due_info("Hello", "Just writing to say hello.", 1000000);
        assert!(due.is_none());
    }

    #[test]
    fn test_business_keys_case_insensitive_dedup() {
        let keys = find_business_keys(
            "Info",
            "Rechnungsnummer INV-123 and RECHNUNGSNUMMER inv-123",
        );
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_amount_dedup_highest_confidence() {
        let amounts = find_amounts("Gesamtbetrag: 50,00 EUR", "Item 50,00 EUR");
        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts[0].confidence, 0.95);
    }

    #[test]
    fn test_usd_comma_thousands_dot_decimals() {
        let amounts = find_amounts("Total", "USD 1,234.56");
        assert_eq!(amounts.len(), 1);
        assert_eq!(amounts[0].amount_cents, 123456);
        assert_eq!(amounts[0].currency, "USD");
    }
}
