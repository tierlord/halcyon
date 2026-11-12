//! Extractor for parcel and letter tracking numbers and public tracking URLs.

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

/// A detected shipment tracking number with its carrier and public tracking URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shipment {
    pub carrier: Carrier,
    pub tracking_number: String,
    /// Public tracking page for this number.
    pub url: String,
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
    Regex::new(r"(?i)\b(sendungsnummer|sendungs-?id|paketnummer|tracking(\s+number|\s*id)?|verfolgung|sendungsverfolgung|waybill|awb)\b").unwrap()
});

/// Finds shipment tracking numbers in a message's subject and plain-text body.
///
/// Unambiguous formats (such as UPS `1Z...`, UPU S10 `...DE`, and USPS `9[2-5]...`)
/// and known carrier tracking URLs are recognized without extra context; all other
/// numeric formats require a carrier name or tracking keyword nearby.
pub fn find_shipments(subject: &str, body_text: &str) -> Vec<Shipment> {
    let mut seen: HashSet<(Carrier, String)> = HashSet::new();
    let mut out = Vec::new();

    let combined = format!("{subject}\n{body_text}");

    // 1. Extract from explicit tracking URLs first.
    extract_from_urls(&combined, &mut seen, &mut out);

    // 2. Scan lines for unambiguous formats and context-backed numbers.
    let lines: Vec<&str> = combined.lines().collect();
    let doc_has_dhl = contains_ci(&combined, "dhl");
    let doc_has_dhl_express =
        contains_ci(&combined, "dhl express") || (doc_has_dhl && contains_ci(&combined, "waybill"));
    let doc_has_dpd = contains_ci(&combined, "dpd");
    let doc_has_hermes = contains_ci(&combined, "hermes");
    let doc_has_gls = contains_ci(&combined, "gls");
    let doc_has_ups = contains_ci(&combined, "ups");
    let doc_has_fedex =
        contains_ci(&combined, "fedex") || contains_ci(&combined, "federal express");
    let doc_has_usps = contains_ci(&combined, "usps");
    let doc_has_dp_brief = contains_ci(&combined, "deutsche post")
        || contains_ci(&combined, "einschreiben")
        || contains_ci(&combined, "prio");

    for (idx, line) in lines.iter().enumerate() {
        let prev = idx.checked_sub(1).map(|i| lines[i]).unwrap_or("");
        let window = format!("{prev}\n{line}");
        let has_tracking_kw = RE_GENERIC_TRACKING_CTX.is_match(&window);

        // Unambiguous: UPS 1Z
        for cap in RE_UPS_1Z.captures_iter(line) {
            let num = cap[1].to_ascii_uppercase();
            if is_valid_ups_1z(&num) {
                push_shipment(Carrier::Ups, num, &mut seen, &mut out);
            }
        }

        // Unambiguous: UPU S10 (...DE)
        for cap in RE_UPU_S10_DE.captures_iter(line) {
            let num = cap[1].to_ascii_uppercase();
            if is_valid_upu_s10(&num) {
                let carrier = if doc_has_dhl && !doc_has_dp_brief {
                    Carrier::Dhl
                } else {
                    Carrier::DeutschePostBrief
                };
                push_shipment(carrier, num, &mut seen, &mut out);
            }
        }

        // Unambiguous: USPS 20/22-digit IMPB starting with 92/93/94/95 (with Mod-10 check digit)
        for cap in RE_USPS_IMPB.captures_iter(line) {
            let num = cap[1].to_string();
            if is_valid_gs1_mod10(&num) {
                push_shipment(Carrier::Usps, num, &mut seen, &mut out);
            }
        }

        // Context-backed: DHL JJD...
        if doc_has_dhl || has_tracking_kw {
            for cap in RE_DHL_JJD.captures_iter(line) {
                push_shipment(
                    Carrier::Dhl,
                    cap[1].to_ascii_uppercase(),
                    &mut seen,
                    &mut out,
                );
            }
        }

        // Context-backed: Deutsche Post Matrixcode (A00... / F00...)
        if doc_has_dp_brief || has_tracking_kw {
            for cap in RE_DEUTSCHE_POST_MATRIX.captures_iter(line) {
                push_shipment(
                    Carrier::DeutschePostBrief,
                    cap[1].to_ascii_uppercase(),
                    &mut seen,
                    &mut out,
                );
            }
        }

        // Context-backed numeric formats
        if !(has_tracking_kw
            || doc_has_dhl
            || doc_has_dpd
            || doc_has_hermes
            || doc_has_gls
            || doc_has_ups
            || doc_has_fedex
            || doc_has_usps)
        {
            continue;
        }

        for cap in RE_TOKEN_DIGITS.captures_iter(line) {
            let num = &cap[1];
            // Skip numbers inside URLs (already handled above)
            if is_inside_url(line, cap.get(1).unwrap().start()) {
                continue;
            }

            // Determine best matching carrier based on context + format + check digit.
            if (doc_has_dhl_express || (doc_has_dhl && contains_ci(&window, "express")))
                && num.len() == 10
                && is_valid_dhl_express_mod7(num)
            {
                push_shipment(Carrier::DhlExpress, num.to_string(), &mut seen, &mut out);
            } else if doc_has_dhl
                && ((num.len() == 20 && num.starts_with("00")) || num.len() == 12)
                && is_valid_gs1_mod10(num)
            {
                push_shipment(Carrier::Dhl, num.to_string(), &mut seen, &mut out);
            } else if doc_has_dpd && num.len() == 14 {
                push_shipment(Carrier::Dpd, num.to_string(), &mut seen, &mut out);
            } else if doc_has_hermes && matches!(num.len(), 12 | 14 | 16) {
                push_shipment(Carrier::Hermes, num.to_string(), &mut seen, &mut out);
            } else if doc_has_gls && matches!(num.len(), 11 | 12) {
                push_shipment(Carrier::Gls, num.to_string(), &mut seen, &mut out);
            } else if doc_has_fedex && matches!(num.len(), 12 | 15 | 20) {
                push_shipment(Carrier::FedEx, num.to_string(), &mut seen, &mut out);
            } else if doc_has_usps && matches!(num.len(), 20 | 22) && is_valid_gs1_mod10(num) {
                push_shipment(Carrier::Usps, num.to_string(), &mut seen, &mut out);
            } else if has_tracking_kw
                && num.len() == 20
                && num.starts_with("00")
                && is_valid_gs1_mod10(num)
            {
                // 00 + 18-digit SSCC with valid GS1 check digit is standard DHL Paket in Germany
                push_shipment(Carrier::Dhl, num.to_string(), &mut seen, &mut out);
            }
        }
    }

    out
}

