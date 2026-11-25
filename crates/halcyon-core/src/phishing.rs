//! Phishing detection: a weighted, explainable verdict per mail.
//!
//! The question is not "is this spam?" but "does this mail try to look like
//! someone it is not, or hide where it leads?". Each signal adds weight; the
//! sum decides ([`SUSPICIOUS_AT`], [`PHISHING_AT`]). Every signal that fired
//! comes with a short German reason, so the user can judge the verdict.
//!
//! The module is pure: it gets everything it needs in an [`Input`] (the
//! store supplies the context, e.g. which display names were seen before
//! from verified senders) and never touches the network or the database.
//!
//! Weights (sum of the signals that fired):
//!
//! | signal                                                        | weight |
//! |---------------------------------------------------------------|--------|
//! | invisible/bidi characters in the display name                 | 4      |
//! | invisible/bidi characters in the subject                      | 4      |
//! | letters of several alphabets mixed inside one word (name, subject) | 4 |
//! | five or more invisible characters splitting words in the body | 3      |
//! | link: percent-encoded host, IP host, `user@` part, script scheme, mixed-alphabet host | 5 |
//! | link: international (punycode) host                           | 2      |
//! | link text names another address than the link opens           | 2      |
//! | display name claims an organisation known from another domain | 5      |
//! | display name carries an address of another domain             | 4      |
//! | official-sounding name, random-looking local part             | 3      |
//! | short lure: 1-2 links plus money/urgency wording              | 2      |
//! | DMARC not passed, organisation-like display name              | 2      |
//!
//! `score >= 4` is [`Level::Suspicious`], `score >= 9` is
//! [`Level::Phishing`]. So any single strong signal holds a mail back, and
//! it takes several independent ones (the typical lure combines them) to
//! call it phishing. A passed DMARC check never lowers the score: it only
//! proves the mail comes from the domain it names, not that this domain is
//! the organisation the display name claims.
//!
//! People the user writes to are not second-guessed on the weak signals
//! (name, lure shape, link text, DMARC); the strong ones (hidden characters,
//! obfuscated link targets) still count, because they have no innocent use.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::linker::base_domain;

/// From this score on a mail is held back and marked.
pub const SUSPICIOUS_AT: u32 = 4;
/// From this score on the mail is called phishing.
pub const PHISHING_AT: u32 = 9;
/// A display-name word counts as "known" after this many verified mails.
pub const NAME_HISTORY_MIN: u32 = 3;
/// Invisible characters splitting words in a body that make it a signal.
/// Newsletters pad their preview text with invisible characters, but never
/// between the letters of words, so few such runs are harmless.
const BODY_SPLIT_RUNS: usize = 5;
/// A lure is short; longer texts are ordinary mail.
const LURE_MAX_CHARS: usize = 800;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    None,
    Suspicious,
    Phishing,
}

impl Level {
    pub fn as_db(self) -> i64 {
        match self {
            Level::None => 0,
            Level::Suspicious => 1,
            Level::Phishing => 2,
        }
    }

    pub fn from_db(v: i64) -> Self {
        match v {
            1 => Level::Suspicious,
            v if v >= 2 => Level::Phishing,
            _ => Level::None,
        }
    }

    pub fn is_flagged(self) -> bool {
        self != Level::None
    }
}

/// One signal that fired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason {
    /// Stable machine name, for statistics.
    pub code: String,
    /// Short German sentence for the user.
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhishingVerdict {
    pub level: Level,
    pub score: u32,
    /// Empty for [`Level::None`], strongest first otherwise.
    pub reasons: Vec<Reason>,
}

/// What travels with a message to the front end.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhishingInfo {
    pub level: Level,
    pub reasons: Vec<Reason>,
    /// The user said "Ich kenne den Absender" (local only).
    pub acknowledged: bool,
}

impl PhishingInfo {
    /// Whether the mail is kept out of "Für dich", bundles and
    /// notifications. Phishing stays held back whatever the user said; a
    /// merely suspicious mail is released once the user vouched for the sender.
    pub fn holds_back(&self) -> bool {
        match self.level {
            Level::None => false,
            Level::Suspicious => !self.acknowledged,
            Level::Phishing => true,
        }
    }
}

/// A link of the body: where it leads and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub href: String,
    pub text: String,
}

/// A base domain a display-name word was seen with (verified senders only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameSighting {
    pub base_domain: String,
    pub count: u32,
}

/// What the store knows about the mail's surroundings.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The user has written to this address.
    pub known_correspondent: bool,
    /// Part of a conversation (`In-Reply-To` / `References`).
    pub is_reply: bool,
    /// `List-Id` / `List-Unsubscribe`: sent to many.
    pub list_mail: bool,
    /// Per folded display-name word of this mail: where verified mail with
    /// that word came from (see [`learnable_name_tokens`]).
    pub name_history: HashMap<String, Vec<NameSighting>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input<'a> {
    pub display_name: &'a str,
    pub from_addr: &'a str,
    /// The receiving server confirmed the sender domain (DMARC or aligned DKIM).
    pub dmarc_pass: bool,
    pub subject: &'a str,
    pub text: Option<&'a str>,
    pub html: Option<&'a str>,
}

// ----- characters ----------------------------------------------------------------

const ZWNJ: char = '\u{200C}';
const ZWJ: char = '\u{200D}';
const LRM: char = '\u{200E}';
const RLM: char = '\u{200F}';

