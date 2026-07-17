use chrono::{DateTime, SecondsFormat, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("invalid RFC 3339 timestamp `{value}`: {source}")]
pub struct TimestampError {
    value: String,
    #[source]
    source: chrono::ParseError,
}

pub fn parse_utc_timestamp(value: &str) -> Result<DateTime<Utc>, TimestampError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|source| TimestampError {
            value: value.to_owned(),
            source,
        })
}

pub fn canonical_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub fn normalize_timestamp(value: &str) -> Result<String, TimestampError> {
    parse_utc_timestamp(value).map(canonical_timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_offsets_and_precision_for_lexical_storage() {
        assert_eq!(
            normalize_timestamp("2026-07-01T02:00:00+02:00").unwrap(),
            "2026-07-01T00:00:00.000000Z"
        );
    }

    #[test]
    fn rejects_sqlite_datetime_without_an_explicit_offset() {
        assert!(normalize_timestamp("2026-07-01 00:00:00").is_err());
    }
}
