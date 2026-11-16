//! Message content: splitting a raw message into text, HTML and attachments,
//! and turning it into a document that is safe to display.
//!
//! Safety comes in layers, so that one mistake is not enough for an attack:
//!
//! 1. [`render`] cleans HTML with an allow-list (ammonia): no scripts, no
//!    event handlers, no forms, frames, objects or `javascript:` links.
//! 2. The returned document carries a Content-Security-Policy that forbids
//!    scripts entirely and blocks remote resources unless the user allowed
//!    them for this message.
//! 3. The front end shows the document in a sandboxed iframe without
//!    `allow-scripts`.
//!
//! Remote images are blocked by default because tracking pixels tell the
//! sender when and where a message was read.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use base64::Engine;
use mail_parser::{MessageParser, PartKind};
use regex::Regex;
use serde::Serialize;

/// Inline images larger than this are not embedded into the document.
const MAX_INLINE_IMAGE: usize = 5 * 1024 * 1024;

/// What a message consists of, without rendering.
#[derive(Debug, Default)]
pub struct MessageContent {
    /// The plain-text body, if the message has one (or the text derived from
    /// HTML, so search and classification always have text).
    pub text: Option<String>,
    /// The original HTML body, if the message has one.
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
    /// `cid:` → `data:` URI for images referenced by the HTML body.
    inline_images: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Attachment {
    /// Index of the MIME part, used to fetch the content later.
    pub part: u32,
    pub name: String,
    pub mime_type: String,
    pub size: usize,
    /// Shown inside the HTML body rather than as a separate file.
    pub inline: bool,
}

/// Splits a raw RFC 5322 message into its parts.
pub fn parse(raw: &[u8]) -> MessageContent {
    let Some(msg) = MessageParser::new().parse(raw) else {
        return MessageContent::default();
    };

    let html = msg
        .html_body()
        .find(|p| matches!(p.kind(), PartKind::Html))
        .and_then(|p| p.text().map(Cow::into_owned));
    // mail-parser derives text from HTML when there is no text part, which
    // is what search and classification want.
    let text = msg.body_text(0).map(Cow::into_owned);

    let referenced_cids: Vec<String> = html
        .as_deref()
        .map(|h| {
            RE_CID
                .captures_iter(h)
                .map(|c| c[1].to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();

    let mut attachments = Vec::new();
    let mut inline_images = HashMap::new();
    for part in msg.attachments() {
        let mime_type = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".into())
            .to_ascii_lowercase();
        let cid = part
            .content_id()
            .map(|c| c.trim_matches(['<', '>']).to_ascii_lowercase());
        let size = part.decoded_len();
        let inline = mime_type.starts_with("image/")
            && cid.as_ref().is_some_and(|c| referenced_cids.contains(c));
        if inline
            && size <= MAX_INLINE_IMAGE
            && let Some(cid) = &cid
        {
            let data = base64::engine::general_purpose::STANDARD.encode(part.decoded());
            inline_images.insert(cid.clone(), format!("data:{mime_type};base64,{data}"));
        }
        attachments.push(Attachment {
            part: part.id(),
            name: part
                .attachment_name()
                .map(safe_file_name)
                .unwrap_or_else(|| default_name(&mime_type)),
            mime_type,
            size,
            inline,
        });
    }

    MessageContent {
        text,
        html,
        attachments,
        inline_images,
    }
}

/// Returns the decoded bytes of one attachment.
pub fn attachment_data(raw: &[u8], part: u32) -> Option<Vec<u8>> {
    let msg = MessageParser::new().parse(raw)?;
    let part = msg.part(part)?;
    Some(part.decoded().into_owned())
}

/// Makes an attachment name safe to use as a file name: no directories, no
/// control characters, no hidden or empty names.
pub fn safe_file_name(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = leaf
        .chars()
        .map(|c| match c {
            c if c.is_control() => '_',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "anhang".into()
    } else {
        cleaned.chars().take(200).collect()
    }
}

fn default_name(mime_type: &str) -> String {
    let ext = mime_type.rsplit('/').next().unwrap_or("bin");
    format!("anhang.{ext}")
}

/// A document ready for a sandboxed iframe (`srcdoc`).
#[derive(Clone, Debug, Serialize)]
pub struct Rendered {
    pub document: String,
    /// Remote images or styles that were blocked.
    pub remote_blocked: usize,
    pub is_html: bool,
    /// The document is shown on a dark page (see [`render`]).
    pub dark: bool,
}

/// Renders a message for display. With `allow_remote`, remote images are
/// loaded (HTTPS only; the user explicitly asked for this message).
///
/// With `dark` (the app is in dark mode), plain-text mails and HTML mails
/// that bring their own dark design (`prefers-color-scheme: dark`) are shown
/// dark. All other HTML mails stay on a light page, the way they were
/// designed, and their dark-mode rules are switched off – otherwise a mail
/// may turn its text light while its backgrounds stay light.
pub fn render(content: &MessageContent, allow_remote: bool, dark: bool) -> Rendered {
    match &content.html {
        Some(html) => {
            let dark = dark && supports_dark_mode(html);
            let (body, remote_blocked) =
                sanitize(html, &content.inline_images, allow_remote, !dark);
            Rendered {
                document: wrap_document(&body, allow_remote, false, dark),
                remote_blocked,
                is_html: true,
                dark,
            }
        }
        None => {
            let body = text_to_html(content.text.as_deref().unwrap_or(""));
            Rendered {
                document: wrap_document(&body, false, true, dark),
                remote_blocked: 0,
                is_html: false,
                dark,
            }
        }
    }
}

static RE_DARK_SUPPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)prefers-color-scheme\s*:\s*dark|light-dark\s*\("#).unwrap());
// The `color-scheme` property (not the media feature): a mail declaring
// `light dark` would otherwise switch `light-dark()` colours to dark.
static RE_COLOR_SCHEME_PROP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|[^-\w])color-scheme\s*:\s*[^;}]+").unwrap());
static RE_DARK_QUERY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)prefers-color-scheme\s*:\s*dark").unwrap());
// `class` cannot pass ammonia's generic allow-list, so it travels under a
// private name and is restored afterwards. Mail styles mostly target classes.
static RE_CLASS_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\s)class(\s*=)").unwrap());
const CLASS_CARRIER: &str = "data-hc";

/// Whether a mail declares its own dark design.
pub fn supports_dark_mode(html: &str) -> bool {
    RE_DARK_SUPPORT.is_match(html)
}

static RE_CID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)cid:([^"'\s>)]+)"#).unwrap());
static RE_CSS_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)@import[^;{}]*;?").unwrap());
static RE_CSS_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)url\(\s*(?:"([^"]*)"|'([^']*)'|([^)]*))\s*\)"#).unwrap());
static RE_CSS_LEGACY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(expression\s*\(|-moz-binding|behavior\s*:|javascript:|vbscript:)").unwrap()
});
static RE_STYLE_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)(<style[^>]*>)(.*?)(</style>)").unwrap());
// Remote images in attributes the allow-list drops anyway; counted so the
// UI can offer to load them.
static RE_REMOTE_DROPPED_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\s(?:srcset|poster)\s*=\s*["']?[^"'>]*(?:https?:)?//"#).unwrap()
});
static RE_ANY_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static RE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(/?)([a-zA-Z][a-zA-Z0-9]*)[^>]*?(/?)>").unwrap());

