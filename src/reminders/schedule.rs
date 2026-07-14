use super::ReminderLifecyclePatch;
use crate::cli::{ReminderAdvancedScheduleArgs, ReminderGeofenceProximityArg, ReminderRepeatArg};
use crate::dates::{parse_start_datetime_in_time_zone, validate_time_zone};
use crate::models::{
    ReminderAlarmReport, ReminderDateKind, ReminderDateReport, ReminderNotificationReport,
    ReminderPlannedAlarmReport, ReminderRecurrenceEndReport, ReminderRecurrenceReport,
    ReminderReport, ReminderStructuredLocationReport,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, Timelike, Utc};
use objc2_foundation::NSTimeZone;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub(super) struct ParsedReminderNotification {
    pub(super) minutes_before: Option<i64>,
    pub(super) input: Option<String>,
    pub(super) absolute_utc: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub(super) struct ParsedReminderGeofence {
    pub(super) title: String,
    pub(super) latitude: f64,
    pub(super) longitude: f64,
    pub(super) radius_meters: f64,
    pub(super) proximity: ReminderGeofenceProximityArg,
}

#[derive(Clone, Debug)]
pub(super) struct ParsedReminderRecurrence {
    pub(super) frequency: ReminderRepeatArg,
    pub(super) interval: usize,
    pub(super) count: Option<usize>,
    pub(super) until_utc: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub(super) struct ParsedReminderDate {
    pub(super) input: String,
    pub(super) report: ReminderDateReport,
    pub(super) components: ReminderDateComponents,
}

#[derive(Clone)]
pub(super) struct ReminderDateComponents {
    pub(super) year: i32,
    pub(super) month: u32,
    pub(super) day: u32,
    pub(super) hour: Option<u32>,
    pub(super) minute: Option<u32>,
    pub(super) second: Option<u32>,
    pub(super) time_zone: Option<ComponentTimeZone>,
}

#[derive(Clone)]
pub(super) enum ComponentTimeZone {
    Named(String),
    FixedOffset(i32),
}

pub(super) fn alarm_reports_from_parsed(
    notifications: &[ParsedReminderNotification],
    geofence: Option<&ParsedReminderGeofence>,
) -> Vec<ReminderAlarmReport> {
    let mut alarms = notifications
        .iter()
        .map(|notification| ReminderAlarmReport {
            relative_offset_seconds: None,
            absolute_date: Some(notification.absolute_utc.to_rfc3339()),
            proximity: "none".to_string(),
            alarm_type: "display".to_string(),
            structured_location: None,
        })
        .collect::<Vec<_>>();
    if let Some(geofence) = geofence {
        alarms.push(ReminderAlarmReport {
            relative_offset_seconds: None,
            absolute_date: None,
            proximity: geofence_proximity_name(geofence.proximity).to_string(),
            alarm_type: "display".to_string(),
            structured_location: Some(ReminderStructuredLocationReport {
                title: Some(geofence.title.clone()),
                radius_meters: geofence.radius_meters,
                latitude: Some(geofence.latitude),
                longitude: Some(geofence.longitude),
            }),
        });
    }
    alarms
}

pub(super) fn clear_parsed_time_zone(value: ParsedReminderDate) -> Result<ParsedReminderDate> {
    if value.report.kind == ReminderDateKind::Date {
        return Ok(value);
    }
    let local = value
        .report
        .local
        .as_deref()
        .context("timed reminder value has no local components")?;
    floating_reminder_date(local)
}

pub(super) fn floating_reminder_date(input: &str) -> Result<ParsedReminderDate> {
    let local = parse_naive_datetime(input)?;
    Ok(ParsedReminderDate {
        input: input.to_string(),
        report: ReminderDateReport {
            kind: ReminderDateKind::Datetime,
            date: None,
            local: Some(local.format("%Y-%m-%dT%H:%M:%S").to_string()),
            normalized: None,
            utc: None,
            time_zone: None,
        },
        components: ReminderDateComponents {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: Some(local.hour()),
            minute: Some(local.minute()),
            second: Some(local.second()),
            time_zone: None,
        },
    })
}

pub(super) fn rezone_unchanged_dates(
    before: &ReminderReport,
    patch: &mut ReminderLifecyclePatch,
    time_zone: Option<&str>,
    clear: bool,
) -> Result<()> {
    if patch.due.is_none()
        && let Some(due) = &before.due
        && due.kind == ReminderDateKind::Datetime
    {
        let local = due
            .local
            .as_deref()
            .context("timed due value has no local components")?;
        patch.due = Some(Some(if clear {
            floating_reminder_date(local)?
        } else {
            parse_reminder_date(local, time_zone)?
        }));
    }
    if patch.start.is_none()
        && let Some(start) = &before.start
        && start.kind == ReminderDateKind::Datetime
    {
        let local = start
            .local
            .as_deref()
            .context("timed start value has no local components")?;
        patch.start = Some(Some(if clear {
            floating_reminder_date(local)?
        } else {
            parse_reminder_date(local, time_zone)?
        }));
    }
    Ok(())
}

pub(super) fn parse_reminder_date(
    input: &str,
    time_zone: Option<&str>,
) -> Result<ParsedReminderDate> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Ok(ParsedReminderDate {
            input: input.to_string(),
            report: ReminderDateReport {
                kind: ReminderDateKind::Date,
                date: Some(date.format("%Y-%m-%d").to_string()),
                local: None,
                normalized: None,
                utc: None,
                time_zone: None,
            },
            components: ReminderDateComponents {
                year: date.year(),
                month: date.month(),
                day: date.day(),
                hour: None,
                minute: None,
                second: None,
                time_zone: None,
            },
        });
    }

    if let Ok(value) = DateTime::parse_from_rfc3339(input) {
        let local = value.naive_local();
        let offset_seconds = value.offset().local_minus_utc();
        return Ok(timed_reminder_date(
            input,
            local,
            value.to_rfc3339(),
            value.with_timezone(&Utc).to_rfc3339(),
            value.offset().to_string(),
            ComponentTimeZone::FixedOffset(offset_seconds),
        ));
    }

    let local = parse_naive_datetime(input)?;
    let instant = parse_start_datetime_in_time_zone(input, time_zone)?;
    match time_zone {
        Some(time_zone) => {
            let zone = validate_time_zone(time_zone)?;
            Ok(timed_reminder_date(
                input,
                local,
                instant.with_timezone(&zone).to_rfc3339(),
                instant.with_timezone(&Utc).to_rfc3339(),
                time_zone.to_string(),
                ComponentTimeZone::Named(time_zone.to_string()),
            ))
        }
        None => {
            let local_zone = NSTimeZone::localTimeZone().name().to_string();
            Ok(timed_reminder_date(
                input,
                local,
                instant.to_rfc3339(),
                instant.with_timezone(&Utc).to_rfc3339(),
                local_zone.clone(),
                ComponentTimeZone::Named(local_zone),
            ))
        }
    }
}

