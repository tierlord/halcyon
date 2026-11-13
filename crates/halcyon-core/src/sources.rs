//! Recurring machine-mail sources ("Quellen") and anomaly detection
//! ("Anders als sonst") as specified in `docs/design/am-zug.md`.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::Error;

/// How a recurring source should be presented on "Heute" and in the inbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    /// Show every message individually (`anzeigen`).
    Show,
    /// Bundle messages into a single expandable summary row (`zusammenfassen`).
    Bundle,
    /// Only keep the newest status message visible (`nur Stand zeigen`).
    LatestOnly,
    /// Hide unless an anomaly occurs (`stumm`).
    Mute,
}

impl fmt::Display for SourceMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SourceMode::Show => "show",
            SourceMode::Bundle => "bundle",
            SourceMode::LatestOnly => "latest_only",
            SourceMode::Mute => "mute",
        })
    }
}

impl FromStr for SourceMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "show" => Ok(SourceMode::Show),
            "bundle" => Ok(SourceMode::Bundle),
            "latest_only" => Ok(SourceMode::LatestOnly),
            "mute" => Ok(SourceMode::Mute),
            other => Err(Error::Other(format!("unknown source mode {other:?}"))),
        }
    }
}

/// Kind of deviation detected for a recurring source ("Anders als sonst").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyKind {
    /// Sender previously passed DMARC reliably, but this message failed DMARC.
    DmarcFailed,
    /// Invoice or debit amount deviates significantly from historical median.
    UnusualAmount,
    /// Established sender sent a never-before-seen normalized subject pattern.
    NewSubjectPattern,
    /// Burst of messages in a very short window compared to typical interval.
    FrequencySpike,
    /// Regular source remained silent for much longer than its typical interval.
    LongSilence,
}

impl fmt::Display for AnomalyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AnomalyKind::DmarcFailed => "dmarc_failed",
            AnomalyKind::UnusualAmount => "unusual_amount",
            AnomalyKind::NewSubjectPattern => "new_subject_pattern",
            AnomalyKind::FrequencySpike => "frequency_spike",
            AnomalyKind::LongSilence => "long_silence",
        })
    }
}

impl FromStr for AnomalyKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dmarc_failed" => Ok(AnomalyKind::DmarcFailed),
            "unusual_amount" => Ok(AnomalyKind::UnusualAmount),
            "new_subject_pattern" => Ok(AnomalyKind::NewSubjectPattern),
            "frequency_spike" => Ok(AnomalyKind::FrequencySpike),
            "long_silence" => Ok(AnomalyKind::LongSilence),
            other => Err(Error::Other(format!("unknown anomaly kind {other:?}"))),
        }
    }
}

/// Stored recurring source row from the database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: i64,
    pub source_key: String,
    pub display_name: String,
    pub domain: Option<String>,
    pub mode: SourceMode,
    pub user_mode: bool,
    pub total_messages: u32,
    pub read_messages: u32,
    pub dmarc_pass_count: u32,
    pub is_newsletter: bool,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub median_interval_secs: Option<i64>,
    pub median_amount_cents: Option<i64>,
    pub amount_sample_count: u32,
    pub anomaly: Option<SourceAnomaly>,
    pub patterns: Vec<String>,
}

/// Privacy-preserving aggregate evaluation numbers for a mailbox (no message
/// contents, addresses, or subjects).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MailboxEvalStats {
    pub total_messages: usize,
    pub unique_sources: usize,
    pub unique_patterns: usize,
    pub recurring_pattern_messages: usize,
    pub top_100_patterns_messages: usize,
    pub sources_show: usize,
    pub sources_bundle: usize,
    pub sources_latest_only: usize,
    pub sources_mute: usize,
    pub active_anomalies: usize,
    pub cases_needs_you: usize,
    pub cases_waiting: usize,
    pub cases_done: usize,
    pub cases_order: usize,
    pub cases_invoice: usize,
    pub cases_contract: usize,
    pub human_threads_total: usize,
    pub human_needs_you: usize,
    pub human_waiting: usize,
    pub human_follow_up_due: usize,
    pub human_reading_along: usize,
    pub human_done: usize,
    pub recent_7d_incoming: usize,
    pub recent_7d_today_rows: usize,
}

