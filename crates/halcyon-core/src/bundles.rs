//! Bundles ("Übersichten", "Werbung", ...): the named, quiet places a mail
//! can belong to besides "Für dich" (see `docs/design/relevanz.md`).
//!
//! Two things live here:
//!
//! - the bundles themselves: built-in ones with stable keys, custom ones the
//!   user adds, and their settings (`presentation`, `mark_read`);
//! - the automatic choice for mail the user has not assigned: a small,
//!   explainable classifier ([`classify`]) that looks at generic signals
//!   only (list headers, recurring-source mode, wording of the subject).
//!   It is deliberately conservative: unsure means "Weitere Mails", and mail
//!   that may want something from the user is left to the rules.
//!
//! The classifier is pure; `assignments` feeds it from the store.

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::sources::SourceMode;
use crate::store::{Store, unix_now};
use crate::{Error, Result};

pub const OVERVIEWS: &str = "overviews";
pub const ACCOUNT: &str = "account";
pub const NEWSLETTER: &str = "newsletter";
pub const PROMO: &str = "promo";
pub const SOCIAL: &str = "social";
/// Catch-all: always last, never disabled, never deleted.
pub const OTHER: &str = "other";

/// Longest name a bundle may have; it is shown as a heading.
const MAX_NAME_CHARS: usize = 60;

/// How a bundle shows up on "Heute".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presentation {
    /// A slim shelf of tiles that stays visible.
    Shelf,
    /// Folded up; opens on a click.
    Collapsed,
}

impl Presentation {
    pub fn as_str(self) -> &'static str {
        match self {
            Presentation::Shelf => "shelf",
            Presentation::Collapsed => "collapsed",
        }
    }

    fn from_db(s: &str) -> Self {
        if s == "shelf" {
            Presentation::Shelf
        } else {
            Presentation::Collapsed
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub key: String,
    pub name: String,
    pub builtin: bool,
    /// Sort order; `other` is always last whatever its number.
    pub position: i64,
    pub enabled: bool,
    pub presentation: Presentation,
    /// Mail the user assigned to this bundle is marked read on the server
    /// when it arrives (see the filing pass).
    pub mark_read: bool,
    /// When `mark_read` was last switched on. Mail that arrived before is
    /// never marked, so the switch cannot reach back.
    #[serde(skip)]
    pub mark_read_since: i64,
}

/// What [`Store::update_bundle`] changes; `None` leaves a setting alone.
#[derive(Clone, Debug, Default)]
pub struct BundlePatch {
    /// Custom bundles only.
    pub name: Option<String>,
    /// Not for `other`.
    pub enabled: Option<bool>,
    pub presentation: Option<Presentation>,
    /// Not for `other`: its mail is "not yet sorted", and marking that read
    /// would hide mail nobody decided about.
    pub mark_read: Option<bool>,
    /// New index among the bundles above `other` (0 = first); clamped.
    pub position: Option<usize>,
}

fn bundle_row(r: &Row<'_>) -> rusqlite::Result<Bundle> {
    Ok(Bundle {
        key: r.get(0)?,
        name: r.get(1)?,
        builtin: r.get(2)?,
        position: r.get(3)?,
        enabled: r.get(4)?,
        presentation: Presentation::from_db(&r.get::<_, String>(5)?),
        mark_read: r.get(6)?,
        mark_read_since: r.get(7)?,
    })
}

const BUNDLE_SELECT: &str =
    "SELECT key, name, builtin, position, enabled, presentation, mark_read, mark_read_since
     FROM bundles";

/// Key for a new bundle: the name in lowercase ASCII, words joined by `-`.
/// `taken` keys are avoided by a numeric suffix.
fn key_from_name(name: &str, taken: &[String]) -> String {
    let mut slug = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => slug.push_str("ae"),
            'ö' => slug.push_str("oe"),
            'ü' => slug.push_str("ue"),
            'ß' => slug.push_str("ss"),
            c if c.is_ascii_alphanumeric() => slug.push(c),
            _ if !slug.ends_with('-') && !slug.is_empty() => slug.push('-'),
            _ => {}
        }
    }
    let slug = slug.trim_end_matches('-');
    // `for_you` is the other half of every target.
    let base = if slug.is_empty() || slug == "for-you" {
        "bundle".to_string()
    } else {
        slug.to_string()
    };
    let mut key = base.clone();
    let mut n = 2;
    while taken.contains(&key) {
        key = format!("{base}-{n}");
        n += 1;
    }
    key
}