pub(super) fn parse_naive_datetime(input: &str) -> Result<NaiveDateTime> {
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(input, format) {
            return Ok(value);
        }
    }
    bail!("expected YYYY-MM-DD, YYYY-MM-DDTHH:MM, or RFC3339 datetime")
}

pub(super) fn timed_reminder_date(
    input: &str,
    local: NaiveDateTime,
    normalized: String,
    utc: String,
    time_zone_name: String,
    component_time_zone: ComponentTimeZone,
) -> ParsedReminderDate {
    ParsedReminderDate {
        input: input.to_string(),
        report: ReminderDateReport {
            kind: ReminderDateKind::Datetime,
            date: None,
            local: Some(local.format("%Y-%m-%dT%H:%M:%S").to_string()),
            normalized: Some(normalized),
            utc: Some(utc),
            time_zone: Some(time_zone_name),
        },
        components: ReminderDateComponents {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: Some(local.hour()),
            minute: Some(local.minute()),
            second: Some(local.second()),
            time_zone: Some(component_time_zone),
        },
    }
}

pub(super) fn build_notifications(
    due: Option<&ReminderDateReport>,
    notify_at_due: bool,
    notify_minutes_before: &[i64],
) -> Result<Vec<ParsedReminderNotification>> {
    if !notify_at_due && notify_minutes_before.is_empty() {
        return Ok(Vec::new());
    }
    let due = due.context("notification flags require a timed --due value")?;
    if due.kind != ReminderDateKind::Datetime {
        bail!("notification flags require a timed --due value, not a date-only due value");
    }
    let due_utc = due
        .utc
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .context("timed due value did not produce a normalized instant")?;

    let mut minutes = BTreeSet::new();
    if notify_at_due {
        minutes.insert(0);
    }
    for value in notify_minutes_before {
        if *value <= 0 {
            bail!("--notify-minutes-before must be greater than zero");
        }
        minutes.insert(*value);
    }
    minutes
        .into_iter()
        .map(|minutes_before| {
            let duration = chrono::Duration::try_minutes(minutes_before)
                .context("--notify-minutes-before is too large")?;
            let absolute_utc = due_utc
                .checked_sub_signed(duration)
                .context("notification time is outside the supported date range")?;
            Ok(ParsedReminderNotification {
                minutes_before: Some(minutes_before),
                input: None,
                absolute_utc,
            })
        })
        .collect()
}

