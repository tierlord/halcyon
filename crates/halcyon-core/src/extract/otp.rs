//! Extractor for one-time passwords and verification codes (2FA / TAN / OTP).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

/// A candidate verification or one-time code found in a message.
#[derive(Clone, Debug, PartialEq)]
pub struct OneTimeCode {
    pub code: String,
    /// 0.0–1.0; the UI only offers codes above a threshold.
    pub confidence: f32,
}

static RE_NUMERIC_GROUPED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d(?:[ -]?\d){3,7})\b").unwrap());

static RE_ALNUM_UPPER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z0-9]{6,8})\b").unwrap());

static RE_POSITIVE_KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(",
        r"bestätigungscode|bestaetigungscode|sicherheitscode|einmalpasswort|einmalcode|",
        r"verifizierungscode|verifizierung|prüfungscode|pruefungscode|zugangscode|",
        r"freischaltcode|anmeldecode|security\s+code|login\s+code|auth\s+code|",
        r"authentication\s+code|verification\s+code|verification|passcode|",
        r"one[- ]time(\s+password|\s+code)?|otp|tan|pin|2fa|mfa|code|verify",
        r")\b",
    ))
    .unwrap()
});

static RE_NEGATIVE_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b(",
        r"bestellnummer|bestell-?nr\.?|bestellung|rechnungsnummer|rechnungs-?nr\.?|",
        r"rechnung|kundennummer|kunden-?nr\.?|vertragsnummer|vertrags-?nr\.?|",
        r"vorgangsnummer|aktenzeichen|sendungsnummer|paketnummer|tracking(\s+number)?|",
        r"order(\s+number|\s+id|\s*#)?|invoice(\s+number|\s+id|\s*#)?|",
        r"customer(\s+number|\s+id)?|reference|ref\.?|iban|bic|ust-?idnr|plz|postleitzahl|",
        r"tel\.?|telefon|phone|fax|mobil|handy",
        r")\b",
    ))
    .unwrap()
});

static RE_TIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{1,2}:\d{2}(:\d{2})?\b").unwrap());

static RE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(\d{1,2}\.\d{1,2}\.\d{2,4}|\d{4}-\d{1,2}-\d{1,2}|\d{1,2}/\d{1,2}/\d{2,4})\b")
        .unwrap()
});

static RE_PRICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\d+[.,]\d{2}\s*(€|eur|usd|chf|\$|£)|(€|eur|usd|chf|\$|£)\s*\d+([.,]\d{2})?)")
        .unwrap()
});

static RE_PHONE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\+\d{1,3}[\s/-]?\d[\d\s/-]{5,}|(\b0\d{2,4}[\s/-]\d{4,}\b))").unwrap()
});

static RE_ADDRESS_PLZ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)(\b(?:D|DE|AT|CH)-\d{4,5}\b|",
        r"\b\d{5}\s+[A-ZÄÖÜ][a-zäöüß]{2,}|",
        r"(?:str\.|straße|strasse|weg|platz|allee|gasse)\s+\d+.*\b\d{5}\b)",
    ))
    .unwrap()
});

/// Finds verification / one-time codes in a message.
///
/// Matches 4–8 digit codes (including space/hyphen-grouped digits) and 6–8
/// character uppercase alphanumeric codes containing at least one digit.
/// High confidence requires a nearby keyword in the same or preceding line.
/// Results are deduplicated by normalized code and sorted by confidence.
pub fn find_codes(subject: &str, body_text: &str) -> Vec<OneTimeCode> {
    let mut best: HashMap<String, f32> = HashMap::new();
    let subject_has_keyword = RE_POSITIVE_KEYWORD.is_match(subject);

    scan_block(subject, true, subject_has_keyword, &mut best);
    scan_block(body_text, false, subject_has_keyword, &mut best);

    let mut out: Vec<OneTimeCode> = best
        .into_iter()
        .map(|(code, confidence)| OneTimeCode { code, confidence })
        .collect();

    out.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| a.code.cmp(&b.code))
    });
    out
}

fn scan_block(
    text: &str,
    is_subject: bool,
    subject_has_keyword: bool,
    best: &mut HashMap<String, f32>,
) {
    let lines: Vec<&str> = text.lines().collect();
    let mut prev_non_empty: Option<&str> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let same_line_kw = RE_POSITIVE_KEYWORD.is_match(trimmed);
        let prev_line_kw = prev_non_empty.is_some_and(|p| RE_POSITIVE_KEYWORD.is_match(p));
        let standalone_with_subject_kw =
            !is_subject && subject_has_keyword && is_standalone_token_line(trimmed);

        if same_line_kw || prev_line_kw || standalone_with_subject_kw {
            let base_conf = if is_subject {
                0.96
            } else if same_line_kw {
                0.90
            } else if prev_line_kw {
                0.85
            } else {
                0.80
            };

            extract_from_line(trimmed, base_conf, same_line_kw, best);
        }

        prev_non_empty = Some(trimmed);
    }
}

fn is_standalone_token_line(line: &str) -> bool {
    let normalized: String = line
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    (4..=8).contains(&normalized.len()) && normalized.chars().all(|c| c.is_ascii_alphanumeric())
}