/// Zero-width, soft-hyphen and bidi control characters: invisible, so they
/// can split a word without the reader noticing while a filter no longer
/// finds it.
pub fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{FEFF}')
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Letters and digits of the Latin script (and ASCII): the only flanks for
/// which an invisible character inside a word is suspicious. In other
/// scripts the joiners have honest uses (Persian, Indic, emoji sequences).
fn is_latin_alnum(c: char) -> bool {
    c.is_alphanumeric() && (c.is_ascii() || (c as u32) < 0x250)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Invisible {
    /// Runs between two Latin letters or digits.
    inside_word: usize,
    /// Runs of bidi controls (spoofing the order of text).
    bidi: usize,
    /// Other runs that have no honest use at the edge of a word.
    stray: usize,
}

impl Invisible {
    fn any(&self) -> bool {
        self.inside_word + self.bidi + self.stray > 0
    }
}

fn invisible_stats(s: &str) -> Invisible {
    let mut st = Invisible::default();
    let mut prev: Option<char> = None;
    let mut run: Vec<char> = Vec::new();
    let mut flush = |prev: Option<char>, next: Option<char>, run: &mut Vec<char>| {
        if run.is_empty() {
            return;
        }
        if run.iter().any(|&c| is_bidi_control(c)) {
            st.bidi += 1;
        } else if prev.is_some_and(is_latin_alnum) && next.is_some_and(is_latin_alnum) {
            st.inside_word += 1;
        } else if run
            .iter()
            .any(|c| !matches!(*c, ZWNJ | ZWJ | LRM | RLM | '\u{00AD}'))
        {
            // Joiners and direction marks next to emoji or non-Latin text
            // are normal; a zero-width space or word joiner never is.
            st.stray += 1;
        }
        run.clear();
    };
    for c in s.chars() {
        if is_invisible(c) {
            run.push(c);
            continue;
        }
        flush(prev, Some(c), &mut run);
        prev = Some(c);
    }
    flush(prev, None, &mut run);
    st
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Other,
}

fn script_of(c: char) -> Script {
    match c as u32 {
        0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x024F => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        _ => Script::Other,
    }
}

/// A word that mixes Latin letters with Cyrillic or Greek ones: the look-alike
/// trick ("раypal").
fn mixes_scripts(word: &str) -> bool {
    let mut latin = false;
    let mut lookalike = false;
    for c in word.chars().filter(|c| c.is_alphabetic()) {
        match script_of(c) {
            Script::Latin => latin = true,
            Script::Cyrillic | Script::Greek => lookalike = true,
            Script::Other => {}
        }
    }
    latin && lookalike
}

fn has_mixed_script_word(s: &str) -> bool {
    s.split(|c: char| !c.is_alphabetic())
        .any(|w| !w.is_empty() && mixes_scripts(w))
}

// ----- normalising ---------------------------------------------------------------

/// Lowercase text without invisible characters and without diacritics, so
/// keyword checks cannot be dodged by splitting a word or by umlauts.
/// (No NFKC without a dependency: the zero-width/soft-hyphen set and the
/// common Latin accents are folded by hand.)
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().filter(|&c| !is_invisible(c)) {
        match c {
            'ß' => out.push_str("ss"),
            'ä' | 'Ä' | 'à' | 'á' | 'â' | 'ã' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Å' => {
                out.push('a')
            }
            'ö' | 'Ö' | 'ò' | 'ó' | 'ô' | 'õ' | 'Ò' | 'Ó' | 'Ô' | 'Õ' => out.push('o'),
            'ü' | 'Ü' | 'ù' | 'ú' | 'û' | 'Ù' | 'Ú' | 'Û' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => out.push('i'),
            'ç' | 'Ç' => out.push('c'),
            'ñ' | 'Ñ' => out.push('n'),
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words of a display name, folded: the units compared with history.
pub fn name_tokens(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in fold(name).split(|c: char| !c.is_alphanumeric()) {
        if t.chars().count() >= 4 && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Words written in capitals (`ELSTER`): how organisations often write their
/// own name, rarely people.
fn all_caps_tokens(name: &str) -> Vec<String> {
    let clean: String = name.chars().filter(|&c| !is_invisible(c)).collect();
    clean
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 4 && w.chars().all(char::is_uppercase))
        .map(fold)
        .collect()
}

/// Words that sound like an office or a service rather than like a name.
/// Long ones also match inside German compounds.
const OFFICIAL_LONG: &[&str] = &[
    "portal",
    "verwaltung",
    "sicherheit",
    "security",
    "kundenservice",
    "kundendienst",
    "kundenportal",
    "abrechnung",
    "erstattung",
    "benachrichtigung",
    "notification",
    "customer",
];
const OFFICIAL_SHORT: &[&str] = &[
    "service", "support", "team", "konto", "account", "billing", "helpdesk", "alert", "info",
];

fn has_official_word(folded_name: &str) -> bool {
    OFFICIAL_LONG.iter().any(|w| folded_name.contains(w))
        || folded_name
            .split(|c: char| !c.is_alphanumeric())
            .any(|t| OFFICIAL_SHORT.contains(&t))
}

fn is_official_token(token: &str) -> bool {
    OFFICIAL_LONG.iter().any(|w| token.contains(w)) || OFFICIAL_SHORT.contains(&token)
}

/// A local part that looks generated: long, with letters and digits
/// alternating, or consonant soup. `anna.schmidt1985` is not random.
fn looks_random(local: &str) -> bool {
    let s: String = local
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if s.len() < 10 {
        return false;
    }
    let digits = s.chars().filter(char::is_ascii_digit).count();
    let letters = s.len() - digits;
    let transitions = s
        .as_bytes()
        .windows(2)
        .filter(|w| w[0].is_ascii_digit() != w[1].is_ascii_digit())
        .count();
    if digits >= 3 && letters >= 3 && transitions >= 3 {
        return true;
    }
    let vowels = s.chars().filter(|c| "aeiou".contains(*c)).count();
    s.len() >= 14 && letters > 0 && (vowels as f32) / (letters as f32) < 0.2
}

/// Whether a display-name word and the sender's registrable domain share a
/// word: the name then belongs to the domain (`Example Shop` / `example.com`).
fn name_matches_domain(tokens: &[String], sender_base: Option<&str>) -> bool {
    let Some(base) = sender_base else {
        return false;
    };
    let label = base.split('.').next().unwrap_or("");
    label.chars().count() >= 4
        && tokens
            .iter()
            .any(|t| t.contains(label) || (t.chars().count() >= 4 && label.contains(t.as_str())))
}

/// Display-name words worth remembering for a *verified* sender, so a later
/// mail using the word from another domain can be recognised. Only words
/// that act like a brand: one that also appears in the sender's own domain,
/// one written in capitals, or the whole name when it is a single word.
/// Plain first names and generic office words are left out.
pub fn learnable_name_tokens(display_name: &str, from_addr: &str) -> Vec<String> {
    let tokens = name_tokens(display_name);
    if tokens.is_empty() {
        return Vec::new();
    }
    let base = base_domain(from_addr);
    let label = base
        .as_deref()
        .and_then(|b| b.split('.').next())
        .filter(|l| l.chars().count() >= 4);
    let caps = all_caps_tokens(display_name);
    let single_word = display_name.split_whitespace().count() == 1;
    tokens
        .into_iter()
        .filter(|t| !is_official_token(t))
        .filter(|t| {
            caps.contains(t)
                || single_word
                || label.is_some_and(|l| t.contains(l) || l.contains(t.as_str()))
        })
        .collect()
}

// ----- HTML helpers --------------------------------------------------------------

static RE_ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<a\b([^>]*)>(.*?)</a\s*>"#).unwrap());
static RE_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap());
static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static RE_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[a-zA-Z]{2,8});").unwrap());
static RE_TEXT_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s<>"')\]]+"#).unwrap());
static RE_SHOWN_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:https?://)?((?:[a-z0-9-]+\.)+[a-z]{2,})(?:[/:?#]\S*)?$").unwrap()
});
static RE_NAME_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[^a-z0-9@.-])((?:[a-z0-9][a-z0-9-]*\.)+(?:com|net|org|de|at|ch|info|eu|io|co|uk|fr|nl|it|es|biz|shop|app|online|site|xyz|top|ru|cn))\b",
    )
    .unwrap()
});
static RE_NAME_MAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[a-z0-9._%+-]+@((?:[a-z0-9-]+\.)+[a-z]{2,})").unwrap());

