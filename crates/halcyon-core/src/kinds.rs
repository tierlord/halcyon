//! Kinds of mail ("Arten von Mails"): how the mail of one sender falls into
//! groups that a person would call "the same sort of mail".
//!
//! A sender rarely sends one sort of mail. A marketplace sends offers that
//! are swiped away at once, and in between shipping notices that matter. So
//! an answer such as "I do not need this" must apply to a *kind* of mail,
//! never to the sender as a whole (see `docs/design/relevanz.md`).
//!
//! The grouping is deterministic and needs no model:
//!
//! 1. Mails are split by the features the extractors already know (tracking
//!    number, invoice, one-time code, order case, reply), because mails with
//!    almost the same subject can still be different things.
//! 2. Inside such a split the subject is normalised (`normalize_subject_pattern`)
//!    and the patterns are merged where the difference looks like an article
//!    or product name: rare patterns that share a leading template ("Angebot:
//!    ...") or have a large overlap of words form one group, while patterns
//!    that repeat often are stable kinds of their own and stay apart.
//! 3. At most [`MAX_GROUPS`] groups remain; the smallest ones are collected
//!    under "Sonstiges".
//!
//! Everything here is pure; the store feeds it (see `assignments`).

use std::collections::BTreeMap;

use crate::extract::{finance, otp, tracking};
use crate::sources::normalize_subject_pattern;

/// Groups per sender, "Sonstiges" included. More would not fit a preview a
/// person can take in at a glance.
pub const MAX_GROUPS: usize = 8;

/// Key and label of the catch-all group.
pub const OTHER_KEY: &str = "other";
pub const OTHER_LABEL: &str = "Sonstiges";

/// A subject pattern that occurred this often is a recurring kind, not a
/// one-off with a variable article name; it is never merged into another.
const STABLE_COUNT: usize = 3;

/// Behind one leading template, this many different patterns mean that the
/// rest varies by nature ("Rechnung: Januar", "Rechnung: Februar", ...), even
/// if each variant came a few times.
const HEAD_MERGE_DISTINCT: usize = 4;

/// A group where the minority with/without an amount is at least this
/// share (and two mails) is split by the amount.
const AMOUNT_SPLIT_SHARE: usize = 5; // one in five

/// What the extractors found in one mail, plus what the store knows about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KindFeatures {
    /// A parcel tracking number.
    pub has_tracking: bool,
    /// An invoice number, or an invoice file attached to an invoice mail.
    pub has_invoice: bool,
    /// A monetary amount.
    pub has_amount: bool,
    /// A one-time code.
    pub has_code: bool,
    /// The mail is a step of an order case (linker).
    pub in_order_case: bool,
    /// A reply, or a mail the user answered.
    pub is_reply: bool,
}

impl KindFeatures {
    /// The features that only need the text of the mail. The store adds the
    /// ones that need its tables (`in_order_case`, `is_reply`).
    pub fn detect(subject: &str, body_text: Option<&str>) -> Self {
        let body = body_text.unwrap_or("");
        KindFeatures {
            has_tracking: !tracking::find_shipments(subject, body).is_empty(),
            has_invoice: finance::find_business_keys(subject, body)
                .iter()
                .any(|k| k.kind == finance::BusinessKeyKind::InvoiceNumber),
            has_amount: !finance::find_amounts(subject, body).is_empty(),
            has_code: !otp::find_codes(subject, body).is_empty(),
            in_order_case: false,
            is_reply: false,
        }
    }

    /// Features that split groups. The amount is left out: it is noisy (a
    /// promotion may or may not quote a price) and is handled separately.
    fn signature(self) -> String {
        let mut parts = Vec::new();
        if self.has_tracking {
            parts.push("trk");
        }
        if self.has_invoice {
            parts.push("inv");
        }
        if self.has_code {
            parts.push("otp");
        }
        if self.in_order_case {
            parts.push("ord");
        }
        if self.is_reply {
            parts.push("re");
        }
        parts.join("+")
    }

