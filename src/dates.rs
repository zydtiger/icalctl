use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Days, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use std::str::FromStr;

pub fn today_range() -> Result<(DateTime<Local>, DateTime<Local>)> {
    let today = Local::now().date_naive();
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .ok_or_else(|| anyhow!("failed to calculate tomorrow"))?;
    Ok((local_start_of_day(today)?, local_start_of_day(tomorrow)?))
}

pub fn parse_start_datetime(input: &str) -> Result<DateTime<Local>> {
    parse_start_datetime_in_time_zone(input, None)
}

pub fn parse_start_datetime_in_time_zone(
    input: &str,
    time_zone: Option<&str>,
) -> Result<DateTime<Local>> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return start_of_day(date, time_zone);
    }

    parse_datetime(input, time_zone)
}

pub fn parse_end_datetime(input: &str) -> Result<DateTime<Local>> {
    parse_end_datetime_in_time_zone(input, None)
}

pub fn parse_end_datetime_in_time_zone(
    input: &str,
    time_zone: Option<&str>,
) -> Result<DateTime<Local>> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        let tomorrow = date
            .checked_add_days(Days::new(1))
            .ok_or_else(|| anyhow!("failed to calculate end date"))?;
        return start_of_day(tomorrow, time_zone);
    }

    parse_datetime(input, time_zone)
}

pub fn validate_time_zone(time_zone: &str) -> Result<Tz> {
    Tz::from_str(time_zone).map_err(|_| anyhow!("unknown IANA time zone: {time_zone}"))
}

pub fn utc_datetime(datetime: DateTime<Local>) -> String {
    datetime.with_timezone(&Utc).to_rfc3339()
}

pub fn datetime_in_time_zone(datetime: DateTime<Local>, time_zone: &str) -> Result<String> {
    Ok(datetime
        .with_timezone(&validate_time_zone(time_zone)?)
        .to_rfc3339())
}

fn parse_datetime(input: &str, time_zone: Option<&str>) -> Result<DateTime<Local>> {
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
            return datetime_in_zone(datetime, time_zone);
        }
    }

    bail!("expected YYYY-MM-DD, YYYY-MM-DDTHH:MM, or RFC3339 datetime")
}

fn local_start_of_day(date: NaiveDate) -> Result<DateTime<Local>> {
    start_of_day(date, None)
}

fn start_of_day(date: NaiveDate, time_zone: Option<&str>) -> Result<DateTime<Local>> {
    let datetime = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("invalid start of day"))?;
    datetime_in_zone(datetime, time_zone)
}

fn datetime_in_zone(datetime: NaiveDateTime, time_zone: Option<&str>) -> Result<DateTime<Local>> {
    let Some(time_zone) = time_zone else {
        return local_datetime(datetime);
    };
    let time_zone_value = validate_time_zone(time_zone)?;
    match time_zone_value.from_local_datetime(&datetime) {
        LocalResult::Single(datetime) => Ok(datetime.with_timezone(&Local)),
        LocalResult::Ambiguous(_, _) => {
            bail!("datetime is ambiguous in time zone {time_zone}; include an explicit UTC offset")
        }
        LocalResult::None => bail!("datetime does not exist in time zone {time_zone}"),
    }
}

fn local_datetime(datetime: NaiveDateTime) -> Result<DateTime<Local>> {
    match Local.from_local_datetime(&datetime) {
        LocalResult::Single(datetime) => Ok(datetime),
        LocalResult::Ambiguous(_, _) => {
            bail!("local datetime is ambiguous; include an explicit UTC offset")
        }
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

    #[test]
    fn crossing_offsets_with_same_clock_time_is_one_hour() {
        let start = parse_start_datetime("2026-07-12T15:55:00+03:00").unwrap();
        let end = parse_end_datetime("2026-07-12T15:55:00+02:00").unwrap();

        assert_eq!((end - start).num_seconds(), 3600);
        assert_eq!(utc_datetime(start), "2026-07-12T12:55:00+00:00");
        assert_eq!(utc_datetime(end), "2026-07-12T13:55:00+00:00");
    }

    #[test]
    fn renders_instants_in_requested_iana_time_zone() {
        let start = parse_start_datetime("2026-07-12T15:55:00+03:00").unwrap();

        assert_eq!(
            datetime_in_time_zone(start, "Europe/Berlin").unwrap(),
            "2026-07-12T14:55:00+02:00"
        );
        assert!(validate_time_zone("Mars/Olympus_Mons").is_err());
    }

    #[test]
    fn time_zone_controls_naive_datetime_parsing() {
        let start =
            parse_start_datetime_in_time_zone("2026-07-12T15:55", Some("Europe/Berlin")).unwrap();
        let end =
            parse_end_datetime_in_time_zone("2026-07-12T16:55", Some("Europe/Berlin")).unwrap();

        assert_eq!(utc_datetime(start), "2026-07-12T13:55:00+00:00");
        assert_eq!(utc_datetime(end), "2026-07-12T14:55:00+00:00");
        assert_eq!((end - start).num_seconds(), 3600);
    }

    #[test]
    fn no_time_zone_flag_uses_mac_local_time() {
        let parsed = parse_start_datetime_in_time_zone("2026-07-12T15:55", None).unwrap();

        assert_eq!(
            parsed.naive_local(),
            NaiveDateTime::parse_from_str("2026-07-12T15:55", "%Y-%m-%dT%H:%M").unwrap()
        );
    }

    #[test]
    fn explicit_offset_remains_authoritative_with_time_zone() {
        let start =
            parse_start_datetime_in_time_zone("2026-07-12T15:55:00+03:00", Some("Europe/Berlin"))
                .unwrap();

        assert_eq!(utc_datetime(start), "2026-07-12T12:55:00+00:00");
    }

    #[test]
    fn explicit_offset_is_authoritative_without_time_zone_flag() {
        let start = parse_start_datetime_in_time_zone("2026-07-12T15:55:00+03:00", None).unwrap();

        assert_eq!(utc_datetime(start), "2026-07-12T12:55:00+00:00");
    }

    #[test]
    fn time_zone_controls_date_only_midnight_boundaries() {
        let start = parse_start_datetime_in_time_zone("2026-07-12", Some("Europe/Berlin")).unwrap();
        let end = parse_end_datetime_in_time_zone("2026-07-12", Some("Europe/Berlin")).unwrap();

        assert_eq!(utc_datetime(start), "2026-07-11T22:00:00+00:00");
        assert_eq!(utc_datetime(end), "2026-07-12T22:00:00+00:00");
    }

    #[test]
    fn rejects_naive_datetime_in_dst_gap() {
        let result = parse_start_datetime_in_time_zone("2026-03-29T02:30", Some("Europe/Berlin"));

        assert_eq!(
            result.unwrap_err().to_string(),
            "datetime does not exist in time zone Europe/Berlin"
        );
    }

    #[test]
    fn rejects_ambiguous_naive_datetime_without_guessing_offset() {
        let result = parse_start_datetime_in_time_zone("2026-10-25T02:30", Some("Europe/Berlin"));

        assert_eq!(
            result.unwrap_err().to_string(),
            "datetime is ambiguous in time zone Europe/Berlin; include an explicit UTC offset"
        );
    }
}