pub(super) fn parse_absolute_notifications(
    values: &[String],
) -> Result<Vec<ParsedReminderNotification>> {
    let mut seen = BTreeSet::new();
    let mut notifications = Vec::new();
    for input in values {
        let absolute_utc = DateTime::parse_from_rfc3339(input)
            .with_context(|| {
                format!("--notify-at must be RFC3339 with an explicit UTC offset: {input}")
            })?
            .with_timezone(&Utc);
        if seen.insert(absolute_utc) {
            notifications.push(ParsedReminderNotification {
                minutes_before: None,
                input: Some(input.clone()),
                absolute_utc,
            });
        }
    }
    Ok(notifications)
}

pub(super) fn parse_geofence(
    schedule: &ReminderAdvancedScheduleArgs,
) -> Result<Option<ParsedReminderGeofence>> {
    let Some(title) = schedule.geofence_title.as_deref() else {
        return Ok(None);
    };
    let title = title.trim();
    if title.is_empty() {
        bail!("--geofence-title must not be empty");
    }
    let latitude = schedule
        .geofence_latitude
        .context("--geofence-title requires --geofence-latitude")?;
    let longitude = schedule
        .geofence_longitude
        .context("--geofence-title requires --geofence-longitude")?;
    let radius_meters = schedule
        .geofence_radius_meters
        .context("--geofence-title requires --geofence-radius-meters")?;
    let proximity = schedule
        .geofence_proximity
        .context("--geofence-title requires --geofence-proximity")?;
    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        bail!("--geofence-latitude must be a finite value from -90 through 90");
    }
    if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
        bail!("--geofence-longitude must be a finite value from -180 through 180");
    }
    if !radius_meters.is_finite() || radius_meters <= 0.0 {
        bail!("--geofence-radius-meters must be a positive finite value");
    }
    Ok(Some(ParsedReminderGeofence {
        title: title.to_string(),
        latitude,
        longitude,
        radius_meters,
        proximity,
    }))
}

pub(super) fn parse_recurrence(
    schedule: &ReminderAdvancedScheduleArgs,
) -> Result<Option<ParsedReminderRecurrence>> {
    let Some(frequency) = schedule.repeat else {
        if schedule.repeat_interval.is_some()
            || schedule.repeat_count.is_some()
            || schedule.repeat_until.is_some()
        {
            bail!("recurrence options require --repeat");
        }
        return Ok(None);
    };
    let interval = schedule.repeat_interval.unwrap_or(1);
    if interval == 0 {
        bail!("--repeat-interval must be greater than zero");
    }
    if interval > isize::MAX as usize {
        bail!("--repeat-interval is too large");
    }
    if schedule.repeat_count == Some(0) {
        bail!("--repeat-count must be greater than zero");
    }
    if schedule.repeat_count.is_some() && schedule.repeat_until.is_some() {
        bail!("--repeat-count conflicts with --repeat-until");
    }
    let until_utc = schedule
        .repeat_until
        .as_deref()
        .map(|input| {
            DateTime::parse_from_rfc3339(input)
                .with_context(|| {
                    format!("--repeat-until must be RFC3339 with an explicit UTC offset: {input}")
                })
                .map(|value| value.with_timezone(&Utc))
        })
        .transpose()?;
    Ok(Some(ParsedReminderRecurrence {
        frequency,
        interval,
        count: schedule.repeat_count,
        until_utc,
    }))
}

pub(super) fn validate_recurrence_end_after_anchor(
    recurrence: &ReminderRecurrenceReport,
    anchor: Option<&ReminderDateReport>,
) -> Result<()> {
    let Some(until) = recurrence.end.end_date.as_deref() else {
        return Ok(());
    };
    let anchor = anchor.context("recurrence requires a due or start date")?;
    let until = DateTime::parse_from_rfc3339(until)
        .context("recurrence end did not contain a valid RFC3339 instant")?;
    match anchor.kind {
        ReminderDateKind::Datetime => {
            let anchor = anchor
                .utc
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .context("timed recurrence anchor did not contain a normalized instant")?;
            if until < anchor {
                bail!("--repeat-until must not be before the recurrence due/start anchor");
            }
        }
        ReminderDateKind::Date => {
            let anchor = anchor
                .date
                .as_deref()
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                .context("date-only recurrence anchor was invalid")?;
            if until.date_naive() < anchor {
                bail!("--repeat-until must not be before the recurrence due/start anchor");
            }
        }
    }
    Ok(())
}

