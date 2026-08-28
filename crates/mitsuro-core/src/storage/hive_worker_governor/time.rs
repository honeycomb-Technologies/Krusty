use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Days, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};

use crate::hive::{parse_timezone, resolve_local_datetime, DstGapPolicy, DstPolicy};

use super::HiveWorkerGovernorPolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLocalDayWindow {
    pub local_day: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerQuietWindow {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

pub fn worker_local_day_window(
    policy: &HiveWorkerGovernorPolicy,
    now: DateTime<Utc>,
) -> Result<WorkerLocalDayWindow> {
    let timezone = parse_timezone(&policy.timezone)?;
    let local_day = now.with_timezone(&timezone).date_naive();
    let next_day = local_day
        .checked_add_days(Days::new(1))
        .ok_or_else(|| anyhow!("Worker local day overflow"))?;
    let midnight = NaiveTime::from_hms_opt(0, 0, 0).expect("midnight is valid");
    // Accounting days must always have concrete UTC bounds, including zones
    // that move clocks at midnight or skip a civil date entirely. Quiet-hour
    // occurrence policy remains independently user-selectable below.
    let dst = DstPolicy {
        gap: DstGapPolicy::ShiftForward,
        fold: policy.quiet_fold_policy,
    };
    let starts_at = resolve_local_datetime(timezone, local_day.and_time(midnight), dst)
        .ok_or_else(|| anyhow!("Worker local-day start is skipped by its DST policy"))?;
    let ends_at = resolve_local_datetime(timezone, next_day.and_time(midnight), dst)
        .ok_or_else(|| anyhow!("Worker local-day end is skipped by its DST policy"))?;
    anyhow::ensure!(
        ends_at > starts_at,
        "Worker local-day window is not monotonic"
    );
    Ok(WorkerLocalDayWindow {
        local_day: local_day.to_string(),
        starts_at,
        ends_at,
    })
}

pub fn worker_quiet_window_at(
    policy: &HiveWorkerGovernorPolicy,
    now: DateTime<Utc>,
) -> Result<Option<WorkerQuietWindow>> {
    let (Some(start_minute), Some(end_minute)) =
        (policy.quiet_start_minute, policy.quiet_end_minute)
    else {
        return Ok(None);
    };
    let timezone = parse_timezone(&policy.timezone)?;
    let local_today = now.with_timezone(&timezone).date_naive();
    let yesterday = local_today
        .checked_sub_days(Days::new(1))
        .ok_or_else(|| anyhow!("Worker quiet-window date underflow"))?;
    let dst = DstPolicy {
        gap: policy.quiet_gap_policy,
        fold: policy.quiet_fold_policy,
    };

    // Checking both anchors is necessary for overnight windows: before the
    // local end time, the active occurrence began on the prior local date.
    for anchor in [yesterday, local_today] {
        let Some(window) =
            resolve_quiet_occurrence(anchor, start_minute, end_minute, timezone, dst)?
        else {
            // DstGapPolicy::Skip omits this exact local occurrence.
            continue;
        };
        if now >= window.starts_at && now < window.ends_at {
            return Ok(Some(window));
        }
    }
    Ok(None)
}

fn resolve_quiet_occurrence(
    anchor: NaiveDate,
    start_minute: u16,
    end_minute: u16,
    timezone: chrono_tz::Tz,
    dst: DstPolicy,
) -> Result<Option<WorkerQuietWindow>> {
    let start_time = minute_of_day(start_minute)?;
    let end_time = minute_of_day(end_minute)?;
    let end_date = if end_minute <= start_minute {
        anchor
            .checked_add_days(Days::new(1))
            .ok_or_else(|| anyhow!("Worker quiet-window date overflow"))?
    } else {
        anchor
    };
    let Some(starts_at) = resolve_local_datetime(timezone, anchor.and_time(start_time), dst) else {
        return Ok(None);
    };
    let Some(ends_at) = resolve_local_datetime(timezone, end_date.and_time(end_time), dst) else {
        return Ok(None);
    };
    anyhow::ensure!(
        ends_at > starts_at,
        "Worker quiet-window end must follow its start"
    );
    Ok(Some(WorkerQuietWindow { starts_at, ends_at }))
}

fn minute_of_day(value: u16) -> Result<NaiveTime> {
    anyhow::ensure!(value < 1_440, "quiet minute must be between 0 and 1439");
    NaiveTime::from_hms_opt(u32::from(value / 60), u32::from(value % 60), 0)
        .context("constructing Worker quiet time")
}