/// Documents nested deeper than this are shown as text: no real mail needs
/// it, and HTML tree building gets very slow on such input (a cheap way to
/// freeze a mail reader).
const MAX_NESTING: usize = 256;
/// HTML larger than this is shown as text for the same reason.
const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr", "param",
];

/// Rough maximum nesting depth from a scan of the tags, without parsing.
fn nesting_depth(html: &str) -> usize {
    let (mut depth, mut max) = (0usize, 0usize);
    for cap in RE_TAG.captures_iter(html) {
        let name = cap[2].to_ascii_lowercase();
        if VOID_ELEMENTS.contains(&name.as_str()) || !cap[3].is_empty() {
            continue;
        }
        if cap[1].is_empty() {
            depth += 1;
            max = max.max(depth);
        } else {
            depth = depth.saturating_sub(1);
        }
    }
    max
}

/// Cleans untrusted HTML. Returns the cleaned body and the number of blocked
/// remote resources.
pub fn sanitize_html(
    html: &str,
    inline_images: &HashMap<String, String>,
    allow_remote: bool,
) -> (String, usize) {
    sanitize(html, inline_images, allow_remote, true)
}

fn sanitize(
    html: &str,
    inline_images: &HashMap<String, String>,
    allow_remote: bool,
    force_light: bool,
) -> (String, usize) {
    if html.len() > MAX_HTML_BYTES || nesting_depth(html) > MAX_NESTING {
        // Strip tags without building a tree: building it is the slow part.
        let text = RE_ANY_TAG.replace_all(html, " ");
        return (text_to_html(&decode_entities(&text)), 0);
    }

    let blocked = Arc::new(AtomicUsize::new(0));
    if !allow_remote {
        blocked.fetch_add(
            RE_REMOTE_DROPPED_ATTR.find_iter(html).count(),
            Ordering::Relaxed,
        );
    }

    let html = RE_CLASS_ATTR.replace_all(html, format!("${{1}}{CLASS_CARRIER}${{2}}"));
    let filter_blocked = blocked.clone();
    let inline = Arc::new(inline_images.clone());
    let mut builder = ammonia::Builder::default();
    builder
        .add_tags([
            "style", "font", "center", "big", "section", "main", "address", "picture",
        ])
        .rm_clean_content_tags(["style"])
        .add_clean_content_tags(["title", "noscript", "template", "textarea", "select"])
        .add_generic_attributes([
            "style", "align", "valign", "width", "height", "bgcolor", "color", "dir", "border",
        ])
        .add_tag_attributes("img", ["src", "alt", "width", "height", "border"])
        .add_tag_attributes("font", ["face", "size", "color"])
        .add_tag_attributes(
            "table",
            ["cellpadding", "cellspacing", "border", "background", "role"],
        )
        .add_tag_attributes("td", ["colspan", "rowspan", "nowrap", "background"])
        .add_tag_attributes("th", ["colspan", "rowspan", "nowrap", "background"])
        .url_schemes(["http", "https", "mailto", "cid", "data"].into())
        .url_relative(ammonia::UrlRelative::Deny)
        .add_generic_attribute_prefixes([CLASS_CARRIER])
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            filter_attribute(
                element,
                attribute,
                value,
                &inline,
                allow_remote,
                force_light,
                &filter_blocked,
            )
        });
    let body = builder.clean(&html).to_string();
    let body = body.replace(&format!(" {CLASS_CARRIER}=\""), " class=\"");
    // <style> blocks pass the allow-list unchanged; clean their CSS the same
    // way as style attributes.
    let body = RE_STYLE_BLOCK
        .replace_all(&body, |c: &regex::Captures<'_>| {
            format!(
                "{}{}{}",
                &c[1],
                clean_css(&c[2], allow_remote, force_light, &blocked),
                &c[3]
            )
        })
        .into_owned();
    (body, blocked.load(Ordering::Relaxed))
}

