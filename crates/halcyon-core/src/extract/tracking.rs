//! Extractor for parcel and letter tracking numbers and public tracking URLs.
//!
//! Evidence, strongest first:
//!
//! 1. **Carrier link**: the number sits in a link to the carrier's own
//!    tracking page. Shops and carriers almost always include one, and a
//!    link is far harder to confuse with an order or invoice number than
//!    free text.
//! 2. **Check digit**: formats with a published check digit (UPS 1Z, UPU S10,
//!    GS1/SSCC, USPS IMPB, DHL Express) are accepted on their own or with
//!    light context.
//! 3. **Context**: formats without a check digit (DPD, Hermes, GLS, FedEx)
//!    need a tracking keyword next to the number, or a link to that carrier
//!    somewhere in the message, and must not be labelled as an order,
//!    invoice, customer or similar number.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

/// Supported parcel and postal carriers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Carrier {
    Dhl,
    DhlExpress,
    Dpd,
    Hermes,
    Gls,
    Ups,
    FedEx,
    Usps,
    DeutschePostBrief,
}

/// Why a number was accepted; callers may treat [`Evidence::Context`] as
/// weaker (e.g. show it, but not auto-categorise on it alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Evidence {
    CarrierLink,
    CheckDigit,
    Context,
}

/// A detected shipment tracking number with its carrier and public tracking URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shipment {
    pub carrier: Carrier,
    pub tracking_number: String,
    /// Public tracking page for this number.
    pub url: String,
    pub evidence: Evidence,
}

// Unambiguous formats (can match without carrier context):
// - UPS 1Z: "1Z" + 16 alphanumeric chars (UPS 1Z specification).
static RE_UPS_1Z: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(1Z[A-Z0-9]{16})\b").unwrap());

// - UPU S10 registered/tracked letter for Germany (UPU Standard S10: 2 letters + 9 digits + "DE").
static RE_UPU_S10_DE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([RLEVC][A-Z]\d{9}DE)\b").unwrap());

// - USPS Intelligent Mail Package Barcode (20 or 22 digits starting with 92/93/94/95,
//   USPS Publication 199).
static RE_USPS_IMPB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(9[2345]\d{18}(?:\d{2})?)\b").unwrap());

// Context-dependent token patterns:
static RE_TOKEN_DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(\d{10,22})\b").unwrap());
static RE_DHL_JJD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(JJD\d{11,18})\b").unwrap());
static RE_DEUTSCHE_POST_MATRIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b([AF]00[0-9A-F]{17})\b").unwrap());

// Tracking URLs in message text:
static RE_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"https?://[^\s<>"'\)]+"#).unwrap());

static RE_GENERIC_TRACKING_CTX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(sendungsnummer|sendungs-?id|paketnummer|paket-?nr\.?|tracking(\s+number|\s*id|\s*nr\.?)?|track(ing)?-?code|sendungsverfolgung|verfolgen|verfolgung|waybill|awb)\b").unwrap()
});

// Labels that mark a number as something other than a tracking number.
static RE_NOT_TRACKING_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(",
        r"bestellnummer|bestell-?nr\.?|bestellung|auftragsnummer|auftrags-?nr\.?|auftrag|",
        r"rechnungsnummer|rechnungs-?nr\.?|rechnung|belegnummer|beleg-?nr\.?|",
        r"kundennummer|kunden-?nr\.?|vertragsnummer|vertrags-?nr\.?|artikelnummer|artikel-?nr\.?|",
        r"vorgangsnummer|aktenzeichen|mandatsreferenz|referenz|",
        r"order(\s+number|\s+no\.?|\s+id|\s*#)?|invoice(\s+number|\s+no\.?|\s*#)?|",
        r"customer(\s+number|\s+id)?|account(\s+number)?|",
        r"konto(nummer)?|iban|bic|telefon|tel\.?|phone|fax|mobil|handy",
        r")\b",
    ))
    .unwrap()
});

// Carriers mentioned as words (substring matches would hit "Backups", "Gruppe"
// or "Priorität").
static RE_CARRIER_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(dhl|dpd|hermes|gls|fedex|federal express|usps|deutsche post|einschreiben|warenpost)\b").unwrap()
});

/// Which carriers a message mentions or links to.
#[derive(Default)]
struct Mentions {
    dhl: bool,
    dhl_express: bool,
    dpd: bool,
    hermes: bool,
    gls: bool,
    fedex: bool,
    usps: bool,
    dp_brief: bool,
}