    /// Machine names of the features that are set.
    pub fn names(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.has_tracking {
            out.push("tracking");
        }
        if self.has_invoice {
            out.push("invoice");
        }
        if self.has_amount {
            out.push("amount");
        }
        if self.has_code {
            out.push("code");
        }
        if self.in_order_case {
            out.push("order");
        }
        if self.is_reply {
            out.push("reply");
        }
        out
    }

    /// German hints for a label, in a fixed order.
    fn hints(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.has_tracking {
            out.push("mit Sendungsnummer");
        }
        if self.has_invoice {
            out.push("mit Rechnung");
        }
        if self.has_code {
            out.push("mit Einmalcode");
        }
        if self.has_amount {
            out.push("mit Betrag");
        }
        if self.in_order_case {
            out.push("zu einer Bestellung");
        }
        if self.is_reply {
            out.push("als Antwort");
        }
        out
    }
}

/// One mail of a sender. The grouping is per sender, so the address is not
/// part of the input; the text enters through [`KindFeatures::detect`].
#[derive(Clone, Copy, Debug)]
pub struct KindMail<'a> {
    pub subject: &'a str,
    pub features: KindFeatures,
}

/// One group of a sender's mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kind {
    /// Stable within the sender; what assignments are stored under.
    pub key: String,
    /// Readable German name.
    pub label: String,
    /// Machine names of the features all members share (see
    /// [`KindFeatures::names`]); empty for "Sonstiges".
    pub features: Vec<&'static str>,
    /// Indices into the input slice, ascending.
    pub members: Vec<usize>,
}

/// A subject pattern with the mails that carry it.
struct Pat {
    pattern: String,
    /// Lowercase words, punctuation at the edges removed.
    tokens: Vec<String>,
    /// Text before a separator such as ":" or " | ", lowercase.
    head: Option<String>,
    members: Vec<usize>,
}

struct Cluster {
    anchor: String,
    label: String,
    members: Vec<usize>,
}

/// Groups the mail of one sender. Deterministic: the result does not depend
/// on the order of `mails`.
pub fn group_mails(mails: &[KindMail<'_>]) -> Vec<Kind> {
    let mut partitions: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, m) in mails.iter().enumerate() {
        partitions
            .entry(m.features.signature())
            .or_default()
            .push(i);
    }

    let mut kinds: Vec<Kind> = Vec::new();
    for (signature, idxs) in partitions {
        for cluster in cluster_subjects(mails, &idxs) {
            for (suffix, members) in split_by_amount(mails, cluster.members) {
                let mut features = mails[members[0]].features;
                // The amount is only a group trait when all members share it.
                features.has_amount = members.iter().all(|&i| mails[i].features.has_amount);
                let mut key = cluster.anchor.clone();
                if !signature.is_empty() {
                    key.push('|');
                    key.push_str(&signature);
                }
                key.push_str(suffix);
                let hints = features.hints();
                let label = if hints.is_empty() {
                    cluster.label.clone()
                } else {
                    format!("{} · {}", cluster.label, hints.join(", "))
                };
                kinds.push(Kind {
                    key,
                    label,
                    features: features.names(),
                    members,
                });
            }
        }
    }

    kinds.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| a.key.cmp(&b.key))
    });
    if kinds.len() > MAX_GROUPS {
        let rest = kinds.split_off(MAX_GROUPS - 1);
        let mut members: Vec<usize> = rest.into_iter().flat_map(|k| k.members).collect();
        members.sort_unstable();
        kinds.push(Kind {
            key: OTHER_KEY.into(),
            label: OTHER_LABEL.into(),
            features: Vec::new(),
            members,
        });
    }
    for k in &mut kinds {
        k.members.sort_unstable();
    }
    kinds
}

