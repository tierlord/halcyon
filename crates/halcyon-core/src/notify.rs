//! Which new mail deserves a desktop notification, and how it is worded.
//!
//! Halcyon only interrupts for mail that is likely meant for you; the rest
//! waits quietly in its place. Kept free of any UI so it can be tested alone.

use crate::sources::SourceMode;

/// What is known about one new mail when deciding on a notification.
#[derive(Clone, Copy, Debug, Default)]
pub struct NotifyInput {
    /// `List-Id` or `List-Unsubscribe` present: sent to many, not to you.
    pub is_list_mail: bool,
    /// Mode of the recurring source the sender belongs to, if it has one.
    pub source_mode: Option<SourceMode>,
    /// The source has an active anomaly that needs you.
    pub source_needs_you: bool,
    pub dmarc_pass: bool,
    /// Halcyon has seen this sender before (a recurring source whose mail
    /// passed DMARC, or someone you wrote to).
    pub sender_known: bool,
    /// What the user assigned to this mail or its kind. Pass the *effective*
    /// target (`Placement::target`), which already leaves out bundles that a
    /// protection holds back.
    pub assignment: Option<Assigned>,
    /// The phishing check holds the mail back (suspicious or phishing).
    pub suspicious: bool,
}

/// What the user decided for a mail, as far as a notification cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assigned {
    /// "Für dich".
    ForYou,
    /// Any bundle: always quiet.
    Bundle,
}

impl From<&crate::assignments::Target> for Assigned {
    fn from(t: &crate::assignments::Target) -> Self {
        match t {
            crate::assignments::Target::ForYou => Assigned::ForYou,
            crate::assignments::Target::Bundle { .. } => Assigned::Bundle,
        }
    }
}

pub fn should_notify(i: &NotifyInput) -> bool {
    // Nothing is worth waking the user for that tries to look like someone
    // else, whatever the sender check, an assignment or a source says.
    if i.suspicious {
        return false;
    }
    // The user's word comes first: mail in a bundle is not worth an
    // interruption, and "Für dich" is, as long as the sender check did not
    // fail (forged mail must not be wanted by accident).
    match i.assignment {
        Some(Assigned::Bundle) => return false,
        Some(Assigned::ForYou) => return i.dmarc_pass,
        None => {}
    }
    // An anomaly (a bill far above the usual, say) is the one case where a
    // quiet source speaks up.
    if i.source_needs_you {
        return true;
    }
    if i.is_list_mail {
        return false;
    }
    if matches!(
        i.source_mode,
        Some(SourceMode::Bundle | SourceMode::LatestOnly | SourceMode::Mute)
    ) {
        return false;
    }
    // Unverified mail from a stranger is more likely phishing or spam than
    // something to be woken for.
    i.dmarc_pass || i.sender_known
}

