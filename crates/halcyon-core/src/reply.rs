//! Pure reply, reply-all, and forward draft-building logic.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Weekday};
use regex::Regex;

use crate::model::{MessageHeader, split_address};

/// A parsed mail recipient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    pub name: Option<String>,
    /// Lowercased email address without angle brackets.
    pub address: String,
}

/// Which kind of draft to build from an existing message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

/// Pre-filled fields for a reply or forward draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftSeed {
    pub to: Vec<Recipient>,
    pub cc: Vec<Recipient>,
    pub subject: String,
    /// Message-ID of the original message (without `<>`), `None` for forwards.
    pub in_reply_to: Option<String>,
    /// Full `References` chain (without `<>`), empty for forwards.
    pub references: Vec<String>,
    /// Pre-filled plain text: two blank lines, then quote or forward block.
    pub body: String,
}

// Matches a single leading reply prefix (case-insensitive) with optional counter
// like `Re[2]:`, `Re(3):`, `Re^2:` and half- or full-width colon.
static RE_REPLY_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?i)(?:re|aw|antw(?:ort)?|sv|vs|rif|réf|odp|ynt|vá|r|回复|答复)",
        r"(?:\s*(?:\[\d+\]|\(\d+\)|\^\d+))?",
        r"\s*[:：]\s*",
    ))
    .unwrap()
});

// Matches a single leading forward prefix (case-insensitive) with optional counter
// and half- or full-width colon.
static RE_FORWARD_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?i)(?:fwd|fw|wg|wtr|tr|rv|doorst|转发)",
        r"(?:\s*(?:\[\d+\]|\(\d+\)|\^\d+))?",
        r"\s*[:：]\s*",
    ))
    .unwrap()
});

// Matches a leading mailing-list tag in square brackets, e.g. `[team] `.
static RE_LIST_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\[[^\[\]]+\]\s*)").unwrap());

/// Builds a reply, reply-all, or forward draft seed from a received message.
///
/// `own_addresses` should contain all lowercased addresses belonging to the
/// user (across all accounts). `text` is the plain-text body of the original
/// message. `utc_offset_minutes` controls the timezone of the German quote
/// timestamp so callers and tests are deterministic.
pub fn prepare(
    kind: ReplyKind,
    original: &MessageHeader,
    text: &str,
    own_addresses: &[String],
    utc_offset_minutes: i32,
) -> DraftSeed {
    let own_set: HashSet<String> = own_addresses
        .iter()
        .map(|a| normalize_address(a))
        .filter(|a| !a.is_empty())
        .collect();

    let (to, cc) = build_recipients(kind, original, &own_set);
    let subject = build_subject(kind, original.subject.as_deref());
    let (in_reply_to, references) = build_threading(kind, original);
    let body = build_body(kind, original, text, utc_offset_minutes);

    DraftSeed {
        to,
        cc,
        subject,
        in_reply_to,
        references,
        body,
    }
}

/// Parses `"Max Muster <max@example.org>"`, `"max@example.org"`, or
/// `"\"Muster, Max\" <max@example.org>"` into a [`Recipient`].
///
/// Returns `None` if the input does not contain a valid email address.
pub fn parse_recipient(s: &str) -> Option<Recipient> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(open) = trimmed.rfind('<')
        && let Some(close_rel) = trimmed[open + 1..].find('>')
    {
        let raw_addr = trimmed[open + 1..open + 1 + close_rel].trim();
        if !trimmed[open + 1 + close_rel + 1..].trim().is_empty() {
            return None;
        }
        let address = validate_address(raw_addr)?;
        let raw_name = trimmed[..open].trim();
        let name = unquote_display_name(raw_name);
        return Some(Recipient { name, address });
    }

    let address = validate_address(trimmed)?;
    Some(Recipient {
        name: None,
        address,
    })
}

fn validate_address(raw: &str) -> Option<String> {
    let addr = normalize_address(raw);
    if addr.is_empty() || addr.chars().any(char::is_whitespace) {
        return None;
    }
    split_address(&addr).ok()?;
    Some(addr)
}

