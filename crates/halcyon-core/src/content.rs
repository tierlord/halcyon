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
}

/// Renders a message for display. With `allow_remote`, remote images are
/// loaded (HTTPS only; the user explicitly asked for this message).
pub fn render(content: &MessageContent, allow_remote: bool) -> Rendered {
    match &content.html {
        Some(html) => {
            let (body, remote_blocked) = sanitize_html(html, &content.inline_images, allow_remote);
            Rendered {
                document: wrap_document(&body, allow_remote, false),
                remote_blocked,
                is_html: true,
            }
        }
        None => {
            let body = text_to_html(content.text.as_deref().unwrap_or(""));
            Rendered {
                document: wrap_document(&body, false, true),
                remote_blocked: 0,
                is_html: false,
            }
        }
    }
}

static RE_CID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)cid:([^"'\s>)]+)"#).unwrap());
static RE_CSS_REMOTE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)url\(\s*['"]?\s*(https?:)?//"#).unwrap());
static RE_CSS_IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)@import").unwrap());

/// Cleans untrusted HTML. Returns the cleaned body and the number of blocked
/// remote resources.
pub fn sanitize_html(
    html: &str,
    inline_images: &HashMap<String, String>,
    allow_remote: bool,
) -> (String, usize) {
    let blocked = Arc::new(AtomicUsize::new(0));
    // Remote CSS (url() in style attributes and <style> blocks) is blocked by
    // the document's CSP; it is only counted here so the UI can offer to
    // load it.
    if !allow_remote {
        blocked.fetch_add(RE_CSS_REMOTE_URL.find_iter(html).count(), Ordering::Relaxed);
    }

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
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            filter_attribute(
                element,
                attribute,
                value,
                &inline,
                allow_remote,
                &filter_blocked,
            )
        });
    let body = builder.clean(html).to_string();
    // @import inside <style> could load further CSS; CSP blocks it, but drop
    // it anyway rather than rely on a single layer.
    let body = RE_CSS_IMPORT
        .replace_all(&body, "/* import removed */")
        .into_owned();
    (body, blocked.load(Ordering::Relaxed))
}

fn filter_attribute<'u>(
    element: &str,
    attribute: &str,
    value: &'u str,
    inline: &HashMap<String, String>,
    allow_remote: bool,
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
        (_, "style") => {
            // Expressions and bindings are long dead in browsers, but cost
            // nothing to remove.
            if lower.contains("expression(")
                || lower.contains("-moz-binding")
                || lower.contains("behavior:")
            {
                None
            } else {
                Some(Cow::Borrowed(value))
            }
        }
        _ => Some(Cow::Borrowed(value)),
    }
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

fn wrap_document(body: &str, allow_remote: bool, plain: bool) -> String {
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
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <style>\
         html{{color-scheme:light;}}\
         body{{margin:16px;{font}color:#15110e;background:#fdfdfe;overflow-wrap:anywhere;}}\
         img{{max-width:100%;height:auto;}}\
         a{{color:#3c6b74;}}\
         .plain div{{white-space:pre-wrap;}}\
         .q1{{color:#3c6b74;border-left:3px solid #1dc6ee;padding-left:8px;}}\
         .q2{{color:#6b6460;border-left:3px solid #f79d65;padding-left:8px;}}\
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
        let doc = render(&content, false).document;
        assert!(doc.contains("script-src 'none'"));
        assert!(doc.contains("img-src data:;"));
        let doc = render(&content, true).document;
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

        let rendered = render(&content, false);
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
}
