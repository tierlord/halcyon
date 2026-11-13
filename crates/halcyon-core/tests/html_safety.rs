use std::collections::HashMap;
use std::time::Instant;

use halcyon_core::content::{parse, render, sanitize_html};
use scraper::{Html, Selector};

const FORBIDDEN_TAGS: &[&str] = &[
    "script", "iframe", "frame", "frameset", "object", "embed", "applet", "meta", "base", "link",
    "form", "input", "button", "noscript", "template", "svg", "math",
];

/// Parses the sanitized HTML fragment with html5ever (`scraper`) and verifies
/// that no executable constructs, forbidden tags, dangerous URLs, or unblocked
/// remote resources remain.
fn assert_html_is_safe(name: &str, raw_html: &str) -> usize {
    let (body, blocked) = sanitize_html(raw_html, &HashMap::new(), false);
    let doc = Html::parse_fragment(&body);
    let all_sel = Selector::parse("*").unwrap();

    for el in doc.select(&all_sel) {
        let tag = el.value().name().to_ascii_lowercase();
        assert!(
            !FORBIDDEN_TAGS.contains(&tag.as_str()),
            "[{name}] forbidden tag <{tag}> survived in:\n{body}"
        );

        for (attr_name, attr_val) in el.value().attrs() {
            let attr = attr_name.to_ascii_lowercase();
            assert!(
                !attr.starts_with("on"),
                "[{name}] event handler {attr}={attr_val:?} survived on <{tag}> in:\n{body}"
            );
            assert!(
                !matches!(attr.as_str(), "ping" | "formaction"),
                "[{name}] forbidden attribute {attr} survived on <{tag}> in:\n{body}"
            );

            if matches!(
                attr.as_str(),
                "href" | "src" | "srcset" | "poster" | "background" | "action" | "data"
            ) {
                let normalized: String = attr_val
                    .chars()
                    .filter(|c| !c.is_control() && !c.is_whitespace())
                    .collect::<String>()
                    .to_ascii_lowercase();

                assert!(
                    !normalized.starts_with("javascript:")
                        && !normalized.starts_with("vbscript:")
                        && !normalized.starts_with("data:text/"),
                    "[{name}] dangerous URL in {attr}={attr_val:?} survived in:\n{body}"
                );

                if matches!(attr.as_str(), "src" | "srcset" | "poster" | "background") {
                    assert!(
                        !normalized.contains("http://")
                            && !normalized.contains("https://")
                            && !normalized.starts_with("//"),
                        "[{name}] remote resource in {attr}={attr_val:?} survived with allow_remote=false in:\n{body}"
                    );
                }
            }

            if attr == "style" {
                assert_css_is_safe(name, attr_val, &body);
            }
        }

        if tag == "style" {
            let css: String = el.text().collect();
            assert_css_is_safe(name, &css, &body);
        }
    }

    // Layer 2: verify CSP in full rendered document.
    let raw_mail = format!(
        "From: sender@mail.example\r\nTo: max@mail.example\r\nSubject: Test\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{raw_html}"
    );
    let rendered = render(&parse(raw_mail.as_bytes()), false);
    assert!(
        rendered.document.contains("script-src 'none'"),
        "[{name}] rendered document missing script-src 'none'"
    );
    assert!(
        rendered.document.contains("img-src data:;"),
        "[{name}] rendered document missing img-src data:;"
    );

    blocked
}

fn assert_css_is_safe(name: &str, css: &str, full_body: &str) {
    let deobfuscated = deobfuscate_css(css);
    assert!(
        !deobfuscated.contains("@import"),
        "[{name}] CSS @import survived in:\n{full_body}"
    );
    assert!(
        !deobfuscated.contains("expression("),
        "[{name}] CSS expression() survived in:\n{full_body}"
    );
    assert!(
        !deobfuscated.contains("-moz-binding"),
        "[{name}] CSS -moz-binding survived in:\n{full_body}"
    );
    assert!(
        !deobfuscated.contains("behavior:"),
        "[{name}] CSS behavior: survived in:\n{full_body}"
    );
    assert!(
        !deobfuscated.contains("url(")
            || (!deobfuscated.contains("http://") && !deobfuscated.contains("https://")),
        "[{name}] remote CSS url() survived with allow_remote=false in:\n{full_body}"
    );
}

/// Decodes simple CSS hex escapes (`\75` -> `u`) and strips CSS comments (`/*...*/`)
/// and whitespace so obfuscated CSS like `\75\72\6c(...)` or `u/**/rl(...)` is caught.
fn deobfuscate_css(css: &str) -> String {
    let mut no_comments = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        no_comments.push_str(&rest[..start]);
        if let Some(end) = rest[start + 2..].find("*/") {
            rest = &rest[start + 2 + end + 2..];
        } else {
            rest = "";
            break;
        }
    }
    no_comments.push_str(rest);

    let mut out = String::with_capacity(no_comments.len());
    let mut chars = no_comments.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut hex = String::new();
            while hex.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                hex.push(chars.next().unwrap());
            }
            if !hex.is_empty() {
                if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                    chars.next();
                }
                if let Ok(code) = u32::from_str_radix(&hex, 16)
                    && let Some(decoded) = char::from_u32(code)
                {
                    out.push(decoded);
                    continue;
                }
            }
        }
        if !ch.is_ascii_whitespace() {
            out.push(ch);
        }
    }
    out.to_ascii_lowercase()
}

macro_rules! fixture_test {
    ($fn_name:ident, $file:literal) => {
        #[test]
        fn $fn_name() {
            assert_html_is_safe($file, include_str!(concat!("fixtures/html/", $file)));
        }
    };
}