fn normalize_address(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_ascii_lowercase()
}

fn unquote_display_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let unquoted = if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\'
                && let Some(escaped) = chars.next()
            {
                out.push(escaped);
                continue;
            }
            out.push(ch);
        }
        out
    } else {
        trimmed.to_string()
    };

    let final_name = unquoted.trim();
    if final_name.is_empty() {
        None
    } else {
        Some(final_name.to_string())
    }
}

fn build_subject(kind: ReplyKind, raw_subject: Option<&str>) -> String {
    let subject = raw_subject.unwrap_or("").trim();
    match kind {
        ReplyKind::Reply | ReplyKind::ReplyAll => {
            // Mailing-list tags (e.g. `[team] Re: Plan`): we follow Thunderbird's
            // convention (`Re: [team] Plan`), placing `Re: ` first while preserving
            // the list tag and stripping any reply prefixes before or after it.
            let stripped = strip_reply_prefixes_with_list_tags(subject);
            if stripped.is_empty() {
                "Re: ".to_string()
            } else {
                format!("Re: {stripped}")
            }
        }
        ReplyKind::Forward => {
            let stripped = strip_prefixes_loop(subject, &RE_FORWARD_PREFIX);
            if stripped.is_empty() {
                "Fwd: ".to_string()
            } else {
                format!("Fwd: {stripped}")
            }
        }
    }
}

fn strip_reply_prefixes_with_list_tags(input: &str) -> String {
    let mut rest = strip_prefixes_loop(input, &RE_REPLY_PREFIX);
    let mut tags = String::new();

    while let Some(m) = RE_LIST_TAG.find(rest) {
        tags.push_str(m.as_str());
        rest = strip_prefixes_loop(&rest[m.end()..], &RE_REPLY_PREFIX);
    }

    format!("{tags}{rest}").trim().to_string()
}

fn strip_prefixes_loop<'a>(mut input: &'a str, re: &Regex) -> &'a str {
    input = input.trim_start();
    while let Some(m) = re.find(input) {
        input = input[m.end()..].trim_start();
    }
    input
}

fn build_recipients(
    kind: ReplyKind,
    original: &MessageHeader,
    own_set: &HashSet<String>,
) -> (Vec<Recipient>, Vec<Recipient>) {
    if kind == ReplyKind::Forward {
        return (Vec::new(), Vec::new());
    }

    let from_recipient = original
        .from_addr
        .as_deref()
        .and_then(validate_address)
        .map(|address| Recipient {
            name: original
                .from_name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
            address,
        });

    let reply_to_recipient = original.reply_to.as_deref().and_then(parse_recipient);

    let from_is_own = from_recipient
        .as_ref()
        .is_some_and(|r| own_set.contains(&r.address));

    let mut to = Vec::new();
    let mut seen_to = HashSet::new();

    if from_is_own && reply_to_recipient.is_none() {
        // Following up on one's own mail: to the people it went to, but not
        // to oneself (a copy to oneself is common in To or Cc).
        for raw in &original.to {
            if let Some(rec) = parse_recipient(raw)
                && !own_set.contains(&rec.address)
                && seen_to.insert(rec.address.clone())
            {
                to.push(rec);
            }
        }
    }

    if to.is_empty()
        && let Some(primary) = reply_to_recipient.or(from_recipient)
    {
        seen_to.insert(primary.address.clone());
        to.push(primary);
    }

    if kind == ReplyKind::Reply {
        return (to, Vec::new());
    }

    // ReplyAll: Cc gets original To + Cc, minus own addresses, duplicates, and `to`.
    let mut cc = Vec::new();
    let mut seen_cc = HashSet::new();
    for raw in original.to.iter().chain(original.cc.iter()) {
        if let Some(rec) = parse_recipient(raw)
            && !own_set.contains(&rec.address)
            && !seen_to.contains(&rec.address)
            && seen_cc.insert(rec.address.clone())
        {
            cc.push(rec);
        }
    }

    (to, cc)
}

