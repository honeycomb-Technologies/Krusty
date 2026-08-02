pub(in super::super) fn earlier_timestamp(
    current: Option<String>,
    candidate: Option<String>,
) -> Option<String> {
    match (current, candidate) {
        (None, candidate) => candidate,
        (current, None) => current,
        (Some(current), Some(candidate)) => {
            if candidate < current {
                Some(candidate)
            } else {
                Some(current)
            }
        }
    }
}

pub(in super::super) fn later_timestamp(
    current: Option<String>,
    candidate: Option<String>,
) -> Option<String> {
    match (current, candidate) {
        (None, candidate) => candidate,
        (current, None) => current,
        (Some(current), Some(candidate)) => {
            if candidate > current {
                Some(candidate)
            } else {
                Some(current)
            }
        }
    }
}

pub(crate) fn parse_timestamp(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date| date.with_timezone(&chrono::Utc))
}
