use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::loop_events::LoopStopReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryJitter {
    None,
    Full,
}

impl RetryJitter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Full => "full",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay_secs: u64,
    pub max_delay_secs: u64,
    pub jitter: RetryJitter,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_secs: 15,
            max_delay_secs: 15 * 60,
            jitter: RetryJitter::Full,
        }
    }
}

/// Calculate the delay after a failed, one-indexed `attempt_number`.
/// Returns `None` when the attempt budget is exhausted.
pub fn retry_delay(
    policy: RetryPolicy,
    attempt_number: u32,
    jitter_unit: f64,
    retry_after: Option<StdDuration>,
) -> Option<StdDuration> {
    if attempt_number == 0 || attempt_number >= policy.max_attempts {
        return None;
    }

    let exponent = attempt_number.saturating_sub(1).min(63);
    let uncapped = policy
        .base_delay_secs
        .saturating_mul(1_u64.checked_shl(exponent).unwrap_or(u64::MAX));
    let cap = uncapped.min(policy.max_delay_secs);
    let calculated_secs = match policy.jitter {
        RetryJitter::None => cap,
        RetryJitter::Full => {
            let unit = if jitter_unit.is_finite() {
                jitter_unit.clamp(0.0, 1.0)
            } else {
                0.0
            };
            (cap as f64 * unit).floor() as u64
        }
    };
    let calculated = StdDuration::from_secs(calculated_secs);
    Some(
        retry_after
            .map(|provider_delay| provider_delay.max(calculated))
            .unwrap_or(calculated),
    )
}

pub fn next_retry_at(
    now: DateTime<Utc>,
    policy: RetryPolicy,
    attempt_number: u32,
    jitter_unit: f64,
    retry_after: Option<StdDuration>,
) -> Option<DateTime<Utc>> {
    let delay = retry_delay(policy, attempt_number, jitter_unit, retry_after)?;
    let delay = Duration::from_std(delay).ok()?;
    now.checked_add_signed(delay)
}

pub fn is_transient_stop_reason(reason: &LoopStopReason) -> bool {
    matches!(
        reason,
        LoopStopReason::ProviderError | LoopStopReason::StreamIdleTimeout
    )
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn exponential_backoff_caps_and_uses_deterministic_full_jitter() {
        let policy = RetryPolicy {
            max_attempts: 8,
            base_delay_secs: 10,
            max_delay_secs: 60,
            jitter: RetryJitter::Full,
        };
        assert_eq!(
            retry_delay(policy, 1, 0.5, None),
            Some(StdDuration::from_secs(5))
        );
        assert_eq!(
            retry_delay(policy, 5, 0.5, None),
            Some(StdDuration::from_secs(30))
        );
    }

    #[test]
    fn provider_retry_after_is_never_shortened() {
        let policy = RetryPolicy {
            jitter: RetryJitter::None,
            ..RetryPolicy::default()
        };
        assert_eq!(
            retry_delay(policy, 1, 0.0, Some(StdDuration::from_secs(120))),
            Some(StdDuration::from_secs(120))
        );
    }

    #[test]
    fn retry_budget_is_persistable_by_attempt_number() {
        let policy = RetryPolicy {
            max_attempts: 3,
            ..RetryPolicy::default()
        };
        assert!(retry_delay(policy, 2, 0.5, None).is_some());
        assert!(retry_delay(policy, 3, 0.5, None).is_none());
    }

    #[test]
    fn next_retry_at_uses_utc_instant() {
        let now = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).single().unwrap();
        let policy = RetryPolicy {
            jitter: RetryJitter::None,
            ..RetryPolicy::default()
        };
        assert_eq!(
            next_retry_at(now, policy, 1, 0.0, None),
            Some(now + Duration::seconds(15))
        );
    }
}
