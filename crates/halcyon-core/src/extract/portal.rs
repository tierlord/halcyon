//! Finds links to a sender's customer portal ("Kundencenter") in mail.
//!
//! The invoice ledger offers such a link when no proof document came with
//! an invoice. Everything here is generic: a link only counts when it
//! stays on the sender's own (DMARC-authenticated) domain and looks like a
//! login or billing page. That drops phishing look-alikes as well as
//! third-party click-tracking redirects, which hide the real target behind a
//! per-recipient token.

use std::sync::LazyLock;

use regex::Regex;

use crate::linker::base_domain;

/// Where a link was found; decides how much it is trusted as "the" portal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The invoice mail itself.
    InvoiceMail,
    /// A welcome or account-creation mail of the same sender.
    Welcome,
    /// Any other mail of the same sender.
    Other,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::InvoiceMail => "invoice",
            Origin::Welcome => "welcome",
            Origin::Other => "other",
        }
    }

    fn base_score(self) -> i32 {
        match self {
            Origin::InvoiceMail => 100,
            Origin::Welcome => 60,
            Origin::Other => 30,
        }
    }
}

/// A link that passed all checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortalLink {
    /// `https://host/path` without query string and fragment.
    pub url: String,
    pub label: String,
    pub score: i32,
}

/// A raw link as found in a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLink {
    pub href: String,
    pub text: String,
}

static RE_ANCHOR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\s[^>]*?href\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))[^>]*>(.*?)</a\s*>"#)
        .unwrap()
});
static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static RE_BARE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s<>"'()\[\]]+"#).unwrap());
static RE_WELCOME_SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)willkommen|registrierung|registriert|kundenkonto|konto\s+(?:wurde\s+)?(?:angelegt|erstellt|eingerichtet|aktiviert)|zugangsdaten|account\s+(?:has\s+been\s+)?created|welcome|your\s+new\s+account|sign(?:ed)?[- ]?up",
    )
    .unwrap()
});

/// Words (as whole-word-insensitive substrings of text, path and host) that
/// make a link look like a customer area or billing page.
static RE_PORTAL_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)kundencenter|kundenportal|kundenbereich|kundenkonto|kundenlogin|mein konto|meine rechnungen|rechnungsportal|rechnungs[uü]bersicht|rechnung(?:en)? (?:online|ansehen|abrufen|herunterladen|anzeigen)|(?:view|download|show) (?:your )?invoices?|bestellung(?:en)? (?:anzeigen|ansehen|einsehen|verfolgen)|bestelldetails|(?:meine|ihre|deine) bestellungen|order details|your orders|view (?:your )?order|rechnung(?:en)? drucken|\blogin\b|log in|sign in|anmelden|\baccounts?\b|customer[ -]?(?:portal|center|centre|area)|billing|\binvoices?\b",
    )
    .unwrap()
});
/// Order wording: leads to the order list or a single order in the account.
static RE_ORDER_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)bestellung|bestelldetails|\borders?\b|order details").unwrap()
});
/// Invoice-specific wording, a stronger hint than a plain login link.
static RE_INVOICE_WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)rechnung|\binvoices?\b|billing|\bbills?\b").unwrap());
/// Links that must never be offered: opt-out, one-time confirmation and
/// reset links (their tokens are secrets), log-out.
static RE_EXCLUDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)unsubscribe|abmelden|abmeldung|abbestellen|austragen|opt[- ]?out|logout|log out|sign out|passwort|password|reset|verify|verif|confirm|bestätig|aktivier|activat|\btoken\b|tracking pixel|\bpixel\b|newsletter|preferences",
    )
    .unwrap()
});
/// Host labels or path segments of click-tracking redirectors.
static RE_TRACKING_PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:click|clicks|track|tracking|trk|links?|redirect|r|url|em|email|e|mailtrack|ct|go|lnk)$")
        .unwrap()
});

/// Whether a subject looks like a welcome or account-creation mail.
pub fn is_welcome_subject(subject: &str) -> bool {
    RE_WELCOME_SUBJECT.is_match(subject)
}

/// Links of a message: anchors from the HTML part and bare URLs from the
/// text, with a bit of surrounding text to judge them by.
pub fn extract_links(html: Option<&str>, text: &str) -> Vec<RawLink> {
    let mut out: Vec<RawLink> = Vec::new();
    if let Some(html) = html {
        for c in RE_ANCHOR.captures_iter(html) {
            let href = c
                .get(1)
                .or_else(|| c.get(2))
                .or_else(|| c.get(3))
                .map_or("", |m| m.as_str());
            let inner = c.get(4).map_or("", |m| m.as_str());
            out.push(RawLink {
                href: decode_entities(href.trim()),
                text: collapse(&decode_entities(&RE_TAG.replace_all(inner, " "))),
            });
        }
    }
    for m in RE_BARE_URL.find_iter(text) {
        let url = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
        // Same line, the words in front of the URL ("Kundencenter: https://…").
        let before = &text[..m.start()];
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let context: String = before[line_start..]
            .chars()
            .rev()
            .take(60)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        out.push(RawLink {
            href: url.to_string(),
            text: collapse(&context),
        });
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|l| seen.insert((l.href.clone(), l.text.clone())));
    out
}