fn build_threading(kind: ReplyKind, original: &MessageHeader) -> (Option<String>, Vec<String>) {
    if kind == ReplyKind::Forward {
        return (None, Vec::new());
    }

    let msg_id = original
        .message_id
        .as_deref()
        .map(strip_angle_brackets)
        .filter(|s| !s.is_empty());

    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for r in original
        .references
        .iter()
        .map(|s| strip_angle_brackets(s))
        .chain(msg_id.iter().cloned())
    {
        if !r.is_empty() && seen.insert(r.clone()) {
            refs.push(r);
        }
    }

    // RFC 5322 3.6.4 recommends keeping the root and the most recent entries when trimming.
    if refs.len() > 20 {
        let mut trimmed = Vec::with_capacity(20);
        trimmed.push(refs[0].clone());
        trimmed.extend_from_slice(&refs[refs.len() - 19..]);
        refs = trimmed;
    }

    (msg_id, refs)
}

fn strip_angle_brackets(s: &str) -> String {
    s.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

fn build_body(
    kind: ReplyKind,
    original: &MessageHeader,
    text: &str,
    utc_offset_minutes: i32,
) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let date_str = original
        .date
        .and_then(|ts| format_german_date(ts, utc_offset_minutes));

    match kind {
        ReplyKind::Reply | ReplyKind::ReplyAll => {
            let sender = original
                .from_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    original
                        .from_addr
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("Unbekannt");

            let attribution = match date_str {
                Some(d) => format!("Am {d} schrieb {sender}:"),
                None => format!("{sender} schrieb:"),
            };

            let quoted = quote_without_signature(&normalized);
            if quoted.is_empty() {
                format!("\n\n{attribution}\n>")
            } else {
                format!("\n\n{attribution}\n{quoted}")
            }
        }
        ReplyKind::Forward => {
            let subject = original.subject.as_deref().unwrap_or("").trim();
            let date_line = date_str.unwrap_or_default();
            let from_line = format_from(original);
            let to_line = original.to.join(", ");

            format!(
                "\n\n-------- Weitergeleitete Nachricht --------\n\
                 Betreff: {subject}\n\
                 Datum: {date_line}\n\
                 Von: {from_line}\n\
                 An: {to_line}\n\n\
                 {normalized}"
            )
        }
    }
}

fn format_from(original: &MessageHeader) -> String {
    let name = original
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let addr = original
        .from_addr
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match (name, addr) {
        (Some(n), Some(a)) => format!("{n} <{a}>"),
        (None, Some(a)) => a.to_string(),
        (Some(n), None) => n.to_string(),
        (None, None) => String::new(),
    }
}