impl Mentions {
    fn from_text(text: &str) -> Self {
        let mut m = Mentions::default();
        for cap in RE_CARRIER_WORDS.captures_iter(text) {
            match cap[1].to_ascii_lowercase().as_str() {
                "dhl" => m.dhl = true,
                "dpd" => m.dpd = true,
                "hermes" => m.hermes = true,
                "gls" => m.gls = true,
                "fedex" | "federal express" => m.fedex = true,
                "usps" => m.usps = true,
                _ => m.dp_brief = true,
            }
        }
        let lower = text.to_ascii_lowercase();
        m.dhl_express = lower.contains("dhl express") || (m.dhl && lower.contains("waybill"));
        m
    }

    fn add_link(&mut self, carrier: Carrier) {
        match carrier {
            Carrier::Dhl => self.dhl = true,
            Carrier::DhlExpress => self.dhl_express = true,
            Carrier::Dpd => self.dpd = true,
            Carrier::Hermes => self.hermes = true,
            Carrier::Gls => self.gls = true,
            Carrier::FedEx => self.fedex = true,
            Carrier::Usps => self.usps = true,
            Carrier::DeutschePostBrief => self.dp_brief = true,
            Carrier::Ups => {}
        }
    }
}

/// Finds shipment tracking numbers in a message's subject and plain-text body.
pub fn find_shipments(subject: &str, body_text: &str) -> Vec<Shipment> {
    find_shipments_with_links(subject, body_text, &[])
}

/// Like [`find_shipments`], with additional link targets (e.g. the `href`s of
/// an HTML body, which never appear in the plain text).
pub fn find_shipments_with_links(subject: &str, body_text: &str, links: &[&str]) -> Vec<Shipment> {
    let mut seen: HashSet<(Carrier, String)> = HashSet::new();
    let mut out = Vec::new();

    let combined = format!("{subject}\n{body_text}");

    // 1. Links: numbers in carrier links, and which carriers are linked at all.
    let mut urls: Vec<String> = RE_URL
        .find_iter(&combined)
        .map(|m| m.as_str().to_string())
        .collect();
    urls.extend(links.iter().map(|l| l.to_string()));
    let mut mentions = Mentions::from_text(&combined);
    for url in &urls {
        for target in unwrap_redirects(url) {
            if let Some(carrier) = link_carrier(&target) {
                mentions.add_link(carrier);
            }
            extract_from_url(&target, &mut seen, &mut out);
        }
    }

    // 2. Text: check-digit formats and context-backed numbers.
    let lines: Vec<&str> = combined.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let prev = idx.checked_sub(1).map(|i| lines[i]).unwrap_or("");
        let window = format!("{prev}\n{line}");
        let has_tracking_kw = RE_GENERIC_TRACKING_CTX.is_match(&window);

        for cap in RE_UPS_1Z.captures_iter(line) {
            let num = cap[1].to_ascii_uppercase();
            if is_valid_ups_1z(&num) {
                push(Carrier::Ups, num, Evidence::CheckDigit, &mut seen, &mut out);
            }
        }

        for cap in RE_UPU_S10_DE.captures_iter(line) {
            let num = cap[1].to_ascii_uppercase();
            if is_valid_upu_s10(&num) {
                let carrier = if mentions.dhl && !mentions.dp_brief {
                    Carrier::Dhl
                } else {
                    Carrier::DeutschePostBrief
                };
                push(carrier, num, Evidence::CheckDigit, &mut seen, &mut out);
            }
        }

        for cap in RE_USPS_IMPB.captures_iter(line) {
            let m = cap.get(1).unwrap();
            if is_valid_gs1_mod10(m.as_str()) && !is_labelled_as_other(line, m.start()) {
                push(
                    Carrier::Usps,
                    m.as_str().to_string(),
                    Evidence::CheckDigit,
                    &mut seen,
                    &mut out,
                );
            }
        }

        if mentions.dhl || has_tracking_kw {
            for cap in RE_DHL_JJD.captures_iter(line) {
                push(
                    Carrier::Dhl,
                    cap[1].to_ascii_uppercase(),
                    Evidence::Context,
                    &mut seen,
                    &mut out,
                );
            }
        }

        if mentions.dp_brief || has_tracking_kw {
            for cap in RE_DEUTSCHE_POST_MATRIX.captures_iter(line) {
                push(
                    Carrier::DeutschePostBrief,
                    cap[1].to_ascii_uppercase(),
                    Evidence::Context,
                    &mut seen,
                    &mut out,
                );
            }
        }

        for cap in RE_TOKEN_DIGITS.captures_iter(line) {
            let m = cap.get(1).unwrap();
            let num = m.as_str();
            if is_inside_url(line, m.start()) || is_labelled_as_other(line, m.start()) {
                continue;
            }
            if let Some((carrier, evidence)) =
                classify_number(num, &mentions, has_tracking_kw, &window)
            {
                push(carrier, num.to_string(), evidence, &mut seen, &mut out);
            }
        }
    }

    out
}