/// Normalises untrusted CSS: comments and escapes are resolved first (so
/// `\75\72\6c(` and `u/**/rl(` cannot hide a `url(`), then imports and
/// legacy script hooks are removed and remote `url()`s are blocked unless
/// allowed. The CSP would stop the loads anyway; this keeps the first layer
/// honest and lets the UI count what was blocked.
fn clean_css(css: &str, allow_remote: bool, force_light: bool, blocked: &AtomicUsize) -> String {
    let plain = decode_css_escapes(&strip_css_comments(css));
    // An unknown value makes the media query false, so dark rules never apply.
    let plain = if force_light {
        let plain = RE_DARK_QUERY.replace_all(&plain, "prefers-color-scheme: halcyon-light-page");
        RE_COLOR_SCHEME_PROP
            .replace_all(&plain, "${1}color-scheme: light")
            .into_owned()
    } else {
        plain
    };
    if !allow_remote {
        let imports = RE_CSS_IMPORT
            .find_iter(&plain)
            .filter(|m| m.as_str().contains("//"))
            .count();
        blocked.fetch_add(imports, Ordering::Relaxed);
    }
    // Imports are never allowed, also not with remote content enabled: they
    // pull in whole style sheets nobody checked.
    let plain = RE_CSS_IMPORT.replace_all(&plain, "");
    let plain = RE_CSS_LEGACY.replace_all(&plain, "blocked-");
    let plain = RE_CSS_URL.replace_all(&plain, |c: &regex::Captures<'_>| {
        let target = c
            .get(1)
            .or(c.get(2))
            .or(c.get(3))
            .map(|m| m.as_str().trim())
            .unwrap_or("");
        let lower = target.to_ascii_lowercase();
        if lower.starts_with("data:image/") || (allow_remote && lower.starts_with("https://")) {
            c[0].to_string()
        } else {
            if lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("//")
            {
                blocked.fetch_add(1, Ordering::Relaxed);
            }
            "none".to_string()
        }
    });
    // Decoded escapes must not be able to close the <style> element.
    plain.replace('<', "\\3c ")
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Resolves CSS escapes: `\75` (hex, optionally followed by one space) and
/// `\x` for any other character.
fn decode_css_escapes(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            hex.push(chars.next().unwrap());
        }
        if hex.is_empty() {
            match chars.next() {
                Some('\n') | None => {}
                Some(other) => out.push(other),
            }
            continue;
        }
        if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            chars.next();
        }
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some('\0') | None => out.push('\u{fffd}'),
            Some(ch) => out.push(ch),
        }
    }
    out
}