fn quote_without_signature(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if line == "-- " {
            break;
        }
        lines.push(line);
    }

    // Trim a single trailing empty line if the text ended with a newline.
    if lines.last() == Some(&"") {
        lines.pop();
    }

    lines
        .into_iter()
        .map(|line| {
            if line.starts_with('>') {
                format!(">{line}")
            } else if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_german_date(timestamp_secs: i64, utc_offset_minutes: i32) -> Option<String> {
    let offset = FixedOffset::east_opt(utc_offset_minutes.saturating_mul(60))?;
    let dt = DateTime::from_timestamp(timestamp_secs, 0)?.with_timezone(&offset);
    let weekday = match dt.weekday() {
        Weekday::Mon => "Mo.",
        Weekday::Tue => "Di.",
        Weekday::Wed => "Mi.",
        Weekday::Thu => "Do.",
        Weekday::Fri => "Fr.",
        Weekday::Sat => "Sa.",
        Weekday::Sun => "So.",
    };
    Some(format!(
        "{weekday}, {:02}.{:02}.{:04} um {:02}:{:02}",
        dt.day(),
        dt.month(),
        dt.year(),
        dt.hour(),
        dt.minute()
    ))
}

#[cfg(test)]
mod tests {
    use super::{Recipient, ReplyKind, parse_recipient, prepare};
    use crate::model::MessageHeader;

    fn base_header() -> MessageHeader {
        MessageHeader {
            uid: 1,
            message_id: Some("msg-1@mail.example".into()),
            in_reply_to: None,
            references: Vec::new(),
            subject: Some("Projektplan".into()),
            from_name: Some("Anna Admin".into()),
            from_addr: Some("anna@team.example".into()),
            reply_to: None,
            to: vec!["Max Muster <max@mail.example>".into()],
            cc: Vec::new(),
            // 2026-11-10 07:15:00 UTC -> 08:15 +01:00 (Tuesday)
            date: Some(1_794_294_900),
            size: None,
            flags: Vec::new(),
            list_id: None,
            list_unsubscribe: Vec::new(),
            list_unsubscribe_post: false,
        }
    }

    fn own() -> Vec<String> {
        vec!["max@mail.example".into(), "max.arbeit@firma.example".into()]
    }

    // --- parse_recipient tests ---

    #[test]
    fn parses_name_and_angle_address() {
        assert_eq!(
            parse_recipient("Max Muster <Max@Example.ORG>"),
            Some(Recipient {
                name: Some("Max Muster".into()),
                address: "max@example.org".into(),
            })
        );
    }

    #[test]
    fn parses_bare_address() {
        assert_eq!(
            parse_recipient("  anna@team.example  "),
            Some(Recipient {
                name: None,
                address: "anna@team.example".into(),
            })
        );
    }

    #[test]
    fn parses_quoted_name_with_comma() {
        assert_eq!(
            parse_recipient(r#""Muster, Max" <max@mail.example>"#),
            Some(Recipient {
                name: Some("Muster, Max".into()),
                address: "max@mail.example".into(),
            })
        );
    }

    #[test]
    fn parses_quoted_name_with_escaped_quotes() {
        assert_eq!(
            parse_recipient(r#""Max \"Der Chef\" Muster" <max@mail.example>"#),
            Some(Recipient {
                name: Some(r#"Max "Der Chef" Muster"#.into()),
                address: "max@mail.example".into(),
            })
        );
    }

    #[test]
    fn parses_angle_only_without_name() {
        assert_eq!(
            parse_recipient("<info@shop.example>"),
            Some(Recipient {
                name: None,
                address: "info@shop.example".into(),
            })
        );
    }

    #[test]
    fn rejects_invalid_recipients() {
        assert_eq!(parse_recipient(""), None);
        assert_eq!(parse_recipient("   "), None);
        assert_eq!(parse_recipient("Nur Ein Name"), None);
        assert_eq!(parse_recipient("Max <ungueltig>"), None);
        assert_eq!(parse_recipient("max@localhost"), None);
        assert_eq!(parse_recipient("<a@b.example> trailing"), None);
    }

    // --- Subject prefix tests ---

    #[test]
    fn reply_adds_re_prefix() {
        let h = base_header();
        let seed = prepare(ReplyKind::Reply, &h, "Hallo", &own(), 60);
        assert_eq!(seed.subject, "Re: Projektplan");
    }

    #[test]
    fn reply_strips_mixed_german_english_prefixes() {
        let mut h = base_header();
        h.subject = Some("Re: AW: Re: RE: Antw: Antwort: Projektplan".into());
        let seed = prepare(ReplyKind::Reply, &h, "Hallo", &own(), 60);
        assert_eq!(seed.subject, "Re: Projektplan");
    }

    #[test]
    fn reply_strips_international_prefixes() {
        for raw in [
            "SV: Besprechung",
            "VS: Besprechung",
            "R: Besprechung",
            "Rif: Besprechung",
            "Réf: Besprechung",
            "Odp: Besprechung",
            "Ynt: Besprechung",
            "Vá: Besprechung",
            "回复: Besprechung",
            "答复：Besprechung",
        ] {
            let mut h = base_header();
            h.subject = Some(raw.into());
            let seed = prepare(ReplyKind::Reply, &h, "Hallo", &own(), 60);
            assert_eq!(seed.subject, "Re: Besprechung", "failed for {raw}");
        }
    }

    #[test]
    fn reply_strips_numbered_prefixes_and_fullwidth_colon() {
        let mut h = base_header();
        h.subject = Some("Re[2]: Aw(3): Re^4： Release".into());
        let seed = prepare(ReplyKind::Reply, &h, "Hallo", &own(), 60);
        assert_eq!(seed.subject, "Re: Release");
    }

    #[test]
    fn reply_preserves_mailing_list_tag_thunderbird_style() {
        let mut h = base_header();
        h.subject = Some("[team] Re: AW: Plan".into());
        let seed = prepare(ReplyKind::Reply, &h, "Hallo", &own(), 60);
        assert_eq!(seed.subject, "Re: [team] Plan");
    }

    #[test]
    fn reply_with_existing_re_and_list_tag_avoids_duplicates() {
        let mut h = base_header();
        h.subject = Some("Re: [team] Re: Plan".into());
        let seed = prepare(ReplyKind::Reply, &h, "Hallo", &own(), 60);
        assert_eq!(seed.subject, "Re: [team] Plan");
    }

    #[test]
    fn reply_empty_or_missing_subject_has_single_space() {
        let mut h = base_header();
        h.subject = None;
        assert_eq!(
            prepare(ReplyKind::Reply, &h, "", &own(), 60).subject,
            "Re: "
        );

        h.subject = Some("Re:   ".into());
        assert_eq!(
            prepare(ReplyKind::Reply, &h, "", &own(), 60).subject,
            "Re: "
        );
    }

    #[test]
    fn forward_strips_forward_prefixes_but_keeps_re() {
        let mut h = base_header();
        h.subject = Some("WG: Fwd: Fw[2]: TR: Re: Angebot".into());
        let seed = prepare(ReplyKind::Forward, &h, "Hallo", &own(), 60);
        assert_eq!(seed.subject, "Fwd: Re: Angebot");
    }

    #[test]
    fn forward_strips_international_forward_prefixes() {
        for raw in [
            "Wtr: Vertrag",
            "RV: Vertrag",
            "Doorst: Vertrag",
            "转发：Vertrag",
        ] {
            let mut h = base_header();
            h.subject = Some(raw.into());
            let seed = prepare(ReplyKind::Forward, &h, "Hallo", &own(), 60);
            assert_eq!(seed.subject, "Fwd: Vertrag", "failed for {raw}");
        }
    }

    #[test]
    fn forward_empty_subject_has_single_space() {
        let mut h = base_header();
        h.subject = Some("   ".into());
        assert_eq!(
            prepare(ReplyKind::Forward, &h, "", &own(), 60).subject,
            "Fwd: "
        );
    }

    // --- Recipient tests ---

    #[test]
    fn reply_uses_from_when_no_reply_to() {
        let h = base_header();
        let seed = prepare(ReplyKind::Reply, &h, "Hi", &own(), 60);
        assert_eq!(
            seed.to,
            vec![Recipient {
                name: Some("Anna Admin".into()),
                address: "anna@team.example".into(),
            }]
        );
        assert!(seed.cc.is_empty());
    }

    #[test]
    fn reply_prefers_reply_to_over_noreply_from() {
        let mut h = base_header();
        h.from_name = Some("Kleinanzeigen".into());
        h.from_addr = Some("noreply@kleinmarkt.example".into());
        h.reply_to = Some("conv-42@reply.kleinmarkt.example".into());

        let seed = prepare(ReplyKind::Reply, &h, "Anfrage", &own(), 60);
        assert_eq!(
            seed.to,
            vec![Recipient {
                name: None,
                address: "conv-42@reply.kleinmarkt.example".into(),
            }]
        );
    }

    #[test]
    fn reply_keeps_noreply_when_no_reply_to_exists() {
        let mut h = base_header();
        h.from_addr = Some("noreply@shop.example".into());
        h.from_name = None;
        h.reply_to = None;

        let seed = prepare(ReplyKind::Reply, &h, "Info", &own(), 60);
        assert_eq!(seed.to[0].address, "noreply@shop.example");
    }

    #[test]
    fn reply_to_own_sent_mail_targets_original_to() {
        let mut h = base_header();
        h.from_name = Some("Max Muster".into());
        h.from_addr = Some("MAX@mail.example".into());
        h.to = vec![
            "Bernd Bäcker <bernd@team.example>".into(),
            "clara@team.example".into(),
        ];

        let seed = prepare(ReplyKind::Reply, &h, "Gesendet", &own(), 60);
        assert_eq!(
            seed.to,
            vec![
                Recipient {
                    name: Some("Bernd Bäcker".into()),
                    address: "bernd@team.example".into(),
                },
                Recipient {
                    name: None,
                    address: "clara@team.example".into(),
                },
            ]
        );
    }

    #[test]
    fn reply_all_excludes_own_addresses_duplicates_and_primary_to() {
        let mut h = base_header();
        h.from_name = Some("Anna Admin".into());
        h.from_addr = Some("anna@team.example".into());
        h.to = vec![
            "Max Muster <MAX@mail.example>".into(),
            "Bernd Bäcker <bernd@team.example>".into(),
            "ANNA@team.example".into(),
        ];
        h.cc = vec![
            "Max Arbeit <max.arbeit@firma.example>".into(),
            "Clara Coder <clara@team.example>".into(),
            "bernd@team.example".into(),
            r#""Muster, Erika" <erika@post.example>"#.into(),
        ];

        let seed = prepare(ReplyKind::ReplyAll, &h, "Hi", &own(), 60);
        assert_eq!(seed.to.len(), 1);
        assert_eq!(seed.to[0].address, "anna@team.example");
        assert_eq!(
            seed.cc,
            vec![
                Recipient {
                    name: Some("Bernd Bäcker".into()),
                    address: "bernd@team.example".into(),
                },
                Recipient {
                    name: Some("Clara Coder".into()),
                    address: "clara@team.example".into(),
                },
                Recipient {
                    name: Some("Muster, Erika".into()),
                    address: "erika@post.example".into(),
                },
            ]
        );
    }

    #[test]
    fn forward_leaves_to_and_cc_empty() {
        let mut h = base_header();
        h.cc = vec!["clara@team.example".into()];
        let seed = prepare(ReplyKind::Forward, &h, "Text", &own(), 60);
        assert!(seed.to.is_empty());
        assert!(seed.cc.is_empty());
    }

    #[test]
    fn reply_handles_missing_from_gracefully() {
        let mut h = base_header();
        h.from_addr = None;
        h.from_name = None;
        let seed = prepare(ReplyKind::Reply, &h, "Text", &own(), 60);
        assert!(seed.to.is_empty());
    }

    // --- Threading tests ---

    #[test]
    fn reply_builds_in_reply_to_and_references() {
        let mut h = base_header();
        h.references = vec!["<root@team.example>".into(), "msg-0@team.example".into()];
        h.message_id = Some("<msg-1@team.example>".into());

        let seed = prepare(ReplyKind::Reply, &h, "Hi", &own(), 60);
        assert_eq!(seed.in_reply_to.as_deref(), Some("msg-1@team.example"));
        assert_eq!(
            seed.references,
            vec![
                "root@team.example",
                "msg-0@team.example",
                "msg-1@team.example",
            ]
        );
    }

    #[test]
    fn reply_deduplicates_references() {
        let mut h = base_header();
        h.references = vec![
            "root@team.example".into(),
            "msg-1@team.example".into(),
            "root@team.example".into(),
        ];
        h.message_id = Some("msg-1@team.example".into());

        let seed = prepare(ReplyKind::Reply, &h, "Hi", &own(), 60);
        assert_eq!(
            seed.references,
            vec!["root@team.example", "msg-1@team.example"]
        );
    }

    #[test]
    fn reply_trims_long_references_keeping_first_and_last_nineteen() {
        let mut h = base_header();
        h.references = (1..=25).map(|i| format!("m{i}@team.example")).collect();
        h.message_id = Some("m26@team.example".into());

        let seed = prepare(ReplyKind::Reply, &h, "Hi", &own(), 60);
        assert_eq!(seed.references.len(), 20);
        assert_eq!(seed.references[0], "m1@team.example");
        assert_eq!(seed.references[1], "m8@team.example");
        assert_eq!(seed.references[19], "m26@team.example");
    }

    #[test]
    fn reply_without_message_id_leaves_in_reply_to_none() {
        let mut h = base_header();
        h.message_id = None;
        let seed = prepare(ReplyKind::Reply, &h, "Hi", &own(), 60);
        assert_eq!(seed.in_reply_to, None);
        assert!(seed.references.is_empty());
    }

    #[test]
    fn forward_clears_threading_headers() {
        let mut h = base_header();
        h.references = vec!["root@team.example".into()];
        let seed = prepare(ReplyKind::Forward, &h, "Hi", &own(), 60);
        assert_eq!(seed.in_reply_to, None);
        assert!(seed.references.is_empty());
    }

    // --- Body, quote, signature, date, line endings tests ---

    #[test]
    fn reply_formats_german_date_and_quotes_lines() {
        let h = base_header();
        let seed = prepare(
            ReplyKind::Reply,
            &h,
            "Hallo Max,\n\nwie geht's?",
            &own(),
            60,
        );
        assert_eq!(
            seed.body,
            "\n\nAm Di., 10.11.2026 um 08:15 schrieb Anna Admin:\n> Hallo Max,\n>\n> wie geht's?"
        );
    }

    #[test]
    fn reply_nests_existing_quotes_without_extra_space() {
        let h = base_header();
        let seed = prepare(
            ReplyKind::Reply,
            &h,
            "Genau!\n> Vorheriger Text\n>> Noch älter",
            &own(),
            60,
        );
        assert_eq!(
            seed.body,
            "\n\nAm Di., 10.11.2026 um 08:15 schrieb Anna Admin:\n> Genau!\n>> Vorheriger Text\n>>> Noch älter"
        );
    }

    #[test]
    fn reply_strips_signature_and_normalizes_crlf() {
        let h = base_header();
        let text = "Erste Zeile\r\nZweite Zeile\r\n-- \r\nAnna Admin\r\nTel: 123";
        let seed = prepare(ReplyKind::Reply, &h, text, &own(), 60);
        assert!(!seed.body.contains('\r'));
        assert!(!seed.body.contains("Tel: 123"));
        assert_eq!(
            seed.body,
            "\n\nAm Di., 10.11.2026 um 08:15 schrieb Anna Admin:\n> Erste Zeile\n> Zweite Zeile"
        );
    }

    #[test]
    fn reply_falls_back_to_from_addr_when_from_name_missing() {
        let mut h = base_header();
        h.from_name = None;
        let seed = prepare(ReplyKind::Reply, &h, "Kurz", &own(), 60);
        assert!(
            seed.body
                .starts_with("\n\nAm Di., 10.11.2026 um 08:15 schrieb anna@team.example:\n")
        );
    }

    #[test]
    fn reply_handles_missing_date() {
        let mut h = base_header();
        h.date = None;
        let seed = prepare(ReplyKind::Reply, &h, "Kurz", &own(), 60);
        assert_eq!(seed.body, "\n\nAnna Admin schrieb:\n> Kurz");
    }

    #[test]
    fn forward_includes_header_block_and_unquoted_text_with_signature() {
        let h = base_header();
        let text = "Bitte prüfen.\r\n-- \r\nAnna Admin";
        let seed = prepare(ReplyKind::Forward, &h, text, &own(), 60);
        assert!(!seed.body.contains('\r'));
        assert_eq!(
            seed.body,
            "\n\n-------- Weitergeleitete Nachricht --------\n\
             Betreff: Projektplan\n\
             Datum: Di., 10.11.2026 um 08:15\n\
             Von: Anna Admin <anna@team.example>\n\
             An: Max Muster <max@mail.example>\n\n\
             Bitte prüfen.\n-- \nAnna Admin"
        );
    }

    #[test]
    fn date_respects_utc_offset_minutes() {
        let h = base_header(); // 07:15 UTC
        let utc = prepare(ReplyKind::Reply, &h, "Hi", &own(), 0);
        assert!(utc.body.contains("Di., 10.11.2026 um 07:15"));

        let minus_five = prepare(ReplyKind::Reply, &h, "Hi", &own(), -300);
        assert!(minus_five.body.contains("Di., 10.11.2026 um 02:15"));
    }

    #[test]
    fn weekdays_format_in_german_across_the_week() {
        let mut h = base_header();
        let day = 86_400;
        let base_ts = 1_794_208_500; // Mon 2026-11-09 07:15 UTC
        for (i, expected) in ["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."]
            .iter()
            .enumerate()
        {
            h.date = Some(base_ts + (i as i64) * day);
            let seed = prepare(ReplyKind::Reply, &h, "x", &own(), 60);
            assert!(
                seed.body.contains(expected),
                "expected {expected} in {}",
                seed.body
            );
        }
    }

    #[test]
    fn reply_with_empty_body_still_quotes_cleanly() {
        let h = base_header();
        let seed = prepare(ReplyKind::Reply, &h, "", &own(), 60);
        assert_eq!(
            seed.body,
            "\n\nAm Di., 10.11.2026 um 08:15 schrieb Anna Admin:\n>"
        );
    }

    #[test]
    fn reply_with_only_signature_produces_empty_quote_marker() {
        let h = base_header();
        let seed = prepare(ReplyKind::Reply, &h, "-- \nSignatur", &own(), 60);
        assert_eq!(
            seed.body,
            "\n\nAm Di., 10.11.2026 um 08:15 schrieb Anna Admin:\n>"
        );
    }

    #[test]
    fn reply_all_ignores_unparseable_to_and_cc_entries() {
        let mut h = base_header();
        h.to = vec!["kaputt".into(), "Bernd <bernd@team.example>".into()];
        h.cc = vec!["nochmal-kaputt".into()];
        let seed = prepare(ReplyKind::ReplyAll, &h, "Hi", &own(), 60);
        assert_eq!(
            seed.cc,
            vec![Recipient {
                name: Some("Bernd".into()),
                address: "bernd@team.example".into(),
            }]
        );
    }

    #[test]
    fn reply_preserves_multiple_list_tags_and_strips_prefixes() {
        let mut h = base_header();
        h.subject = Some("[org] [team] Aw: Re: Roadmap".into());
        let seed = prepare(ReplyKind::Reply, &h, "Hi", &own(), 60);
        assert_eq!(seed.subject, "Re: [org] [team] Roadmap");
    }

    #[test]
    fn reply_all_when_own_mail_has_other_recipients_in_to() {
        let mut h = base_header();
        h.from_addr = Some("max@mail.example".into());
        h.to = vec!["anna@team.example".into(), "bernd@team.example".into()];
        h.cc = vec!["clara@team.example".into()];
        let seed = prepare(ReplyKind::ReplyAll, &h, "Hi", &own(), 60);
        assert_eq!(seed.to.len(), 2);
        assert_eq!(
            seed.cc,
            vec![Recipient {
                name: None,
                address: "clara@team.example".into(),
            }]
        );
    }

    #[test]
    fn reply_to_own_sent_mail_skips_own_addresses() {
        let original = MessageHeader {
            from_addr: Some("max@mail.example".into()),
            to: vec![
                "Max <MAX@mail.example>".into(),
                "Jonas <jonas@werkstatt.example>".into(),
            ],
            ..Default::default()
        };
        let own = vec!["max@mail.example".to_string()];
        let d = prepare(ReplyKind::Reply, &original, "", &own, 0);
        let to: Vec<_> = d.to.iter().map(|r| r.address.as_str()).collect();
        assert_eq!(to, ["jonas@werkstatt.example"]);
    }
}