/// Decodes the entities that matter for text (the usual named ones, numeric
/// ones, and the names of the invisible characters).
pub fn unescape_html(s: &str) -> String {
    RE_ENTITY
        .replace_all(s, |c: &regex::Captures<'_>| {
            let name = &c[1];
            let decoded =
                if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else if let Some(dec) = name.strip_prefix('#') {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    match name {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        "nbsp" => Some(' '),
                        "shy" => Some('\u{00AD}'),
                        "zwnj" => Some(ZWNJ),
                        "zwj" => Some(ZWJ),
                        "lrm" => Some(LRM),
                        "rlm" => Some(RLM),
                        _ => None,
                    }
                };
            decoded.map_or_else(|| c[0].to_string(), String::from)
        })
        .into_owned()
}

fn html_to_text(html: &str) -> String {
    unescape_html(&RE_TAG.replace_all(html, " "))
}

/// Links of a message: anchors of the HTML (with their visible text), else
/// the URLs written in the plain text.
pub fn extract_links(html: Option<&str>, text: Option<&str>) -> Vec<Link> {
    let mut out = Vec::new();
    if let Some(html) = html {
        for a in RE_ANCHOR.captures_iter(html) {
            let Some(h) = RE_HREF.captures(&a[1]) else {
                continue;
            };
            let href = h
                .get(1)
                .or_else(|| h.get(2))
                .or_else(|| h.get(3))
                .map_or("", |m| m.as_str());
            out.push(Link {
                href: unescape_html(href).trim().to_string(),
                text: collapse_ws(&html_to_text(&a[2])),
            });
        }
    }
    if out.is_empty()
        && let Some(text) = text
    {
        for m in RE_TEXT_URL.find_iter(text) {
            let url = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
            out.push(Link {
                href: url.to_string(),
                text: url.to_string(),
            });
        }
    }
    out
}

// ----- links ---------------------------------------------------------------------

/// What is wrong with where a link leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkIssue {
    /// `javascript:`, `data:` and the like.
    ScriptScheme,
    /// `%63` and friends inside the host name: a decoy for readers and filters.
    PercentEncodedHost,
    /// A number instead of a name.
    IpHost,
    /// `https://trusted.example@elsewhere.invalid/`.
    UserInfo,
    /// Letters of several alphabets in one host label.
    Homograph,
    /// An international name (`xn--`) that is not a look-alike.
    Punycode,
}

impl LinkIssue {
    /// Whether this issue has no innocent use (everything but plain IDNs).
    pub fn is_strong(self) -> bool {
        self != LinkIssue::Punycode
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFacts {
    /// The real target host: percent-decoded, lowercase, international names
    /// left in their `xn--` form. Empty for script links.
    pub host: String,
    pub issues: Vec<LinkIssue>,
}

fn hex_val(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// Decodes `%XX`; the flag says whether anything was decoded.
fn percent_decode(s: &str) -> (String, bool) {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut changed = false;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex_val(b[i + 1]), hex_val(b[i + 2]))
        {
            out.push(h * 16 + l);
            changed = true;
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    (String::from_utf8_lossy(&out).into_owned(), changed)
}

/// RFC 3492 decoder, enough to look at the letters behind an `xn--` label.
fn punycode_decode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const T_MIN: u32 = 1;
    const T_MAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;
    let (basic, extended) = match input.rfind('-') {
        Some(p) => (&input[..p], &input[p + 1..]),
        None => ("", input),
    };
    let mut out: Vec<char> = basic.chars().collect();
    let (mut n, mut i, mut bias) = (128u32, 0u32, 72u32);
    let digits: Vec<u32> = extended
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' => Some((b - b'a') as u32),
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'0'..=b'9' => Some((b - b'0') as u32 + 26),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let mut pos = 0;
    while pos < digits.len() {
        let old_i = i;
        let mut w = 1u32;
        let mut k = BASE;
        loop {
            let d = *digits.get(pos)?;
            pos += 1;
            i = i.checked_add(d.checked_mul(w)?)?;
            let t = if k <= bias {
                T_MIN
            } else if k >= bias + T_MAX {
                T_MAX
            } else {
                k - bias
            };
            if d < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k += BASE;
        }
        let len = out.len() as u32 + 1;
        // Bias adaptation (RFC 3492, section 6.1).
        let mut delta = (i - old_i) / if old_i == 0 { DAMP } else { 2 };
        delta += delta / len;
        let mut k2 = 0;
        while delta > ((BASE - T_MIN) * T_MAX) / 2 {
            delta /= BASE - T_MIN;
            k2 += BASE;
        }
        bias = k2 + (BASE - T_MIN + 1) * delta / (delta + SKEW);
        n = n.checked_add(i / len)?;
        i %= len;
        out.insert(i as usize, char::from_u32(n)?);
        i += 1;
    }
    Some(out.into_iter().collect())
}

fn is_numeric_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let numeric = |l: &str| {
        !l.is_empty()
            && (l.chars().all(|c| c.is_ascii_digit())
                || l.strip_prefix("0x")
                    .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit())))
    };
    // Four numbers, or one big number (browsers accept both as an address).
    (labels.len() == 4 && labels.iter().all(|l| numeric(l)))
        || (labels.len() == 1 && numeric(labels[0]))
}

