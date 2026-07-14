use super::*;

pub(super) fn date_part(value: &str) -> &str {
    value.get(0..10).unwrap_or(value)
}

pub(super) fn time_with_offset(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|datetime| datetime.format("%H:%M%:z").to_string())
        .unwrap_or_else(|_| value.get(11..16).unwrap_or(value).to_string())
}
