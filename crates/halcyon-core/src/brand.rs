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
const MAX_HTML_BYTES: usize = 256 * 1024;

/// Cache TTL for resolved logos (30 days).
pub const LOGO_HIT_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// Cache TTL for negative lookups (24 hours) to avoid hammering unreachable hosts.
pub const LOGO_MISS_TTL_SECS: i64 = 24 * 60 * 60;

static RE_BIMI_LOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|;\s*)l\s*=\s*(https://[^\s;]+)").unwrap());
static RE_LINK_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<link\b[^>]*>").unwrap());
static RE_ATTR_REL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\brel\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());
static RE_ATTR_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());

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

/// Whether `domain` is a plain host name (letters, digits, hyphens, dots).
/// Anything else could smuggle paths, ports or credentials into the URLs
/// built from it.
pub fn is_valid_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        // The top-level label has letters: no IP addresses (local network).
        && domain
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_alphabetic()))
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Resolves a brand logo for `domain`, asking only the brand itself:
/// 1. BIMI DNS TXT record (`default._bimi.<domain>`).
/// 2. Icons on the brand's own website (`apple-touch-icon.png`, `favicon.ico`).
///
/// No third-party icon service is asked: it would learn whom the user
/// corresponds with.
pub async fn resolve_logo(domain: &str) -> Option<String> {
    if !is_valid_domain(domain) {
        return None;
    }
    let http = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // Follow a few redirects, but never away from HTTPS.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 || attempt.url().scheme() != "https" {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
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

    // 3. Icons declared via <link rel="apple-touch-icon|icon"> in the homepage HTML.
    let homepage_candidates = [
        format!("https://{domain}/"),
        format!("https://www.{domain}/"),
    ];
    for page_url in &homepage_candidates {
        let icon_urls = fetch_html_icon_urls(&http, page_url).await;
        for url in icon_urls.into_iter().take(3) {
            if let Some(data_url) = fetch_image_as_data_url(&http, &url, MIN_LOGO_BYTES).await {
                return Some(data_url);
            }
        }
    }

    None
}

async fn fetch_bimi_logo_url(domain: &str) -> Option<String> {
    // BIMI logos must be served over HTTPS (BIMI spec, section 4.4).
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

    // Read at most MAX_LOGO_BYTES: a hostile server must not be able to
    // make Halcyon download (and hold) gigabytes.
    if res
        .content_length()
        .is_some_and(|l| l as usize > MAX_LOGO_BYTES)
    {
        return None;
    }
    let mut bytes = Vec::new();
    let mut res = res;
    while let Some(chunk) = res.chunk().await.ok()? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_LOGO_BYTES {
            return None;
        }
    }
    if bytes.len() < min_bytes {
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
    } else if (content_type.contains("octet-stream") || content_type.is_empty())
        && url.to_ascii_lowercase().contains(".ico")
    {
        "image/x-icon".to_string()
    } else if content_type.is_empty() && url.to_ascii_lowercase().contains(".png") {
        "image/png".to_string()
    } else {
        return None;
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

async fn fetch_html_icon_urls(http: &reqwest::Client, page_url: &str) -> Vec<String> {
    let Ok(res) = http.get(page_url).send().await else {
        return Vec::new();
    };
    if !res.status().is_success() {
        return Vec::new();
    }
    let content_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.contains("text/html") && !content_type.contains("application/xhtml") {
        return Vec::new();
    }

    let final_url = res.url().clone();
    let mut bytes = Vec::new();
    let mut res = res;
    while let Ok(Some(chunk)) = res.chunk().await {
        bytes.extend_from_slice(&chunk);
        if bytes.len() >= MAX_HTML_BYTES {
            break;
        }
    }
    let html = String::from_utf8_lossy(&bytes);
    parse_html_icon_urls(&html, &final_url)
}

fn parse_html_icon_urls(html: &str, base: &reqwest::Url) -> Vec<String> {
    let head_end = html
        .to_ascii_lowercase()
        .find("</head>")
        .unwrap_or_else(|| html.len().min(64 * 1024));
    let head = &html[..head_end];

    let mut ranked: Vec<(u8, String)> = Vec::new();
    for m in RE_LINK_TAG.find_iter(head) {
        let tag = m.as_str();
        let Some(rel) = attr_value(&RE_ATTR_REL, tag) else {
            continue;
        };
        let rel_lower = rel.to_ascii_lowercase();
        let tokens: Vec<&str> = rel_lower.split_whitespace().collect();
        let priority = if tokens.contains(&"apple-touch-icon")
            || tokens.contains(&"apple-touch-icon-precomposed")
        {
            0
        } else if tokens == ["icon"] {
            1
        } else if tokens.contains(&"icon") && !tokens.contains(&"mask-icon") {
            2
        } else {
            continue;
        };

        let Some(href) = attr_value(&RE_ATTR_HREF, tag) else {
            continue;
        };
        let href = href.trim().replace("&amp;", "&");
        if href.is_empty() {
            continue;
        }
        let Ok(resolved) = base.join(&href) else {
            continue;
        };
        if resolved.scheme() != "https" || !resolved.host_str().is_some_and(is_valid_domain) {
            continue;
        }
        let url_str = resolved.to_string();
        if !ranked.iter().any(|(_, u)| u == &url_str) {
            ranked.push((priority, url_str));
        }
    }

    ranked.sort_by_key(|(prio, _)| *prio);
    ranked.into_iter().map(|(_, u)| u).collect()
}

fn attr_value<'a>(re: &Regex, tag: &'a str) -> Option<&'a str> {
    let caps = re.captures(tag)?;
    caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str())
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

    #[test]
    fn only_plain_host_names_are_valid_domains() {
        assert!(super::is_valid_domain("shop.example"));
        assert!(super::is_valid_domain("mail.my-shop.example"));
        for bad in [
            "",
            "localhost",
            "evil.example/x?",
            "a@b.example",
            "127.0.0.1:80",
            "-x.example",
            "x..example",
            "user:pw@x.example",
        ] {
            assert!(!super::is_valid_domain(bad), "{bad}");
        }
    }

    #[test]
    fn parses_html_icon_links_preferring_apple_touch_icon() {
        let html = r##"
        <html><head>
          <link rel="mask-icon" href="/safari-pinned-tab.svg" color="#94c502">
          <link rel="shortcut icon" type="image/x-icon" href="/eportal/img/fav/eop/favicon.ico?v=1">
          <link rel="apple-touch-icon" href="/eportal/img/fav/eop/apple-touch-icon.png?v=2" sizes="180x180">
          <link rel="icon" href="http://insecure.example/icon.png">
        </head></html>
        "##;
        let base = reqwest::Url::parse("https://www.elster.de/eportal/start").unwrap();
        let urls = super::parse_html_icon_urls(html, &base);
        assert_eq!(
            urls,
            vec![
                "https://www.elster.de/eportal/img/fav/eop/apple-touch-icon.png?v=2",
                "https://www.elster.de/eportal/img/fav/eop/favicon.ico?v=1",
            ]
        );
    }
}