/// Turns the few entities ammonia's text output contains back into text.
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
}

fn filter_attribute<'u>(
    element: &str,
    attribute: &str,
    value: &'u str,
    inline: &HashMap<String, String>,
    allow_remote: bool,
    force_light: bool,
    blocked: &AtomicUsize,
) -> Option<Cow<'u, str>> {
    let lower = value.trim().to_ascii_lowercase();
    match (element, attribute) {
        ("img", "src") => {
            if let Some(cid) = lower.strip_prefix("cid:") {
                return inline.get(cid).map(|d| Cow::Owned(d.clone()));
            }
            if lower.starts_with("data:image/") {
                return Some(Cow::Borrowed(value));
            }
            if lower.starts_with("https://") || lower.starts_with("http://") {
                if allow_remote && lower.starts_with("https://") {
                    return Some(Cow::Borrowed(value));
                }
                blocked.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            None
        }
        (_, "background") => {
            if !allow_remote && (lower.starts_with("http") || lower.starts_with("//")) {
                blocked.fetch_add(1, Ordering::Relaxed);
            }
            // Old-style background images; CSS covers the same.
            None
        }
        ("a", "href") => {
            if lower.starts_with("https://")
                || lower.starts_with("http://")
                || lower.starts_with("mailto:")
            {
                Some(Cow::Borrowed(value))
            } else {
                None
            }
        }
        // Old `bgcolor` attributes ignore the mail's own dark-mode CSS and leave
        // light bands around dark content; on a dark page that CSS decides.
        (_, "bgcolor") if !force_light => None,
        (_, "style") => {
            let mut css = clean_css(value, allow_remote, force_light, blocked);
            if !force_light && has_white_bg_without_color(&css) {
                if !css.trim_end().ends_with(';') {
                    css.push(';');
                }
                css.push_str("color:#15110e");
            }
            Some(Cow::Owned(css))
        }
        _ => Some(Cow::Borrowed(value)),
    }
}

static RE_WHITE_BG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|;)\s*background(?:-color)?\s*:\s*(?:#fff(?:fff)?|white|rgb\(\s*255\s*,\s*255\s*,\s*255\s*\))\s*(?:!important)?\s*(?:;|$)").unwrap()
});
static RE_TEXT_COLOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|;)\s*color\s*:").unwrap());