/// Anomaly assessment for a source or incoming message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAnomaly {
    pub kind: AnomalyKind,
    /// True if the anomaly requires user attention ("Braucht dich").
    pub needs_you: bool,
    /// Concise German explanation shown under "Warum?".
    pub reason: String,
}

/// Historical profile of a recurring source used for anomaly detection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStats {
    pub total_messages: u32,
    pub dmarc_pass_count: u32,
    /// Known normalized subject patterns for this source.
    pub known_patterns: Vec<String>,
    /// Typical (median) interval between messages in seconds, if regular.
    pub median_interval_secs: Option<i64>,
    /// Timestamp of the previous message from this source.
    pub last_seen_at: Option<i64>,
    /// Number of messages received within the last hour.
    pub recent_hour_count: u32,
    /// Typical (median) amount in cents for sources that carry amounts.
    pub median_amount_cents: Option<i64>,
    /// Number of historical messages that carried an amount.
    pub amount_sample_count: u32,
}

/// Attributes of a newly arrived message evaluated against [`SourceStats`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomingSourceMail {
    pub occurred_at: i64,
    pub normalized_pattern: String,
    pub dmarc_pass: bool,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
}

/// Derives a stable, lowercase source identifier from `list_id` (preferred)
/// or `from_addr`. Returns `None` if neither yields a usable identifier.
pub fn derive_source_key(from_addr: Option<&str>, list_id: Option<&str>) -> Option<String> {
    if let Some(raw) = list_id {
        let trimmed = raw.trim();
        let cleaned = if trimmed.starts_with('<') && trimmed.ends_with('>') {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };
        let cleaned = cleaned
            .trim()
            .trim_matches('.')
            .trim_matches(|c| c == '<' || c == '>')
            .to_ascii_lowercase();
        if !cleaned.is_empty()
            && cleaned.chars().count() >= 3
            && (cleaned.contains('.') || cleaned.contains('-') || cleaned.contains('@'))
        {
            return Some(format!("list:{cleaned}"));
        }
    }

    if let Some(addr) = from_addr {
        let trimmed = addr.trim();
        let extracted = if let Some(start) = trimmed.find('<') {
            if let Some(end) = trimmed[start..].find('>') {
                &trimmed[start + 1..start + end]
            } else {
                trimmed
            }
        } else {
            trimmed
        };
        let extracted = extracted.trim().to_ascii_lowercase();
        if let Some(at_idx) = extracted.find('@')
            && at_idx > 0
            && at_idx + 1 < extracted.len()
        {
            let local = &extracted[..at_idx];
            let domain = &extracted[at_idx + 1..];
            if domain.contains('.') {
                // Normalize dynamic reply/bounce tokens in local part
                // 1. If local contains '+'
                let local_base = if let Some(plus_idx) = local.find('+') {
                    &local[..plus_idx]
                } else {
                    local
                };

                // 2. Check for hex/digit token of >= 6 chars after '-' or '_'
                let local_cleaned = if let Some(sep_idx) = local_base.find(['-', '_']) {
                    let suffix = &local_base[sep_idx + 1..];
                    if suffix.chars().count() >= 6
                        && suffix
                            .chars()
                            .all(|c| c.is_ascii_hexdigit() || c.is_ascii_digit())
                    {
                        &local_base[..sep_idx]
                    } else {
                        local_base
                    }
                } else {
                    local_base
                };

                if !local_cleaned.is_empty() {
                    return Some(format!("addr:{}@{}", local_cleaned, domain));
                }
            }
        }
    }

    None
}

