use super::*;

pub(super) fn event_recurrence_args_is_empty(args: &EventRecurrenceArgs) -> bool {
    args.repeat.is_none()
        && args.interval.is_none()
        && args.weekdays.is_empty()
        && args.month_days.is_empty()
        && args.count.is_none()
        && args.until.is_none()
}

pub(crate) fn parse_event_recurrence(
    args: &EventRecurrenceArgs,
    start_input: &str,
    time_zone: Option<&str>,
) -> Result<Option<EventRecurrenceReport>> {
    let Some(frequency) = args.repeat else {
        if !event_recurrence_args_is_empty(args) {
            bail!("event recurrence options require --repeat");
        }
        return Ok(None);
    };
    let interval = args.interval.unwrap_or(1);
    if interval == 0 {
        bail!("--repeat-interval must be greater than zero");
    }
    isize::try_from(interval).context("--repeat-interval is too large for EventKit")?;
    if args.count == Some(0) {
        bail!("--repeat-count must be greater than zero");
    }
    if frequency == EventRepeatArg::Daily
        && (!args.weekdays.is_empty() || !args.month_days.is_empty())
    {
        bail!("daily recurrence cannot use --repeat-weekday or --repeat-month-day");
    }
    if frequency != EventRepeatArg::Monthly && !args.month_days.is_empty() {
        bail!("--repeat-month-day requires --repeat monthly");
    }
    if !args.weekdays.is_empty() && !args.month_days.is_empty() {
        bail!("--repeat-weekday and --repeat-month-day cannot be combined");
    }
    let mut month_days = args.month_days.clone();
    month_days.sort_unstable();
    month_days.dedup();
    if let Some(value) = month_days
        .iter()
        .find(|value| **value == 0 || value.unsigned_abs() > 31)
    {
        bail!("--repeat-month-day must be from 1 through 31 or -1 through -31: {value}");
    }
    let mut weekdays = args
        .weekdays
        .iter()
        .map(|value| EventRecurrenceWeekdayReport {
            weekday: event_weekday_number(*value),
            week_number: 0,
        })
        .collect::<Vec<_>>();
    weekdays.sort_by_key(|value| value.weekday);
    weekdays.dedup_by_key(|value| value.weekday);
    let start = parse_start_datetime_in_time_zone(start_input, time_zone)
        .context("invalid recurrence anchor")?;
    let end = if let Some(count) = args.count {
        EventRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(count),
            end_date: None,
        }
    } else if let Some(until) = args.until.as_deref() {
        let until = DateTime::parse_from_rfc3339(until)
            .context("--repeat-until must be RFC3339 with an explicit UTC offset")?
            .with_timezone(&Local);
        let stored_until = canonical_eventkit_recurrence_end_utc(&until.to_rfc3339())?;
        let stored_until_local = DateTime::parse_from_rfc3339(&stored_until)
            .context("canonical EventKit recurrence end must be RFC3339")?
            .with_timezone(&Local);
        if stored_until_local < start {
            bail!("--repeat-until must not be before the event start");
        }
        EventRecurrenceEndReport {
            kind: "date".to_string(),
            occurrence_count: None,
            end_date: Some(stored_until),
        }
    } else {
        EventRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        }
    };
    Ok(Some(EventRecurrenceReport {
        frequency: match frequency {
            EventRepeatArg::Daily => "daily",
            EventRepeatArg::Weekly => "weekly",
            EventRepeatArg::Monthly => "monthly",
            EventRepeatArg::Yearly => "yearly",
        }
        .to_string(),
        interval,
        first_day_of_week: if frequency == EventRepeatArg::Weekly && interval > 1 {
            2
        } else {
            0
        },
        end,
        days_of_week: (!weekdays.is_empty()).then_some(weekdays),
        days_of_month: (!month_days.is_empty()).then_some(month_days),
        months_of_year: None,
        weeks_of_year: None,
        days_of_year: None,
        set_positions: None,
    }))
}

fn event_weekday_number(value: EventWeekdayArg) -> isize {
    match value {
        EventWeekdayArg::Sunday => 1,
        EventWeekdayArg::Monday => 2,
        EventWeekdayArg::Tuesday => 3,
        EventWeekdayArg::Wednesday => 4,
        EventWeekdayArg::Thursday => 5,
        EventWeekdayArg::Friday => 6,
        EventWeekdayArg::Saturday => 7,
    }
}

pub(crate) fn validate_recurring_all_day_inputs(
    all_day: bool,
    recurring: bool,
    start: &str,
    end: &str,
) -> Result<()> {
    if all_day
        && recurring
        && (NaiveDate::parse_from_str(start, "%Y-%m-%d").is_err()
            || NaiveDate::parse_from_str(end, "%Y-%m-%d").is_err())
    {
        bail!("recurring all-day events require date-only --start and --end values");
    }
    Ok(())
}

pub(crate) fn recurrence_rules_match(
    requested: Option<&EventRecurrenceReport>,
    existing: &[EventRecurrenceReport],
) -> bool {
    match requested {
        Some(requested) => {
            existing.len() == 1
                && existing.first().is_some_and(|existing| {
                    canonical_recurrence(existing) == canonical_recurrence(requested)
                })
        }
        None => existing.is_empty(),
    }
}

fn canonical_recurrence(rule: &EventRecurrenceReport) -> EventRecurrenceReport {
    let mut rule = rule.clone();
    rule.first_day_of_week = if rule.frequency == "weekly" && rule.interval > 1 {
        if rule.first_day_of_week == 0 {
            2
        } else {
            rule.first_day_of_week
        }
    } else {
        0
    };
    if let Some(end_date) = rule.end.end_date.as_deref()
        && let Ok(value) = canonical_eventkit_recurrence_end_utc(end_date)
    {
        rule.end.end_date = Some(value);
    }
    normalize_optional_vec(&mut rule.days_of_week);
    normalize_optional_vec(&mut rule.days_of_month);
    normalize_optional_vec(&mut rule.months_of_year);
    normalize_optional_vec(&mut rule.weeks_of_year);
    normalize_optional_vec(&mut rule.days_of_year);
    normalize_optional_vec(&mut rule.set_positions);
    rule
}

fn normalize_optional_vec<T: Ord>(values: &mut Option<Vec<T>>) {
    if let Some(values) = values {
        values.sort_unstable();
        values.dedup();
    }
    if values.as_ref().is_some_and(Vec::is_empty) {
        *values = None;
    }
}