fn has_white_bg_without_color(css: &str) -> bool {
    RE_WHITE_BG.is_match(css) && !RE_TEXT_COLOR.is_match(css)
}

static RE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(https?://[^\s<>"']+[^\s<>"'.,;:!?)\]]|mailto:[^\s<>"']+[^\s<>"'.,;:!?)\]])"#,
    )
    .unwrap()
});

/// Plain text as HTML: escaped, links clickable, quoted lines marked.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str("<div class=\"plain\">");
    for line in text.lines() {
        let depth = line
            .chars()
            .take_while(|c| *c == '>' || *c == ' ')
            .filter(|c| *c == '>')
            .count();
        let class = match depth {
            0 => "",
            1 => " class=\"q1\"",
            _ => " class=\"q2\"",
        };
        out.push_str(&format!("<div{class}>"));
        let mut last = 0;
        for m in RE_LINK.find_iter(line) {
            out.push_str(&escape(&line[last..m.start()]));
            let url = escape(m.as_str());
            out.push_str(&format!(
                "<a href=\"{url}\" rel=\"noopener noreferrer\">{url}</a>"
            ));
            last = m.end();
        }
        out.push_str(&escape(&line[last..]));
        if line.is_empty() {
            out.push_str("<br>");
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn wrap_document(body: &str, allow_remote: bool, plain: bool, dark: bool) -> String {
    let img_src = if allow_remote {
        "data: https:"
    } else {
        "data:"
    };
    let csp = format!(
        "default-src 'none'; img-src {img_src}; style-src 'unsafe-inline'; font-src data:; \
         script-src 'none'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'"
    );
    let font = if plain {
        "font: 14px/1.5 ui-monospace, 'DejaVu Sans Mono', monospace;"
    } else {
        "font: 14px/1.5 system-ui, sans-serif;"
    };
    // Dark page: Pitch Black lifted a little so mail backgrounds still show.
    let (scheme, fg, bg, link, quote) = if dark {
        ("dark", "#f7f0f0", "#1d1b1a", "#4cb8d4", "#9fb6c0")
    } else {
        ("light", "#15110e", "#ffffff", "#3b5966", "#3b5966")
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <style>\
         html{{color-scheme:{scheme};}}\
         body{{margin:16px;{font}color:{fg};background:{bg};overflow-wrap:anywhere;}}\
         img{{max-width:100%;height:auto;}}\
         a{{color:{link};}}\
         .plain div{{white-space:pre-wrap;}}\
         .q1{{color:{quote};border-left:3px solid #1698b5;padding-left:8px;}}\
         .q2{{color:#6b6460;border-left:3px solid #df7618;padding-left:8px;}}\
         </style></head><body>{body}</body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> (String, usize) {
        sanitize_html(html, &HashMap::new(), false)
    }

    #[test]
    fn removes_scripts_handlers_and_dangerous_links() {
        let (out, _) = clean(
            r#"<p onclick="x()">Hi<script>alert(1)</script></p>
               <img src="x" onerror="alert(1)">
               <a href="javascript:alert(1)">a</a>
               <a href="https://example.org/">b</a>
               <iframe src="https://example.org"></iframe>
               <form action="https://evil.example"><input name=pw></form>
               <svg><script>alert(1)</script></svg>
               <object data="x"></object>"#,
        );
        let lower = out.to_lowercase();
        for bad in [
            "<script",
            "onclick",
            "onerror",
            "javascript:",
            "<iframe",
            "<form",
            "<input",
            "<svg",
            "<object",
        ] {
            assert!(!lower.contains(bad), "{bad} survived: {out}");
        }
        assert!(out.contains(r#"href="https://example.org/""#));
    }

    #[test]
    fn blocks_remote_images_but_keeps_inline_ones() {
        let mut inline = HashMap::new();
        inline.insert(
            "logo@x".to_string(),
            "data:image/png;base64,AAAA".to_string(),
        );
        let html = r#"<img src="https://tracker.example/p.gif"><img src="cid:logo@x"><img src="http://x.example/a.png">"#;
        let (out, blocked) = sanitize_html(html, &inline, false);
        assert_eq!(blocked, 2);
        assert!(!out.contains("tracker.example"));
        assert!(out.contains("data:image/png;base64,AAAA"));

        let (out, blocked) = sanitize_html(html, &inline, true);
        assert_eq!(blocked, 1, "plain http stays blocked even when allowed");
        assert!(out.contains("https://tracker.example/p.gif"));
    }

    #[test]
    fn keeps_newsletter_styling_but_counts_remote_css() {
        let (out, blocked) = clean(
            r##"<style>.x{color:red} @import url(https://evil.example/a.css);</style>
               <table bgcolor="#fff" width="600"><tr><td style="background:url('https://t.example/bg.png')">x</td></tr></table>"##,
        );
        assert!(out.contains("<style>"));
        assert!(out.contains(".x{color:red}"));
        assert!(!out.to_lowercase().contains("@import"));
        assert!(out.contains(r#"width="600""#));
        assert_eq!(blocked, 2);
    }

    #[test]
    fn document_forbids_scripts_and_remote_loads() {
        let content = MessageContent {
            html: Some("<p>Hallo</p>".into()),
            ..Default::default()
        };
        let doc = render(&content, false, false).document;
        assert!(doc.contains("script-src 'none'"));
        assert!(doc.contains("img-src data:;"));
        let doc = render(&content, true, false).document;
        assert!(doc.contains("img-src data: https:;"));
    }

    #[test]
    fn plain_text_is_escaped_and_linked() {
        let html = text_to_html(
            "Hallo <b>Max</b>,\nsiehe https://example.org/a?b=1.\n> zitiert\n>> tiefer",
        );
        assert!(html.contains("&lt;b&gt;Max&lt;/b&gt;"));
        assert!(html.contains(r#"<a href="https://example.org/a?b=1""#));
        assert!(html.contains(r#"class="q1""#));
        assert!(html.contains(r#"class="q2""#));
    }

    #[test]
    fn parses_multipart_with_inline_image_and_attachment() {
        let raw = concat!(
            "From: a@example.org\r\nSubject: Test\r\nMIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n",
            "--m\r\nContent-Type: multipart/related; boundary=\"r\"\r\n\r\n",
            "--r\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
            "<p>Hallo <img src=\"cid:logo@x\"></p>\r\n",
            "--r\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n",
            "--r--\r\n",
            "--m\r\nContent-Type: application/pdf; name=\"../../Rechnung 11.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"../../Rechnung 11.pdf\"\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n",
            "--m--\r\n",
        );
        let content = parse(raw.as_bytes());
        assert!(content.html.as_deref().unwrap().contains("Hallo"));
        assert!(content.text.as_deref().unwrap().contains("Hallo"));
        assert_eq!(content.attachments.len(), 2);
        let pdf = content.attachments.iter().find(|a| !a.inline).unwrap();
        assert_eq!(pdf.name, "Rechnung 11.pdf");
        assert_eq!(pdf.mime_type, "application/pdf");
        assert_eq!(
            attachment_data(raw.as_bytes(), pdf.part).unwrap(),
            b"%PDF-1.4"
        );

        let rendered = render(&content, false, false);
        assert!(rendered.document.contains("data:image/png;base64,"));
        assert_eq!(rendered.remote_blocked, 0);
    }

    #[test]
    fn file_names_cannot_escape_or_hide() {
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(safe_file_name(".bashrc"), "bashrc");
        assert_eq!(safe_file_name("a\u{0}b\nc.txt"), "a_b_c.txt");
        assert_eq!(safe_file_name("  "), "anhang");
    }

    #[test]
    fn remote_css_follows_the_users_choice() {
        let html = r#"<div style="background:url('https://cdn.example/bg.png')">a</div><style>p{background:\75\72\6c(http://t.example/x.gif)}</style>"#;
        let (out, blocked) = clean(html);
        assert_eq!(blocked, 2);
        assert!(!out.contains("cdn.example") && !out.contains("t.example"));
        let (out, blocked) = sanitize_html(html, &HashMap::new(), true);
        // HTTPS loads once allowed; plain HTTP never.
        assert!(out.contains("https://cdn.example/bg.png"));
        assert!(!out.contains("t.example"));
        assert_eq!(
            blocked, 1,
            "the plain-HTTP load is still counted as blocked"
        );
    }

    #[test]
    fn absurd_nesting_falls_back_to_escaped_text() {
        let html = format!(
            "{}<b onclick=x>Hallo</b>{}",
            "<div>".repeat(1000),
            "</div>".repeat(1000)
        );
        let (out, _) = clean(&html);
        assert!(out.contains("Hallo"));
        assert!(!out.contains("<b") && !out.contains("onclick"));
    }

    #[test]
    fn mail_classes_survive_for_its_styles() {
        let (out, _) =
            clean(r#"<table class="box wide" onclick="x"><tr><td CLASS='c'>a</td></tr></table>"#);
        assert!(out.contains(r#"class="box wide""#), "{out}");
        assert!(out.contains(r#"class="c""#), "{out}");
        assert!(!out.contains("data-hc") && !out.contains("onclick"));
    }

    #[test]
    fn dark_mode_only_for_mails_that_support_it() {
        let styled = MessageContent {
            html: Some(
                "<style>.box{color:#222;background:#eee}@media (prefers-color-scheme: dark){.box{color:#eee!important;background:#222!important}}</style><table class=\"box\" bgcolor=\"#eeeeee\"><tr><td>Hallo</td></tr></table>".into(),
            ),
            ..Default::default()
        };
        let r = render(&styled, false, true);
        assert!(r.dark);
        assert!(r.document.contains("color-scheme:dark"));
        assert!(r.document.contains("prefers-color-scheme: dark"));

        // In light mode the same mail's dark rules are switched off.
        let r = render(&styled, false, false);
        assert!(!r.dark);
        assert!(!r.document.contains("prefers-color-scheme: dark"));

        let declared = MessageContent {
            html: Some("<style>:root{color-scheme: light dark}p{color:light-dark(#111,#eee)}</style><p>x</p>".into()),
            ..Default::default()
        };
        let r = render(&declared, false, false);
        assert!(
            r.document.contains(":root{color-scheme: light}"),
            "{}",
            r.document
        );

        // A mail without dark design stays on a light page, also in dark mode.
        let plain_html = MessageContent {
            html: Some("<p style=\"color:#333\">Hallo</p>".into()),
            ..Default::default()
        };
        let r = render(&plain_html, false, true);
        assert!(!r.dark);
        assert!(r.document.contains("color-scheme:light"));

        // Boilerplate `<meta name="color-scheme" content="light dark">` alone
        // does not mean the mail supports dark mode.
        let meta_only = MessageContent {
            html: Some(
                "<meta name=\"color-scheme\" content=\"light dark\"><td style=\"background-color:#ffffff\">Hallo</td>"
                    .into(),
            ),
            ..Default::default()
        };
        let r = render(&meta_only, false, true);
        assert!(!r.dark);

        // Dark-mode mail with an inline white container still gets dark text
        // if no explicit color was set on that element.
        let mixed = MessageContent {
            html: Some(
                "<style>@media (prefers-color-scheme: dark){.x{color:#eee}}</style><table><tr><td style=\"background-color:#ffffff\">Hallo</td></tr></table>"
                    .into(),
            ),
            ..Default::default()
        };
        let r = render(&mixed, false, true);
        assert!(r.dark);
        assert!(r.document.contains("color:#15110e"), "{}", r.document);

        // Plain text follows the app.
        let text = MessageContent {
            text: Some("Hallo".into()),
            ..Default::default()
        };
        assert!(render(&text, false, true).dark);
    }

    #[test]
    fn dark_page_drops_legacy_background_colors() {
        let c = MessageContent {
            html: Some("<style>@media (prefers-color-scheme: dark){body{background:#111}}</style><table bgcolor=\"#eaeded\"><tr><td>x</td></tr></table>".into()),
            ..Default::default()
        };
        assert!(!render(&c, false, true).document.contains("bgcolor"));
        assert!(render(&c, false, false).document.contains("bgcolor"));
    }
}
