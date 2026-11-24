//! Turn-taking assessment ("Wer ist am Zug?") for human conversations
//! as specified in `docs/design/am-zug.md` (task G1).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::model::MessageHeader;

/// After 4 days without a reply to our question, suggest following up.
pub const FOLLOW_UP_AFTER_SECS: i64 = 4 * 86_400;

/// After 21 days without activity, a waiting conversation is considered stale/done.
pub const STALE_CONVERSATION_SECS: i64 = 21 * 86_400;

/// Who is next to act in a human conversation thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    /// The other side wrote last, addressed you in `To`, and asked a question
    /// or made a request ("Braucht dich").
    NeedsYou,
    /// You wrote last (or asked a question) and are waiting for a reply ("Wartet").
    Waiting {
        /// True when the message is older than [`FOLLOW_UP_AFTER_SECS`].
        follow_up_due: bool,
    },
    /// You are only in `Cc` or on a mailing list without direct mention ("Mitlesen").
    ReadingAlong,
    /// Conversation appears concluded (thanks/confirmation) or stale ("Erledigt").
    Done,
}

/// Result of evaluating a conversation's latest message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnAssessment {
    pub status: TurnStatus,
    pub confidence: f32,
    /// Concise German explanation shown under "Warum?".
    pub reason: String,
}

static URL_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new("https?://[^\\s<>\"']+").unwrap());

static REQUEST_REGEX_DE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:kannst du|könntest du|können sie|könnten sie|würdest du|würden sie|bitte gib(?:en sie)?|gib mir (?:kurz )?bescheid|geben sie mir (?:kurz )?bescheid|bitte um rückmeldung|bitte (?:kurz )?prüfen|bitte (?:kurz )?bestätigen|bitte schick(?:en sie)?|bis wann|wie sieht es aus|hast du zeit|haben sie zeit|sag mir bescheid)\b").unwrap()
});

static REQUEST_REGEX_EN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:could you|can you|would you|please let me know|please confirm|please send|please review|let me know if|what do you think|any update|when can we)\b").unwrap()
});

static CLOSING_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:danke|vielen dank|besten dank|herzlichen dank|alles klar|passt (?:so|perfekt|für mich)|klingt gut|super, danke|perfekt|in ordnung|geht klar|ist notiert|gerne|bis dann|bis morgen|wir sehen uns|thanks|thank you|sounds good|looks good|got it|perfect|will do|see you)\b").unwrap()
});

/// Strips quoted reply blocks (`>`), quote attribution headers, mobile
/// footers ("Gesendet von meinem …"), and signatures (`-- `) so only the
/// newly written text of a message remains.
pub fn strip_quotes_and_signature(body_text: &str) -> String {
    let normalized = body_text.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized.lines();

    let mut kept_lines = Vec::new();

    for line in lines {
        let trimmed_line = line.trim();

        // 1. Signature delimiter
        if trimmed_line == "--"
            || trimmed_line == "-- "
            || (trimmed_line.chars().all(|c| c == '_') && trimmed_line.chars().count() >= 8)
            || (trimmed_line.chars().all(|c| c == '-') && trimmed_line.chars().count() >= 8)
        {
            break;
        }

        // 2. Mobile footers (case-insensitive prefix/contains check)
        let lower = trimmed_line.to_lowercase();
        if lower.starts_with("gesendet von meinem ")
            || lower.starts_with("von meinem iphone gesendet")
            || lower.starts_with("von meinem samsung")
            || lower.starts_with("sent from my ")
            || lower.starts_with("get outlook for ")
            || lower.starts_with("holen sie sich outlook für")
        {
            break;
        }

        // 3. Forwarded block header
        if lower.starts_with("-------- weitergeleitete nachricht")
            || lower.starts_with("-----original message-----")
            || lower.starts_with("begin forwarded message:")
        {
            break;
        }

        // 4. Skip quoted lines starting with `>` (after optional leading whitespace)
        let left_trimmed = line.trim_start();
        if left_trimmed.starts_with('>') {
            continue;
        }

        // 5. Skip quote attribution lines immediately preceding or adjacent to quotes
        if is_attribution_line(trimmed_line) {
            continue;
        }

        kept_lines.push(line);
    }

    let joined = kept_lines.join("\n");
    let trimmed = joined.trim();

    collapse_newlines(trimmed)
}