impl Store {
    /// All bundles in display order, `other` last.
    pub fn bundles(&self) -> Result<Vec<Bundle>> {
        let mut stmt = self.conn.prepare(&format!(
            "{BUNDLE_SELECT} ORDER BY (key = 'other'), position, id"
        ))?;
        let rows = stmt.query_map([], bundle_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn bundle_by_key(&self, key: &str) -> Result<Option<Bundle>> {
        Ok(self
            .conn
            .query_row(
                &format!("{BUNDLE_SELECT} WHERE key = ?1"),
                [key],
                bundle_row,
            )
            .optional()?)
    }

    fn require_bundle(&self, key: &str) -> Result<Bundle> {
        self.bundle_by_key(key)?
            .ok_or_else(|| Error::Other(format!("unknown bundle {key:?}")))
    }

    /// Adds a custom bundle above `other`, enabled and collapsed.
    pub fn create_bundle(&self, name: &str) -> Result<Bundle> {
        let name = checked_name(name)?;
        let taken: Vec<String> = self.bundles()?.into_iter().map(|b| b.key).collect();
        let key = key_from_name(&name, &taken);
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM bundles WHERE key != 'other'",
            [],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO bundles (key, name, builtin, position, enabled, presentation, mark_read, created_at)
             VALUES (?1, ?2, 0, ?3, 1, 'collapsed', 0, ?4)",
            params![key, name, position, unix_now()],
        )?;
        self.require_bundle(&key)
    }

    pub fn update_bundle(&self, key: &str, patch: &BundlePatch) -> Result<Bundle> {
        self.update_bundle_at(key, patch, unix_now())
    }

    pub fn update_bundle_at(&self, key: &str, patch: &BundlePatch, now: i64) -> Result<Bundle> {
        let b = self.require_bundle(key)?;
        let is_other = key == OTHER;
        if patch.name.is_some() && b.builtin {
            return Err(Error::Other("built-in bundles keep their name".into()));
        }
        if patch.enabled == Some(false) && is_other {
            return Err(Error::Other("this bundle cannot be switched off".into()));
        }
        if patch.mark_read == Some(true) && is_other {
            return Err(Error::Other("this bundle cannot mark mail as read".into()));
        }
        let tx = self.conn.unchecked_transaction()?;
        if let Some(name) = &patch.name {
            tx.execute(
                "UPDATE bundles SET name = ?2 WHERE key = ?1",
                params![key, checked_name(name)?],
            )?;
        }
        if let Some(enabled) = patch.enabled {
            tx.execute(
                "UPDATE bundles SET enabled = ?2 WHERE key = ?1",
                params![key, enabled],
            )?;
        }
        if let Some(p) = patch.presentation {
            tx.execute(
                "UPDATE bundles SET presentation = ?2 WHERE key = ?1",
                params![key, p.as_str()],
            )?;
        }
        if let Some(mark_read) = patch.mark_read {
            // Only the switch from off to on moves the cut-off.
            let since = if mark_read && !b.mark_read {
                now
            } else {
                b.mark_read_since
            };
            tx.execute(
                "UPDATE bundles SET mark_read = ?2, mark_read_since = ?3 WHERE key = ?1",
                params![key, mark_read, since],
            )?;
        }
        if let Some(index) = patch.position
            && !is_other
        {
            let mut keys: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT key FROM bundles WHERE key != 'other' AND key != ?1
                     ORDER BY position, id",
                )?;
                stmt.query_map([key], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?
            };
            keys.insert(index.min(keys.len()), key.to_string());
            for (i, k) in keys.iter().enumerate() {
                tx.execute(
                    "UPDATE bundles SET position = ?2 WHERE key = ?1",
                    params![k, i as i64],
                )?;
            }
        }
        tx.commit()?;
        self.require_bundle(key)
    }

    /// Deletes a custom bundle. What the user put into it goes to `other`.
    pub fn delete_bundle(&self, key: &str) -> Result<()> {
        let b = self.require_bundle(key)?;
        if b.builtin {
            return Err(Error::Other("built-in bundles cannot be deleted".into()));
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE assignments SET target = 'bundle:other' WHERE target = ?1",
            [format!("bundle:{key}")],
        )?;
        tx.execute("DELETE FROM bundles WHERE key = ?1", [key])?;
        tx.commit()?;
        Ok(())
    }
}

