//! PII masking and confidentiality gate before sending any message excerpt
//! to an external Stage-4 LLM (`docs/design/klassifizierung.md`).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// Why a message must never leave the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidentialityReason {
    /// Contains a one-time code / 2FA TAN.
    OneTimeCode,
    /// Contains a password, password-reset token, or secret key.
    PasswordOrSecret,
    /// Belongs to a category marked confidential (e.g. login data, contracts).
    ConfidentialCategory,
}

/// Sanitized subject and body with consistent placeholders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskResult {
    pub subject: String,
    pub body: String,
    /// Total number of sensitive spans replaced.
    pub redactions: usize,
}

static SECRET_KEYWORDS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(?:passwort zurücksetzen|neues passwort|initialpasswort|vorläufiges passwort|zugangsdaten|geheimschlüssel|wiederherstellungsschlüssel|backup-codes|password reset|reset your password|temporary password|recovery key|secret key|api[- ]?key|private key|seed phrase)\b"#).unwrap()
});

static URL_RESET_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)https?://[^\s<>"]*(?:reset[-_]?password|password[-_]?reset|verify[-_]?email|magic[-_]?link|auth[-_]?token)[^\s<>"]*"#).unwrap()
});

/// Checks whether a message must be blocked completely from Stage-4 cloud AI.
pub fn check_confidential(
    subject: &str,
    body_text: &str,
    category_confidential: bool,
) -> Option<ConfidentialityReason> {
    if category_confidential {
        return Some(ConfidentialityReason::ConfidentialCategory);
    }

    let codes = crate::extract::otp::find_codes(subject, body_text);
    for code in codes {
        if code.confidence >= 0.7 {
            return Some(ConfidentialityReason::OneTimeCode);
        }
    }

    let combined = format!("{} \n {}", subject, body_text);
    if SECRET_KEYWORDS_REGEX.is_match(&combined) || URL_RESET_REGEX.is_match(&combined) {
        return Some(ConfidentialityReason::PasswordOrSecret);
    }

    None
}

static URL_SENSITIVE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    // Matches https?://[^\s<>"'\)]+
    Regex::new(r#"https?://[^\s<>"'\)]+"#).unwrap()
});

static IBAN_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){3,7}(?:\s?[A-Z0-9]{1,4})?\b"#).unwrap()
});

static EMAIL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"#).unwrap());

static PHONE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\+\d{1,3}[\s./-]?|\b0\d{2,4}[\s./-])(?:\d[\s./-]?){5,12}\d\b"#).unwrap()
});

static DATE_PRICE_EXCLUDE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    // e.g. 14.11.2026 or 12,99
    Regex::new(r#"^\d{1,2}\.\d{1,2}\.\d{2,4}$|^\d+,\d{2}$"#).unwrap()
});

static STREET_ADDRESS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b[A-ZÄÖÜ][a-zäöüß]+(?:straße|strasse|str\.|weg|platz|allee|gasse|ring|damm|ufer)\s+\d{1,4}\s*[a-zA-Z]?\b"#).unwrap()
});

static PLZ_CITY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\b(?:D-|DE-|AT-|CH-)?\d{5}\s+[A-ZÄÖÜ][a-zäöüßA-ZÄÖÜ -]{2,25}\b"#).unwrap()
});

struct Mapper {
    emails: HashMap<String, String>,
    ibans: HashMap<String, String>,
    phones: HashMap<String, String>,
    addresses: HashMap<String, String>,
    persons: HashMap<String, String>,
    tokens: HashMap<String, String>,
    redactions: usize,
}

impl Mapper {
    fn new() -> Self {
        Self {
            emails: HashMap::new(),
            ibans: HashMap::new(),
            phones: HashMap::new(),
            addresses: HashMap::new(),
            persons: HashMap::new(),
            tokens: HashMap::new(),
            redactions: 0,
        }
    }

    fn map_email(&mut self, val: &str) -> String {
        let key = val.to_lowercase();
        if let Some(existing) = self.emails.get(&key) {
            existing.clone()
        } else {
            self.redactions += 1;
            let placeholder = format!("[EMAIL_{}]", self.emails.len() + 1);
            self.emails.insert(key, placeholder.clone());
            placeholder
        }
    }

    fn map_iban(&mut self, val: &str) -> String {
        let key = val
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_uppercase();
        if let Some(existing) = self.ibans.get(&key) {
            existing.clone()
        } else {
            self.redactions += 1;
            let placeholder = format!("[IBAN_{}]", self.ibans.len() + 1);
            self.ibans.insert(key, placeholder.clone());
            placeholder
        }
    }

