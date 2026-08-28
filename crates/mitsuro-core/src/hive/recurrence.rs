use std::collections::BTreeSet;

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
    Weekday,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_DAY_SEARCH: i64 = 366 * 20;
const MAX_MONTH_SEARCH: i32 = 12 * 100;
const MAX_GAP_SEARCH_MINUTES: i64 = 48 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DstGapPolicy {
    ShiftForward,
    Skip,
}

impl DstGapPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShiftForward => "shift_forward",
            Self::Skip => "skip",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "shift_forward" => Some(Self::ShiftForward),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DstFoldPolicy {
    First,
    Second,
}

impl DstFoldPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Second => "second",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "first" => Some(Self::First),
            "second" => Some(Self::Second),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DstPolicy {
    pub gap: DstGapPolicy,
    pub fold: DstFoldPolicy,
}

impl Default for DstPolicy {
    fn default() -> Self {
        Self {
            gap: DstGapPolicy::ShiftForward,
            fold: DstFoldPolicy::First,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleWeekday {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl ScheduleWeekday {
    fn matches(self, weekday: Weekday) -> bool {
        matches!(
            (self, weekday),
            (Self::Sunday, Weekday::Sun)
                | (Self::Monday, Weekday::Mon)
                | (Self::Tuesday, Weekday::Tue)
                | (Self::Wednesday, Weekday::Wed)
                | (Self::Thursday, Weekday::Thu)
                | (Self::Friday, Weekday::Fri)
                | (Self::Saturday, Weekday::Sat)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonthlyDayPolicy {
    Skip,
    LastDay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecurrenceV1 {
    Once {
        at: DateTime<Utc>,
    },
    Daily {
        start_date: NaiveDate,
        time: NaiveTime,
    },
    Weekdays {
        start_date: NaiveDate,
        time: NaiveTime,
    },
    Weekly {
        start_date: NaiveDate,
        time: NaiveTime,
        weekdays: BTreeSet<ScheduleWeekday>,
    },
    Monthly {
        start_date: NaiveDate,
        time: NaiveTime,
        day: u8,
        invalid_day_policy: MonthlyDayPolicy,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecurrenceError {
    #[error("unknown IANA timezone: {0}")]
    InvalidTimezone(String),
    #[error("weekly recurrence requires at least one weekday")]
    EmptyWeekdays,
    #[error("monthly day must be between 1 and 31")]
    InvalidMonthlyDay,
    #[error("no future occurrence could be resolved within the supported search horizon")]
    SearchHorizonExceeded,
}

pub fn parse_timezone(value: &str) -> Result<Tz, RecurrenceError> {
    value
        .parse::<Tz>()
        .map_err(|_| RecurrenceError::InvalidTimezone(value.to_string()))
}

impl RecurrenceV1 {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Once { .. } => "once",
            Self::Daily { .. } => "daily",
            Self::Weekdays { .. } => "weekdays",
            Self::Weekly { .. } => "weekly",
            Self::Monthly { .. } => "monthly",
        }
    }

    pub fn validate(&self) -> Result<(), RecurrenceError> {
        match self {
            Self::Weekly { weekdays, .. } if weekdays.is_empty() => {
                Err(RecurrenceError::EmptyWeekdays)
            }
            Self::Monthly { day, .. } if !(1..=31).contains(day) => {
                Err(RecurrenceError::InvalidMonthlyDay)
            }
            _ => Ok(()),
        }
    }

    /// Return the first logical occurrence strictly after `after`.
    pub fn next_after(
        &self,
        timezone: Tz,
        after: DateTime<Utc>,
        dst: DstPolicy,
    ) -> Result<Option<DateTime<Utc>>, RecurrenceError> {
        self.validate()?;
        match self {
            Self::Once { at } => Ok((*at > after).then_some(*at)),
            Self::Daily { start_date, time } => {
                next_matching_day(*start_date, *time, timezone, after, dst, |_| true)
            }
            Self::Weekdays { start_date, time } => {
                next_matching_day(*start_date, *time, timezone, after, dst, |date| {
                    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
                })
            }
            Self::Weekly {
                start_date,
                time,
                weekdays,
            } => next_matching_day(*start_date, *time, timezone, after, dst, |date| {
                weekdays.iter().any(|day| day.matches(date.weekday()))
            }),
            Self::Monthly {
                start_date,
                time,
                day,
                invalid_day_policy,
            } => next_monthly(
                *start_date,
                *time,
                *day,
                *invalid_day_policy,
                timezone,
                after,
                dst,
            ),
        }
    }
}

pub fn occurrences_between(
    recurrence: &RecurrenceV1,
    timezone: Tz,
    after_exclusive: DateTime<Utc>,
    through_inclusive: DateTime<Utc>,
    dst: DstPolicy,
    limit: usize,
) -> Result<Vec<DateTime<Utc>>, RecurrenceError> {
    if limit == 0 || through_inclusive <= after_exclusive {
        return Ok(Vec::new());
    }

    let mut occurrences = Vec::new();
    let mut cursor = after_exclusive;
    while occurrences.len() < limit {
        let Some(next) = recurrence.next_after(timezone, cursor, dst)? else {
            break;
        };
        if next > through_inclusive {
            break;
        }
        occurrences.push(next);
        cursor = next;
    }
    Ok(occurrences)
}

fn next_matching_day<F>(
    start_date: NaiveDate,
    time: NaiveTime,
    timezone: Tz,
    after: DateTime<Utc>,
    dst: DstPolicy,
    predicate: F,
) -> Result<Option<DateTime<Utc>>, RecurrenceError>
where
    F: Fn(NaiveDate) -> bool,
{
    let local_after = after.with_timezone(&timezone).date_naive();
    let first_date = start_date.max(local_after);

    for day_offset in 0..=MAX_DAY_SEARCH {
        let Some(date) = first_date.checked_add_signed(Duration::days(day_offset)) else {
            break;
        };
        if !predicate(date) {
            continue;
        }
        let local = date.and_time(time);
        if let Some(candidate) = resolve_local_datetime(timezone, local, dst) {
            if candidate > after {
                return Ok(Some(candidate));
            }
        }
    }

    Err(RecurrenceError::SearchHorizonExceeded)
}

#[allow(clippy::too_many_arguments)]
fn next_monthly(
    start_date: NaiveDate,
    time: NaiveTime,
    day: u8,
    invalid_day_policy: MonthlyDayPolicy,
    timezone: Tz,
    after: DateTime<Utc>,
    dst: DstPolicy,
) -> Result<Option<DateTime<Utc>>, RecurrenceError> {
    let local_after = after.with_timezone(&timezone).date_naive();
    let anchor = start_date.max(local_after);

    for month_offset in 0..=MAX_MONTH_SEARCH {
        let (year, month) = add_months(anchor.year(), anchor.month(), month_offset);
        let Some(last_day) = days_in_month(year, month) else {
            break;
        };
        let resolved_day = match invalid_day_policy {
            MonthlyDayPolicy::Skip if u32::from(day) > last_day => continue,
            MonthlyDayPolicy::Skip => u32::from(day),
            MonthlyDayPolicy::LastDay => u32::from(day).min(last_day),
        };
        let Some(date) = NaiveDate::from_ymd_opt(year, month, resolved_day) else {
            continue;
        };
        if date < start_date {
            continue;
        }
        if let Some(candidate) = resolve_local_datetime(timezone, date.and_time(time), dst) {
            if candidate > after {
                return Ok(Some(candidate));
            }
        }
    }

    Err(RecurrenceError::SearchHorizonExceeded)
}

/// Resolve one wall-clock timestamp under the same explicit gap/fold policy
/// used by recurring Hive schedules. Governor quiet hours and local budget
/// days call this helper so DST semantics cannot drift between subsystems.
pub(crate) fn resolve_local_datetime(
    timezone: Tz,
    local: NaiveDateTime,
    dst: DstPolicy,
) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Some(select_fold(first, second, dst.fold)),
        LocalResult::None if dst.gap == DstGapPolicy::Skip => None,
        LocalResult::None => shift_forward_across_gap(timezone, local, dst.fold),
    }
}

fn shift_forward_across_gap(
    timezone: Tz,
    local: NaiveDateTime,
    fold: DstFoldPolicy,
) -> Option<DateTime<Utc>> {
    for minute in 1..=MAX_GAP_SEARCH_MINUTES {
        let probe = local.checked_add_signed(Duration::minutes(minute))?;
        let Some(mut resolved) = resolve_non_gap(timezone, probe, fold) else {
            continue;
        };

        // The probe preserves the source second. Walk back to the first valid
        // second so `02:30:15` in a one-hour gap resolves to `03:00:00`.
        for second in 1..=59 {
            let Some(previous) = probe.checked_sub_signed(Duration::seconds(second)) else {
                break;
            };
            let Some(previous_resolved) = resolve_non_gap(timezone, previous, fold) else {
                break;
            };
            resolved = previous_resolved;
        }
        return Some(resolved);
    }
    None
}

fn resolve_non_gap(
    timezone: Tz,
    local: NaiveDateTime,
    fold: DstFoldPolicy,
) -> Option<DateTime<Utc>> {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Some(select_fold(first, second, fold)),
        LocalResult::None => None,
    }
}

fn select_fold(first: DateTime<Tz>, second: DateTime<Tz>, policy: DstFoldPolicy) -> DateTime<Utc> {
    let first = first.with_timezone(&Utc);
    let second = second.with_timezone(&Utc);
    match policy {
        DstFoldPolicy::First => first.min(second),
        DstFoldPolicy::Second => first.max(second),
    }
}

fn add_months(year: i32, month: u32, offset: i32) -> (i32, u32) {
    let zero_based = i64::from(year) * 12 + i64::from(month - 1) + i64::from(offset);
    let result_year = zero_based.div_euclid(12) as i32;
    let result_month = zero_based.rem_euclid(12) as u32 + 1;
    (result_year, result_month)
}

fn days_in_month(year: i32, month: u32) -> Option<u32> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)?
        .pred_opt()
        .map(|date| date.day())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Timelike};

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("valid UTC fixture")
    }

    #[test]
    fn spring_gap_can_shift_to_first_valid_instant() {
        let recurrence = RecurrenceV1::Daily {
            start_date: NaiveDate::from_ymd_opt(2026, 3, 8).unwrap(),
            time: NaiveTime::from_hms_opt(2, 30, 15).unwrap(),
        };
        let next = recurrence
            .next_after(
                chrono_tz::America::Los_Angeles,
                at(2026, 3, 8, 0, 0),
                DstPolicy::default(),
            )
            .unwrap()
            .unwrap();

        assert_eq!(next, at(2026, 3, 8, 10, 0));
        assert_eq!(next.second(), 0);
    }

    #[test]
    fn spring_gap_can_skip_the_invalid_day() {
        let recurrence = RecurrenceV1::Daily {
            start_date: NaiveDate::from_ymd_opt(2026, 3, 8).unwrap(),
            time: NaiveTime::from_hms_opt(2, 30, 0).unwrap(),
        };
        let next = recurrence
            .next_after(
                chrono_tz::America::Los_Angeles,
                at(2026, 3, 8, 0, 0),
                DstPolicy {
                    gap: DstGapPolicy::Skip,
                    fold: DstFoldPolicy::First,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(next, at(2026, 3, 9, 9, 30));
    }

    #[test]
    fn fall_fold_uses_explicit_first_or_second_policy() {
        let recurrence = RecurrenceV1::Daily {
            start_date: NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            time: NaiveTime::from_hms_opt(1, 30, 0).unwrap(),
        };
        let first = recurrence
            .next_after(
                chrono_tz::America::Los_Angeles,
                at(2026, 11, 1, 0, 0),
                DstPolicy::default(),
            )
            .unwrap()
            .unwrap();
        let second = recurrence
            .next_after(
                chrono_tz::America::Los_Angeles,
                at(2026, 11, 1, 0, 0),
                DstPolicy {
                    gap: DstGapPolicy::ShiftForward,
                    fold: DstFoldPolicy::Second,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(first, at(2026, 11, 1, 8, 30));
        assert_eq!(second, at(2026, 11, 1, 9, 30));
    }

    #[test]
    fn weekly_recurrence_respects_selected_days() {
        let recurrence = RecurrenceV1::Weekly {
            start_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            time: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            weekdays: BTreeSet::from([ScheduleWeekday::Tuesday, ScheduleWeekday::Thursday]),
        };
        let next = recurrence
            .next_after(chrono_tz::UTC, at(2026, 7, 1, 10, 0), DstPolicy::default())
            .unwrap()
            .unwrap();
        assert_eq!(next, at(2026, 7, 2, 9, 0));
    }

    #[test]
    fn monthly_day_policy_can_skip_or_clamp_short_months() {
        let start_date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let time = NaiveTime::from_hms_opt(9, 0, 0).unwrap();
        let skip = RecurrenceV1::Monthly {
            start_date,
            time,
            day: 31,
            invalid_day_policy: MonthlyDayPolicy::Skip,
        };
        let clamp = RecurrenceV1::Monthly {
            start_date,
            time,
            day: 31,
            invalid_day_policy: MonthlyDayPolicy::LastDay,
        };

        assert_eq!(
            skip.next_after(chrono_tz::UTC, at(2026, 4, 1, 0, 0), DstPolicy::default())
                .unwrap(),
            Some(at(2026, 5, 31, 9, 0))
        );
        assert_eq!(
            clamp
                .next_after(chrono_tz::UTC, at(2026, 4, 1, 0, 0), DstPolicy::default())
                .unwrap(),
            Some(at(2026, 4, 30, 9, 0))
        );
    }

    #[test]
    fn once_is_strictly_after_cursor() {
        let occurrence = at(2026, 7, 1, 12, 0);
        let recurrence = RecurrenceV1::Once { at: occurrence };
        assert_eq!(
            recurrence
                .next_after(chrono_tz::UTC, occurrence, DstPolicy::default())
                .unwrap(),
            None
        );
    }
}