fn checked_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Other("a bundle needs a name".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(Error::Other("bundle name too long".into()));
    }
    Ok(name.to_string())
}

// ----- automatic choice ------------------------------------------------------

/// What the classifier may look at.
#[derive(Clone, Copy, Debug, Default)]
pub struct AutoInput<'a> {
    pub subject: &'a str,
    /// The start of the text; later parts are not trusted for classifying.
    pub body: &'a str,
    /// `List-Id` or `List-Unsubscribe` present.
    pub list_mail: bool,
    /// Mode of the recurring source the mail belongs to.
    pub source_mode: Option<SourceMode>,
    pub source_newsletter: bool,
    /// Most subjects of the source read like promotions.
    pub source_mostly_promo: bool,
    pub has_code: bool,
}

/// The classifier's pick and why, for a "Warum?".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoChoice {
    pub bundle: &'static str,
    pub reason: String,
}

fn choice(bundle: &'static str, reason: impl Into<String>) -> Option<AutoChoice> {
    Some(AutoChoice {
        bundle,
        reason: reason.into(),
    })
}

/// How many characters of the body count as "the start".
const BODY_HEAD_CHARS: usize = 200;

/// Which bundle a mail without an assignment belongs to; `None` if the mail
/// must stay with the rules (it may be meant for the user). The caller has
/// already left out mail from the user's correspondents and mail that waits
/// for an answer.
///
/// The order matters and is the conservative one: wording that is only ever
/// promotional wins over everything but security notices, a periodic report
/// from a known source is an overview, and anything with an unsubscribe link
/// that says nothing more is a newsletter.
pub fn classify(i: &AutoInput<'_>) -> Option<AutoChoice> {
    // A one-time code must stay in sight, whatever else the mail looks like.
    if i.has_code {
        return None;
    }
    let subject = normalize(i.subject);
    // Mail the user is waiting for right now (reset link, address
    // confirmation, sign-in link) must not vanish into a folded bundle.
    if matches_any(&subject, AWAITED_RULES) {
        return None;
    }
    // List mail has a generic body; its wording says more about the sender's
    // marketing than about the mail.
    let text = if i.list_mail {
        subject.clone()
    } else {
        format!("{subject} {}", normalize(&head(i.body)))
    };

    if let Some(word) = find(&subject, PROMO_STRONG) {
        return choice(PROMO, format!("Werbewort im Betreff: „{word}“"));
    }
    if matches_any(&text, ACCOUNT_RULES) {
        return choice(ACCOUNT, "Hinweis zu deinem Konto");
    }
    if is_statement(&text) {
        return choice(
            OVERVIEWS,
            "Regelmäßiger Auszug oder Bericht (Betreff oder Anfang der Mail)",
        );
    }
    if i.source_mode == Some(SourceMode::LatestOnly) {
        return choice(
            OVERVIEWS,
            "Regelmäßige Mail dieses Absenders, bei der nur der neueste Stand zählt",
        );
    }
    let listed = i.list_mail || i.source_newsletter;
    if listed && let Some(word) = find(&subject, PROMO_WEAK) {
        return choice(PROMO, format!("Werbewort im Betreff: „{word}“"));
    }
    if listed {
        if i.source_mostly_promo {
            return choice(PROMO, "Dieser Newsletter besteht überwiegend aus Werbung");
        }
        return choice(
            NEWSLETTER,
            if i.list_mail {
                "Hat einen Abmelde-Link oder eine Listenkennung"
            } else {
                "Kommt von einem Newsletter-Absender"
            },
        );
    }
    choice(OTHER, "Passt zu keinem Bündel, nichts Eiliges")
}

/// Whether most of the subjects read like promotions; for a newsletter
/// source. Needs a few mails to say anything.
pub fn mostly_promo(subjects: &[String]) -> bool {
    if subjects.len() < 3 {
        return false;
    }
    let promo = subjects
        .iter()
        .filter(|s| {
            let s = normalize(s);
            find(&s, PROMO_STRONG).is_some() || find(&s, PROMO_WEAK).is_some()
        })
        .count();
    promo * 10 >= subjects.len() * 6
}

fn head(body: &str) -> String {
    body.chars().take(BODY_HEAD_CHARS).collect()
}

