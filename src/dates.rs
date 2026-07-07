use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Days, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone};

pub fn today_range() -> Result<(DateTime<Local>, DateTime<Local>)> {
    let today = Local::now().date_naive();
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .ok_or_else(|| anyhow!("failed to calculate tomorrow"))?;
    Ok((local_start_of_day(today)?, local_start_of_day(tomorrow)?))
}

pub fn parse_start_datetime(input: &str) -> Result<DateTime<Local>> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return local_start_of_day(date);
    }

    parse_datetime(input)
}

pub fn parse_end_datetime(input: &str) -> Result<DateTime<Local>> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        let tomorrow = date
            .checked_add_days(Days::new(1))
            .ok_or_else(|| anyhow!("failed to calculate end date"))?;
        return local_start_of_day(tomorrow);
    }

    parse_datetime(input)
}

fn parse_datetime(input: &str) -> Result<DateTime<Local>> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(input) {
        return Ok(datetime.with_timezone(&Local));
    }

    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(datetime) = NaiveDateTime::parse_from_str(input, format) {
            return local_datetime(datetime);
        }
    }

    bail!("expected YYYY-MM-DD, YYYY-MM-DDTHH:MM, or RFC3339 datetime")
}

fn local_start_of_day(date: NaiveDate) -> Result<DateTime<Local>> {
    let datetime = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("invalid start of day"))?;
    local_datetime(datetime)
}

fn local_datetime(datetime: NaiveDateTime) -> Result<DateTime<Local>> {
    match Local.from_local_datetime(&datetime) {
        LocalResult::Single(datetime) => Ok(datetime),
        LocalResult::Ambiguous(first, _) => Ok(first),
        LocalResult::None => bail!("local datetime does not exist in the current timezone"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_only_start_uses_start_of_day() {
        let parsed = parse_start_datetime("2026-07-06").unwrap();

        assert!(parsed.to_rfc3339().starts_with("2026-07-06T00:00:00"));
    }

    #[test]
    fn date_only_end_is_exclusive_next_day() {
        let parsed = parse_end_datetime("2026-07-06").unwrap();

        assert!(parsed.to_rfc3339().starts_with("2026-07-07T00:00:00"));
    }

    #[test]
    fn local_datetime_accepts_minute_precision() {
        let parsed = parse_start_datetime("2026-07-06T09:30").unwrap();

        assert!(parsed.to_rfc3339().starts_with("2026-07-06T09:30:00"));
    }
}