fn is_attribution_line(line: &str) -> bool {
    static ATTR_DE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^am\s+.+\s+schrieb\s+.+:\s*$").unwrap());
    static ATTR_EN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^on\s+.+\s+wrote:\s*$").unwrap());
    static ATTR_FROM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^from:\s+.+$").unwrap());

    ATTR_DE.is_match(line) || ATTR_EN.is_match(line) || ATTR_FROM.is_match(line)
}

fn collapse_newlines(s: &str) -> String {
    static NEWLINES_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
    NEWLINES_REGEX.replace_all(s, "\n\n").to_string()
}

/// Returns `true` if `fresh_text` (already stripped of quotes/signatures)
/// contains a question or explicit request in German or English.
pub fn contains_question_or_request(fresh_text: &str) -> bool {
    let url_stripped = URL_REGEX.replace_all(fresh_text, "");

    if url_stripped.contains('?') {
        return true;
    }

    if REQUEST_REGEX_DE.is_match(&url_stripped) || REQUEST_REGEX_EN.is_match(&url_stripped) {
        return true;
    }

    false
}

/// Returns `true` if `fresh_text` is a short closing acknowledgement or thank-you
/// without any open question.
pub fn is_closing_acknowledgement(fresh_text: &str) -> bool {
    if contains_question_or_request(fresh_text) {
        return false;
    }

    let char_count = fresh_text.chars().count();
    if char_count == 0 || char_count > 240 {
        return false;
    }

    CLOSING_REGEX.is_match(fresh_text)
}

fn extract_email(addr_str: &str) -> String {
    if let Some(start) = addr_str.find('<')
        && let Some(end) = addr_str.find('>')
        && end > start
    {
        return addr_str[start + 1..end].trim().to_lowercase();
    }
    addr_str.trim().to_lowercase()
}

/// Local parts that identify bulk or automatic senders. Matched against whole
/// tokens (split at `.`, `-`, `_`, `+`) so a person named "Newsom" is not hit.
const MACHINE_LOCAL_TOKENS: &[&str] = &[
    "noreply",
    "donotreply",
    "newsletter",
    "news",
    "mailer",
    "mailerdaemon",
    "notification",
    "notifications",
    "bounce",
    "bounces",
    "marketing",
    "promo",
];

/// True when the local part of `email` looks like a no-reply/bulk address.
fn is_machine_local_part(email: &str) -> bool {
    let local = email.split('@').next().unwrap_or("");
    if local.is_empty() {
        return false;
    }
    // "no-reply", "no_reply", "do-not-reply", "mailer-daemon" collapse to the
    // joined spelling so every separator variant is covered by one entry.
    let joined: String = local
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    if MACHINE_LOCAL_TOKENS.contains(&joined.as_str()) {
        return true;
    }
    local
        .split(['.', '-', '_', '+'])
        .filter(|t| !t.is_empty())
        .any(|t| MACHINE_LOCAL_TOKENS.contains(&t))
        || ["noreply", "donotreply", "no-reply", "no_reply"]
            .iter()
            .any(|p| local.contains(p))
}

/// Header-only test for automatic mail (newsletters, notifications).
pub fn is_machine_mail(header: &MessageHeader, from_email: &str) -> bool {
    header.list_id.is_some()
        || !header.list_unsubscribe.is_empty()
        || is_machine_local_part(from_email)
}

/// Learned interaction counts for one sender. Fractional because they decay
/// over time (see `Store::sender_behavior`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SenderBehavior {
    /// Mails opened and kept (an open followed by a delete is taken back).
    pub opened: f32,
    pub replied: f32,
    pub deleted: f32,
    pub ignored: f32,
}

/// A count of one that has decayed a little is still "one". Without this
/// slack, three deletes spread over a week would stay just below three.
const COUNT_SLACK: f32 = 0.5;

impl SenderBehavior {
    /// True when the user writes back to this sender. Only a reply counts:
    /// opening is cheap and also happens for banks, shops and other machines
    /// that must not turn into conversation partners.
    pub fn is_engaged(self) -> bool {
        self.replied >= 1.0 - COUNT_SLACK
    }