/// Decides which carrier a bare number belongs to, if any.
fn classify_number(
    num: &str,
    m: &Mentions,
    has_tracking_kw: bool,
    window: &str,
) -> Option<(Carrier, Evidence)> {
    let len = num.len();
    let sscc = len == 20 && num.starts_with("00") && is_valid_gs1_mod10(num);

    if (m.dhl_express || (m.dhl && contains_ci(window, "express")))
        && len == 10
        && is_valid_dhl_express_mod7(num)
    {
        return Some((Carrier::DhlExpress, Evidence::CheckDigit));
    }
    if m.dhl && (sscc || (len == 12 && is_valid_gs1_mod10(num))) {
        return Some((Carrier::Dhl, Evidence::CheckDigit));
    }
    if m.usps && matches!(len, 20 | 22) && is_valid_gs1_mod10(num) {
        return Some((Carrier::Usps, Evidence::CheckDigit));
    }
    if has_tracking_kw && sscc {
        // 00 + 18-digit SSCC with valid GS1 check digit is standard DHL Paket in Germany.
        return Some((Carrier::Dhl, Evidence::CheckDigit));
    }

    // No check digit: a carrier mention alone is not enough, since shop mails
    // name the carrier next to order and invoice numbers. A tracking keyword
    // next to the number is.
    if !has_tracking_kw {
        return None;
    }
    if m.dpd && len == 14 {
        Some((Carrier::Dpd, Evidence::Context))
    } else if m.hermes && matches!(len, 12 | 14 | 16) {
        Some((Carrier::Hermes, Evidence::Context))
    } else if m.gls && matches!(len, 11 | 12) {
        Some((Carrier::Gls, Evidence::Context))
    } else if m.fedex && matches!(len, 12 | 15 | 20) {
        Some((Carrier::FedEx, Evidence::Context))
    } else {
        None
    }
}

/// True if the nearest label before `pos` on this line marks the number as an
/// order, invoice, customer, phone or similar number.
fn is_labelled_as_other(line: &str, pos: usize) -> bool {
    let prefix = &line[..pos];
    let Some(other) = RE_NOT_TRACKING_LABEL.find_iter(prefix).last() else {
        return false;
    };
    // A tracking keyword after the other label wins ("Bestellung 123 – Sendungsnummer 456").
    !RE_GENERIC_TRACKING_CTX.is_match(&prefix[other.end()..])
}

/// The link itself plus any target URL embedded in redirect parameters
/// (newsletter click trackers). Redirects are decoded, never followed: opening
/// them would tell the sender the mail was read.
fn unwrap_redirects(url: &str) -> Vec<String> {
    let mut out = vec![url.trim_end_matches(['.', ',', ';']).to_string()];
    let mut current = out[0].clone();
    for _ in 0..3 {
        let Some(inner) = embedded_url(&current) else {
            break;
        };
        out.push(inner.clone());
        current = inner;
    }
    out
}

