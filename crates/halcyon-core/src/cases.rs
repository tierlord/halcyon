//! Data model for "Vorgänge" (cases): cross-message stories such as orders,
//! contracts, invoices, conversations, and travel bookings.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Broad category of a case, used to pick icons, labels, and state rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseKind {
    Order,
    Contract,
    Invoice,
    Conversation,
    Travel,
    Security,
    Support,
    Other,
}

impl fmt::Display for CaseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CaseKind::Order => "order",
            CaseKind::Contract => "contract",
            CaseKind::Invoice => "invoice",
            CaseKind::Conversation => "conversation",
            CaseKind::Travel => "travel",
            CaseKind::Security => "security",
            CaseKind::Support => "support",
            CaseKind::Other => "other",
        })
    }
}

impl FromStr for CaseKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "order" => Ok(CaseKind::Order),
            "contract" => Ok(CaseKind::Contract),
            "invoice" => Ok(CaseKind::Invoice),
            "conversation" => Ok(CaseKind::Conversation),
            "travel" => Ok(CaseKind::Travel),
            "security" => Ok(CaseKind::Security),
            "support" => Ok(CaseKind::Support),
            "other" => Ok(CaseKind::Other),
            other => Err(Error::Other(format!("unknown case kind {other:?}"))),
        }
    }
}

/// Current lifecycle status of a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    NeedsYou,
    Waiting,
    Aside,
    Done,
    /// User explicitly marked "Dies ist kein Vorgang".
    Rejected,
}

impl fmt::Display for CaseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CaseStatus::NeedsYou => "needs_you",
            CaseStatus::Waiting => "waiting",
            CaseStatus::Aside => "aside",
            CaseStatus::Done => "done",
            CaseStatus::Rejected => "rejected",
        })
    }
}

impl FromStr for CaseStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "needs_you" => Ok(CaseStatus::NeedsYou),
            "waiting" => Ok(CaseStatus::Waiting),
            "aside" => Ok(CaseStatus::Aside),
            "done" => Ok(CaseStatus::Done),
            "rejected" => Ok(CaseStatus::Rejected),
            other => Err(Error::Other(format!("unknown case status {other:?}"))),
        }
    }
}

/// Role of an event on a case's chronology.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseEventPhase {
    Start,
    Update,
    Due,
    Done,
}

impl fmt::Display for CaseEventPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CaseEventPhase::Start => "start",
            CaseEventPhase::Update => "update",
            CaseEventPhase::Due => "due",
            CaseEventPhase::Done => "done",
        })
    }
}

impl FromStr for CaseEventPhase {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "start" => Ok(CaseEventPhase::Start),
            "update" => Ok(CaseEventPhase::Update),
            "due" => Ok(CaseEventPhase::Due),
            "done" => Ok(CaseEventPhase::Done),
            other => Err(Error::Other(format!("unknown case event phase {other:?}"))),
        }
    }
}

/// Direction of the message that produced an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventDirection {
    In,
    Out,
}

impl fmt::Display for EventDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EventDirection::In => "in",
            EventDirection::Out => "out",
        })
    }
}

impl FromStr for EventDirection {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "in" => Ok(EventDirection::In),
            "out" => Ok(EventDirection::Out),
            other => Err(Error::Other(format!("unknown event direction {other:?}"))),
        }
    }
}

/// Stored case row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    pub id: i64,
    pub title: String,
    pub counterparty: String,
    pub domain: Option<String>,
    pub kind: CaseKind,
    pub status: CaseStatus,
    pub next_step: String,
    pub due_at: Option<i64>,
    pub due_label: Option<String>,
    /// Open amount in minor units (cents), if known.
    pub amount_cents: Option<i64>,
    /// ISO 4217 code for `amount_cents`, e.g. `EUR`.
    pub currency: Option<String>,
    pub reason: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Input for creating a new case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCase {
    pub title: String,
    pub counterparty: String,
    pub domain: Option<String>,
    pub kind: CaseKind,
    pub status: CaseStatus,
    pub next_step: String,
    pub due_at: Option<i64>,
    pub due_label: Option<String>,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
    pub reason: String,
}

/// Stored event belonging to a case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseEvent {
    pub id: i64,
    pub case_id: i64,
    pub message_id: Option<i64>,
    pub occurred_at: i64,
    /// Machine-readable event type such as `order.shipped`; `None` for
    /// manual or unclassified events.
    pub kind: Option<String>,
    pub label: String,
    pub phase: CaseEventPhase,
    pub direction: Option<EventDirection>,
    pub confidence: f32,
    pub manual: bool,
}

/// Input for appending an event to a case.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewCaseEvent {
    pub message_id: Option<i64>,
    pub occurred_at: i64,
    /// Machine-readable event type such as `order.shipped`; `None` for
    /// manual or unclassified events.
    pub kind: Option<String>,
    pub label: String,
    pub phase: CaseEventPhase,
    pub direction: Option<EventDirection>,
    pub confidence: f32,
    pub manual: bool,
}

/// Identifying key (order number, invoice number, contract id, thread root, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseKey {
    pub label: String,
    pub value: String,
    pub normalized: String,
}

/// Full case view with its chronology, keys, and linked message ids.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseView {
    #[serde(flatten)]
    pub case: Case,
    pub events: Vec<CaseEvent>,
    pub keys: Vec<CaseKey>,
    pub message_ids: Vec<i64>,
}

/// Normalizes a case key value for case-insensitive matching.
pub fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
