use serde::{Deserialize, Serialize};

/// Largest delivery body accepted by the ledger, matching the DM message
/// bound so a relayed peer message can never exceed what a user could send.
pub const MAX_HIVE_DELIVERY_BODY_BYTES: usize = 64 * 1024;

/// Default delivery attempt budget before a row dead-letters.
pub const DEFAULT_HIVE_DELIVERY_MAX_ATTEMPTS: u32 = 5;

/// What kind of payload this delivery carries. Only Worker-to-Worker
/// messages exist today; the CHECK constraint is the extension point for
/// future kinds (heartbeats, schedule retargets, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HiveDeliveryKind {
    #[default]
    WorkerMessage,
}

impl HiveDeliveryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkerMessage => "worker_message",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "worker_message" => Some(Self::WorkerMessage),
            _ => None,
        }
    }
}

/// Delivery urgency. `High` may steer the recipient's active run; `Normal`
/// waits until the recipient's lane is idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HiveDeliveryPriority {
    #[default]
    Normal,
    High,
}

impl HiveDeliveryPriority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Ledger state machine:
///
/// ```text
/// pending ──claim──▶ delivering ──effect commit──▶ delivered ──run terminal──▶ acked
///    ▲                   │  │
///    │   wait/backoff    │  └── attempts exhausted ──▶ dead_letter
///    └───────────────────┘
/// ```
///
/// A crash between claim and effect leaves the row `delivering`; its
/// claim-time `available_at` backoff makes it due again and the effect is
/// replayed idempotently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveDeliveryStatus {
    Pending,
    Delivering,
    Delivered,
    Acked,
    DeadLetter,
}

impl HiveDeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::Delivered => "delivered",
            Self::Acked => "acked",
            Self::DeadLetter => "dead_letter",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "delivering" => Some(Self::Delivering),
            "delivered" => Some(Self::Delivered),
            "acked" => Some(Self::Acked),
            "dead_letter" => Some(Self::DeadLetter),
            _ => None,
        }
    }
}

/// One durable message-per-recipient row in the delivery ledger.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HiveDelivery {
    pub id: String,
    pub kind: HiveDeliveryKind,
    /// Sending Worker; NULL for system-originated deliveries.
    pub from_worker_id: Option<String>,
    pub to_worker_id: String,
    /// Group the sender was working in when it sent this, as context only;
    /// delivery always lands on the recipient's private DM lane.
    pub group_id: Option<String>,
    pub body: String,
    pub priority: HiveDeliveryPriority,
    /// Sender-scoped idempotency key; a retried enqueue with the same key
    /// adopts the existing row instead of duplicating it.
    pub dedupe_key: Option<String>,
    pub status: HiveDeliveryStatus,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub available_at: String,
    pub delivered_at: Option<String>,
    pub acked_at: Option<String>,
    pub last_error: Option<String>,
    /// The run this delivery woke or steered ("why did this Worker wake").
    pub run_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for enqueueing one delivery.
#[derive(Debug, Clone)]
pub struct NewHiveDelivery {
    pub kind: HiveDeliveryKind,
    pub from_worker_id: Option<String>,
    pub to_worker_id: String,
    pub group_id: Option<String>,
    pub body: String,
    pub priority: HiveDeliveryPriority,
    pub dedupe_key: Option<String>,
    pub max_attempts: u32,
}

impl NewHiveDelivery {
    pub fn worker_message(to_worker_id: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind: HiveDeliveryKind::WorkerMessage,
            from_worker_id: None,
            to_worker_id: to_worker_id.into(),
            group_id: None,
            body: body.into(),
            priority: HiveDeliveryPriority::Normal,
            dedupe_key: None,
            max_attempts: DEFAULT_HIVE_DELIVERY_MAX_ATTEMPTS,
        }
    }
}

/// Result of an idempotent enqueue.
#[derive(Debug, Clone)]
pub struct HiveDeliveryEnqueue {
    pub delivery: HiveDelivery,
    /// True when an existing row with the same dedupe key was adopted.
    pub deduplicated: bool,
}