fn embedded_url(url: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    for pair in query.split('&') {
        let Some((_, value)) = pair.split_once('=') else {
            continue;
        };
        let decoded = percent_decode(value);
        if decoded.starts_with("http://") || decoded.starts_with("https://") {
            return Some(decoded);
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        (b as char).to_digit(16).map(|d| d as u8)
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Host of a URL, lower-cased, without credentials or port.
fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?.split(':').next()?;
    Some(host.to_ascii_lowercase())
}

fn host_is(url: &str, domains: &[&str]) -> bool {
    host(url).is_some_and(|h| {
        domains
            .iter()
            .any(|d| h == *d || h.ends_with(&format!(".{d}")))
    })
}

fn link_carrier(url: &str) -> Option<Carrier> {
    Some(if host_is(url, &["ups.com"]) {
        Carrier::Ups
    } else if host_is(url, &["dhl.de", "dhl.com"]) {
        if url.to_ascii_lowercase().contains("express") {
            Carrier::DhlExpress
        } else {
            Carrier::Dhl
        }
    } else if host_is(url, &["dpd.de", "dpd.com"]) {
        Carrier::Dpd
    } else if host_is(url, &["myhermes.de", "hermesworld.com", "hermes-europe.de"]) {
        Carrier::Hermes
    } else if host_is(url, &["gls-pakete.de", "gls-group.com", "gls-group.eu"]) {
        Carrier::Gls
    } else if host_is(url, &["fedex.com"]) {
        Carrier::FedEx
    } else if host_is(url, &["usps.com"]) {
        Carrier::Usps
    } else if host_is(url, &["deutschepost.de"]) {
        Carrier::DeutschePostBrief
    } else {
        return None;
    })
}

fn extract_from_url(url: &str, seen: &mut HashSet<(Carrier, String)>, out: &mut Vec<Shipment>) {
    {
        let lower = url.to_ascii_lowercase();

        if host_is(url, &["ups.com"]) {
            if let Some(val) = query_param(url, &["tracknum", "inquirynumber1"]) {
                let num = val.to_ascii_uppercase();
                if num.starts_with("1Z") && is_valid_ups_1z(&num) {
                    push_link(Carrier::Ups, num, seen, out);
                }
            }
        } else if host_is(url, &["dhl.de", "dhl.com"]) {
            if let Some(val) = query_param(url, &["piececode", "tracking-id", "idc"]) {
                let num = val.to_ascii_uppercase();
                if lower.contains("tracking-express")
                    && num.len() == 10
                    && is_valid_dhl_express_mod7(&num)
                {
                    push_link(Carrier::DhlExpress, num, seen, out);
                } else if is_valid_dhl_url_number(&num) {
                    push_link(Carrier::Dhl, num, seen, out);
                }
            }
        } else if host_is(url, &["dpd.de", "dpd.com"]) {
            if let Some(val) = query_param(url, &["parcelno", "query"])
                .or_else(|| path_segment_after(url, "parcel"))
                && val.len() == 14
                && val.chars().all(|c| c.is_ascii_alphanumeric())
            {
                push_link(Carrier::Dpd, val.to_ascii_uppercase(), seen, out);
            }
        } else if host_is(url, &["myhermes.de", "hermesworld.com", "hermes-europe.de"]) {
            if let Some(val) = query_param(url, &["trackingid", "shipmentid"])
                .or_else(|| url.split('#').nth(1).map(str::to_string))
                && matches!(val.len(), 11..=16)
                && val.chars().all(|c| c.is_ascii_digit())
            {
                push_link(Carrier::Hermes, val, seen, out);
            }
        } else if host_is(url, &["gls-pakete.de", "gls-group.com", "gls-group.eu"]) {
            if let Some(val) = query_param(url, &["match", "tracking"])
                && matches!(val.len(), 8..=12)
                && val.chars().all(|c| c.is_ascii_alphanumeric())
            {
                push_link(Carrier::Gls, val.to_ascii_uppercase(), seen, out);
            }
        } else if host_is(url, &["fedex.com"]) {
            if let Some(val) = query_param(url, &["trknbr", "tracknumbers"])
                && matches!(val.len(), 12 | 15 | 20)
                && val.chars().all(|c| c.is_ascii_digit())
            {
                push_link(Carrier::FedEx, val, seen, out);
            }
        } else if host_is(url, &["usps.com"]) {
            if let Some(val) = query_param(url, &["tlabels"])
                && matches!(val.len(), 20 | 22)
                && is_valid_gs1_mod10(&val)
            {
                push_link(Carrier::Usps, val, seen, out);
            }
        } else if host_is(url, &["deutschepost.de"])
            && let Some(val) = query_param(url, &["piececode"])
        {
            let num = val.to_ascii_uppercase();
            if is_valid_upu_s10(&num) || RE_DEUTSCHE_POST_MATRIX.is_match(&num) {
                push_link(Carrier::DeutschePostBrief, num, seen, out);
            }
        }
    }
}

fn is_valid_dhl_url_number(num: &str) -> bool {
    if num.starts_with("JJD") && (14..=21).contains(&num.len()) {
        return true;
    }
    if is_valid_upu_s10(num) {
        return true;
    }
    if ((num.len() == 20 && num.starts_with("00")) || num.len() == 12) && is_valid_gs1_mod10(num) {
        return true;
    }
    false
}

fn query_param(url: &str, names: &[&str]) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    let query = query.split('#').next().unwrap_or(query);
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if names.iter().any(|name| k.eq_ignore_ascii_case(name)) && !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

fn path_segment_after(url: &str, marker: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let segments: Vec<&str> = path.split('/').collect();
    segments
        .windows(2)
        .find(|w| w[0].eq_ignore_ascii_case(marker))
        .map(|w| w[1].to_string())
        .filter(|s| !s.is_empty())
}

fn is_inside_url(line: &str, pos: usize) -> bool {
    RE_URL
        .find_iter(line)
        .any(|m| pos >= m.start() && pos < m.end())
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle)
}

fn push_link(
    carrier: Carrier,
    tracking_number: String,
    seen: &mut HashSet<(Carrier, String)>,
    out: &mut Vec<Shipment>,
) {
    push(carrier, tracking_number, Evidence::CarrierLink, seen, out);
}

fn push(
    carrier: Carrier,
    tracking_number: String,
    evidence: Evidence,
    seen: &mut HashSet<(Carrier, String)>,
    out: &mut Vec<Shipment>,
) {
    if seen.insert((carrier, tracking_number.clone())) {
        let url = tracking_url(carrier, &tracking_number);
        out.push(Shipment {
            carrier,
            tracking_number,
            url,
            evidence,
        });
    }
}

fn tracking_url(carrier: Carrier, num: &str) -> String {
    match carrier {
        Carrier::Dhl => {
            format!(
                "https://www.dhl.de/de/privatkunden/pakete-empfangen/verfolgen.html?piececode={num}"
            )
        }
        Carrier::DhlExpress => {
            format!(
                "https://www.dhl.com/de-de/home/tracking/tracking-express.html?submit=1&tracking-id={num}"
            )
        }
        Carrier::Dpd => format!("https://tracking.dpd.de/status/de_DE/parcel/{num}"),
        Carrier::Hermes => {
            format!(
                "https://www.myhermes.de/empfangen/sendungsverfolgung/sendungsinformation#{num}"
            )
        }
        Carrier::Gls => format!("https://www.gls-pakete.de/sendungsverfolgung?match={num}"),
        Carrier::Ups => format!("https://www.ups.com/track?tracknum={num}"),
        Carrier::FedEx => format!("https://www.fedex.com/fedextrack/?trknbr={num}"),
        Carrier::Usps => format!("https://tools.usps.com/go/TrackConfirmAction?tLabels={num}"),
        Carrier::DeutschePostBrief => {
            format!("https://www.deutschepost.de/de/s/sendungsverfolgung.html?piececode={num}")
        }
    }
}

/// UPS 1Z check-digit validation (UPS 1Z Mod-10 specification):
/// Strip "1Z", map letters A..Z via `(ascii - 'A' + 2) % 10`, sum odd
/// positions (1-based) * 1 + even positions * 2 over the 15 payload chars,
/// check digit is `(10 - (sum % 10)) % 10`.
fn is_valid_ups_1z(num: &str) -> bool {
    let bytes = num.as_bytes();
    if bytes.len() != 18 || !bytes.starts_with(b"1Z") {
        return false;
    }
    let Some(expected) = (bytes[17] as char).to_digit(10) else {
        return false;
    };

    let mut sum = 0u32;
    for (idx, &b) in bytes[2..17].iter().enumerate() {
        let val = match b {
            b'0'..=b'9' => (b - b'0') as u32,
            b'A'..=b'Z' => ((b - b'A' + 2) % 10) as u32,
            _ => return false,
        };
        let weight = if idx % 2 == 0 { 1 } else { 2 };
        sum += val * weight;
    }

    ((10 - (sum % 10)) % 10) == expected
}

/// GS1 / SSCC / USPS IMPB Mod-10 check-digit validation
/// (GS1 General Specifications Section 7.9 & USPS Publication 199):
/// Moving right-to-left over payload digits (excluding the trailing check digit),
/// multiply alternating digits by weights 3 and 1; check digit is `(10 - (sum % 10)) % 10`.
fn is_valid_gs1_mod10(num: &str) -> bool {
    let bytes = num.as_bytes();
    if bytes.len() < 2 || !bytes.iter().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let expected = (bytes[bytes.len() - 1] - b'0') as u32;
    let payload = &bytes[..bytes.len() - 1];

    let sum: u32 = payload
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &b)| {
            let weight = if i % 2 == 0 { 3 } else { 1 };
            ((b - b'0') as u32) * weight
        })
        .sum();

    ((10 - (sum % 10)) % 10) == expected
}