/// All qualifying links of a message for a sender, best first.
pub fn find_portal_links(
    html: Option<&str>,
    text: &str,
    sender_domain: &str,
    origin: Origin,
) -> Vec<PortalLink> {
    let mut found: Vec<PortalLink> = extract_links(html, text)
        .iter()
        .filter_map(|l| qualify(l, sender_domain, origin))
        .collect();
    found.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.url.cmp(&b.url)));
    found.dedup_by(|a, b| a.url == b.url);
    found
}

/// Checks one link; `None` when it must not be offered.
pub fn qualify(link: &RawLink, sender_domain: &str, origin: Origin) -> Option<PortalLink> {
    let (host, path) = parse_https(&link.href)?;
    // The core phishing guard: only the sender's own domain.
    if base_domain(&host).as_deref() != Some(sender_domain) {
        return None;
    }
    let path_words = path.replace(['-', '_', '/', '.'], " ");
    let host_words = host.replace(['-', '.'], " ");
    let all = format!("{} {} {}", link.text, path_words, host_words);
    if RE_EXCLUDED.is_match(&all) || is_asset(&path) || looks_like_tracking(&host, &path) {
        return None;
    }
    let host_hint = host.split('.').next().is_some_and(|l| {
        matches!(
            l,
            "mein"
                | "meine"
                | "my"
                | "login"
                | "account"
                | "kundencenter"
                | "service"
                | "portal"
                | "kunden"
                | "billing"
                | "customer"
        )
    });
    let word_hit = RE_PORTAL_WORDS.is_match(&format!("{} {}", link.text, path_words))
        || RE_PORTAL_WORDS.is_match(&host_words);
    if !host_hint && !word_hit {
        return None;
    }
    // A bare "service." host without any other hint is too vague.
    if !word_hit && host.starts_with("service.") && path.len() <= 1 && link.text.trim().is_empty() {
        return None;
    }
    let specific = RE_INVOICE_WORDS.is_match(&format!("{} {}", link.text, path_words));
    let lower = all.to_lowercase();
    let label = if specific {
        "Rechnung online abrufen"
    } else if RE_ORDER_WORDS.is_match(&format!("{} {}", link.text, path_words)) {
        "Zur Bestellung im Kundenkonto"
    } else if lower.contains("kundencenter") || lower.contains("customer center") {
        "Zum Kundencenter"
    } else if lower.contains("kundenportal") || lower.contains("customer portal") {
        "Zum Kundenportal"
    } else if lower.contains("konto") || lower.contains("account") {
        "Zum Kundenkonto"
    } else {
        "Zum Kundenbereich"
    };
    let mut score = origin.base_score();
    if specific {
        score += 20;
    }
    if word_hit && host_hint {
        score += 5;
    }
    // Shallow pages are the portal itself; deep ones are single documents.
    score -= path.matches('/').count().saturating_sub(2) as i32;
    Some(PortalLink {
        url: format!("https://{host}{path}"),
        label: label.into(),
        score,
    })
}

/// Splits an `https` URL into host and path, dropping query, fragment and
/// the default port. `None` for anything else, including user-info tricks
/// like `https://shop.example.org@evil.example.net/`.
fn parse_https(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let rest = url
        .get(..8)
        .filter(|p| p.eq_ignore_ascii_case("https://"))
        .map(|_| &url[8..])?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') || authority.is_empty() {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((h, "443")) => h,
        Some(_) => return None,
        None => authority,
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if !host.contains('.')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    let path = &tail[..path_end];
    if path
        .chars()
        .any(|c| !c.is_ascii_graphic() || matches!(c, '<' | '>' | '"' | '\'' | '\\'))
    {
        return None;
    }
    let path = if path.is_empty() { "/" } else { path };
    Some((host, path.to_string()))
}

fn is_asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico", ".css", ".js", ".woff", ".woff2",
    ]
    .iter()
    .any(|e| lower.ends_with(e))
}