/// Title and body of the notification for the given `(sender, subject)`
/// pairs, newest last or first does not matter. `None` for an empty list.
pub fn notification_text(mails: &[(String, String)]) -> Option<(String, String)> {
    match mails {
        [] => None,
        [(sender, subject)] => Some((sender.clone(), subject.clone())),
        many => {
            let mut names: Vec<&str> = Vec::new();
            for (sender, _) in many {
                if !names.contains(&sender.as_str()) {
                    names.push(sender);
                }
            }
            let title = match names.as_slice() {
                [one] => format!("Neues von {one}"),
                [a, b] => format!("Neues von {a} und {b}"),
                [a, b, c] => format!("Neues von {a}, {b} und {c}"),
                [a, b, rest @ ..] => format!("Neues von {a}, {b} und {} weiteren", rest.len()),
                [] => unreachable!("`many` has at least two entries"),
            };
            let body = many
                .iter()
                .take(3)
                .map(|(_, subject)| subject.as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            Some((title, body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person() -> NotifyInput {
        NotifyInput {
            dmarc_pass: true,
            ..Default::default()
        }
    }

    #[test]
    fn personal_mail_notifies() {
        assert!(should_notify(&person()));
        // Shown (default) sources still notify.
        assert!(should_notify(&NotifyInput {
            source_mode: Some(SourceMode::Show),
            ..person()
        }));
    }

    #[test]
    fn lists_and_quiet_sources_stay_silent() {
        assert!(!should_notify(&NotifyInput {
            is_list_mail: true,
            ..person()
        }));
        for mode in [SourceMode::Bundle, SourceMode::LatestOnly, SourceMode::Mute] {
            assert!(
                !should_notify(&NotifyInput {
                    source_mode: Some(mode),
                    ..person()
                }),
                "{mode}"
            );
        }
    }

    #[test]
    fn anomaly_overrides_quiet_modes() {
        assert!(should_notify(&NotifyInput {
            is_list_mail: true,
            source_mode: Some(SourceMode::Mute),
            source_needs_you: true,
            ..person()
        }));
    }

    #[test]
    fn unverified_strangers_are_silent_but_known_senders_are_not() {
        let stranger = NotifyInput::default();
        assert!(!should_notify(&stranger));
        assert!(should_notify(&NotifyInput {
            sender_known: true,
            ..stranger
        }));
    }

    #[test]
    fn wording() {
        let m = |s: &str, t: &str| (s.to_string(), t.to_string());
        assert_eq!(notification_text(&[]), None);
        assert_eq!(
            notification_text(&[m("Anna", "Hallo")]),
            Some(("Anna".into(), "Hallo".into()))
        );
        let two = notification_text(&[m("Anna", "A"), m("Thomas", "B"), m("Anna", "C")]).unwrap();
        assert_eq!(two.0, "Neues von Anna und Thomas");
        assert_eq!(two.1, "A · B · C");
        let many = notification_text(&[
            m("Anna", "1"),
            m("Thomas", "2"),
            m("Eva", "3"),
            m("Jan", "4"),
            m("Lea", "5"),
        ])
        .unwrap();
        assert_eq!(many.0, "Neues von Anna, Thomas und 3 weiteren");
        assert_eq!(
            notification_text(&[m("Anna", "1"), m("Thomas", "2"), m("Eva", "3")])
                .unwrap()
                .0,
            "Neues von Anna, Thomas und Eva"
        );
        assert_eq!(
            notification_text(&[m("Anna", "1"), m("Anna", "2")])
                .unwrap()
                .0,
            "Neues von Anna"
        );
    }

    #[test]
    fn suspicious_mail_never_notifies() {
        let flagged = |base: NotifyInput| NotifyInput {
            suspicious: true,
            ..base
        };
        assert!(should_notify(&person()));
        assert!(!should_notify(&flagged(person())));
        // Not even when the user once assigned the kind to "Für dich" or a
        // source needs attention.
        assert!(!should_notify(&flagged(NotifyInput {
            assignment: Some(Assigned::ForYou),
            ..person()
        })));
        assert!(!should_notify(&flagged(NotifyInput {
            source_needs_you: true,
            ..person()
        })));
    }

    #[test]
    fn assignments_decide_before_the_rules() {
        let with = |assignment, base: NotifyInput| NotifyInput {
            assignment: Some(assignment),
            ..base
        };
        // A bundle silences even personal mail and anomalies.
        assert!(!should_notify(&with(Assigned::Bundle, person())));
        assert!(!should_notify(&with(
            Assigned::Bundle,
            NotifyInput {
                source_needs_you: true,
                ..person()
            }
        )));
        // "Für dich" notifies for lists and quiet sources ...
        let quiet = NotifyInput {
            is_list_mail: true,
            source_mode: Some(SourceMode::Mute),
            ..person()
        };
        assert!(should_notify(&with(Assigned::ForYou, quiet)));
        // ... but not when the sender check failed.
        assert!(!should_notify(&with(
            Assigned::ForYou,
            NotifyInput {
                dmarc_pass: false,
                sender_known: true,
                ..quiet
            }
        )));
    }
}