// <script> variants
fixture_test!(script_basic, "script-basic.html");
fixture_test!(script_uppercase, "script-uppercase.html");
fixture_test!(script_nested, "script-nested.html");
fixture_test!(script_in_svg, "script-in-svg.html");
fixture_test!(script_in_math, "script-in-math.html");
fixture_test!(script_after_comment, "script-after-comment.html");
fixture_test!(
    script_in_noscript_template,
    "script-in-noscript-template.html"
);

// Event handlers
fixture_test!(event_img_onerror, "event-img-onerror.html");
fixture_test!(event_body_onload, "event-body-onload.html");
fixture_test!(event_uppercase_unquoted, "event-uppercase-unquoted.html");
fixture_test!(event_autofocus_onfocus, "event-autofocus-onfocus.html");
fixture_test!(event_svg_onload, "event-svg-onload.html");
fixture_test!(event_details_ontoggle, "event-details-ontoggle.html");

// Dangerous URLs
fixture_test!(href_javascript, "href-javascript.html");
fixture_test!(href_javascript_uppercase, "href-javascript-uppercase.html");
fixture_test!(href_javascript_entity, "href-javascript-entity.html");
fixture_test!(href_javascript_tab, "href-javascript-tab.html");
fixture_test!(
    href_javascript_leading_space,
    "href-javascript-leading-space.html"
);
fixture_test!(href_vbscript, "href-vbscript.html");
fixture_test!(src_data_text_html, "src-data-text-html.html");

// Forbidden tags & active elements
fixture_test!(tag_meta_refresh, "tag-meta-refresh.html");
fixture_test!(tag_base_href, "tag-base-href.html");
fixture_test!(tag_link_stylesheet, "tag-link-stylesheet.html");
fixture_test!(tag_iframe, "tag-iframe.html");
fixture_test!(tag_frame_frameset, "tag-frame-frameset.html");
fixture_test!(tag_object_embed_applet, "tag-object-embed-applet.html");
fixture_test!(tag_form_input, "tag-form-input.html");
fixture_test!(tag_button_formaction, "tag-button-formaction.html");
fixture_test!(tag_a_ping, "tag-a-ping.html");

// CSS attacks
fixture_test!(css_expression, "css-expression.html");
fixture_test!(css_moz_binding, "css-moz-binding.html");
fixture_test!(css_behavior, "css-behavior.html");

#[test]
fn css_import_variants() {
    assert_html_is_safe(
        "css-import-variants.html",
        include_str!("fixtures/html/css-import-variants.html"),
    );
}

#[test]
fn css_url_escaped() {
    assert_html_is_safe(
        "css-url-escaped.html",
        include_str!("fixtures/html/css-url-escaped.html"),
    );
}

fixture_test!(css_style_breakout, "css-style-breakout.html");

// Mutation XSS
fixture_test!(mxss_svg_style, "mxss-svg-style.html");
fixture_test!(mxss_math_mglyph, "mxss-math-mglyph.html");
fixture_test!(mxss_noscript_title, "mxss-noscript-title.html");

// Tracking pixels & remote resources (must also report blocked > 0)
#[test]
fn track_img_blocked() {
    let blocked = assert_html_is_safe(
        "track-img.html",
        include_str!("fixtures/html/track-img.html"),
    );
    assert!(blocked > 0, "expected blocked > 0 for track-img.html");
}

#[test]
fn track_srcset_blocked() {
    let blocked = assert_html_is_safe(
        "track-srcset.html",
        include_str!("fixtures/html/track-srcset.html"),
    );
    assert!(blocked > 0, "expected blocked > 0 for track-srcset.html");
}

#[test]
fn track_table_background_blocked() {
    let blocked = assert_html_is_safe(
        "track-table-background.html",
        include_str!("fixtures/html/track-table-background.html"),
    );
    assert!(
        blocked > 0,
        "expected blocked > 0 for track-table-background.html"
    );
}

#[test]
fn track_style_attr_url_blocked() {
    let blocked = assert_html_is_safe(
        "track-style-attr-url.html",
        include_str!("fixtures/html/track-style-attr-url.html"),
    );
    assert!(
        blocked > 0,
        "expected blocked > 0 for track-style-attr-url.html"
    );
}

#[test]
fn track_style_block_url_blocked() {
    let blocked = assert_html_is_safe(
        "track-style-block-url.html",
        include_str!("fixtures/html/track-style-block-url.html"),
    );
    assert!(
        blocked > 0,
        "expected blocked > 0 for track-style-block-url.html"
    );
}

#[test]
fn track_picture_video_poster_blocked() {
    let blocked = assert_html_is_safe(
        "track-picture-video-poster.html",
        include_str!("fixtures/html/track-picture-video-poster.html"),
    );
    assert!(
        blocked > 0,
        "expected blocked > 0 for track-picture-video-poster.html"
    );
}

// Robustness (must not panic and must complete in < 2s)
#[test]
fn robust_deep_nesting_completes_fast() {
    let start = Instant::now();
    assert_html_is_safe(
        "robust-deep-nesting.html",
        include_str!("fixtures/html/robust-deep-nesting.html"),
    );
    assert!(
        start.elapsed().as_secs_f32() < 2.0,
        "deep nesting took {:?}",
        start.elapsed()
    );
}

fixture_test!(robust_long_attribute, "robust-long-attribute.html");
fixture_test!(robust_broken_html, "robust-broken-html.html");
fixture_test!(robust_nul_bytes, "robust-nul-bytes.html");