/// Click-tracking hosts and paths, and paths carrying a per-recipient token
/// (long mixed segments), which must not be stored.
fn looks_like_tracking(host: &str, path: &str) -> bool {
    if host
        .split('.')
        .next()
        .is_some_and(|l| RE_TRACKING_PART.is_match(l))
    {
        return true;
    }
    path.split('/').filter(|s| !s.is_empty()).any(|seg| {
        RE_TRACKING_PART.is_match(seg)
            || (seg.len() >= 20 && seg.bytes().any(|b| b.is_ascii_digit()))
    })
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(href: &str, text: &str) -> RawLink {
        RawLink {
            href: href.into(),
            text: text.into(),
        }
    }

    #[test]
    fn order_page_links_are_accepted_with_an_order_label() {
        for (href, text) in [
            (
                "https://www.shop.example.org/konto/bestellungen?id=7",
                "Bestellung anzeigen",
            ),
            ("https://www.shop.example.org/orders", "View order"),
            ("https://www.shop.example.org/konto", "Meine Bestellungen"),
            ("https://www.shop.example.org/konto", "Bestelldetails"),
        ] {
            let l = qualify(&link(href, text), "example.org", Origin::InvoiceMail)
                .unwrap_or_else(|| panic!("{text}"));
            assert_eq!(l.label, "Zur Bestellung im Kundenkonto", "{text}");
            assert!(!l.url.contains('?'));
        }
        // Still the sender's own domain only, and no token-like path.
        assert!(
            qualify(
                &link(
                    "https://evil.example.net/bestellungen",
                    "Bestellung anzeigen"
                ),
                "example.org",
                Origin::InvoiceMail
            )
            .is_none()
        );
        assert!(
            qualify(
                &link(
                    "https://www.shop.example.org/orders/a1b2c3d4e5f6a1b2c3d4e5f6",
                    "Bestellung anzeigen"
                ),
                "example.org",
                Origin::InvoiceMail
            )
            .is_none()
        );
    }

    #[test]
    fn same_domain_portal_link_is_accepted_and_query_stripped() {
        let l = qualify(
            &link(
                "https://kundencenter.shop.example.org/login?token=abc&x=1#frag",
                "Zum Kundencenter",
            ),
            "example.org",
            Origin::InvoiceMail,
        )
        .unwrap();
        assert_eq!(l.url, "https://kundencenter.shop.example.org/login");
        assert_eq!(l.label, "Zum Kundencenter");
    }

    #[test]
    fn invoice_specific_link_gets_invoice_label_and_higher_score() {
        let generic = qualify(
            &link("https://www.shop.example.org/login", "Anmelden"),
            "example.org",
            Origin::InvoiceMail,
        )
        .unwrap();
        let specific = qualify(
            &link(
                "https://www.shop.example.org/konto/rechnungen",
                "Rechnung online abrufen",
            ),
            "example.org",
            Origin::InvoiceMail,
        )
        .unwrap();
        assert_eq!(specific.label, "Rechnung online abrufen");
        assert!(specific.score > generic.score);
        let welcome = qualify(
            &link("https://www.shop.example.org/login", "Anmelden"),
            "example.org",
            Origin::Welcome,
        )
        .unwrap();
        assert!(generic.score > welcome.score);
    }

    #[test]
    fn foreign_domain_http_and_userinfo_tricks_are_rejected() {
        let d = "example.org";
        for href in [
            "https://track.example.net/click?u=https://shop.example.org/login",
            "https://shop.example.org.evil.example.net/login",
            "http://www.shop.example.org/login",
            "https://shop.example.org@evil.example.net/login",
            "https://www.shop.example.org:8443/login",
            "javascript:alert(1)",
        ] {
            assert!(
                qualify(&link(href, "Zum Kundencenter"), d, Origin::InvoiceMail).is_none(),
                "{href}"
            );
        }
    }

    #[test]
    fn unsubscribe_images_tracking_and_tokens_are_rejected() {
        let d = "example.org";
        for (href, text) in [
            (
                "https://www.shop.example.org/account/unsubscribe",
                "Account",
            ),
            (
                "https://www.shop.example.org/login",
                "Abmelden vom Newsletter",
            ),
            ("https://www.shop.example.org/account/logo.png", "Account"),
            ("https://click.shop.example.org/account", "Mein Konto"),
            (
                "https://www.shop.example.org/account/9f8a7b6c5d4e3f2a1b0c9d8e",
                "Konto",
            ),
            (
                "https://www.shop.example.org/account/reset-password",
                "Konto",
            ),
            ("https://www.shop.example.org/produkte", "Neue Angebote"),
        ] {
            assert!(
                qualify(&link(href, text), d, Origin::InvoiceMail).is_none(),
                "{href}"
            );
        }
    }

    #[test]
    fn links_come_from_html_anchors_and_plain_text() {
        let html = r#"<p><a href="https://mein.shop.example.org/start?id=7&amp;x=1"><b>Mein</b> Konto</a>
            <a href='https://www.shop.example.org/img.png'><img src="x"></a></p>"#;
        let text = "Ihr Kundencenter: https://kundencenter.shop.example.org/.\nBis bald";
        let found = find_portal_links(Some(html), text, "example.org", Origin::Welcome);
        let urls: Vec<&str> = found.iter().map(|l| l.url.as_str()).collect();
        assert!(urls.contains(&"https://mein.shop.example.org/start"));
        assert!(urls.contains(&"https://kundencenter.shop.example.org/"));
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn welcome_subjects() {
        assert!(is_welcome_subject("Willkommen bei Beispiel"));
        assert!(is_welcome_subject("Ihr Konto wurde angelegt"));
        assert!(is_welcome_subject("Your account has been created"));
        assert!(!is_welcome_subject("Ihre Rechnung 12345"));
    }
}