    fn map_phone(&mut self, val: &str) -> String {
        let key = val
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>();
        if let Some(existing) = self.phones.get(&key) {
            existing.clone()
        } else {
            self.redactions += 1;
            let placeholder = format!("[PHONE_{}]", self.phones.len() + 1);
            self.phones.insert(key, placeholder.clone());
            placeholder
        }
    }

    fn map_address(&mut self, val: &str) -> String {
        let key = val.trim().to_lowercase();
        if let Some(existing) = self.addresses.get(&key) {
            existing.clone()
        } else {
            self.redactions += 1;
            let placeholder = format!("[ADDRESS_{}]", self.addresses.len() + 1);
            self.addresses.insert(key, placeholder.clone());
            placeholder
        }
    }

    fn map_person(&mut self, val: &str) -> String {
        let key = val.trim().to_lowercase();
        if let Some(existing) = self.persons.get(&key) {
            existing.clone()
        } else {
            self.redactions += 1;
            let placeholder = format!("[PERSON_{}]", self.persons.len() + 1);
            self.persons.insert(key, placeholder.clone());
            placeholder
        }
    }

    fn map_token(&mut self, val: &str) -> String {
        let key = val.to_string();
        if let Some(existing) = self.tokens.get(&key) {
            existing.clone()
        } else {
            self.redactions += 1;
            let placeholder = "[TOKEN]".to_string();
            self.tokens.insert(key, placeholder.clone());
            placeholder
        }
    }
}