/// Looks at where a link really leads. `None` for links that are no web
/// links (`mailto:`, `tel:`, fragments).
pub fn analyze_link(href: &str) -> Option<LinkFacts> {
    // Browsers drop tabs, newlines and spaces inside a URL; so do we, or
    // `java\tscript:` would slip through.
    let url: String = href
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace() && !is_invisible(*c))
        .collect();
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(i) = lower.find(':')
        && lower[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !lower[..i].is_empty()
    {
        let scheme = &lower[..i];
        match scheme {
            "javascript" | "data" | "vbscript" => {
                return Some(LinkFacts {
                    host: String::new(),
                    issues: vec![LinkIssue::ScriptScheme],
                });
            }
            "http" | "https" => &url[i + 1..],
            _ => return None,
        }
    } else if url.starts_with("//") {
        &url[..]
    } else {
        return None;
    };
    // Browsers treat `\` like `/` in web URLs.
    let after = rest.trim_start_matches(['/', '\\']);
    let authority_end = after.find(['/', '\\', '?', '#']).unwrap_or(after.len());
    let authority = &after[..authority_end];
    let mut issues = Vec::new();
    let host_port = match authority.rfind('@') {
        Some(p) => {
            issues.push(LinkIssue::UserInfo);
            &authority[p + 1..]
        }
        None => authority,
    };
    let (raw_host, ipv6) = if let Some(inner) = host_port.strip_prefix('[') {
        (inner.split(']').next().unwrap_or(""), true)
    } else {
        let no_port = match host_port.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => host_port,
        };
        (no_port, false)
    };
    let (decoded, encoded) = percent_decode(raw_host);
    let host = decoded.trim_end_matches('.').to_lowercase();
    if encoded {
        issues.push(LinkIssue::PercentEncodedHost);
    }
    if ipv6 || is_numeric_host(&host) {
        issues.push(LinkIssue::IpHost);
    }
    let mut idn = false;
    let mut homograph = false;
    for label in host.split('.') {
        let unicode = match label.strip_prefix("xn--") {
            Some(p) => {
                idn = true;
                punycode_decode(p).unwrap_or_default()
            }
            None if !label.is_ascii() => {
                idn = true;
                label.to_string()
            }
            None => continue,
        };
        homograph |= mixes_scripts(&unicode);
    }
    if homograph {
        issues.push(LinkIssue::Homograph);
    } else if idn {
        issues.push(LinkIssue::Punycode);
    }
    Some(LinkFacts { host, issues })
}

/// The address a link text pretends to be (`example.com/login`), if it
/// looks like one.
fn shown_host(text: &str) -> Option<String> {
    let clean: String = text.chars().filter(|&c| !is_invisible(c)).collect();
    RE_SHOWN_HOST
        .captures(clean.trim())
        .map(|c| c[1].to_ascii_lowercase())
}

// ----- the assessment ------------------------------------------------------------

struct Hit {
    code: &'static str,
    text: String,
    weight: u32,
    /// No innocent use: counts even for people the user writes to.
    strong: bool,
}

const LURE_WORDS: &[&str] = &[
    "erstattung",
    "ruckerstattung",
    "rueckerstattung",
    "zahlung",
    "uberweisung",
    "gutschrift",
    "konto gesperrt",
    "konto wurde gesperrt",
    "bestatigung erforderlich",
    "bitte bestatigen",
    "frist",
    "sofort",
    "letzte mahnung",
    "mahnung",
    "statusabgleich",
    "steuer",
    "verifizier",
    "refund",
    "payment",
    "account suspended",
    "has been suspended",
    "verify your",
    "confirm your",
    "urgent",
    "immediately",
    "final notice",
    "overdue",
    "deadline",
];