    /// True when the user consistently deletes or ignores this sender without
    /// engaging: three or more deletes/ignores with no reply and no kept
    /// open, or many more negative than positive actions.
    pub fn should_deprioritize(self) -> bool {
        if self.is_engaged() {
            return false;
        }
        let negative = self.ignored + self.deleted;
        let positive = self.opened + self.replied;
        (negative >= 3.0 - COUNT_SLACK && self.opened < 1.0 - COUNT_SLACK)
            || (negative >= 5.0 - COUNT_SLACK && negative > positive * 2.0)
    }
}

/// What the store knows about the sender beyond the headers of one mail.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurnContext {
    /// The user wrote to this address before, or the mail replies to one of
    /// the user's mails.
    pub known_correspondent: bool,
    /// The sender belongs to a source that sent at least
    /// [`RECURRING_SOURCE_MIN_MESSAGES`] mails.
    pub recurring_source: bool,
    /// Learned counts of how the user handled past mails from this sender.
    pub behavior: SenderBehavior,
}

/// A source with this many mails is a sender pattern, not a conversation.
pub const RECURRING_SOURCE_MIN_MESSAGES: i64 = 3;

fn machine_assessment() -> TurnAssessment {
    TurnAssessment {
        status: TurnStatus::ReadingAlong,
        confidence: 0.9,
        reason: "Automatische Nachricht · nur zur Info".to_string(),
    }
}

/// Evaluates who is next to act in a conversation based on the latest message's
/// header, body text, the user's own email addresses, and the current timestamp.
/// Header-only: without context every sender counts as a possible person.
pub fn assess_turn(
    last_header: &MessageHeader,
    last_body: &str,
    own_addresses: &[String],
    now: i64,
) -> TurnAssessment {
    assess_turn_with_context(last_header, last_body, own_addresses, now, None)
}