/// Splits a group by the amount when both halves are substantial, so that
/// "invoice with amount" and "reminder without" do not hide in one group.
fn split_by_amount(mails: &[KindMail<'_>], members: Vec<usize>) -> Vec<(&'static str, Vec<usize>)> {
    let (with, without): (Vec<usize>, Vec<usize>) =
        members.iter().partition(|&&i| mails[i].features.has_amount);
    let minority = with.len().min(without.len());
    if minority >= 2 && minority * AMOUNT_SPLIT_SHARE >= members.len() {
        vec![("|amt", with), ("", without)]
    } else {
        vec![("", members)]
    }
}

fn cluster_subjects(mails: &[KindMail<'_>], idxs: &[usize]) -> Vec<Cluster> {
    let mut by_pattern: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &i in idxs {
        by_pattern
            .entry(normalize_subject_pattern(mails[i].subject))
            .or_default()
            .push(i);
    }
    let mut pats: Vec<Pat> = by_pattern
        .into_iter()
        .map(|(pattern, members)| Pat {
            tokens: tokens_of(&pattern),
            head: head_of(&pattern),
            pattern,
            members,
        })
        .collect();
    // Largest first, so the stable patterns lead; ties by text for determinism.
    pats.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then_with(|| a.pattern.cmp(&b.pattern))
    });

    let mut out: Vec<Cluster> = Vec::new();
    let mut taken = vec![false; pats.len()];

    // Patterns behind the same leading template ("Angebot: <article>") are
    // one group: the part after the separator is what varies. Recurring
    // patterns stay apart unless there are many different ones.
    let mut by_head: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (pi, p) in pats.iter().enumerate() {
        if let Some(head) = &p.head {
            by_head.entry(head).or_default().push(pi);
        }
    }
    for (head, group) in by_head {
        let many = group.len() >= HEAD_MERGE_DISTINCT;
        let group: Vec<usize> = group
            .into_iter()
            .filter(|&pi| many || pats[pi].members.len() < STABLE_COUNT)
            .collect();
        if group.is_empty() {
            continue;
        }
        for &pi in &group {
            taken[pi] = true;
        }
        let refs: Vec<&Pat> = group.iter().map(|&pi| &pats[pi]).collect();
        out.push(Cluster {
            anchor: format!("h:{head}"),
            label: label_for(&refs),
            members: refs.iter().flat_map(|p| p.members.clone()).collect(),
        });
    }

    // The rest: a rare pattern joins the first group it resembles.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (pi, p) in pats.iter().enumerate() {
        if taken[pi] {
            continue;
        }
        let home = if p.members.len() < STABLE_COUNT {
            groups.iter().position(|g| resembles(&pats[g[0]], p))
        } else {
            None
        };
        match home {
            Some(g) => groups[g].push(pi),
            None => groups.push(vec![pi]),
        }
    }
    for group in groups {
        let refs: Vec<&Pat> = group.iter().map(|&pi| &pats[pi]).collect();
        out.push(Cluster {
            anchor: anchor_for(&refs),
            label: label_for(&refs),
            members: refs.iter().flat_map(|p| p.members.clone()).collect(),
        });
    }
    out
}

/// Key part of a group made of resembling patterns. The most frequent pattern
/// names the group; when all are one-offs the shared leading words do, so
/// that another one-off joining later does not rename the group.
fn anchor_for(group: &[&Pat]) -> String {
    let rep = group[0];
    if group.len() > 1 && rep.members.len() < STABLE_COUNT {
        let prefix = common_prefix(group);
        if prefix >= 2 {
            return format!("t:{}", rep.tokens[..prefix].join(" "));
        }
    }
    format!("p:{}", rep.pattern.to_lowercase())
}

fn tokens_of(pattern: &str) -> Vec<String> {
    pattern
        .split_whitespace()
        .map(|t| {
            t.to_lowercase()
                .trim_matches(|c: char| ":;,.!?".contains(c))
                .to_string()
        })
        .collect()
}