// ----- wording ---------------------------------------------------------------
//
// Needles are matched against text that `normalize` has lowercased and
// stripped of punctuation ("E-Mail" is "e mail"). A needle matches at the
// start of a word, so stems cover German compounds ("rabatt" finds
// "Rabattaktion"); a trailing `$` demands the end of the word, too. Needles
// starting with `%` match anywhere ("50% off").

/// Wording that is promotional wherever it appears.
const PROMO_STRONG: &[&str] = &[
    "rabatt",
    "gutschein",
    "gewinnspiel",
    "nur heute",
    "nur noch heute",
    "jetzt sparen",
    "sale$",
    "black friday",
    "cyber monday",
    "% off",
    "discount",
    "coupon",
    "promo code",
    "special offer",
    "limited time",
];

/// Wording that is promotional on list mail ("Angebot" is also what a
/// craftsman's quote is called, which must not end up in "Werbung").
const PROMO_WEAK: &[&str] = &["angebot$", "angebote$", "deal$", "deals$"];

/// Notices about the user's account. Every group of a rule must be hit.
/// Mail a user has just asked for and is waiting for.
const AWAITED_RULES: &[&[&[&str]]] = &[
    &[
        &["passwort", "kennwort", "password"],
        &[
            "zurücksetzen",
            "zuruecksetzen",
            "vergessen",
            "reset",
            "forgot",
        ],
    ],
    &[
        &[
            "bestätige deine",
            "bestätige ihre",
            "bestätigen sie ihre",
            "bitte bestätigen",
            "confirm your",
            "verify your",
        ],
        &[
            "e mail",
            "email",
            "mail adresse",
            "adresse",
            "address",
            "registrierung",
            "anmeldung",
            "konto",
            "account",
        ],
    ],
    &[&[
        "anmeldelink",
        "login link",
        "sign in link",
        "magic link",
        "registrierung abschließen",
        "complete your registration",
    ]],
];

const ACCOUNT_RULES: &[&[&[&str]]] = &[
    &[&[
        "neue anmeldung",
        "neuer login",
        "anmeldeversuch",
        "anmeldung von einem neuen",
        "anmeldung auf einem neuen",
        "new sign in",
        "new login",
        "sign in attempt",
        "login attempt",
        "signed in from a new",
        "sign in from a new",
    ]],
    // Only the notice afterwards: asking for a reset is something the user
    // is waiting for right now.
    &[
        &["passwort", "kennwort", "password"],
        &[
            "geändert",
            "geaendert",
            "aktualisiert",
            "changed",
            "updated",
        ],
    ],
    &[&[
        "sicherheitswarnung",
        "sicherheitshinweis",
        "sicherheitsbenachrichtigung",
        "verdächtige",
        "security alert",
        "security notice",
        "security notification",
        "suspicious",
    ]],
    &[
        &[
            "agb$",
            "nutzungsbedingungen",
            "geschäftsbedingungen",
            "terms of service",
            "terms of use",
            "terms and conditions",
            "terms$",
        ],
        &[
            "geändert",
            "änderung",
            "aktualisier",
            "neue",
            "update",
            "changes",
            "changing",
            "changed",
            "updating",
        ],
    ],
    &[
        &[
            "datenschutzerklärung",
            "datenschutzhinweise",
            "datenschutzbestimmungen",
            "privacy policy",
            "privacy notice",
        ],
        &[
            "geändert",
            "änderung",
            "aktualisier",
            "neue",
            "update",
            "changes",
            "changing",
            "changed",
            "updating",
        ],
    ],
];

/// Periodic personal statements and reports.
const STATEMENT_WORDS: &[&str] = &[
    "kontoauszug",
    "depotauszug",
    "kreditkartenabrechnung",
    "wochenbericht",
    "monatsbericht",
    "quartalsbericht",
    "wochenübersicht",
    "monatsübersicht",
    "zusammenfassung deiner",
    "zusammenfassung deines",
    "zusammenfassung ihrer",
    "zusammenfassung ihres",
    "weekly report",
    "monthly report",
    "weekly summary",
    "monthly summary",
    "account statement",
    "monthly statement",
    "your statement",
    "statement is ready",
    "summary of your",
    "your weekly",
    "your monthly",
];