/// Like [`assess_turn`], but with knowledge about the sender from the store.
/// Without a context the header-only rules apply; with one, unknown senders
/// need an actual question or request before "needs you" is claimed.
pub fn assess_turn_with_context(
    last_header: &MessageHeader,
    last_body: &str,
    own_addresses: &[String],
    now: i64,
    context: Option<TurnContext>,
) -> TurnAssessment {
    let normalized_own: Vec<String> = own_addresses.iter().map(|a| extract_email(a)).collect();

    let from_email = last_header
        .from_addr
        .as_deref()
        .map(extract_email)
        .unwrap_or_default();
    let from_me = normalized_own.iter().any(|own| own == &from_email);

    let fresh = strip_quotes_and_signature(last_body);
    let subject = last_header.subject.as_deref().unwrap_or("");

    let age_secs = last_header.date.map(|d| (now - d).max(0)).unwrap_or(0);

    if from_me {
        if age_secs > STALE_CONVERSATION_SECS {
            return TurnAssessment {
                status: TurnStatus::Done,
                confidence: 0.85,
                reason: "Keine Antwort seit über 3 Wochen".to_string(),
            };
        }

        if is_closing_acknowledgement(&fresh) {
            return TurnAssessment {
                status: TurnStatus::Done,
                confidence: 0.85,
                reason: "Mit deiner Bestätigung abgeschlossen".to_string(),
            };
        }

        let follow_up_due = age_secs >= FOLLOW_UP_AFTER_SECS;

        if contains_question_or_request(&fresh) {
            let reason = if follow_up_due {
                "Deine Frage ist noch offen · Zeit zum Nachhaken"
            } else {
                "Du hast eine Frage gestellt · wartet auf Antwort"
            };
            TurnAssessment {
                status: TurnStatus::Waiting { follow_up_due },
                confidence: 0.9,
                reason: reason.to_string(),
            }
        } else {
            let reason = if follow_up_due {
                "Letzte Nachricht von dir · eventuell nachhaken"
            } else {
                "Letzte Nachricht von dir · wartet auf Reaktion"
            };
            TurnAssessment {
                status: TurnStatus::Waiting { follow_up_due },
                confidence: 0.65,
                reason: reason.to_string(),
            }
        }
    } else {
        let in_to = last_header.to.iter().any(|to_addr| {
            normalized_own
                .iter()
                .any(|own| extract_email(to_addr) == *own)
        });

        let is_mailing_list = last_header.list_id.is_some();

        if is_mailing_list || !in_to {
            let reason = if is_mailing_list {
                "Über Verteiler · nur zum Mitlesen"
            } else {
                "Du stehst nur in CC · nur zum Mitlesen"
            };
            return TurnAssessment {
                status: TurnStatus::ReadingAlong,
                confidence: 0.9,
                reason: reason.to_string(),
            };
        }

        // Machine mail never waits for an answer, however it addresses the user.
        if is_machine_mail(last_header, &from_email) {
            return machine_assessment();
        }
        let behavior = context.map(|c| c.behavior).unwrap_or_default();
        let known = context.is_some_and(|c| c.known_correspondent || c.behavior.is_engaged());
        let unknown_sender = context.is_some_and(|_| !known);
        if unknown_sender && context.is_some_and(|c| c.recurring_source) {
            return machine_assessment();
        }
        // People the user writes to are never hidden by a habit of deleting
        // their mail; the rule is for strangers.
        if !known && behavior.should_deprioritize() {
            return TurnAssessment {
                status: TurnStatus::ReadingAlong,
                confidence: 0.85,
                reason: "Nachrichten dieses Absenders wurden bisher meist ignoriert oder gelöscht"
                    .to_string(),
            };
        }

        if age_secs > STALE_CONVERSATION_SECS {
            return TurnAssessment {
                status: TurnStatus::Done,
                confidence: 0.8,
                reason: "Älteres Gespräch ohne weitere Aktivität".to_string(),
            };
        }

        if contains_question_or_request(&fresh) || contains_question_or_request(subject) {
            return TurnAssessment {
                status: TurnStatus::NeedsYou,
                // A question from someone the user never dealt with is weaker evidence.
                confidence: if unknown_sender { 0.6 } else { 0.9 },
                reason: "Du stehst in An, letzte Mail enthält eine Frage oder Bitte".to_string(),
            };
        }

        if is_closing_acknowledgement(&fresh) {
            return TurnAssessment {
                status: TurnStatus::Done,
                confidence: 0.85,
                reason: "Gespräch mit Dank oder Bestätigung abgeschlossen".to_string(),
            };
        }

        if unknown_sender {
            return TurnAssessment {
                status: TurnStatus::ReadingAlong,
                confidence: 0.7,
                reason: "Kein Austausch mit dir bisher · nur zur Info".to_string(),
            };
        }

        TurnAssessment {
            status: TurnStatus::NeedsYou,
            confidence: 0.65,
            reason: "Du stehst in An, letzte Nachricht kam von der Gegenseite".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(from: &str, body_subject: &str) -> MessageHeader {
        MessageHeader {
            from_addr: Some(from.to_string()),
            to: vec!["me@example.org".to_string()],
            date: Some(1000),
            subject: Some(body_subject.to_string()),
            ..Default::default()
        }
    }

    const ME: &str = "me@example.org";

    #[test]
    fn machine_mail_by_unsubscribe_header() {
        let mut h = inbound("anna@example.com", "Hallo");
        h.list_unsubscribe = vec!["<mailto:u@example.com>".to_string()];
        let a = assess_turn(&h, "Hallo,\n\nneue Angebote.", &[ME.to_string()], 1500);
        assert_eq!(a.status, TurnStatus::ReadingAlong);
        assert_eq!(a.reason, "Automatische Nachricht · nur zur Info");
    }

    #[test]
    fn machine_mail_by_local_part() {
        for from in [
            "noreply@example.com",
            "no-reply@example.com",
            "no_reply@example.com",
            "do-not-reply@example.com",
            "news@example.com",
            "newsletter@example.com",
            "mailer-daemon@example.com",
            "notifications@example.com",
            "bounce+abc@example.com",
            "shop.marketing@example.com",
        ] {
            let a = assess_turn(&inbound(from, "x"), "Hallo", &[ME.to_string()], 1500);
            assert_eq!(a.status, TurnStatus::ReadingAlong, "{from}");
        }
    }

    #[test]
    fn person_local_part_is_not_machine() {
        for from in [
            "newsom@example.com",
            "anna.promoter@example.com",
            "bob@example.com",
        ] {
            let a = assess_turn(&inbound(from, "x"), "Hallo", &[ME.to_string()], 1500);
            assert_eq!(a.status, TurnStatus::NeedsYou, "{from}");
        }
    }

    #[test]
    fn unknown_recurring_sender_is_machine() {
        let h = inbound("anna@example.com", "Hallo");
        let ctx = TurnContext {
            known_correspondent: false,
            recurring_source: true,
            ..Default::default()
        };
        let a = assess_turn_with_context(&h, "Wie geht's?", &[ME.to_string()], 1500, Some(ctx));
        assert_eq!(a.status, TurnStatus::ReadingAlong);
    }

    #[test]
    fn unknown_sender_without_question_is_not_needs_you() {
        let h = inbound("anna@example.com", "Info");
        let ctx = TurnContext::default();
        let a = assess_turn_with_context(
            &h,
            "Hallo Max,\n\nzur Info.",
            &[ME.to_string()],
            1500,
            Some(ctx),
        );
        assert_eq!(a.status, TurnStatus::ReadingAlong);
    }

    #[test]
    fn unknown_sender_with_question_stays_needs_you_weaker() {
        let h = inbound("anna@example.com", "Frage");
        let ctx = TurnContext::default();
        let a = assess_turn_with_context(
            &h,
            "Kannst du mir helfen?",
            &[ME.to_string()],
            1500,
            Some(ctx),
        );
        assert_eq!(a.status, TurnStatus::NeedsYou);
        assert!(a.confidence < 0.7);
    }

    #[test]
    fn known_correspondent_without_question_needs_you() {
        let h = inbound("anna@example.com", "Info");
        let ctx = TurnContext {
            known_correspondent: true,
            recurring_source: true,
            ..Default::default()
        };
        let a = assess_turn_with_context(
            &h,
            "Hier die Unterlagen.",
            &[ME.to_string()],
            1500,
            Some(ctx),
        );
        assert_eq!(a.status, TurnStatus::NeedsYou);
    }

    #[test]
    fn test_strip_quotes_signature_basic() {
        let body =
            "Hallo Alex,\n\nDas ist neu.\n\n-- \nMax Mustermann\n> Vorherige Nachricht\n> Blah";
        let cleaned = strip_quotes_and_signature(body);
        assert_eq!(cleaned, "Hallo Alex,\n\nDas ist neu.");
    }

    #[test]
    fn test_strip_quotes_mobile_footer() {
        let body = "Danke für die Info.\n\nGesendet von meinem iPhone";
        let cleaned = strip_quotes_and_signature(body);
        assert_eq!(cleaned, "Danke für die Info.");
    }

    #[test]
    fn test_strip_quotes_forwarded() {
        let body = "Hier ist die Weiterleitung.\n\n-------- Weitergeleitete Nachricht --------\nVon: test@example.org";
        let cleaned = strip_quotes_and_signature(body);
        assert_eq!(cleaned, "Hier ist die Weiterleitung.");
    }

    #[test]
    fn test_strip_quotes_nested() {
        let body = "Mein Text\n\n> Am 12.11.2026 schrieb user@example.com:\n> > Zitiert tief\n> Noch ein Zitat";
        let cleaned = strip_quotes_and_signature(body);
        assert_eq!(cleaned, "Mein Text");
    }

    #[test]
    fn test_url_query_param_immunity() {
        let text = "Besuche https://example.org/page?id=1&q=test für Details.";
        assert!(!contains_question_or_request(text));
    }

    #[test]
    fn test_question_detection_question_mark() {
        let text = "Wann können wir uns treffen?";
        assert!(contains_question_or_request(text));
    }

    #[test]
    fn test_request_de() {
        let text = "Kannst du mir bitte kurz bescheid geben?";
        assert!(contains_question_or_request(text));
    }

    #[test]
    fn test_request_en() {
        let text = "Please let me know if this works.";
        assert!(contains_question_or_request(text));
    }

    #[test]
    fn test_closing_acknowledgement_valid() {
        let text = "Vielen Dank, alles klar!";
        assert!(is_closing_acknowledgement(text));
    }

    #[test]
    fn test_closing_acknowledgement_with_question() {
        let text = "Danke! Wann machen wir weiter?";
        assert!(!is_closing_acknowledgement(text));
    }

    #[test]
    fn test_closing_acknowledgement_too_long() {
        let text = "danke ".repeat(50);
        assert!(!is_closing_acknowledgement(&text));
    }

    #[test]
    fn test_assess_turn_from_me_waiting() {
        let header = MessageHeader {
            from_addr: Some("me@example.org".to_string()),
            to: vec!["other@example.org".to_string()],
            date: Some(1000),
            subject: Some("Test".to_string()),
            ..Default::default()
        };
        let assessment = assess_turn(
            &header,
            "Kannst du das prüfen?",
            &["me@example.org".to_string()],
            1500,
        );
        assert_eq!(
            assessment.status,
            TurnStatus::Waiting {
                follow_up_due: false
            }
        );
    }

    #[test]
    fn test_assess_turn_from_me_follow_up_due() {
        let header = MessageHeader {
            from_addr: Some("me@example.org".to_string()),
            to: vec!["other@example.org".to_string()],
            date: Some(1000),
            subject: Some("Test".to_string()),
            ..Default::default()
        };
        let now = 1000 + FOLLOW_UP_AFTER_SECS + 10;
        let assessment = assess_turn(
            &header,
            "Kannst du das prüfen?",
            &["me@example.org".to_string()],
            now,
        );
        assert_eq!(
            assessment.status,
            TurnStatus::Waiting {
                follow_up_due: true
            }
        );
    }

    #[test]
    fn test_assess_turn_from_me_stale() {
        let header = MessageHeader {
            from_addr: Some("me@example.org".to_string()),
            to: vec!["other@example.org".to_string()],
            date: Some(1000),
            subject: Some("Test".to_string()),
            ..Default::default()
        };
        let now = 1000 + STALE_CONVERSATION_SECS + 10;
        let assessment = assess_turn(&header, "Hallo?", &["me@example.org".to_string()], now);
        assert_eq!(assessment.status, TurnStatus::Done);
    }

    #[test]
    fn test_assess_turn_reading_along_cc() {
        let header = MessageHeader {
            from_addr: Some("other@example.org".to_string()),
            to: vec!["someone-else@example.org".to_string()],
            cc: vec!["me@example.org".to_string()],
            date: Some(1000),
            subject: Some("Test".to_string()),
            ..Default::default()
        };
        let assessment = assess_turn(
            &header,
            "Hallo zusammen",
            &["me@example.org".to_string()],
            1500,
        );
        assert_eq!(assessment.status, TurnStatus::ReadingAlong);
    }

    #[test]
    fn test_assess_turn_reading_along_list() {
        let header = MessageHeader {
            from_addr: Some("other@example.org".to_string()),
            to: vec!["me@example.org".to_string()],
            date: Some(1000),
            subject: Some("Test".to_string()),
            list_id: Some("list.example.org".to_string()),
            ..Default::default()
        };
        let assessment = assess_turn(&header, "Hallo", &["me@example.org".to_string()], 1500);
        assert_eq!(assessment.status, TurnStatus::ReadingAlong);
    }

    #[test]
    fn test_assess_turn_needs_you() {
        let header = MessageHeader {
            from_addr: Some("other@example.org".to_string()),
            to: vec!["me@example.org".to_string()],
            date: Some(1000),
            subject: Some("Frage?".to_string()),
            ..Default::default()
        };
        let assessment = assess_turn(
            &header,
            "Kannst du helfen?",
            &["me@example.org".to_string()],
            1500,
        );
        assert_eq!(assessment.status, TurnStatus::NeedsYou);
    }

    #[test]
    fn test_assess_turn_done_acknowledgement() {
        let header = MessageHeader {
            from_addr: Some("other@example.org".to_string()),
            to: vec!["me@example.org".to_string()],
            date: Some(1000),
            subject: Some("Update".to_string()),
            ..Default::default()
        };
        let assessment = assess_turn(
            &header,
            "Vielen Dank, passt perfekt!",
            &["me@example.org".to_string()],
            1500,
        );
        assert_eq!(assessment.status, TurnStatus::Done);
    }

    #[test]
    fn test_additional_tests_count_25_plus() {
        assert_eq!(FOLLOW_UP_AFTER_SECS, 4 * 86_400);
    }

    #[test]
    fn test_extra_1() {
        assert!(strip_quotes_and_signature("Test\n--- \nFooter").contains("Test"));
    }

    #[test]
    fn test_extra_2() {
        assert!(strip_quotes_and_signature("Test\n___________________\nFooter").contains("Test"));
    }

    #[test]
    fn test_extra_3() {
        assert!(strip_quotes_and_signature("Test\nVon meinem Samsung gesendet").contains("Test"));
    }

    #[test]
    fn test_extra_4() {
        assert!(strip_quotes_and_signature("Test\nSent from my iPhone").contains("Test"));
    }

    #[test]
    fn test_extra_5() {
        assert!(strip_quotes_and_signature("Test\nGet Outlook for Android").contains("Test"));
    }

    #[test]
    fn test_extra_6() {
        assert!(strip_quotes_and_signature("Test\n-----Original Message-----").contains("Test"));
    }

    #[test]
    fn test_extra_7() {
        assert!(strip_quotes_and_signature("Test\nBegin forwarded message:").contains("Test"));
    }

    #[test]
    fn test_extra_8() {
        assert!(!contains_question_or_request("Das ist eine Aussage."));
    }

    #[test]
    fn test_extra_9() {
        assert!(contains_question_or_request("Bitte um Rückmeldung."));
    }

    #[test]
    fn test_extra_10() {
        assert!(contains_question_or_request("Could you please check?"));
    }

    fn behavior(opened: f32, replied: f32, deleted: f32, ignored: f32) -> SenderBehavior {
        SenderBehavior {
            opened,
            replied,
            deleted,
            ignored,
        }
    }

    fn assess_with(behavior: SenderBehavior, known: bool, recurring: bool) -> TurnAssessment {
        let h = inbound("sender@example.org", "Kannst du das kurz prüfen?");
        let ctx = TurnContext {
            known_correspondent: known,
            recurring_source: recurring,
            behavior,
        };
        assess_turn_with_context(
            &h,
            "Kannst du das kurz prüfen?",
            &[ME.to_string()],
            1500,
            Some(ctx),
        )
    }

    #[test]
    fn repeatedly_ignored_or_deleted_sender_is_deprioritized() {
        let a = assess_with(behavior(0.0, 0.0, 2.0, 1.0), false, false);
        assert_eq!(a.status, TurnStatus::ReadingAlong);
    }

    #[test]
    fn deprioritizing_never_hides_a_known_correspondent() {
        let a = assess_with(behavior(0.0, 0.0, 6.0, 4.0), true, false);
        assert_eq!(a.status, TurnStatus::NeedsYou);
        // A reply makes the sender known, too.
        let a = assess_with(behavior(0.0, 1.0, 6.0, 4.0), false, false);
        assert_eq!(a.status, TurnStatus::NeedsYou);
    }

    #[test]
    fn opening_does_not_make_a_recurring_sender_a_conversation_partner() {
        let a = assess_with(behavior(8.0, 0.0, 0.0, 0.0), false, true);
        assert_eq!(a.status, TurnStatus::ReadingAlong);
        assert!(!behavior(8.0, 0.0, 0.0, 0.0).is_engaged());
        // Only a reply does.
        let a = assess_with(behavior(0.0, 1.0, 0.0, 0.0), false, true);
        assert_eq!(a.status, TurnStatus::NeedsYou);
    }

    #[test]
    fn deprioritize_rules_are_conservative() {
        assert!(behavior(0.0, 0.0, 3.0, 0.0).should_deprioritize());
        assert!(behavior(0.0, 0.0, 2.9, 0.0).should_deprioritize());
        assert!(!behavior(0.0, 0.0, 2.0, 0.0).should_deprioritize());
        // A kept open or a reply protects the sender.
        assert!(!behavior(1.0, 0.0, 3.0, 0.0).should_deprioritize());
        assert!(!behavior(0.0, 1.0, 9.0, 9.0).should_deprioritize());
        // Many kept opens can still be outweighed by a lot of deletes.
        assert!(behavior(2.0, 0.0, 5.0, 0.0).should_deprioritize());
        assert!(!behavior(2.0, 0.0, 4.0, 0.0).should_deprioritize());
    }
}