/// DHL Express 10-digit Air Waybill check digit (IATA / DHL Express Mod-7 rule):
/// The 10th digit equals `(first 9 digits as integer) % 7`.
fn is_valid_dhl_express_mod7(num: &str) -> bool {
    if num.len() != 10 || !num.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Ok(prefix) = num[..9].parse::<u64>() else {
        return false;
    };
    let check = (num.as_bytes()[9] - b'0') as u64;
    prefix % 7 == check
}

/// UPU Standard S10 check-digit validation (2-letter service code + 8 digits + check digit + 2-letter country):
/// Weights `[8, 6, 4, 2, 3, 5, 9, 7]` on the 8 serial digits; `C = 11 - (sum % 11)`,
/// mapping 10 -> 0 and 11 -> 5.
fn is_valid_upu_s10(num: &str) -> bool {
    const WEIGHTS: [u32; 8] = [8, 6, 4, 2, 3, 5, 9, 7];
    let bytes = num.as_bytes();
    if bytes.len() != 13 {
        return false;
    }
    if !bytes[0..2].iter().all(|b| b.is_ascii_uppercase())
        || !bytes[2..11].iter().all(|b| b.is_ascii_digit())
        || !bytes[11..13].iter().all(|b| b.is_ascii_uppercase())
    {
        return false;
    }

    let sum: u32 = bytes[2..10]
        .iter()
        .zip(WEIGHTS)
        .map(|(&b, w)| ((b - b'0') as u32) * w)
        .sum();
    let check = match 11 - (sum % 11) {
        10 => 0,
        11 => 5,
        c => c,
    };
    check == (bytes[10] - b'0') as u32
}