fn extract_from_line(
    line: &str,
    base_conf: f32,
    same_line_kw: bool,
    best: &mut HashMap<String, f32>,
) {
    let excluded_spans = collect_excluded_spans(line);

    for cap in RE_NUMERIC_GROUPED.captures_iter(line) {
        let Some(m) = cap.get(1) else { continue };
        if is_excluded(line, m.start(), m.end(), &excluded_spans, same_line_kw) {
            continue;
        }

        let normalized: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
        if !(4..=8).contains(&normalized.len()) {
            continue;
        }

        // Reject years (1900..=2099) unless explicitly labeled by a keyword on the same line.
        if normalized.len() == 4
            && let Ok(year) = normalized.parse::<u16>()
            && (1900..=2099).contains(&year)
            && !has_direct_otp_label_before(line, m.start())
        {
            continue;
        }

        let conf = adjust_confidence(line, m.start(), base_conf, true);
        record(best, normalized, conf);
    }

    for cap in RE_ALNUM_UPPER.captures_iter(line) {
        let Some(m) = cap.get(1) else { continue };
        let raw = m.as_str();
        let has_digit = raw.chars().any(|c| c.is_ascii_digit());
        let has_alpha = raw.chars().any(|c| c.is_ascii_uppercase());
        if !has_digit || !has_alpha {
            continue;
        }
        if is_excluded(line, m.start(), m.end(), &excluded_spans, same_line_kw) {
            continue;
        }

        let conf = adjust_confidence(line, m.start(), base_conf * 0.96, false);
        record(best, raw.to_string(), conf);
    }
}

fn collect_excluded_spans(line: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for re in [
        &*RE_TIME,
        &*RE_DATE,
        &*RE_PRICE,
        &*RE_PHONE_LINE,
        &*RE_ADDRESS_PLZ,
    ] {
        for m in re.find_iter(line) {
            spans.push((m.start(), m.end()));
        }
    }
    spans
}

fn is_excluded(
    line: &str,
    start: usize,
    end: usize,
    excluded_spans: &[(usize, usize)],
    same_line_kw: bool,
) -> bool {
    if excluded_spans.iter().any(|&(s, e)| start < e && end > s) {
        return true;
    }

    // Check immediate neighbors for decimal/currency/date/phone characters.
    // A trailing '.' or ',' at sentence end (not followed by a digit) is allowed.
    let before = &line[..start];
    if let Some(prev) = before.chars().next_back() {
        let is_decimal_or_date_prev = matches!(prev, '.' | ',')
            && before[..before.len() - prev.len_utf8()]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit());
        if is_decimal_or_date_prev || matches!(prev, '+' | '/' | ':' | '#' | '€' | '$' | '£') {
            return true;
        }
    }
    let after = &line[end..];
    if let Some(next) = after.chars().next() {
        let is_decimal_or_date_next = matches!(next, '.' | ',')
            && after[next.len_utf8()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
        if is_decimal_or_date_next || matches!(next, ':' | '/' | '€' | '$' | '£') {
            return true;
        }
    }

    // Check if a negative label (Bestellnummer, Rechnungsnr., Kundennummer, etc.)
    // immediately precedes this candidate on the same line without an OTP keyword in between.
    let prefix = &line[..start];
    if let Some(neg) = RE_NEGATIVE_LABEL.find_iter(prefix).last() {
        let between = &prefix[neg.end()..];
        let otp_after_neg = RE_POSITIVE_KEYWORD.is_match(between);
        if !otp_after_neg && (!same_line_kw || between.len() < 24) {
            return true;
        }
    }

    false
}

fn has_direct_otp_label_before(line: &str, start: usize) -> bool {
    let prefix = &line[..start];
    let Some(m) = RE_POSITIVE_KEYWORD.find_iter(prefix).last() else {
        return false;
    };
    let between = prefix[m.end()..].trim();
    between.is_empty() || matches!(between, ":" | "=" | "-" | "ist" | "is" | "lautet")
}

fn adjust_confidence(line: &str, start: usize, base: f32, numeric: bool) -> f32 {
    let mut conf = base;
    if has_direct_otp_label_before(line, start) {
        conf = (conf + 0.04).min(1.0);
    }
    if !numeric {
        conf = conf.min(0.95);
    }
    conf
}

fn record(best: &mut HashMap<String, f32>, code: String, conf: f32) {
    best.entry(code)
        .and_modify(|existing| {
            if conf > *existing {
                *existing = conf;
            }
        })
        .or_insert(conf);
}

#[cfg(test)]
mod tests {
    use super::find_codes;