fn process_text(text: &str, mapper: &mut Mapper, known_names: &[String]) -> String {
    // 1. URL sensitive query values / tokens
    let text = URL_SENSITIVE_REGEX
        .replace_all(text, |caps: &regex::Captures| {
            let url_str = &caps[0];
            process_url(url_str, mapper)
        })
        .to_string();

    // 2. IBANs
    let text = IBAN_REGEX
        .replace_all(&text, |caps: &regex::Captures| {
            let iban_str = &caps[0];
            // Validate basic IBAN format (15-34 alphanumeric after stripping spaces, starts with 2 letters + 2 digits)
            let stripped: String = iban_str.chars().filter(|c| !c.is_whitespace()).collect();
            if stripped.len() >= 15 && stripped.len() <= 34 {
                let b = stripped.as_bytes();
                if b[0].is_ascii_uppercase()
                    && b[1].is_ascii_uppercase()
                    && b[2].is_ascii_digit()
                    && b[3].is_ascii_digit()
                {
                    return mapper.map_iban(iban_str);
                }
            }
            iban_str.to_string()
        })
        .to_string();

    // 3. Email addresses
    let text = EMAIL_REGEX
        .replace_all(&text, |caps: &regex::Captures| mapper.map_email(&caps[0]))
        .to_string();

    // 4. Phone numbers
    let text = PHONE_REGEX
        .replace_all(&text, |caps: &regex::Captures| {
            let p = &caps[0];
            if DATE_PRICE_EXCLUDE_REGEX.is_match(p) {
                return p.to_string();
            }
            // Ensure it has at least 6 digits
            let digit_count = p.chars().filter(|c| c.is_ascii_digit()).count();
            if digit_count >= 6 {
                mapper.map_phone(p)
            } else {
                p.to_string()
            }
        })
        .to_string();

    // 5. Postal street addresses (German/DACH style)
    let text = STREET_ADDRESS_REGEX
        .replace_all(&text, |caps: &regex::Captures| mapper.map_address(&caps[0]))
        .to_string();

    let text = PLZ_CITY_REGEX
        .replace_all(&text, |caps: &regex::Captures| mapper.map_address(&caps[0]))
        .to_string();

    // 6. Known contact names (`known_names`)
    // Sort known_names by length descending to match full names before parts
    let mut sorted_names: Vec<String> = known_names
        .iter()
        .map(|n| n.trim().to_string())
        .filter(|n| n.len() >= 3)
        .collect();
    sorted_names.sort_by_key(|a| std::cmp::Reverse(a.len()));

    // Also expand with individual name parts >= 3 chars
    let mut full_and_parts = HashSet::new();
    for name in &sorted_names {
        full_and_parts.insert(name.clone());
        for part in name.split_whitespace() {
            if part.len() >= 3 {
                full_and_parts.insert(part.to_string());
            }
        }
    }

    let mut expanded_names: Vec<String> = full_and_parts.into_iter().collect();
    expanded_names.sort_by_key(|a| std::cmp::Reverse(a.len()));

    let mut current_text = text;
    for name in expanded_names {
        // Build word boundary case-insensitive regex for this name/part
        // Escape special regex chars in name just in case
        let escaped = regex::escape(&name);
        let pat = format!(r#"(?i)\b{}\b"#, escaped);
        if let Ok(re) = Regex::new(&pat) {
            // Check if already inside a placeholder
            current_text = re
                .replace_all(&current_text, |caps: &regex::Captures| {
                    mapper.map_person(&caps[0])
                })
                .to_string();
        }
    }

    current_text
}

fn process_url(url: &str, mapper: &mut Mapper) -> String {
    // Query parameter values: =[^&\s#]+ -> =[TOKEN]
    // Also long path segments >= 20 chars of [A-Za-z0-9_-]
    let query_re = Regex::new(r#"=([^&\s#]+)"#).unwrap();
    let replaced_query = query_re
        .replace_all(url, |caps: &regex::Captures| {
            let val = &caps[1];
            // If it looks like a token/id
            mapper.map_token(val);
            "=[TOKEN]".to_string()
        })
        .to_string();

    // Path segments of >= 20 chars [A-Za-z0-9_-]
    let path_seg_re = Regex::new(r#"/([A-Za-z0-9_-]{20,})"#).unwrap();
    path_seg_re
        .replace_all(&replaced_query, |caps: &regex::Captures| {
            let seg = &caps[1];
            mapper.map_token(seg);
            "/[TOKEN]".to_string()
        })
        .to_string()
}

/// Masks email addresses, phone numbers, IBANs, street addresses, URL query
/// tokens, and known contact names with consistent numbered placeholders
/// (`[EMAIL_1]`, `[IBAN_1]`, `[PHONE_1]`, `[PERSON_1]`, `[ADDRESS_1]`).
pub fn mask_message(subject: &str, body_text: &str, known_names: &[String]) -> MaskResult {
    let mut mapper = Mapper::new();
    let masked_subject = process_text(subject, &mut mapper, known_names);
    let masked_body = process_text(body_text, &mut mapper, known_names);

    MaskResult {
        subject: masked_subject,
        body: masked_body,
        redactions: mapper.redactions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidential_category() {
        assert_eq!(
            check_confidential("Hello", "Body", true),
            Some(ConfidentialityReason::ConfidentialCategory)
        );
    }

    #[test]
    fn test_confidential_otp() {
        // find_codes needs a code pattern with positive keyword or grouped numbers
        let subject = "Dein Bestätigungscode";
        let body = "Dein Code ist 123456";
        assert_eq!(
            check_confidential(subject, body, false),
            Some(ConfidentialityReason::OneTimeCode)
        );
    }

    #[test]
    fn test_confidential_password_reset_de() {
        assert_eq!(
            check_confidential(
                "Passwort zurücksetzen",
                "Bitte klicke hier, um dein neues Passwort zu setzen.",
                false
            ),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_confidential_password_reset_en() {
        assert_eq!(
            check_confidential(
                "Password Reset",
                "Please use this temporary password to access your account.",
                false
            ),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_confidential_recovery_key() {
        assert_eq!(
            check_confidential(
                "Wiederherstellung",
                "Hier ist dein Wiederherstellungsschlüssel.",
                false
            ),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_confidential_url_reset() {
        assert_eq!(
            check_confidential(
                "Link",
                "https://example.org/reset-password?token=abc",
                false
            ),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_confidential_safe_invoice() {
        assert_eq!(
            check_confidential(
                "Rechnung Nr. 12345",
                "Vielen Dank für Ihre Bestellung bei example.com. Anbei Ihre Rechnung.",
                false
            ),
            None
        );
    }

    #[test]
    fn test_confidential_safe_newsletter() {
        assert_eq!(
            check_confidential(
                "Newsletter KW 46",
                "Hier sind die Neuigkeiten der Woche von example.org.",
                false
            ),
            None
        );
    }

    #[test]
    fn test_mask_consistent_email() {
        let subject = "Contact max@example.org";
        let body = "Please email max@example.org or other@example.com";
        let res = mask_message(subject, body, &[]);
        assert!(res.subject.contains("[EMAIL_1]"));
        assert!(res.body.contains("[EMAIL_1]"));
        assert!(res.body.contains("[EMAIL_2]"));
        assert!(!res.subject.contains("max@example.org"));
        assert!(!res.body.contains("max@example.org"));
    }

    #[test]
    fn test_mask_iban() {
        let res = mask_message(
            "IBAN",
            "DE89370400440532013000 and DE89 3704 0044 0532 0130 00",
            &[],
        );
        assert!(res.body.contains("[IBAN_1]"));
        // Since both normalize to the same IBAN string, they should get the same placeholder
        let occurrences = res.body.matches("[IBAN_1]").count();
        assert_eq!(occurrences, 2);
    }

    #[test]
    fn test_mask_phone_numbers() {
        let res = mask_message(
            "Call",
            "Tel: +49 30 12345678 or 089 98765432 (Date: 14.11.2026, Price: 12,99)",
            &[],
        );
        assert!(res.body.contains("[PHONE_1]"));
        assert!(res.body.contains("[PHONE_2]"));
        assert!(res.body.contains("14.11.2026"));
        assert!(res.body.contains("12,99"));
    }

    #[test]
    fn test_mask_addresses() {
        let res = mask_message("Address", "Hauptstraße 12a, 10115 Berlin", &[]);
        assert!(res.body.contains("[ADDRESS_1]"));
        assert!(res.body.contains("[ADDRESS_2]"));
    }

    #[test]
    fn test_mask_url_tokens() {
        let res = mask_message(
            "URL",
            "Visit https://example.org/verify?token=abcdef1234567890abcdef and path https://example.org/auth/1234567890123456789012345",
            &[],
        );
        assert!(res.body.contains("[TOKEN]"));
        assert!(!res.body.contains("abcdef1234567890abcdef"));
        assert!(!res.body.contains("1234567890123456789012345"));
    }

    #[test]
    fn test_mask_known_names() {
        let names = vec!["Maximilian Mustermann".to_string()];
        let res = mask_message(
            "Hello Maximilian",
            "Dear Maximilian Mustermann, best regards, Maximilian",
            &names,
        );
        assert!(res.subject.contains("[PERSON_1]"));
        assert!(res.body.contains("[PERSON_1]"));
        assert!(!res.body.contains("Maximilian"));
    }

    #[test]
    fn test_torture_test_nothing_slips_through() {
        let known = vec!["Erika Musterfrau".to_string()];
        let subject = "Security alert for max@example.org regarding IBAN DE89370400440532013000";
        let body = "Dear Erika Musterfrau, please verify at https://example.org/auth?session=secrettoken1234567890abcdef from Hauptstraße 10, 80331 München or call +49 89 5551234. Your email is max@example.org and secondary is erika@example.org.";

        let res = mask_message(subject, body, &known);

        assert!(!res.subject.contains("max@example.org"));
        assert!(!res.subject.contains("DE89370400440532013000"));
        assert!(!res.body.contains("Erika Musterfrau"));
        assert!(!res.body.contains("Erika"));
        assert!(!res.body.contains("Hauptstraße 10"));
        assert!(!res.body.contains("80331 München"));
        assert!(!res.body.contains("+49 89 5551234"));
        assert!(!res.body.contains("max@example.org"));
        assert!(!res.body.contains("erika@example.org"));
        assert!(!res.body.contains("secrettoken1234567890abcdef"));
        assert!(res.redactions > 0);
    }

    #[test]
    fn test_additional_coverage_1() {
        let res = mask_message("Sub", "At 10115 Berlin", &[]);
        assert!(res.body.contains("[ADDRESS_1]"));
    }

    #[test]
    fn test_additional_coverage_2() {
        let res = res_helper("test@example.com");
        assert!(res.body.contains("[EMAIL_1]"));
    }

    fn res_helper(email: &str) -> MaskResult {
        mask_message("Sub", email, &[])
    }

    #[test]
    fn test_additional_coverage_3() {
        let res = mask_message("Sub", "IBAN: AT611904300234567892", &[]);
        assert!(res.body.contains("[IBAN_1]"));
    }

    #[test]
    fn test_additional_coverage_4() {
        let res = mask_message("Sub", "CH930076201123456781", &[]);
        assert!(res.body.contains("[IBAN_1]"));
    }

    #[test]
    fn test_additional_coverage_5() {
        let res = mask_message("Sub", "Phone +1-800-555-0199", &[]);
        assert!(res.body.contains("[PHONE_1]"));
    }

    #[test]
    fn test_additional_coverage_6() {
        let res = mask_message("Sub", "Phone 030/1234567", &[]);
        assert!(res.body.contains("[PHONE_1]"));
    }

    #[test]
    fn test_additional_coverage_7() {
        assert_eq!(
            check_confidential("API Key", "Here is your api-key: secret", false),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_additional_coverage_8() {
        assert_eq!(
            check_confidential("Seed", "Seed phrase: word word word", false),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_additional_coverage_9() {
        assert_eq!(
            check_confidential("Secret", "Private key attached", false),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }

    #[test]
    fn test_additional_coverage_10() {
        assert_eq!(
            check_confidential("Initial", "Ihr Initialpasswort lautet start123", false),
            Some(ConfidentialityReason::PasswordOrSecret)
        );
    }
}