/// Normalizes a subject line into a reusable pattern by stripping reply/forward
/// prefixes and replacing quoted items, dates, monetary amounts, IDs, and
/// numbers with placeholders (`{item}`, `{date}`, `{amount}`, `{id}`, `{num}`).
pub fn normalize_subject_pattern(subject: &str) -> String {
    static PREFIX_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)^(?:(?:re|aw|antw|fwd|fw|wg|wtr)(?:\[\d+\])?:\s*)+(.+)$"#).unwrap()
    });

    let mut current = subject.trim().to_string();

    // Repeatedly strip prefixes
    loop {
        if let Some(caps) = PREFIX_REGEX.captures(&current)
            && let Some(m) = caps.get(1)
        {
            current = m.as_str().trim().to_string();
            continue;
        }
        // Also handle optional leading counter if missed by main prefix regex
        static LEADING_BRACKET_NUM: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)^\[\d+\]:\s*(.+)$"#).unwrap());
        if let Some(caps) = LEADING_BRACKET_NUM.captures(&current)
            && let Some(m) = caps.get(1)
        {
            current = m.as_str().trim().to_string();
            continue;
        }
        break;
    }

    // 2. Replace quoted strings ("...", '...', „...“, “...”) of length >= 3 with {item}
    static QUOTED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:„[^“"']{1,}“|“[^“"']{1,}"|"[^"]{1,}"|'[^']{1,}')"#).unwrap()
    });
    current = QUOTED_REGEX.replace_all(&current, "{item}").to_string();

    // 3. Replace date expressions with {date}
    static DATE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(?:KW\s*\d{1,2}(?:/\d{2,4})?|\d{1,2}\.\d{1,2}\.\d{2,4}|\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{2,4}|(?:Januar|Februar|März|Maerz|April|Mai|Juni|Juli|August|September|Oktober|November|Dezember|January|February|March|May|June|July|October|December|Jan|Feb|Mär|Mar|Apr|Jun|Jul|Aug|Sep|Okt|Oct|Nov|Dez|Dec)\.?\s+\d{4})\b"#).unwrap()
    });
    current = DATE_REGEX.replace_all(&current, "{date}").to_string();

    // 4. Replace monetary amounts with {amount}
    static AMOUNT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:\d{1,3}(?:[.,]\d{3})*[.,]\d{2}\s*(?:€|eur|usd|chf|gbp|\$|£)|(?:€|eur|usd|chf|gbp|\$|£)\s*\d+(?:[.,]\d{2})?)"#).unwrap()
    });
    current = AMOUNT_REGEX.replace_all(&current, "{amount}").to_string();

    // 5. Replace alphanumeric IDs/codes containing both letters and digits (or preceded by #) of length >= 4
    static ID_REGEX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?:#[a-zA-Z0-9-]{3,}|\b[a-zA-Z0-9]+(?:[a-zA-Z]+[0-9]+|[0-9]+[a-zA-Z]+)[a-zA-Z0-9-]*)\b"#,
        )
        .unwrap()
    });
    current = ID_REGEX.replace_all(&current, "{id}").to_string();

    // 6. Replace remaining digit sequences of 2 or more digits (\b\d{2,}\b) with {num}
    static NUM_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b\d{2,}\b"#).unwrap());
    current = NUM_REGEX.replace_all(&current, "{num}").to_string();

    // 7. Collapse consecutive whitespace into a single space and trim
    static WS_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\s+"#).unwrap());
    WS_REGEX.replace_all(&current, " ").trim().to_string()
}

/// Suggests a default [`SourceMode`] based on historical volume, read ratio,
/// and whether the source is a newsletter or status update.
pub fn suggest_source_mode(
    total_messages: u32,
    read_messages: u32,
    is_newsletter: bool,
    is_status_update: bool,
) -> SourceMode {
    if total_messages < 3 {
        return SourceMode::Show;
    }
    if is_status_update {
        return SourceMode::LatestOnly;
    }
    let read_ratio = read_messages as f32 / total_messages as f32;
    if total_messages >= 10 && read_ratio < 0.05 {
        return SourceMode::Mute;
    }
    if is_newsletter || (total_messages >= 5 && read_ratio < 0.5) {
        return SourceMode::Bundle;
    }
    SourceMode::Show
}

