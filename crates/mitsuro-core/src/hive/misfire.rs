use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    Skip,
    FireOnce,
    CatchUp,
}

impl MisfirePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::FireOnce => "fire_once",
            Self::CatchUp => "catch_up",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "skip" => Some(Self::Skip),
            "fire_once" => Some(Self::FireOnce),
            "catch_up" => Some(Self::CatchUp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MisfireConfig {
    pub policy: MisfirePolicy,
    pub grace_secs: u64,
    pub catch_up_limit: usize,
}

impl Default for MisfireConfig {
    fn default() -> Self {
        Self {
            policy: MisfirePolicy::FireOnce,
            grace_secs: 300,
            catch_up_limit: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MisfireDispatch {
    pub scheduled_for: DateTime<Utc>,
    pub coalesced_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MisfireResolution {
    pub enqueue: Vec<MisfireDispatch>,
    pub skipped: Vec<DateTime<Utc>>,
}

pub fn resolve_misfires(
    due_occurrences: &[DateTime<Utc>],
    now: DateTime<Utc>,
    config: MisfireConfig,
) -> MisfireResolution {
    let mut due = due_occurrences
        .iter()
        .copied()
        .filter(|occurrence| *occurrence <= now)
        .collect::<Vec<_>>();
    due.sort_unstable();
    due.dedup();
    if due.is_empty() {
        return MisfireResolution::default();
    }

    let grace = Duration::seconds(config.grace_secs.min(i64::MAX as u64) as i64);
    let first_on_time = due
        .iter()
        .position(|occurrence| now.signed_duration_since(*occurrence) <= grace)
        .unwrap_or(due.len());
    let (missed, on_time) = due.split_at(first_on_time);

    match config.policy {
        MisfirePolicy::Skip => MisfireResolution {
            enqueue: on_time
                .iter()
                .copied()
                .map(|scheduled_for| MisfireDispatch {
                    scheduled_for,
                    coalesced_count: 0,
                })
                .collect(),
            skipped: missed.to_vec(),
        },
        MisfirePolicy::FireOnce if missed.is_empty() => MisfireResolution {
            enqueue: on_time
                .iter()
                .copied()
                .map(|scheduled_for| MisfireDispatch {
                    scheduled_for,
                    coalesced_count: 0,
                })
                .collect(),
            skipped: Vec::new(),
        },
        MisfirePolicy::FireOnce => {
            let latest = *due.last().expect("due list is non-empty");
            MisfireResolution {
                enqueue: vec![MisfireDispatch {
                    scheduled_for: latest,
                    coalesced_count: due.len().saturating_sub(1),
                }],
                skipped: due[..due.len().saturating_sub(1)].to_vec(),
            }
        }
        MisfirePolicy::CatchUp => {
            let keep_missed = config.catch_up_limit.min(missed.len());
            let skipped_count = missed.len().saturating_sub(keep_missed);
            let enqueue = missed[skipped_count..]
                .iter()
                .chain(on_time.iter())
                .copied()
                .map(|scheduled_for| MisfireDispatch {
                    scheduled_for,
                    coalesced_count: 0,
                })
                .collect();
            MisfireResolution {
                enqueue,
                skipped: missed[..skipped_count].to_vec(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 1, 12, minute, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn fire_once_coalesces_all_overdue_work_into_latest_occurrence() {
        let resolution = resolve_misfires(
            &[at(0), at(10), at(20)],
            at(30),
            MisfireConfig {
                policy: MisfirePolicy::FireOnce,
                grace_secs: 60,
                catch_up_limit: 3,
            },
        );
        assert_eq!(resolution.enqueue.len(), 1);
        assert_eq!(resolution.enqueue[0].scheduled_for, at(20));
        assert_eq!(resolution.enqueue[0].coalesced_count, 2);
        assert_eq!(resolution.skipped, vec![at(0), at(10)]);
    }

    #[test]
    fn skip_preserves_occurrences_inside_grace() {
        let resolution = resolve_misfires(
            &[at(0), at(29)],
            at(30),
            MisfireConfig {
                policy: MisfirePolicy::Skip,
                grace_secs: 120,
                catch_up_limit: 3,
            },
        );
        assert_eq!(resolution.skipped, vec![at(0)]);
        assert_eq!(resolution.enqueue[0].scheduled_for, at(29));
    }

    #[test]
    fn catch_up_keeps_only_most_recent_bounded_misfires() {
        let resolution = resolve_misfires(
            &[at(0), at(5), at(10), at(29)],
            at(30),
            MisfireConfig {
                policy: MisfirePolicy::CatchUp,
                grace_secs: 120,
                catch_up_limit: 2,
            },
        );
        assert_eq!(resolution.skipped, vec![at(0)]);
        assert_eq!(
            resolution
                .enqueue
                .iter()
                .map(|item| item.scheduled_for)
                .collect::<Vec<_>>(),
            vec![at(5), at(10), at(29)]
        );
    }
}