fn extract_from_urls(text: &str, seen: &mut HashSet<(Carrier, String)>, out: &mut Vec<Shipment>) {
    for m in RE_URL.find_iter(text) {
        let url = m.as_str().trim_end_matches(['.', ',', ';']);
        let lower = url.to_ascii_lowercase();

        if lower.contains("ups.com") {
            if let Some(val) = query_param(url, &["tracknum", "inquirynumber1"]) {
                let num = val.to_ascii_uppercase();
                if num.starts_with("1Z") && is_valid_ups_1z(&num) {
                    push_shipment(Carrier::Ups, num, seen, out);
                }
            }
        } else if lower.contains("dhl.de") || lower.contains("dhl.com") {
            if let Some(val) = query_param(url, &["piececode", "tracking-id", "idc"]) {
                let num = val.to_ascii_uppercase();
                if lower.contains("tracking-express")
                    && num.len() == 10
                    && is_valid_dhl_express_mod7(&num)
                {
                    push_shipment(Carrier::DhlExpress, num, seen, out);
                } else if is_valid_dhl_url_number(&num) {
                    push_shipment(Carrier::Dhl, num, seen, out);
                }
            }
        } else if lower.contains("dpd.de") || lower.contains("dpd.com") {
            if let Some(val) = query_param(url, &["parcelno", "query"])
                .or_else(|| path_segment_after(url, "parcel"))
                && val.len() == 14
                && val.chars().all(|c| c.is_ascii_alphanumeric())
            {
                push_shipment(Carrier::Dpd, val.to_ascii_uppercase(), seen, out);
            }
        } else if lower.contains("myhermes.de") || lower.contains("hermesworld.com") {
            if let Some(val) = query_param(url, &["trackingid", "shipmentid"])
                .or_else(|| url.split('#').nth(1).map(str::to_string))
                && matches!(val.len(), 11..=16)
                && val.chars().all(|c| c.is_ascii_digit())
            {
                push_shipment(Carrier::Hermes, val, seen, out);
            }
        } else if lower.contains("gls-pakete.de") || lower.contains("gls-group.") {
            if let Some(val) = query_param(url, &["match", "tracking"])
                && matches!(val.len(), 8..=12)
                && val.chars().all(|c| c.is_ascii_alphanumeric())
            {
                push_shipment(Carrier::Gls, val.to_ascii_uppercase(), seen, out);
            }
        } else if lower.contains("fedex.com") {
            if let Some(val) = query_param(url, &["trknbr", "tracknumbers"])
                && matches!(val.len(), 12 | 15 | 20)
                && val.chars().all(|c| c.is_ascii_digit())
            {
                push_shipment(Carrier::FedEx, val, seen, out);
            }
        } else if lower.contains("usps.com") {
            if let Some(val) = query_param(url, &["tlabels"])
                && matches!(val.len(), 20 | 22)
                && is_valid_gs1_mod10(&val)
            {
                push_shipment(Carrier::Usps, val, seen, out);
            }
        } else if lower.contains("deutschepost.de")
            && let Some(val) = query_param(url, &["piececode"])
        {
            let num = val.to_ascii_uppercase();
            if is_valid_upu_s10(&num) || RE_DEUTSCHE_POST_MATRIX.is_match(&num) {
                push_shipment(Carrier::DeutschePostBrief, num, seen, out);
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

fn push_shipment(
    carrier: Carrier,
    tracking_number: String,
    seen: &mut HashSet<(Carrier, String)>,
    out: &mut Vec<Shipment>,
) {
    if seen.insert((carrier, tracking_number.clone())) {
        let url = tracking_url(carrier, &tracking_number);
        out.push(Shipment {
            carrier,
            tracking_number,
            url,
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
    use super::{Carrier, find_shipments};

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
}
