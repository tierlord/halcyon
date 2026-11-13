//! Brand domain extraction and logo resolution (BIMI, website icons, fallbacks).

use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;
use regex::Regex;
use serde::{Deserialize, Serialize};

const FETCH_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_LOGO_BYTES: usize = 512 * 1024;
const MIN_LOGO_BYTES: usize = 100;

/// Cache TTL for resolved logos (30 days).
pub const LOGO_HIT_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// Cache TTL for negative lookups (24 hours) to avoid hammering unreachable hosts.
pub const LOGO_MISS_TTL_SECS: i64 = 24 * 60 * 60;

static RE_BIMI_LOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|;\s*)l\s*=\s*(https://[^\s;]+)").unwrap());

const FREEMAIL_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "web.de",
    "gmx.de",
    "gmx.net",
    "gmx.at",
    "gmx.ch",
    "t-online.de",
    "freenet.de",
    "posteo.de",
    "posteo.net",
    "mailbox.org",
    "outlook.com",
    "outlook.de",
    "hotmail.com",
    "hotmail.de",
    "live.com",
    "live.de",
    "msn.com",
    "yahoo.com",
    "yahoo.de",
    "icloud.com",
    "me.com",
    "mac.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "aol.com",
    "zoho.com",
    "tutanota.com",
    "tutamail.com",
    "yandex.com",
    "mail.de",
    "email.de",
    "arcor.de",
    "vodafone.de",
    "1und1.de",
    "ionos.de",
];

const TWO_LEVEL_TLDS: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "com.au", "net.au", "co.jp", "co.nz", "com.br", "co.za", "com.tr",
];

/// A cached or freshly resolved brand logo with optional visual appearance metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrandLogo {
    pub data_url: String,
    pub bg: Option<String>,
    pub padded: bool,
    pub scale: f32,
}

/// Extracts the registrable brand domain from an email address.
///
/// Subdomains are collapsed to the root domain (respecting common two-level
/// TLDs like `co.uk`), and known freemail providers return `None` so personal
/// contacts keep their initials/avatar instead of showing a provider logo.
pub fn extract_brand_domain(email: &str) -> Option<String> {
    let (_, raw_domain) = email.trim().rsplit_once('@')?;
    let full = raw_domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if full.is_empty() || !full.contains('.') || full.chars().any(char::is_whitespace) {
        return None;
    }
    if FREEMAIL_DOMAINS.contains(&full.as_str()) {
        return None;
    }

    let parts: Vec<&str> = full.split('.').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }

    let tail2 = format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]);
    let root = if parts.len() >= 3 && TWO_LEVEL_TLDS.contains(&tail2.as_str()) {
        format!("{}.{tail2}", parts[parts.len() - 3])
    } else {
        tail2
    };

    if FREEMAIL_DOMAINS.contains(&root.as_str()) {
        return None;
    }

    Some(root)
}