/// "Abrechnung für November", "Statement for March": a kind of document, the
/// word "for" and a month.
const STATEMENT_KINDS: &[&str] = &[
    "abrechnung",
    "übersicht",
    "uebersicht",
    "auszug",
    "bericht",
    "statement",
    "report",
    "summary",
];
const FOR_WORDS: &[&str] = &["für$", "fuer$", "for$"];
const MONTHS: &[&str] = &[
    "januar$",
    "februar$",
    "märz$",
    "maerz$",
    "april$",
    "mai$",
    "juni$",
    "juli$",
    "august$",
    "september$",
    "oktober$",
    "november$",
    "dezember$",
    "january$",
    "february$",
    "march$",
    "may$",
    "june$",
    "july$",
    "october$",
    "december$",
];

/// A periodic statement is not an order, an invoice or a shipment, even if
/// it is called a summary.
const NOT_A_STATEMENT: &[&str] = &[
    "bestellung",
    "order$",
    "rechnung",
    "invoice",
    "versand",
    "sendung",
    "paket",
    "lieferung",
    "mahnung",
    "zahlungserinnerung",
    "shipping",
    "delivery",
];

fn is_statement(text: &str) -> bool {
    if find(text, NOT_A_STATEMENT).is_some() {
        return false;
    }
    find(text, STATEMENT_WORDS).is_some()
        || (find(text, STATEMENT_KINDS).is_some()
            && find(text, FOR_WORDS).is_some()
            && find(text, MONTHS).is_some())
}

fn matches_any(text: &str, rules: &[&[&[&str]]]) -> bool {
    rules
        .iter()
        .any(|rule| rule.iter().all(|group| find(text, group).is_some()))
}

/// Lowercase, everything but letters, digits and `%` turned into single
/// spaces.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '%' {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