pub(super) fn planned_alarm_reports(
    notifications: &[ParsedReminderNotification],
    geofence: Option<&ParsedReminderGeofence>,
) -> Vec<ReminderPlannedAlarmReport> {
    let mut reports = notifications
        .iter()
        .map(|notification| ReminderPlannedAlarmReport {
            kind: match notification.minutes_before {
                Some(0) => "at_due",
                Some(_) => "before_due",
                None => "absolute",
            }
            .to_string(),
            input: notification.input.clone(),
            absolute_utc: Some(notification.absolute_utc.to_rfc3339()),
            minutes_before_due: notification.minutes_before,
            proximity: None,
            structured_location: None,
        })
        .collect::<Vec<_>>();
    if let Some(geofence) = geofence {
        reports.push(ReminderPlannedAlarmReport {
            kind: "geofence".to_string(),
            input: None,
            absolute_utc: None,
            minutes_before_due: None,
            proximity: Some(geofence_proximity_name(geofence.proximity).to_string()),
            structured_location: Some(ReminderStructuredLocationReport {
                title: Some(geofence.title.clone()),
                radius_meters: geofence.radius_meters,
                latitude: Some(geofence.latitude),
                longitude: Some(geofence.longitude),
            }),
        });
    }
    reports
}

pub(super) fn recurrence_report_from_parsed(
    recurrence: &ParsedReminderRecurrence,
) -> ReminderRecurrenceReport {
    let end = match (recurrence.count, recurrence.until_utc) {
        (Some(count), _) => ReminderRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(count),
            end_date: None,
        },
        (_, Some(until)) => ReminderRecurrenceEndReport {
            kind: "date".to_string(),
            occurrence_count: None,
            end_date: Some(until.to_rfc3339()),
        },
        _ => ReminderRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        },
    };
    ReminderRecurrenceReport {
        frequency: repeat_name(recurrence.frequency).to_string(),
        interval: recurrence.interval,
        first_day_of_week: 0,
        end,
        days_of_week: None,
        days_of_month: None,
        months_of_year: None,
        weeks_of_year: None,
        days_of_year: None,
        set_positions: None,
    }
}

pub(super) fn geofence_proximity_name(value: ReminderGeofenceProximityArg) -> &'static str {
    match value {
        ReminderGeofenceProximityArg::Arrive => "arrive",
        ReminderGeofenceProximityArg::Leave => "leave",
    }
}

pub(super) fn repeat_name(value: ReminderRepeatArg) -> &'static str {
    match value {
        ReminderRepeatArg::Daily => "daily",
        ReminderRepeatArg::Weekly => "weekly",
        ReminderRepeatArg::Monthly => "monthly",
        ReminderRepeatArg::Yearly => "yearly",
    }
}

pub(super) fn notification_reports(
    notifications: &[ParsedReminderNotification],
    due: Option<&ReminderDateReport>,
) -> Vec<ReminderNotificationReport> {
    notifications
        .iter()
        .filter(|notification| notification.minutes_before.is_some())
        .map(|notification| ReminderNotificationReport {
            kind: if notification.minutes_before == Some(0) {
                "at_due"
            } else {
                "before_due"
            }
            .to_string(),
            minutes_before: notification.minutes_before.unwrap_or_default(),
            absolute_utc: notification.absolute_utc.to_rfc3339(),
            absolute_in_due_time_zone: render_notification_in_due_time_zone(
                notification.absolute_utc,
                due,
            ),
        })
        .collect()
}

pub(super) fn render_notification_in_due_time_zone(
    instant: DateTime<Utc>,
    due: Option<&ReminderDateReport>,
) -> String {
    let Some(due) = due else {
        return instant.to_rfc3339();
    };
    if let Some(zone) = due
        .time_zone
        .as_deref()
        .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
    {
        return instant.with_timezone(&zone).to_rfc3339();
    }
    if let Some(offset) = due
        .normalized
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| *value.offset())
    {
        return instant.with_timezone(&offset).to_rfc3339();
    }
    instant.with_timezone(&Local).to_rfc3339()
}