/// Resolves a brand logo for `domain` using a 3-stage cascade:
/// 1. BIMI DNS TXT record (`default._bimi.<domain>`).
/// 2. Direct website icons (`apple-touch-icon.png`, `favicon.ico`).
/// 3. Public icon services (DuckDuckGo, Google).
pub async fn resolve_logo(domain: &str) -> Option<String> {
    let http = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .user_agent(concat!("Halcyon/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;

    // 1. Official BIMI SVG logo via DNS TXT.
    if let Some(url) = fetch_bimi_logo_url(domain).await
        && let Some(data_url) = fetch_image_as_data_url(&http, &url, MIN_LOGO_BYTES).await
    {
        return Some(data_url);
    }

    // 2. Direct icons on the sender's root domain.
    let direct_candidates = [
        format!("https://{domain}/apple-touch-icon.png"),
        format!("https://www.{domain}/apple-touch-icon.png"),
        format!("https://{domain}/favicon.ico"),
        format!("https://www.{domain}/favicon.ico"),
    ];
    for url in &direct_candidates {
        if let Some(data_url) = fetch_image_as_data_url(&http, url, 120).await {
            return Some(data_url);
        }
    }

    // 3. Smart fallback via public icon services.
    let fallback_candidates = [
        format!("https://icons.duckduckgo.com/ip3/{domain}.ico"),
        format!("https://www.google.com/s2/favicons?domain={domain}&sz=64"),
    ];
    for url in &fallback_candidates {
        if let Some(data_url) = fetch_image_as_data_url(&http, url, 120).await {
            return Some(data_url);
        }
    }

    None
}

async fn fetch_bimi_logo_url(domain: &str) -> Option<String> {
    let resolver = TokioResolver::builder_tokio().ok()?.build().ok()?;
    let query = format!("default._bimi.{domain}.");
    let lookup = resolver.txt_lookup(query).await.ok()?;
    for record in lookup.answers() {
        let RData::TXT(txt_record) = &record.data else {
            continue;
        };
        let txt: String = txt_record
            .txt_data
            .iter()
            .map(|chunk| String::from_utf8_lossy(chunk))
            .collect();
        if let Some(url) = parse_bimi_txt(&txt) {
            return Some(url);
        }
    }
    None
}

fn parse_bimi_txt(txt: &str) -> Option<String> {
    let lower = txt.to_ascii_lowercase();
    if !lower.contains("v=bimi1") && !lower.contains("v = bimi1") {
        return None;
    }
    RE_BIMI_LOC
        .captures(txt)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

async fn fetch_image_as_data_url(
    http: &reqwest::Client,
    url: &str,
    min_bytes: usize,
) -> Option<String> {
    let res = http.get(url).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }

    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    let bytes = res.bytes().await.ok()?;
    if bytes.len() < min_bytes || bytes.len() > MAX_LOGO_BYTES {
        return None;
    }

    let mime = if content_type.contains("svg") || url.to_ascii_lowercase().ends_with(".svg") {
        let text = std::str::from_utf8(&bytes).ok()?;
        if !text.contains("<svg") {
            return None;
        }
        "image/svg+xml".to_string()
    } else if content_type.starts_with("image/") {
        content_type
            .split(';')
            .next()
            .unwrap_or("image/png")
            .trim()
            .to_string()
    } else if content_type.contains("octet-stream") && url.to_ascii_lowercase().ends_with(".ico") {
        "image/x-icon".to_string()
    } else {
        return None;
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::{extract_brand_domain, parse_bimi_txt};

    #[test]
    fn extracts_root_domain_from_subdomains() {
        assert_eq!(
            extract_brand_domain("r92q123@mail.kleinanzeigen.de").as_deref(),
            Some("kleinanzeigen.de")
        );
        assert_eq!(
            extract_brand_domain("billing@news.shop.co.uk").as_deref(),
            Some("shop.co.uk")
        );
    }

    #[test]
    fn filters_freemail_domains() {
        assert_eq!(extract_brand_domain("max@gmail.com"), None);
        assert_eq!(extract_brand_domain("max@sub.web.de"), None);
        assert_eq!(extract_brand_domain("max@gmx.de"), None);
        assert_eq!(extract_brand_domain("max@proton.me"), None);
        assert_eq!(extract_brand_domain("max@t-online.de"), None);
    }

    #[test]
    fn rejects_invalid_addresses() {
        assert_eq!(extract_brand_domain(""), None);
        assert_eq!(extract_brand_domain("localhost"), None);
        assert_eq!(extract_brand_domain("user@localhost"), None);
    }

    #[test]
    fn parses_bimi_txt_record() {
        assert_eq!(
            parse_bimi_txt("v=BIMI1; l=https://cdn.example/logo.svg; a=https://cdn.example/v.pem")
                .as_deref(),
            Some("https://cdn.example/logo.svg")
        );
        assert_eq!(parse_bimi_txt("v=spf1 include:example ~all"), None);
    }
}