/// The words before the first separator, when they look like a template
/// ("Angebot", "Neue Nachricht von Anna") rather than a whole sentence.
fn head_of(pattern: &str) -> Option<String> {
    let mut cut: Option<usize> = None;
    for sep in [": ", " – ", " — ", " | ", " - "] {
        if let Some(i) = pattern.find(sep) {
            cut = Some(cut.map_or(i, |c| c.min(i)));
        }
    }
    let head = pattern[..cut?].trim();
    let rest = pattern[cut? + 1..].trim_start_matches([' ', '–', '—', '|', '-']);
    let words = head.split_whitespace().count();
    if rest.is_empty() || !(1..=4).contains(&words) || head.chars().count() < 3 {
        return None;
    }
    Some(head.to_lowercase())
}

fn common_prefix(group: &[&Pat]) -> usize {
    let first = &group[0].tokens;
    let mut n = first.len();
    for p in &group[1..] {
        n = n.min(
            first
                .iter()
                .zip(&p.tokens)
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    n
}

fn common_suffix(group: &[&Pat], limit: usize) -> usize {
    let first = &group[0].tokens;
    let mut n = first.len().min(limit);
    for p in &group[1..] {
        n = n.min(
            first
                .iter()
                .rev()
                .zip(p.tokens.iter().rev())
                .take_while(|(a, b)| a == b)
                .count(),
        );
        n = n.min(p.tokens.len().saturating_sub(common_prefix(group)));
    }
    n
}

/// Whether two one-off patterns look like the same template with another
/// article: two shared leading words, three shared trailing words, one number
/// swapped for another, or most words in common.
fn resembles(a: &Pat, b: &Pat) -> bool {
    if a.tokens.is_empty() || b.tokens.is_empty() {
        return false;
    }
    let pair = [a, b];
    let prefix = common_prefix(&pair);
    if prefix >= 2 {
        return true;
    }
    let suffix = a
        .tokens
        .iter()
        .rev()
        .zip(b.tokens.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    if suffix >= 3 {
        return true;
    }
    // The same words with one number or code replaced ("Wochenbrief 7").
    if a.tokens.len() == b.tokens.len() {
        let mut differing = a.tokens.iter().zip(&b.tokens).filter(|(x, y)| x != y);
        if let (Some((x, y)), None) = (differing.next(), differing.next())
            && varies_by_nature(x)
            && varies_by_nature(y)
        {
            return true;
        }
    }
    let shared = a.tokens.iter().filter(|t| b.tokens.contains(t)).count();
    let union = a.tokens.len() + b.tokens.len() - shared;
    shared * 10 >= union * 7
}

/// A word that holds a number or a placeholder: a counter, a date, an id.
fn varies_by_nature(token: &str) -> bool {
    token.contains(|c: char| c.is_ascii_digit() || c == '{')
}

/// Readable name of a group: the shared template with "…" for what varies.
fn label_for(group: &[&Pat]) -> String {
    let rep = group[0];
    let words: Vec<&str> = rep.pattern.split_whitespace().collect();
    if words.is_empty() {
        return "(kein Betreff)".into();
    }
    if group.len() == 1 {
        return render(words.iter().map(|w| blank(w)).collect());
    }
    let prefix = common_prefix(group).min(words.len());
    let suffix = common_suffix(group, words.len() - prefix);
    let mut parts: Vec<String> = Vec::new();
    if prefix + suffix == 0 {
        // Only loosely similar: show the words they have in common.
        let common: Vec<&str> = words
            .iter()
            .zip(&rep.tokens)
            .filter(|(_, t)| group[1..].iter().all(|p| p.tokens.contains(t)))
            .map(|(w, _)| *w)
            .collect();
        if common.is_empty() {
            return "Verschiedene Betreffs".into();
        }
        parts.push("…".into());
        for w in common {
            parts.push(blank(w));
            parts.push("…".into());
        }
        return render(parts);
    }
    parts.extend(words[..prefix].iter().map(|w| blank(w)));
    let varies = group.iter().any(|p| p.tokens.len() > prefix + suffix);
    if varies {
        parts.push("…".into());
    }
    parts.extend(words[words.len() - suffix..].iter().map(|w| blank(w)));
    render(parts)
}

/// Replaces a `{placeholder}` in a word by "…", keeping punctuation around it.
fn blank(word: &str) -> String {
    match (word.find('{'), word.find('}')) {
        (Some(a), Some(b)) if a < b => format!("{}…{}", &word[..a], &word[b + 1..]),
        _ => word.to_string(),
    }
}

/// Joins words and folds neighbouring "…" into one.
fn render(parts: Vec<String>) -> String {
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        if p == "…" && out.last().is_some_and(|l| l == "…") {
            continue;
        }
        out.push(p);
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(subject: &str) -> KindMail<'_> {
        KindMail {
            subject,
            features: KindFeatures::default(),
        }
    }

    fn with(subject: &str, f: impl FnOnce(&mut KindFeatures)) -> KindMail<'_> {
        let mut features = KindFeatures::default();
        f(&mut features);
        KindMail { subject, features }
    }

    fn kind_of(kinds: &[Kind], index: usize) -> &Kind {
        kinds.iter().find(|k| k.members.contains(&index)).unwrap()
    }

    #[test]
    fn merges_variable_product_names_but_keeps_other_kinds_apart() {
        let mails = vec![
            mail("Angebot: rotes Fahrrad 28 Zoll"),
            mail("Angebot: Kinderwagen"),
            mail("Angebot: Gartenstuhl aus Holz"),
            with("Versandbestätigung für Bestellung 123456", |f| {
                f.has_tracking = true
            }),
            with("Versandbestätigung für Bestellung 998877", |f| {
                f.has_tracking = true
            }),
        ];
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 2, "{kinds:#?}");
        let offers = kind_of(&kinds, 0);
        assert_eq!(offers.members, vec![0, 1, 2]);
        assert_eq!(offers.label, "Angebot: …");
        let shipping = kind_of(&kinds, 3);
        assert_eq!(shipping.members, vec![3, 4]);
        assert!(shipping.label.contains("Versandbestätigung für Bestellung"));
        assert!(shipping.label.ends_with("mit Sendungsnummer"));
        assert_eq!(shipping.features, vec!["tracking"]);
        assert_ne!(offers.key, shipping.key);
    }

    #[test]
    fn features_split_nearly_identical_subjects() {
        let mails = vec![
            mail("Ihre Nachricht von Shop"),
            mail("Ihre Nachricht von Shop"),
            with("Ihre Nachricht von Shop", |f| f.has_tracking = true),
            with("Ihre Nachricht von Shop", |f| f.has_code = true),
            with("Ihre Nachricht von Shop", |f| f.is_reply = true),
            with("Ihre Nachricht von Shop", |f| f.in_order_case = true),
            with("Ihre Nachricht von Shop", |f| f.has_invoice = true),
        ];
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 6);
        assert_eq!(kind_of(&kinds, 0).members, vec![0, 1]);
    }

    #[test]
    fn stable_patterns_stay_apart_even_with_a_shared_head() {
        let mut subjects = Vec::new();
        for _ in 0..4 {
            subjects.push("Bestellung: versandt");
            subjects.push("Bestellung: geliefert");
        }
        let mails: Vec<_> = subjects.into_iter().map(mail).collect();
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 2);
    }

    #[test]
    fn one_off_variants_join_a_stable_template() {
        let mut subjects = vec!["Neue Nachricht von Anna"; 4];
        subjects.push("Neue Nachricht von Anna K.");
        subjects.push("Neue Nachricht von Bernd");
        let mails: Vec<_> = subjects.into_iter().map(mail).collect();
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 1, "{kinds:#?}");
        assert_eq!(kinds[0].members.len(), 6);
    }

    #[test]
    fn counters_and_many_variants_behind_one_head_merge() {
        let counters: Vec<String> = (1..=7).map(|i| format!("Wochenbrief {i}")).collect();
        let mails: Vec<_> = counters.iter().map(|s| mail(s)).collect();
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 1, "{kinds:#?}");
        assert_eq!(kinds[0].label, "Wochenbrief …");

        // Four different tails behind one head vary by nature, even when
        // each one repeats.
        let mut subjects = Vec::new();
        for month in ["Januar", "Februar", "Mai", "Juni"] {
            for _ in 0..3 {
                subjects.push(format!("Rechnung: {month}"));
            }
        }
        let mails: Vec<_> = subjects.iter().map(|s| mail(s)).collect();
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 1, "{kinds:#?}");
        assert_eq!(kinds[0].label, "Rechnung: …");
    }

    #[test]
    fn caps_groups_and_collects_the_rest_under_sonstiges() {
        let subjects: Vec<String> = (0..12)
            .map(|i| {
                format!(
                    "{} Thema",
                    [
                        "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta",
                        "Iota", "Kappa", "Lambda", "My"
                    ][i]
                )
            })
            .collect();
        let mails: Vec<_> = subjects.iter().map(|s| mail(s)).collect();
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), MAX_GROUPS);
        let other = kinds.last().unwrap();
        assert_eq!(other.key, OTHER_KEY);
        assert_eq!(other.label, OTHER_LABEL);
        let total: usize = kinds.iter().map(|k| k.members.len()).sum();
        assert_eq!(total, 12);
    }

    #[test]
    fn result_does_not_depend_on_input_order() {
        let subjects = [
            "Angebot: rotes Fahrrad",
            "Angebot: Kinderwagen",
            "Versand Ihrer Bestellung 12345",
            "Versand Ihrer Bestellung 67890",
            "Willkommen",
            "Neue Angebote für Sie",
        ];
        let forward: Vec<_> = subjects.iter().map(|s| mail(s)).collect();
        let backward: Vec<_> = subjects.iter().rev().map(|s| mail(s)).collect();
        let a = group_mails(&forward);
        let b = group_mails(&backward);
        let describe = |kinds: &[Kind], reversed: bool| -> Vec<(String, String, Vec<&str>)> {
            kinds
                .iter()
                .map(|k| {
                    let mut subs: Vec<&str> = k
                        .members
                        .iter()
                        .map(|&i| {
                            if reversed {
                                subjects[subjects.len() - 1 - i]
                            } else {
                                subjects[i]
                            }
                        })
                        .collect();
                    subs.sort_unstable();
                    (k.key.clone(), k.label.clone(), subs)
                })
                .collect()
        };
        assert_eq!(describe(&a, false), describe(&b, true));
    }

    #[test]
    fn labels_are_readable() {
        let mails = vec![
            mail("Ihre Rechnung vom 12.03.2026"),
            mail("Ihre Rechnung vom 14.04.2026"),
            mail(""),
        ];
        let kinds = group_mails(&mails);
        assert_eq!(kind_of(&kinds, 0).label, "Ihre Rechnung vom …");
        assert_eq!(kind_of(&kinds, 2).label, "(kein Betreff)");
    }

    #[test]
    fn amount_splits_only_when_both_halves_are_substantial() {
        let mut mails = Vec::new();
        for _ in 0..3 {
            mails.push(with("Kontoinformation", |f| f.has_amount = true));
        }
        for _ in 0..3 {
            mails.push(mail("Kontoinformation"));
        }
        assert_eq!(group_mails(&mails).len(), 2);

        // One odd mail does not fragment the group.
        let mut mails: Vec<_> = (0..10).map(|_| mail("Kontoinformation")).collect();
        mails.push(with("Kontoinformation", |f| f.has_amount = true));
        let kinds = group_mails(&mails);
        assert_eq!(kinds.len(), 1);
        assert!(!kinds[0].features.contains(&"amount"));
    }

    #[test]
    fn detect_finds_text_features() {
        let f = KindFeatures::detect(
            "Dein Code",
            Some("Dein Bestätigungscode lautet 482913. Betrag 12,50 EUR."),
        );
        assert!(f.has_code);
        assert!(!f.has_tracking);
        assert_eq!(KindFeatures::detect("Hallo", None), KindFeatures::default());
    }
}