    #[test]
    fn extracts_six_digit_code_from_german_body() {
        let codes = find_codes(
            "Anmeldung bestätigen",
            "Ihr Bestätigungscode lautet: 482913\nEr ist 10 Minuten gültig.",
        );
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "482913");
        assert!(codes[0].confidence >= 0.85);
    }

    #[test]
    fn extracts_grouped_code_with_space_and_hyphen() {
        let space = find_codes("Login", "Dein Sicherheitscode: 482 913");
        assert_eq!(space[0].code, "482913");

        let hyphen = find_codes("Login", "Your verification code is 482-913.");
        assert_eq!(hyphen[0].code, "482913");
    }

    #[test]
    fn extracts_four_and_eight_digit_codes() {
        let four = find_codes("TAN-Verfahren", "Ihre TAN für die Überweisung: 7391");
        assert_eq!(four[0].code, "7391");

        let eight = find_codes("Passcode", "Your one-time passcode is 1234-5678.");
        assert_eq!(eight[0].code, "12345678");
    }

    #[test]
    fn extracts_alphanumeric_uppercase_code() {
        let codes = find_codes(
            "Verify your login",
            "Please enter this security code: K7Q2MX",
        );
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "K7Q2MX");
        assert!(codes[0].confidence >= 0.80);
    }

    #[test]
    fn extracts_code_when_keyword_is_on_preceding_line() {
        let body = "Ihr Einmalpasswort für die Anmeldung:\n\n904218\n\nBitte nicht weitergeben.";
        let codes = find_codes("Anmeldeversuch", body);
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "904218");
    }

    #[test]
    fn extracts_standalone_code_when_keyword_is_only_in_subject() {
        let codes = find_codes(
            "Ihr Bestätigungscode",
            "Hallo Max,\n\n551209\n\nViele Grüße",
        );
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "551209");
    }

    #[test]
    fn subject_match_ranks_higher_than_body_match_and_deduplicates() {
        let codes = find_codes(
            "Ihr OTP: 112233",
            "Ihr Einmalcode lautet 112233.\nAlternativer PIN: 445566",
        );
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].code, "112233");
        assert!(codes[0].confidence > codes[1].confidence);
    }

    #[test]
    fn extracts_pin_and_mfa_english_variants() {
        let pin = find_codes("Konto", "Ihre temporäre PIN lautet 8492.");
        assert_eq!(pin[0].code, "8492");

        let mfa = find_codes("Portal", "Use MFA login code 630194 to continue.");
        assert_eq!(mfa[0].code, "630194");
    }

    #[test]
    fn extracts_tan_and_freischaltcode() {
        let tan = find_codes("Freigabe", "Freischaltcode: 981204");
        assert_eq!(tan[0].code, "981204");

        let access = find_codes("Zugang", "Ihr Zugangscode für das Portal: A9B8C7");
        assert_eq!(access[0].code, "A9B8C7");
    }

    // --- Negative test cases (must NOT be recognized as OTPs) ---

    #[test]
    fn ignores_years_without_direct_keyword() {
        let codes = find_codes(
            "Jahresrückblick 2026",
            "Vielen Dank für Ihr Vertrauen im Jahr 2025 und 2026.",
        );
        assert!(codes.is_empty());

        // Even if "code" appears elsewhere in the line, a bare year shouldn't match.
        let codes2 = find_codes("Info", "Ihr Code gilt bis Ende 2026.");
        assert!(codes2.is_empty());
    }

    #[test]
    fn ignores_times_and_dates() {
        let codes = find_codes(
            "Verifizierung am 12.11.2026",
            "Bitte bestätigen Sie Ihren Termin um 14:30 oder am 2026-11-12.",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_prices_and_currency_amounts() {
        let codes = find_codes(
            "Rabatt-Code",
            "Sparen Sie 12,99 € oder EUR 1200 mit diesem Angebot.",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_phone_numbers() {
        let codes = find_codes(
            "Support-Anfrage",
            "Bei Fragen zu Ihrem Code rufen Sie an: +49 30 123456 oder 0171-9876543.",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_postal_codes_in_addresses() {
        let codes = find_codes(
            "Verifizierung Ihrer Adresse",
            "Musterstraße 12, 10115 Berlin\nAlternativ: D-80331 München",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_order_invoice_and_customer_numbers() {
        let codes = find_codes(
            "Ihre Rechnung",
            "Bestellnummer: 884120\nRechnungsnr.: 20260411\nKundennummer: 90012345",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_order_number_even_when_code_word_appears_elsewhere() {
        let codes = find_codes(
            "Gutschein-Code",
            "Bestellnummer: 445566 – vielen Dank für Ihren Einkauf.",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_plain_uppercase_words_without_digits() {
        let codes = find_codes(
            "Verification",
            "Your verification code word is SECRET or PORTAL.",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn ignores_random_numbers_without_any_keyword() {
        let codes = find_codes(
            "Wochenbericht",
            "In dieser Woche wurden 482913 Pakete und 123456 Briefe zugestellt.",
        );
        assert!(codes.is_empty());
    }

    #[test]
    fn extracts_two_factor_code_with_hyphenated_label() {
        let codes = find_codes("2FA", "Ihr 2FA-Code lautet 319420.");
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "319420");
    }

    #[test]
    fn ignores_overlong_nine_digit_numbers() {
        let codes = find_codes(
            "Bestätigungscode",
            "Ihr Code ist nicht diese lange Referenz 123456789.",
        );
        assert!(codes.is_empty());
    }
}