#[cfg(test)]
mod tests {
    use super::{Carrier, Evidence, find_shipments, find_shipments_with_links};

    /// Helper to append a valid GS1 Mod-10 check digit to a numeric prefix.
    fn with_gs1_check(prefix: &str) -> String {
        let sum: u32 = prefix
            .as_bytes()
            .iter()
            .rev()
            .enumerate()
            .map(|(i, &b)| ((b - b'0') as u32) * if i % 2 == 0 { 3 } else { 1 })
            .sum();
        let check = (10 - (sum % 10)) % 10;
        format!("{prefix}{check}")
    }

    /// Helper to compute a valid UPS 1Z tracking number from a 15-char payload.
    fn make_ups_1z(payload15: &str) -> String {
        let sum: u32 = payload15
            .as_bytes()
            .iter()
            .enumerate()
            .map(|(idx, &b)| {
                let val = match b {
                    b'0'..=b'9' => (b - b'0') as u32,
                    b'A'..=b'Z' => ((b - b'A' + 2) % 10) as u32,
                    _ => panic!("invalid char"),
                };
                val * if idx % 2 == 0 { 1 } else { 2 }
            })
            .sum();
        let check = (10 - (sum % 10)) % 10;
        format!("1Z{payload15}{check}")
    }

    #[test]
    fn extracts_ups_1z_without_context() {
        let num = make_ups_1z("999AA1012345678");
        let shipments = find_shipments("Lieferung", &format!("Nummer: {num}"));
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Ups);
        assert_eq!(shipments[0].tracking_number, num);
        assert!(shipments[0].url.contains(&num));
    }

    #[test]
    fn extracts_dhl_20_digit_with_context() {
        let num = with_gs1_check("0034043416109401234");
        let shipments = find_shipments(
            "Ihr DHL Paket ist unterwegs",
            &format!("Sendungsnummer: {num}"),
        );
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Dhl);
        assert_eq!(shipments[0].tracking_number, num);
    }

    #[test]
    fn extracts_dhl_12_digit_with_context() {
        let num = with_gs1_check("35712345678");
        let shipments = find_shipments("DHL Sendung", &format!("Paketnummer: {num}"));
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Dhl);
        assert_eq!(shipments[0].tracking_number, num);
    }

    #[test]
    fn extracts_dhl_jjd_number() {
        let shipments = find_shipments(
            "Versandbestätigung",
            "Ihre DHL Sendungsnummer lautet JJD141020304050.",
        );
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Dhl);
        assert_eq!(shipments[0].tracking_number, "JJD141020304050");
    }

    #[test]
    fn extracts_dhl_express_10_digit_mod7() {
        let prefix = 123456789u64;
        let check = prefix % 7;
        let num = format!("{prefix}{check}");
        let shipments = find_shipments("DHL Express Lieferung", &format!("Waybill number: {num}"));
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::DhlExpress);
        assert_eq!(shipments[0].tracking_number, num);
    }

    #[test]
    fn extracts_dpd_14_digit_with_context() {
        let shipments = find_shipments(
            "Ihr DPD Paket",
            "Mit der Paketnummer 01501234567890 können Sie Ihr Paket verfolgen.",
        );
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Dpd);
        assert_eq!(shipments[0].tracking_number, "01501234567890");
    }

    #[test]
    fn extracts_hermes_with_context() {
        let shipments = find_shipments(
            "Hermes Sendungsankündigung",
            "Ihre Sendungs-ID: 72104810293841",
        );
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Hermes);
        assert_eq!(shipments[0].tracking_number, "72104810293841");
    }

    #[test]
    fn extracts_gls_with_context() {
        let shipments = find_shipments("GLS Paketdienst", "Tracking number: 40123456789");
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Gls);
        assert_eq!(shipments[0].tracking_number, "40123456789");
    }

    #[test]
    fn extracts_fedex_with_context() {
        let shipments = find_shipments("FedEx Shipment", "Your tracking number is 781234567890.");
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::FedEx);
        assert_eq!(shipments[0].tracking_number, "781234567890");
    }

    #[test]
    fn extracts_usps_impb_with_valid_check_digit() {
        let num = with_gs1_check("940011189922334455667");
        let shipments = find_shipments("Package shipped", &format!("Tracking: {num}"));
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::Usps);
        assert_eq!(shipments[0].tracking_number, num);
    }

    #[test]
    fn extracts_deutsche_post_upu_s10_brief() {
        // S = 1*8 + 2*6 + 3*4 + 4*2 + 5*3 + 6*5 + 7*9 + 8*7 = 8+12+12+8+15+30+63+56 = 204
        // 204 % 11 = 6; 11 - 6 = 5
        let num = "RT123456785DE";
        let shipments = find_shipments("Einschreiben", &format!("Sendungsnummer: {num}"));
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].carrier, Carrier::DeutschePostBrief);
        assert_eq!(shipments[0].tracking_number, num);
    }

    #[test]
    fn extracts_tracking_numbers_from_urls() {
        let dhl_num = with_gs1_check("0034043416109401234");
        let body = format!(
            "Hier verfolgen: https://www.dhl.de/de/privatkunden/pakete-empfangen/verfolgen.html?piececode={dhl_num}\n\
             Und DPD: https://tracking.dpd.de/status/de_DE/parcel/01501234567890"
        );
        let shipments = find_shipments("Ihre Sendungen", &body);
        assert_eq!(shipments.len(), 2);
        assert_eq!(shipments[0].carrier, Carrier::Dhl);
        assert_eq!(shipments[1].carrier, Carrier::Dpd);
    }

    #[test]
    fn deduplicates_url_and_text_occurrence() {
        let num = make_ups_1z("12345E020527168");
        let body = format!("UPS Tracking: {num}\nLink: https://www.ups.com/track?tracknum={num}");
        let shipments = find_shipments("UPS", &body);
        assert_eq!(shipments.len(), 1);
        assert_eq!(shipments[0].tracking_number, num);
    }

    // --- Negative test cases (must NOT be recognized) ---

    #[test]
    fn rejects_ups_1z_with_invalid_check_digit() {
        let valid = make_ups_1z("999AA1012345678");
        let last = valid.chars().last().unwrap();
        let wrong_digit = if last == '0' { '1' } else { '0' };
        let invalid = format!("{}{}", &valid[..17], wrong_digit);
        assert!(find_shipments("UPS", &format!("Tracking: {invalid}")).is_empty());
    }

    #[test]
    fn rejects_dhl_20_digit_with_invalid_check_digit() {
        let valid = with_gs1_check("0034043416109401234");
        let last = valid.chars().last().unwrap();
        let wrong_digit = if last == '0' { '1' } else { '0' };
        let invalid = format!("{}{}", &valid[..19], wrong_digit);
        assert!(find_shipments("DHL Paket", &format!("Sendungsnummer: {invalid}")).is_empty());
    }

    #[test]
    fn rejects_dhl_express_with_invalid_mod7_check_digit() {
        let prefix = 123456789u64;
        let wrong = (prefix % 7 + 1) % 7;
        let invalid = format!("{prefix}{wrong}");
        assert!(find_shipments("DHL Express", &format!("Waybill: {invalid}")).is_empty());
    }

    #[test]
    fn rejects_usps_with_invalid_check_digit() {
        let valid = with_gs1_check("940011189922334455667");
        let last = valid.chars().last().unwrap();
        let wrong_digit = if last == '0' { '1' } else { '0' };
        let invalid = format!("{}{}", &valid[..21], wrong_digit);
        assert!(find_shipments("USPS", &format!("Tracking: {invalid}")).is_empty());
    }

    #[test]
    fn rejects_upu_s10_with_invalid_check_digit() {
        assert!(find_shipments("Deutsche Post", "Sendung: RT123456789DE").is_empty());
    }

    #[test]
    fn rejects_random_12_digit_number_without_carrier_context() {
        let num = with_gs1_check("35712345678");
        assert!(find_shipments("Rechnung", &format!("Kundennummer: {num}")).is_empty());
    }

    #[test]
    fn rejects_phone_numbers_and_iban_digits() {
        let body = "Telefon: 00493012345678\nKonto: 12345678901234";
        assert!(find_shipments("Kontakt", body).is_empty());
    }

    // --- Review additions: links as evidence, order/invoice numbers ---

    #[test]
    fn order_and_invoice_numbers_are_not_shipments() {
        assert!(
            find_shipments(
                "Ihre Bestellung",
                "Ihre Bestellung 123456789012 wird mit Hermes versendet."
            )
            .is_empty()
        );
        assert!(
            find_shipments(
                "Versand",
                "Rechnungsnummer 12345678901234, Versand mit DPD."
            )
            .is_empty()
        );
        assert!(find_shipments("FedEx", "Order number 781234567890 shipped via FedEx.").is_empty());
    }

    #[test]
    fn tracking_keyword_after_order_label_wins() {
        let s = find_shipments(
            "Versandbestätigung",
            "Bestellung 4711 – Hermes Sendungsnummer: 72104810293841",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].carrier, Carrier::Hermes);
        assert_eq!(s[0].evidence, Evidence::Context);
    }

    #[test]
    fn carrier_name_alone_is_not_enough_without_check_digit() {
        assert!(
            find_shipments("Ihr Paket", "Wir versenden mit DPD. Nr. 01501234567890").is_empty()
        );
    }

    #[test]
    fn carrier_words_need_word_boundaries() {
        // "ups" in "Backups", "gls" in nothing, "prio" in "Priorität".
        assert!(find_shipments("Backups", "Your backups 1234567890123 are done.").is_empty());
        let s = find_shipments("Priorität hoch", "Einschreiben RT123456785DE");
        assert_eq!(s[0].carrier, Carrier::DeutschePostBrief);
    }

    #[test]
    fn link_is_strongest_evidence() {
        let s = find_shipments(
            "Ihre Bestellung 2026-0417",
            "Bestellnummer 123456789012\n\
             Paket verfolgen: https://tracking.dpd.de/status/de_DE/parcel/01501234567890",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].carrier, Carrier::Dpd);
        assert_eq!(s[0].tracking_number, "01501234567890");
        assert_eq!(s[0].evidence, Evidence::CarrierLink);
    }

    #[test]
    fn decodes_click_tracker_redirects_without_following_them() {
        let target = "https%3A%2F%2Fwww.myhermes.de%2Fempfangen%2Fsendungsverfolgung%3FtrackingId%3D72104810293841";
        let body = format!(
            "Sendung verfolgen: https://click.mailer.example/ls/click?upn=abc&url={target}"
        );
        let s = find_shipments("Dein Paket kommt", &body);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].carrier, Carrier::Hermes);
        assert_eq!(s[0].evidence, Evidence::CarrierLink);
    }

    #[test]
    fn html_links_count_like_text_links() {
        let s = find_shipments_with_links(
            "Versandbestätigung",
            "Klicken Sie auf „Sendung verfolgen“.",
            &[
                "https://www.dhl.de/de/privatkunden/pakete-empfangen/verfolgen.html?piececode=JJD141020304050",
            ],
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].carrier, Carrier::Dhl);
        assert_eq!(s[0].evidence, Evidence::CarrierLink);
    }

    #[test]
    fn lookalike_domains_are_not_carrier_links() {
        let s = find_shipments(
            "Paket",
            "https://dhl.de.example/verfolgen?piececode=JJD141020304050",
        );
        assert!(s.iter().all(|x| x.evidence != Evidence::CarrierLink));
    }

    #[test]
    fn broken_percent_escapes_do_not_panic() {
        let _ = find_shipments("x", "https://click.example/?url=%E4%%zz%ä");
    }
}