/// The first needle found in normalized `text`.
fn find<'n>(text: &str, needles: &[&'n str]) -> Option<&'n str> {
    let padded = format!(" {text} ");
    needles.iter().copied().find(|needle| {
        if needle.starts_with('%') {
            return padded.contains(needle);
        }
        match needle.strip_suffix('$') {
            Some(whole) => padded.contains(&format!(" {whole} ")),
            None => padded.contains(&format!(" {needle}")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(subject: &str) -> Option<&'static str> {
        classify(&AutoInput {
            subject,
            ..Default::default()
        })
        .map(|c| c.bundle)
    }

    fn listed(subject: &str) -> Option<&'static str> {
        classify(&AutoInput {
            subject,
            list_mail: true,
            ..Default::default()
        })
        .map(|c| c.bundle)
    }

    #[test]
    fn statements_are_overviews() {
        for s in [
            "Dein Kontoauszug ist da",
            "Depotauszug 3. Quartal",
            "Abrechnung für November",
            "Übersicht für Dezember 2026",
            "Wochenbericht",
            "Zusammenfassung deiner Anzeigen",
            "Your monthly statement is ready",
            "Statement for March",
            "Weekly summary",
        ] {
            assert_eq!(pick(s), Some(OVERVIEWS), "{s}");
        }
        // The month alone is not a statement; neither is an order summary.
        assert_eq!(pick("Termin im November"), Some(OTHER));
        assert_eq!(pick("Zusammenfassung deiner Bestellung"), Some(OTHER));
        assert_eq!(pick("Rechnung für November"), Some(OTHER));
    }

    #[test]
    fn statement_wording_in_the_body_counts_for_personal_mail_only() {
        let body = "Dein Kontoauszug für Oktober liegt bereit.";
        let direct = classify(&AutoInput {
            subject: "Neue Nachricht",
            body,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(direct.bundle, OVERVIEWS);
        let list = classify(&AutoInput {
            subject: "Neue Nachricht",
            body,
            list_mail: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(list.bundle, NEWSLETTER);
    }

    #[test]
    fn latest_only_sources_are_overviews() {
        let c = classify(&AutoInput {
            subject: "Stand heute",
            source_mode: Some(SourceMode::LatestOnly),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.bundle, OVERVIEWS);
        for mode in [SourceMode::Show, SourceMode::Bundle, SourceMode::Mute] {
            let c = classify(&AutoInput {
                subject: "Stand heute",
                source_mode: Some(mode),
                ..Default::default()
            })
            .unwrap();
            assert_eq!(c.bundle, OTHER, "{mode:?}");
        }
    }

    #[test]
    fn account_notices() {
        for s in [
            "Neue Anmeldung bei deinem Konto",
            "Dein Passwort wurde geändert",
            "Your password was changed",
            "Sicherheitswarnung für dein Konto",
            "Security alert",
            "Neue AGB ab Januar",
            "Wir aktualisieren unsere Nutzungsbedingungen",
            "Datenschutzerklärung geändert",
            "Our privacy policy is changing",
            "New sign-in from a new device",
        ] {
            assert_eq!(pick(s), Some(ACCOUNT), "{s}");
        }
        // Mail the user is waiting for stays in sight, in no bundle.
        for s in [
            "Passwort zurücksetzen",
            "Reset your password",
            "Bestätige deine E-Mail-Adresse",
            "Please verify your email address",
            "Dein Anmeldelink",
        ] {
            assert_eq!(pick(s), None, "{s}");
        }
        // "agb" inside another word is not the terms.
        assert_eq!(pick("Tagbuch aktualisiert"), Some(OTHER));
    }

    #[test]
    fn a_one_time_code_is_never_bundled() {
        for s in [
            "Dein Passwort wurde geändert",
            "Bestätige deine E-Mail-Adresse",
            "Kontoauszug",
            "20% Rabatt",
        ] {
            let c = classify(&AutoInput {
                subject: s,
                has_code: true,
                list_mail: true,
                ..Default::default()
            });
            assert_eq!(c, None, "{s}");
        }
    }

    #[test]
    fn promo_wording() {
        for s in [
            "20% Rabatt auf alles",
            "Dein Gutschein wartet",
            "Gewinnspiel: ein Wochenende gewinnen",
            "Nur heute: Versand frei",
            "Jetzt sparen",
            "Summer Sale",
            "Black Friday",
            "50% off everything",
            "Your coupon code",
        ] {
            assert_eq!(pick(s), Some(PROMO), "{s}");
        }
        assert_eq!(pick("Wholesale prices"), Some(OTHER));
    }

    #[test]
    fn weak_promo_words_need_list_mail() {
        // A craftsman's quote is not an advertisement.
        assert_eq!(pick("Ihr Angebot Nr. 4711"), Some(OTHER));
        assert_eq!(listed("Angebote der Woche"), Some(PROMO));
        assert_eq!(listed("Deals für dich"), Some(PROMO));
        assert_eq!(listed("Neues aus dem Verein"), Some(NEWSLETTER));
    }

    #[test]
    fn newsletter_sources() {
        let c = classify(&AutoInput {
            subject: "Neues im Dezember",
            source_newsletter: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(c.bundle, NEWSLETTER);
        assert!(!c.reason.is_empty());
        let promo = classify(&AutoInput {
            subject: "Neues im Dezember",
            source_newsletter: true,
            source_mostly_promo: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(promo.bundle, PROMO);
        // Without list signs a mostly-promo history means nothing.
        let plain = classify(&AutoInput {
            subject: "Neues im Dezember",
            source_mostly_promo: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(plain.bundle, OTHER);
    }

    #[test]
    fn mostly_promo_needs_a_majority_and_a_few_mails() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!mostly_promo(&s(&["20% Rabatt", "Sale"])));
        assert!(mostly_promo(&s(&["20% Rabatt", "Sale", "Angebote", "Neu"])));
        assert!(!mostly_promo(&s(&["Neu", "Neu 2", "Sale", "Aktuell"])));
    }

    #[test]
    fn security_notices_beat_promo_only_when_not_promotional() {
        // Strong promo wording first: the account rules never mention it.
        assert_eq!(pick("Passwort geändert: 10% Rabatt für dich"), Some(PROMO));
    }

    #[test]
    fn keys_are_ascii_unique_and_never_for_you() {
        let taken = vec!["verein".to_string(), "other".to_string()];
        assert_eq!(key_from_name("Schützenverein!", &[]), "schuetzenverein");
        assert_eq!(key_from_name("Verein", &taken), "verein-2");
        assert_eq!(key_from_name("Für dich", &[]), "fuer-dich");
        assert_eq!(key_from_name("For you", &[]), "bundle");
        assert_eq!(key_from_name("???", &[]), "bundle");
    }

    #[test]
    fn built_in_bundles_exist_in_order() {
        let store = Store::open_in_memory().unwrap();
        let all = store.bundles().unwrap();
        let keys: Vec<_> = all.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "overviews",
                "account",
                "newsletter",
                "promo",
                "social",
                "other"
            ]
        );
        let by = |k: &str| all.iter().find(|b| b.key == k).unwrap();
        assert_eq!(by("overviews").name, "Übersichten");
        assert_eq!(by("overviews").presentation, Presentation::Shelf);
        assert!(!by("overviews").mark_read);
        assert!(by("promo").mark_read);
        assert!(!by("social").enabled);
        assert!(all.iter().filter(|b| b.key != "social").all(|b| b.enabled));
        assert!(all.iter().all(|b| b.builtin));
        assert!(
            all.iter()
                .filter(|b| b.key != "overviews")
                .all(|b| b.presentation == Presentation::Collapsed)
        );
    }

    #[test]
    fn custom_bundles_can_be_created_renamed_reordered_and_deleted() {
        let store = Store::open_in_memory().unwrap();
        let verein = store.create_bundle("  Verein ").unwrap();
        assert_eq!(verein.key, "verein");
        assert_eq!(verein.name, "Verein");
        assert!(!verein.builtin && verein.enabled && !verein.mark_read);
        store.create_bundle("Reisen").unwrap();
        // New ones come before `other`, which stays last.
        let keys = |s: &Store| {
            s.bundles()
                .unwrap()
                .into_iter()
                .map(|b| b.key)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            keys(&store),
            [
                "overviews",
                "account",
                "newsletter",
                "promo",
                "social",
                "verein",
                "reisen",
                "other"
            ]
        );
        // Same name again gets another key.
        assert_eq!(store.create_bundle("Verein").unwrap().key, "verein-2");

        store
            .update_bundle(
                "reisen",
                &BundlePatch {
                    name: Some("Reisen & Urlaub".into()),
                    position: Some(0),
                    presentation: Some(Presentation::Shelf),
                    ..Default::default()
                },
            )
            .unwrap();
        let all = store.bundles().unwrap();
        assert_eq!(all[0].key, "reisen");
        assert_eq!(all[0].name, "Reisen & Urlaub");
        assert_eq!(all[0].presentation, Presentation::Shelf);
        // Out of range goes just before `other`.
        store
            .update_bundle(
                "reisen",
                &BundlePatch {
                    position: Some(99),
                    ..Default::default()
                },
            )
            .unwrap();
        let k = keys(&store);
        assert_eq!(k.last().unwrap(), "other");
        assert_eq!(k[k.len() - 2], "reisen");

        // Rules for the built-in ones.
        let no = |key: &str, patch: BundlePatch| {
            assert!(store.update_bundle(key, &patch).is_err(), "{key}");
        };
        no(
            "promo",
            BundlePatch {
                name: Some("X".into()),
                ..Default::default()
            },
        );
        no(
            "other",
            BundlePatch {
                enabled: Some(false),
                ..Default::default()
            },
        );
        no(
            "other",
            BundlePatch {
                mark_read: Some(true),
                ..Default::default()
            },
        );
        no(
            "verein",
            BundlePatch {
                name: Some("  ".into()),
                ..Default::default()
            },
        );
        no("missing", BundlePatch::default());
        assert!(store.delete_bundle("promo").is_err());
        assert!(store.delete_bundle("other").is_err());
        assert!(store.create_bundle("").is_err());

        // Built-in ones may be switched off and moved.
        let promo = store
            .update_bundle(
                "promo",
                &BundlePatch {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!promo.enabled);

        store.delete_bundle("verein").unwrap();
        assert!(!keys(&store).contains(&"verein".to_string()));
    }

    #[test]
    fn mark_read_since_moves_only_when_switched_on() {
        let store = Store::open_in_memory().unwrap();
        let on = |now| {
            store
                .update_bundle_at(
                    "account",
                    &BundlePatch {
                        mark_read: Some(true),
                        ..Default::default()
                    },
                    now,
                )
                .unwrap()
        };
        assert_eq!(on(500).mark_read_since, 500);
        assert_eq!(on(900).mark_read_since, 500, "already on: unchanged");
        let off = store
            .update_bundle_at(
                "account",
                &BundlePatch {
                    mark_read: Some(false),
                    ..Default::default()
                },
                1000,
            )
            .unwrap();
        assert!(!off.mark_read);
        assert_eq!(on(2000).mark_read_since, 2000);
    }
}