/// Characters of the text without quoted lines and signature: what the
/// sender wrote this time.
fn written_chars(text: &str) -> usize {
    let mut kept = String::new();
    for line in text.lines() {
        if line.trim_end() == "--" || line.starts_with("-- ") {
            break;
        }
        if line.trim_start().starts_with('>') {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let clean: String = kept.chars().filter(|&c| !is_invisible(c)).collect();
    collapse_ws(&clean).chars().count()
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

/// Judges one mail. Without body (header-only mail) the body signals simply
/// stay silent; hidden characters and a name that does not fit the sender
/// already show in the headers.
pub fn assess(i: &Input<'_>, ctx: &Context) -> PhishingVerdict {
    let mut hits: Vec<Hit> = Vec::new();
    let mut hit = |code: &'static str, text: &str, weight: u32, strong: bool| {
        hits.push(Hit {
            code,
            text: text.to_string(),
            weight,
            strong,
        });
    };

    let from = i.from_addr.trim().to_ascii_lowercase();
    let (local, _) = from.rsplit_once('@').unwrap_or((from.as_str(), ""));
    let sender_base = base_domain(&from);
    let name_folded = fold(i.display_name);
    let tokens = name_tokens(i.display_name);
    let caps = all_caps_tokens(i.display_name);
    let org_like = has_official_word(&name_folded) || !caps.is_empty();
    let name_is_domain = name_matches_domain(&tokens, sender_base.as_deref());

    // --- hidden and deceptive characters ---
    if invisible_stats(i.display_name).any() {
        hit(
            "invisible_name",
            "Der Absendername enthält unsichtbare Zeichen. Das ist ein Trick, um Filter zu umgehen.",
            4,
            true,
        );
    }
    if invisible_stats(i.subject).any() {
        hit(
            "invisible_subject",
            "Der Betreff enthält unsichtbare Zeichen mitten in Wörtern. So werden Stichwortfilter umgangen.",
            4,
            true,
        );
    }
    if has_mixed_script_word(i.display_name) || has_mixed_script_word(i.subject) {
        hit(
            "mixed_script",
            "Ein Wort in Name oder Betreff mischt Buchstaben verschiedener Alphabete, um ein anderes Wort nachzuahmen.",
            4,
            true,
        );
    }
    let body_text: String = match i.text.filter(|t| !t.trim().is_empty()) {
        Some(t) => t.to_string(),
        None => i.html.map(html_to_text).unwrap_or_default(),
    };
    if invisible_stats(&body_text).inside_word >= BODY_SPLIT_RUNS {
        hit(
            "invisible_body",
            "Der Text ist mit unsichtbaren Zeichen durchsetzt, die Wörter zerteilen.",
            3,
            true,
        );
    }

    // --- a name that belongs to someone else ---
    let mut claimed: Option<(String, String)> = None;
    for token in &tokens {
        let Some(seen) = ctx.name_history.get(token) else {
            continue;
        };
        let total: u32 = seen.iter().map(|s| s.count).sum();
        let top = seen.iter().max_by_key(|s| s.count);
        let (Some(top), Some(sender_base)) = (top, sender_base.as_deref()) else {
            continue;
        };
        if total >= NAME_HISTORY_MIN
            && top.count * 10 >= total * 8
            && seen.len() <= 2
            && !is_official_token(token)
            && !seen.iter().any(|s| s.base_domain == sender_base)
        {
            claimed = Some((capitalise(token), top.base_domain.clone()));
            break;
        }
    }
    if let Some((word, domain)) = &claimed {
        hit(
            "name_other_org",
            &format!(
                "Der Name enthält „{word}“, bekannt von {domain}. Diese Mail kommt aber von einer anderen Adresse."
            ),
            5,
            false,
        );
    }
    let clean_name: String = i
        .display_name
        .chars()
        .filter(|&c| !is_invisible(c))
        .collect();
    let claimed_domain = RE_NAME_MAIL
        .captures(&clean_name)
        .map(|c| c[1].to_ascii_lowercase())
        .or_else(|| {
            RE_NAME_HOST
                .captures(&clean_name)
                .map(|c| c[1].to_ascii_lowercase())
        });
    if let Some(d) = claimed_domain
        && let (Some(theirs), Some(ours)) = (base_domain(&d), sender_base.as_deref())
        && theirs != ours
    {
        hit(
            "name_has_domain",
            &format!("Der Absendername nennt {d}, die Mail kommt aber von einer anderen Adresse."),
            4,
            false,
        );
    }
    if has_official_word(&name_folded) && looks_random(local) && !name_is_domain {
        hit(
            "official_random",
            "Der Name klingt nach einer Behörde oder einem Dienst, die Absenderadresse sieht aber zufällig erzeugt aus.",
            3,
            false,
        );
    }
    if !i.dmarc_pass && org_like {
        hit(
            "dmarc_org",
            "Der Absender konnte nicht bestätigt werden, der Name klingt aber nach einer Organisation.",
            2,
            false,
        );
    }

    // --- links ---
    let links = extract_links(i.html, i.text);
    let mut seen_issue: Vec<LinkIssue> = Vec::new();
    let mut mismatch = false;
    for link in &links {
        let Some(facts) = analyze_link(&link.href) else {
            continue;
        };
        for issue in &facts.issues {
            if !seen_issue.contains(issue) {
                seen_issue.push(*issue);
            }
        }
        if let (Some(shown), false) = (shown_host(&link.text), facts.host.is_empty())
            && base_domain(&shown) != base_domain(&facts.host)
        {
            mismatch = true;
        }
    }
    for issue in &seen_issue {
        let (code, text, weight) = match issue {
            LinkIssue::PercentEncodedHost => (
                "link_host_encoded",
                "Ein Link versteckt sein Ziel mit kodierten Zeichen im Servernamen.",
                5,
            ),
            LinkIssue::IpHost => (
                "link_ip_host",
                "Ein Link führt auf eine Zahlenadresse statt auf einen Namen.",
                5,
            ),
            LinkIssue::UserInfo => (
                "link_userinfo",
                "Ein Link enthält vor dem Servernamen einen Benutzernamen. So wird das echte Ziel getarnt.",
                5,
            ),
            LinkIssue::ScriptScheme => (
                "link_script",
                "Ein Link führt Programmcode aus, statt eine Webseite zu öffnen.",
                5,
            ),
            LinkIssue::Homograph => (
                "link_homograph",
                "Ein Link nutzt im Servernamen Buchstaben verschiedener Alphabete, die wie andere aussehen.",
                5,
            ),
            LinkIssue::Punycode => (
                "link_idn",
                "Ein Link führt auf eine Adresse mit Sonderzeichen im Namen.",
                2,
            ),
        };
        hit(code, text, weight, issue.is_strong());
    }
    if mismatch && !ctx.list_mail {
        hit(
            "link_text_mismatch",
            "Ein Link zeigt eine andere Adresse an, als er öffnet.",
            2,
            false,
        );
    }

    // --- shape of a lure ---
    let mut web_targets: Vec<&str> = links
        .iter()
        .filter(|l| analyze_link(&l.href).is_some_and(|f| !f.host.is_empty()))
        .map(|l| l.href.as_str())
        .collect();
    web_targets.sort_unstable();
    web_targets.dedup();
    if !ctx.list_mail
        && !ctx.is_reply
        && (1..=2).contains(&web_targets.len())
        && !body_text.trim().is_empty()
        && written_chars(&body_text) < LURE_MAX_CHARS
    {
        let hay = fold(&format!("{} {}", i.subject, body_text));
        if LURE_WORDS.iter().any(|w| hay.contains(w)) {
            hit(
                "lure_shape",
                "Kurze Mail mit einem Link und Druck oder Geldthema (zum Beispiel Zahlung oder Frist).",
                2,
                false,
            );
        }
    }

    if ctx.known_correspondent {
        hits.retain(|h| h.strong);
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.weight));
    let score: u32 = hits.iter().map(|h| h.weight).sum();
    let level = if score >= PHISHING_AT {
        Level::Phishing
    } else if score >= SUSPICIOUS_AT {
        Level::Suspicious
    } else {
        Level::None
    };
    let reasons = if level == Level::None {
        Vec::new()
    } else {
        hits.into_iter()
            .map(|h| Reason {
                code: h.code.to_string(),
                text: h.text,
            })
            .collect()
    };
    PhishingVerdict {
        level,
        score,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZWSP: &str = "\u{200B}";

    fn input<'a>(name: &'a str, from: &'a str, subject: &'a str) -> Input<'a> {
        Input {
            display_name: name,
            from_addr: from,
            dmarc_pass: true,
            subject,
            text: None,
            html: None,
        }
    }

    fn codes(v: &PhishingVerdict) -> Vec<&str> {
        v.reasons.iter().map(|r| r.code.as_str()).collect()
    }

    fn history(token: &str, domain: &str, count: u32) -> HashMap<String, Vec<NameSighting>> {
        HashMap::from([(
            token.to_string(),
            vec![NameSighting {
                base_domain: domain.to_string(),
                count,
            }],
        )])
    }

    /// Splits `word` with a zero-width space after every second letter.
    fn split_word(word: &str) -> String {
        word.chars()
            .enumerate()
            .flat_map(|(i, c)| {
                let mut v = vec![c];
                if i % 2 == 1 {
                    v.extend(ZWSP.chars());
                }
                v
            })
            .collect()
    }

    /// The shape of the real case, with invented names: organisation-like
    /// name with hidden characters, random sender at a webmail domain that
    /// passed its own check, hidden characters in the subject and the body,
    /// one link whose host is percent-encoded.
    fn lure() -> (String, String, String, String) {
        let name = format!("{} Erstattungsverwaltungsportal", split_word("ELSTER"));
        let subject = format!(
            "{}: {}",
            split_word("Gewerbesteuerzahlung"),
            split_word("Statusabgleich")
        );
        let body = format!(
            "{}\n\nhttps://exa%6dple.%65xample.invalid/p/1\n",
            split_word(
                "Ihre Rueckerstattung ist bereit. Bitte bestaetigen Sie Ihre Angaben sofort, \
                 sonst verfaellt der Anspruch nach Ablauf der Frist. Die Pruefung dauert nur \
                 wenige Minuten und ist kostenlos."
            )
        );
        (
            name,
            "k7x2m9q4vb81@webmail.example.net".into(),
            subject,
            body,
        )
    }

    #[test]
    fn real_case_shape_is_phishing() {
        let (name, from, subject, body) = lure();
        let html = format!(
            "<p>{body}</p><a href=\"https://exa%6dple.%65xample.invalid/p/1\">Jetzt prüfen</a>"
        );
        let ctx = Context {
            name_history: history("elster", "elster.example.org", 6),
            ..Context::default()
        };
        let v = assess(
            &Input {
                text: Some(&body),
                html: Some(&html),
                ..input(&name, &from, &subject)
            },
            &ctx,
        );
        assert_eq!(v.level, Level::Phishing, "{v:?}");
        for code in [
            "invisible_name",
            "invisible_subject",
            "invisible_body",
            "link_host_encoded",
            "name_other_org",
            "official_random",
            "lure_shape",
        ] {
            assert!(codes(&v).contains(&code), "missing {code}: {v:?}");
        }
        // Sorted strongest first, and the dmarc pass did not matter.
        assert!(v.reasons.windows(2).count() > 0);
        assert!(v.score >= PHISHING_AT);
    }

    #[test]
    fn real_case_without_history_is_still_phishing() {
        let (name, from, subject, body) = lure();
        let v = assess(
            &Input {
                text: Some(&body),
                ..input(&name, &from, &subject)
            },
            &Context::default(),
        );
        assert_eq!(v.level, Level::Phishing, "{v:?}");
    }

    #[test]
    fn headers_alone_can_already_hold_a_mail_back() {
        let (name, from, subject, _) = lure();
        let v = assess(&input(&name, &from, &subject), &Context::default());
        assert!(v.level >= Level::Suspicious, "{v:?}");
    }

    #[test]
    fn invisible_characters_alone_are_suspicious() {
        let subject = split_word("Rechnung");
        let v = assess(
            &input("Anna Beispiel", "anna@example.org", &subject),
            &Context::default(),
        );
        assert_eq!(v.level, Level::Suspicious);
        assert_eq!(codes(&v), ["invisible_subject"]);
        let v = assess(
            &input(&split_word("Anna"), "anna@example.org", "Hallo"),
            &Context::default(),
        );
        assert_eq!(codes(&v), ["invisible_name"]);
    }

    #[test]
    fn bidi_controls_and_look_alike_letters_count() {
        let v = assess(
            &input("Anna", "anna@example.org", "Rechnung \u{202E}fdp.exe"),
            &Context::default(),
        );
        assert!(v.level >= Level::Suspicious);
        // Cyrillic "а" inside a Latin word.
        let v = assess(
            &input("P\u{0430}ypal Support", "x@example.org", "Hallo"),
            &Context::default(),
        );
        assert!(codes(&v).contains(&"mixed_script"));
    }

    #[test]
    fn joiners_in_emoji_and_other_scripts_are_fine() {
        // Family emoji (ZWJ between emoji), Persian ZWNJ between Arabic
        // letters, and a direction mark.
        for subject in [
            "Urlaub \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} \u{1F334}",
            "می\u{200C}خواهم",
            "Hallo\u{200E}",
        ] {
            let v = assess(
                &input("Anna", "anna@example.org", subject),
                &Context::default(),
            );
            assert_eq!(v.level, Level::None, "{subject:?}: {v:?}");
        }
    }

    #[test]
    fn preview_padding_in_newsletters_is_fine() {
        // Preheader padding: runs of invisible characters between spaces.
        let pad = "\u{200C}\u{00A0}".repeat(60);
        let html = format!(
            "<span>Unser Angebot der Woche</span><span>{pad}</span>\
             <a href=\"https://click.example.com/r?u=https%3A%2F%2Fshop.example.com%2Fa%3Fb%3D1&amp;t=%41%42\">Jetzt ansehen</a>\
             <a href=\"https://click.example.com/u/12\">Abbestellen</a>"
        );
        let ctx = Context {
            list_mail: true,
            ..Context::default()
        };
        let v = assess(
            &Input {
                html: Some(&html),
                text: Some("Unser Angebot der Woche"),
                ..input(
                    "Beispiel Shop",
                    "news@shop.example.com",
                    "Angebote bis Sonntag",
                )
            },
            &ctx,
        );
        assert_eq!(v.level, Level::None, "{v:?}");
    }

    #[test]
    fn percent_encoded_path_is_not_a_signal_but_host_is() {
        let ok = analyze_link("https://tracker.example.com/r/%68%74%74%70?x=%41").unwrap();
        assert_eq!(ok.host, "tracker.example.com");
        assert!(ok.issues.is_empty());
        let bad = analyze_link("https://sear%63h.app.%67oo.invalid/x").unwrap();
        assert_eq!(bad.host, "search.app.goo.invalid");
        assert_eq!(bad.issues, [LinkIssue::PercentEncodedHost]);
    }

    #[test]
    fn link_tricks_are_found() {
        let issues = |u: &str| analyze_link(u).map(|f| f.issues).unwrap_or_default();
        assert_eq!(issues("http://192.0.2.7/login"), [LinkIssue::IpHost]);
        assert_eq!(issues("http://3221225985/x"), [LinkIssue::IpHost]);
        assert_eq!(issues("https://[2001:db8::1]/x"), [LinkIssue::IpHost]);
        assert_eq!(
            issues("https://bank.example.com@evil.example.net/x"),
            [LinkIssue::UserInfo]
        );
        assert_eq!(issues("jav\tascript:alert(1)"), [LinkIssue::ScriptScheme]);
        assert_eq!(issues("data:text/html,x"), [LinkIssue::ScriptScheme]);
        assert_eq!(issues("https://example.org:8443/a"), []);
        assert!(analyze_link("mailto:a@example.org").is_none());
        assert!(analyze_link("#top").is_none());
        // Backslash is a slash for browsers: the host ends before it.
        assert_eq!(
            analyze_link("https://example.org\\@evil.example.net/")
                .unwrap()
                .host,
            "example.org"
        );
    }

    #[test]
    fn international_hosts_are_weak_unless_they_mix_alphabets() {
        // "bücher" -> xn--bcher-kva
        let f = analyze_link("https://xn--bcher-kva.example/").unwrap();
        assert_eq!(f.issues, [LinkIssue::Punycode]);
        // Cyrillic "а" beside Latin letters: xn--pypl-53d.
        let f = analyze_link("https://xn--pypl-53d.example/").unwrap();
        assert_eq!(f.issues, [LinkIssue::Homograph]);
        assert_eq!(f.host, "xn--pypl-53d.example");
        // One idn link on its own stays below the threshold.
        let html = "<a href=\"https://xn--bcher-kva.example/\">Zum Shop</a>";
        let v = assess(
            &Input {
                html: Some(html),
                ..input("Beispiel", "a@example.org", "Hallo")
            },
            &Context::default(),
        );
        assert_eq!(v.level, Level::None);
    }

    #[test]
    fn link_text_naming_another_site_is_weak_and_ignores_trackers_of_the_same_domain() {
        let html = "<a href=\"https://evil.example.net/x\">www.example.com/konto</a>";
        let v = assess(
            &Input {
                html: Some(html),
                ..input("Beispiel", "a@example.org", "Hallo")
            },
            &Context::default(),
        );
        assert_eq!(v.level, Level::None);
        assert_eq!(v.score, 2);
        let same = "<a href=\"https://go.example.com/x\">www.example.com/konto</a>";
        let v = assess(
            &Input {
                html: Some(same),
                ..input("Beispiel", "a@example.org", "Hallo")
            },
            &Context::default(),
        );
        assert_eq!(v.score, 0);
    }

    #[test]
    fn names_known_from_another_domain_are_flagged_only_with_enough_history() {
        let input = |dmarc| Input {
            dmarc_pass: dmarc,
            ..input("Beispielamt Kundenportal", "x@webmail.example.net", "Hallo")
        };
        let ctx = |n| Context {
            name_history: history("beispielamt", "beispielamt.example", n),
            ..Context::default()
        };
        let v = assess(&input(true), &ctx(5));
        assert_eq!(v.level, Level::Suspicious);
        assert_eq!(codes(&v), ["name_other_org"]);
        // Too little history, then no claim.
        assert!(assess(&input(true), &ctx(2)).reasons.is_empty());
        // The genuine sender is clean.
        let genuine = Input {
            from_addr: "post@beispielamt.example",
            ..input(true)
        };
        assert_eq!(assess(&genuine, &ctx(5)).level, Level::None);
        // Seen from many domains: not distinctive.
        let mut many = ctx(5);
        many.name_history.get_mut("beispielamt").unwrap().extend([
            NameSighting {
                base_domain: "b.example.com".into(),
                count: 4,
            },
            NameSighting {
                base_domain: "c.example.com".into(),
                count: 4,
            },
        ]);
        assert_eq!(assess(&input(true), &many).level, Level::None);
    }

    #[test]
    fn a_passed_check_for_another_domain_does_not_lower_the_score() {
        let ctx = Context {
            name_history: history("beispielamt", "beispielamt.example", 5),
            ..Context::default()
        };
        let score = |dmarc| {
            assess(
                &Input {
                    dmarc_pass: dmarc,
                    ..input("Beispielamt", "x@webmail.example.net", "Hallo")
                },
                &ctx,
            )
            .score
        };
        assert!(score(true) >= 5);
        assert!(score(true) <= score(false));
    }

    #[test]
    fn name_with_a_foreign_address_is_flagged_own_domain_is_not() {
        let v = assess(
            &input("service@bank.example.com", "x@webmail.example.net", "Hallo"),
            &Context::default(),
        );
        assert_eq!(codes(&v), ["name_has_domain"]);
        assert_eq!(v.level, Level::Suspicious);
        let v = assess(
            &input(
                "Beispiel (bank.example.com)",
                "info@mail.bank.example.com",
                "Hallo",
            ),
            &Context::default(),
        );
        assert_eq!(v.level, Level::None);
        // A person's abbreviation is no host name.
        let v = assess(
            &input("Dr.Muster Anna", "anna@example.org", "Hallo"),
            &Context::default(),
        );
        assert_eq!(v.level, Level::None);
    }

    #[test]
    fn official_name_with_random_address_needs_both() {
        let name = "Kundenservice Verwaltung";
        let weak = |from| assess(&input(name, from, "Hallo"), &Context::default()).score;
        assert_eq!(weak("q8x2k7m4pz91@webmail.example.net"), 3);
        assert_eq!(weak("anna.schmidt1985@webmail.example.net"), 0);
        // The name matches its own domain.
        assert_eq!(weak("q8x2k7m4pz91@kundenservice.example"), 0);
    }

    #[test]
    fn unauthenticated_organisation_name_is_weak_alone() {
        let v = assess(
            &Input {
                dmarc_pass: false,
                ..input("ELSTER Portal", "post@example.org", "Hallo")
            },
            &Context::default(),
        );
        assert_eq!(v.level, Level::None);
        assert_eq!(v.score, 2);
        let v = assess(
            &Input {
                dmarc_pass: false,
                ..input("Anna Beispiel", "anna@example.org", "Hallo")
            },
            &Context::default(),
        );
        assert_eq!(v.score, 0);
    }

    #[test]
    fn lure_shape_is_weak_alone_and_needs_wording_and_few_links() {
        let text = "Bitte pruefen Sie die Zahlung bis zur Frist: https://pay.example.com/x";
        let score = |text: &str, ctx: &Context| {
            assess(
                &Input {
                    text: Some(text),
                    ..input("Anna Beispiel", "anna@example.org", "Hallo")
                },
                ctx,
            )
            .score
        };
        assert_eq!(score(text, &Context::default()), 2);
        // No lure wording.
        assert_eq!(
            score(
                "Anbei der Link zum Foto: https://pay.example.com/x",
                &Context::default()
            ),
            0
        );
        // Three links are no lure.
        let many = format!("{text} https://a.example.com https://b.example.com");
        assert_eq!(score(&many, &Context::default()), 0);
        // A long letter is no lure.
        let long = format!("{text}\n{}", "Text. ".repeat(300));
        assert_eq!(score(&long, &Context::default()), 0);
        // A conversation is no lure.
        assert_eq!(
            score(
                text,
                &Context {
                    is_reply: true,
                    ..Context::default()
                }
            ),
            0
        );
    }

    #[test]
    fn known_correspondents_keep_only_the_strong_signals() {
        let known = Context {
            known_correspondent: true,
            ..Context::default()
        };
        let text = "Bitte pruefen Sie die Zahlung bis zur Frist: https://pay.example.com/x";
        // Short mail with one link and money wording from someone you write to.
        let v = assess(
            &Input {
                text: Some(text),
                dmarc_pass: false,
                ..input("Anna Portal", "q8x2k7m4pz91@example.org", "Zahlung")
            },
            &known,
        );
        assert_eq!(v.score, 0);
        // But hidden characters and a disguised link still count.
        let html = "<a href=\"https://sear%63h.example.invalid/\">Hier</a>";
        let v = assess(
            &Input {
                html: Some(html),
                ..input("Anna", "anna@example.org", "Hallo")
            },
            &known,
        );
        assert_eq!(v.level, Level::Suspicious);
        assert_eq!(codes(&v), ["link_host_encoded"]);
    }

    #[test]
    fn ordinary_notification_from_a_verified_domain_is_clean() {
        let ctx = Context {
            name_history: history("beispiel", "beispiel.example", 9),
            ..Context::default()
        };
        let v = assess(
            &Input {
                text: Some(
                    "Ihre Bestellung ist unterwegs. Verfolgen: https://www.beispiel.example/track/1",
                ),
                html: Some(
                    "<p>Ihre Bestellung ist unterwegs.</p><a href=\"https://www.beispiel.example/track/1\">www.beispiel.example</a>",
                ),
                ..input(
                    "Beispiel Versand",
                    "versand@mail.beispiel.example",
                    "Versandbestätigung",
                )
            },
            &ctx,
        );
        assert_eq!(v.level, Level::None, "{v:?}");
    }

    #[test]
    fn learns_only_brand_like_words() {
        // Domain word, capitals, single word: learned.
        assert_eq!(
            learnable_name_tokens("Beispiel Versand", "a@mail.beispiel.example"),
            ["beispiel"]
        );
        assert_eq!(
            learnable_name_tokens("ELSTER Kundenportal", "a@elster.example.org"),
            ["elster"]
        );
        assert_eq!(
            learnable_name_tokens("Zentralamt", "a@amt.example.org"),
            ["zentralamt"]
        );
        // A person's name or generic words are not.
        assert!(learnable_name_tokens("Anna Beispiel", "anna@example.org").is_empty());
        assert!(learnable_name_tokens("Kundenservice Team", "a@example.org").is_empty());
        // Hidden characters do not hide the word.
        assert_eq!(
            learnable_name_tokens(&split_word("ELSTER"), "a@elster.example.org"),
            ["elster"]
        );
    }

    #[test]
    fn extracts_links_from_html_and_plain_text() {
        let html = r#"<a class="x" href="https://a.example.com/?a=1&amp;b=2">Text <b>fett</b></a>
            <a href='mailto:x@example.org'>Mail</a><a name="anchor">kein Link</a>"#;
        let links = extract_links(Some(html), None);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].href, "https://a.example.com/?a=1&b=2");
        assert_eq!(links[0].text, "Text fett");
        let plain = extract_links(None, Some("Siehe https://b.example.com/x, danke."));
        assert_eq!(plain[0].href, "https://b.example.com/x");
    }

    #[test]
    fn punycode_decodes() {
        assert_eq!(punycode_decode("bcher-kva").as_deref(), Some("bücher"));
        assert_eq!(punycode_decode("pypl-53d").as_deref(), Some("pаypl"));
        assert!(punycode_decode("!!").is_none());
    }

    #[test]
    fn folding_removes_hidden_characters_and_accents() {
        assert_eq!(
            fold(&format!("Rück{ZWSP}erstattung \u{00AD}Straße")),
            "ruckerstattung strasse"
        );
    }
}