/// Evaluates whether `incoming` deviates from `stats` ("Anders als sonst").
pub fn detect_anomaly(stats: &SourceStats, incoming: &IncomingSourceMail) -> Option<SourceAnomaly> {
    // 1. DmarcFailed
    if stats.total_messages >= 3 {
        let pass_ratio = stats.dmarc_pass_count as f32 / stats.total_messages as f32;
        if pass_ratio >= 0.9 && !incoming.dmarc_pass {
            return Some(SourceAnomaly {
                kind: AnomalyKind::DmarcFailed,
                needs_you: true,
                reason: "DMARC-Prüfung fehlgeschlagen (sonst authentifiziert)".into(),
            });
        }
    }

    // 2. UnusualAmount
    if let (Some(median), Some(actual)) = (stats.median_amount_cents, incoming.amount_cents)
        && stats.amount_sample_count >= 3
        && median > 0
    {
        let diff = (actual - median).abs();
        if diff >= 500 && (diff as f64 / median as f64) > 0.25 {
            let needs_you = actual > median;
            return Some(SourceAnomaly {
                kind: AnomalyKind::UnusualAmount,
                needs_you,
                reason: "Betrag weicht deutlich vom üblichen Wert ab".into(),
            });
        }
    }

    // 3. FrequencySpike
    if stats.total_messages >= 5
        && stats.recent_hour_count >= 5
        && stats.median_interval_secs.is_some_and(|iv| iv >= 6 * 3600)
    {
        return Some(SourceAnomaly {
            kind: AnomalyKind::FrequencySpike,
            needs_you: false,
            reason: format!(
                "Ungewöhnliche Häufung ({} Mails in der letzten Stunde)",
                stats.recent_hour_count
            ),
        });
    }

    // 4. LongSilence
    if stats.total_messages >= 5
        && let (Some(last), Some(iv)) = (stats.last_seen_at, stats.median_interval_secs)
        && iv >= 86_400
    {
        let gap = incoming.occurred_at - last;
        if gap > iv * 3 {
            return Some(SourceAnomaly {
                kind: AnomalyKind::LongSilence,
                needs_you: false,
                reason: "Quelle meldet sich nach ungewöhnlich langer Pause wieder".into(),
            });
        }
    }

    // 5. NewSubjectPattern
    if stats.total_messages >= 8
        && !stats.known_patterns.is_empty()
        && stats.known_patterns.len() <= 4
        && !stats
            .known_patterns
            .iter()
            .any(|p| p == &incoming.normalized_pattern)
    {
        return Some(SourceAnomaly {
            kind: AnomalyKind::NewSubjectPattern,
            needs_you: false,
            reason: "Neues Betreffmuster von einem bekannten Absender".into(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_source_key_list_id() {
        assert_eq!(
            derive_source_key(None, Some("<news.example.org>")),
            Some("list:news.example.org".into())
        );
        assert_eq!(
            derive_source_key(None, Some("  list-name.example.org  ")),
            Some("list:list-name.example.org".into())
        );
        assert_eq!(derive_source_key(None, Some("ab")), None);
        assert_eq!(derive_source_key(None, None), None);
    }

    #[test]
    fn test_derive_source_key_from_addr() {
        assert_eq!(
            derive_source_key(Some("Alice <alice@example.com>"), None),
            Some("addr:alice@example.com".into())
        );
        assert_eq!(
            derive_source_key(Some("bounce+12345@news.example.org"), None),
            Some("addr:bounce@news.example.org".into())
        );
        assert_eq!(
            derive_source_key(Some("noreply-89ab34f@shop.example"), None),
            Some("addr:noreply@shop.example".into())
        );
        assert_eq!(derive_source_key(Some("invalid-email"), None), None);
    }

    #[test]
    fn test_normalize_subject_pattern_prefixes() {
        assert_eq!(
            normalize_subject_pattern("Re: [2]: Fwd: Deine Bestellung"),
            normalize_subject_pattern("Deine Bestellung")
        );
        assert_eq!(
            normalize_subject_pattern("AW: Re: Re[3]: Status Update"),
            normalize_subject_pattern("Status Update")
        );
    }

    #[test]
    fn test_normalize_subject_pattern_quoted() {
        assert_eq!(
            normalize_subject_pattern("Rechnung „12345“ erhalten"),
            "Rechnung {item} erhalten"
        );
        assert_eq!(
            normalize_subject_pattern("Bestellung 'AB-99' versandt"),
            "Bestellung {item} versandt"
        );
    }

    #[test]
    fn test_normalize_subject_pattern_dates() {
        assert_eq!(
            normalize_subject_pattern("Termin am 12.10.2026"),
            "Termin am {date}"
        );
        assert_eq!(
            normalize_subject_pattern("Report 2026-08-15"),
            "Report {date}"
        );
        assert_eq!(
            normalize_subject_pattern("Wochenbericht KW 42/2026"),
            "Wochenbericht {date}"
        );
        assert_eq!(
            normalize_subject_pattern("Newsletter Januar 2026"),
            "Newsletter {date}"
        );
    }

    #[test]
    fn test_normalize_subject_pattern_amounts() {
        assert_eq!(
            normalize_subject_pattern("Zahlung von 1.234,56 €"),
            "Zahlung von {amount}"
        );
        assert_eq!(
            normalize_subject_pattern("Rechnung EUR 49,99 fällig"),
            "Rechnung {amount} fällig"
        );
    }

    #[test]
    fn test_normalize_subject_pattern_ids_and_nums() {
        assert_eq!(
            normalize_subject_pattern("Ticket #12345 erstellt"),
            "Ticket {id} erstellt"
        );
        assert_eq!(
            normalize_subject_pattern("Fehler 404 aufgetreten"),
            "Fehler {num} aufgetreten"
        );
    }

    #[test]
    fn test_suggest_source_mode_cases() {
        assert_eq!(suggest_source_mode(2, 2, false, false), SourceMode::Show);
        assert_eq!(
            suggest_source_mode(10, 5, false, true),
            SourceMode::LatestOnly
        );
        assert_eq!(suggest_source_mode(20, 0, false, false), SourceMode::Mute);
        assert_eq!(suggest_source_mode(5, 4, true, false), SourceMode::Bundle);
        assert_eq!(suggest_source_mode(10, 2, false, false), SourceMode::Bundle);
        assert_eq!(suggest_source_mode(10, 8, false, false), SourceMode::Show);
    }

    #[test]
    fn test_detect_anomaly_dmarc_failed() {
        let stats = SourceStats {
            total_messages: 10,
            dmarc_pass_count: 10,
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 1000,
            normalized_pattern: "Test".into(),
            dmarc_pass: false,
            amount_cents: None,
            currency: None,
        };
        let anomaly = detect_anomaly(&stats, &incoming).unwrap();
        assert_eq!(anomaly.kind, AnomalyKind::DmarcFailed);
        assert!(anomaly.needs_you);
    }

    #[test]
    fn test_detect_anomaly_unusual_amount() {
        let stats = SourceStats {
            total_messages: 5,
            median_amount_cents: Some(10000),
            amount_sample_count: 5,
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 1000,
            normalized_pattern: "Test".into(),
            dmarc_pass: true,
            amount_cents: Some(20000),
            currency: Some("EUR".into()),
        };
        let anomaly = detect_anomaly(&stats, &incoming).unwrap();
        assert_eq!(anomaly.kind, AnomalyKind::UnusualAmount);
        assert!(anomaly.needs_you);
    }

    #[test]
    fn test_detect_anomaly_frequency_spike() {
        let stats = SourceStats {
            total_messages: 10,
            recent_hour_count: 5,
            median_interval_secs: Some(86400),
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 1000,
            normalized_pattern: "Test".into(),
            dmarc_pass: true,
            amount_cents: None,
            currency: None,
        };
        let anomaly = detect_anomaly(&stats, &incoming).unwrap();
        assert_eq!(anomaly.kind, AnomalyKind::FrequencySpike);
        assert!(!anomaly.needs_you);
    }

    #[test]
    fn test_detect_anomaly_long_silence() {
        let stats = SourceStats {
            total_messages: 10,
            last_seen_at: Some(0),
            median_interval_secs: Some(100_000),
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 400_000,
            normalized_pattern: "Test".into(),
            dmarc_pass: true,
            amount_cents: None,
            currency: None,
        };
        let anomaly = detect_anomaly(&stats, &incoming).unwrap();
        assert_eq!(anomaly.kind, AnomalyKind::LongSilence);
        assert!(!anomaly.needs_you);
    }

    #[test]
    fn test_detect_anomaly_new_subject_pattern() {
        let stats = SourceStats {
            total_messages: 10,
            known_patterns: vec!["Pattern A".into()],
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 1000,
            normalized_pattern: "Brand New Pattern".into(),
            dmarc_pass: true,
            amount_cents: None,
            currency: None,
        };
        let anomaly = detect_anomaly(&stats, &incoming).unwrap();
        assert_eq!(anomaly.kind, AnomalyKind::NewSubjectPattern);
        assert!(!anomaly.needs_you);
    }

    #[test]
    fn test_detect_anomaly_none() {
        let stats = SourceStats {
            total_messages: 10,
            dmarc_pass_count: 10,
            known_patterns: vec!["Test".into()],
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 1000,
            normalized_pattern: "Test".into(),
            dmarc_pass: true,
            amount_cents: Some(10000),
            currency: Some("EUR".into()),
        };
        assert!(detect_anomaly(&stats, &incoming).is_none());
    }

    #[test]
    fn test_extra_derive_key() {
        assert_eq!(
            derive_source_key(Some("test@example.com"), None),
            Some("addr:test@example.com".into())
        );
        assert_eq!(
            derive_source_key(Some("<user@example.org>"), None),
            Some("addr:user@example.org".into())
        );
        assert_eq!(
            derive_source_key(Some("invalid"), Some("<list.org>")),
            Some("list:list.org".into())
        );
        assert_eq!(derive_source_key(None, Some("a")), None);
        assert_eq!(derive_source_key(Some("no-at-sign"), None), None);
    }

    #[test]
    fn test_extra_normalize_patterns() {
        assert_eq!(normalize_subject_pattern("   "), "");
        assert_eq!(normalize_subject_pattern("Re: Hallo Welt"), "Hallo Welt");
        assert_eq!(normalize_subject_pattern("Bericht"), "Bericht");
    }

    #[test]
    fn test_extra_suggest_mode() {
        assert_eq!(suggest_source_mode(1, 0, false, false), SourceMode::Show);
        assert_eq!(suggest_source_mode(4, 4, false, false), SourceMode::Show);
        assert_eq!(suggest_source_mode(15, 12, true, false), SourceMode::Bundle);
    }

    #[test]
    fn test_extra_anomalies_edge_cases() {
        let stats = SourceStats {
            total_messages: 2,
            dmarc_pass_count: 2,
            ..Default::default()
        };
        let incoming = IncomingSourceMail {
            occurred_at: 100,
            normalized_pattern: "A".into(),
            dmarc_pass: false,
            amount_cents: None,
            currency: None,
        };
        assert!(detect_anomaly(&stats, &incoming).is_none());

        let stats2 = SourceStats {
            total_messages: 5,
            median_amount_cents: Some(1000),
            amount_sample_count: 5,
            ..Default::default()
        };
        let incoming2 = IncomingSourceMail {
            occurred_at: 100,
            normalized_pattern: "A".into(),
            dmarc_pass: true,
            amount_cents: Some(1200),
            currency: None,
        };
        assert!(detect_anomaly(&stats2, &incoming2).is_none());
    }
}
