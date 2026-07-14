use super::super::{
    ComponentTimeZone, ParsedReminderGeofence, ParsedReminderNotification,
    ParsedReminderRecurrence, ReminderDateComponents,
};
use crate::cli::{ReminderGeofenceProximityArg, ReminderRepeatArg};
use crate::models::{
    ReminderAlarmReport, ReminderDateKind, ReminderDateReport, ReminderListReport,
    ReminderPriority, ReminderRecurrenceEndReport, ReminderRecurrenceReport, ReminderReport,
    ReminderStructuredLocationReport,
};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, FixedOffset, Local, TimeZone, Utc};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_location::CLLocation;
use objc2_event_kit::{
    EKAlarm, EKAlarmProximity, EKAlarmType, EKCalendar, EKCalendarType, EKRecurrenceEnd,
    EKRecurrenceFrequency, EKRecurrenceRule, EKReminder, EKSourceType, EKStructuredLocation,
};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSDateComponentUndefined,
    NSDateComponents, NSNumber, NSString, NSTimeZone, NSURL,
};

pub(in crate::reminders) fn reminder_date_components(
    value: &ReminderDateComponents,
) -> Result<Retained<NSDateComponents>> {
    let components = NSDateComponents::new();
    let calendar = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
        .context("Gregorian calendar is unavailable")?;
    components.setCalendar(Some(&calendar));
    components.setYear(value.year as isize);
    components.setMonth(value.month as isize);
    components.setDay(value.day as isize);
    if let Some(hour) = value.hour {
        components.setHour(hour as isize);
    }
    if let Some(minute) = value.minute {
        components.setMinute(minute as isize);
    }
    if let Some(second) = value.second {
        components.setSecond(second as isize);
    }
    if let Some(time_zone) = &value.time_zone {
        let time_zone = match time_zone {
            ComponentTimeZone::Named(name) => {
                NSTimeZone::timeZoneWithName(&NSString::from_str(name))
                    .with_context(|| format!("unknown EventKit time zone: {name}"))?
            }
            ComponentTimeZone::FixedOffset(seconds) => {
                NSTimeZone::timeZoneForSecondsFromGMT(*seconds as isize)
            }
        };
        components.setTimeZone(Some(&time_zone));
    }
    Ok(components)
}

pub(in crate::reminders) fn set_reminder_url(reminder: &EKReminder, value: &str) -> Result<()> {
    let value = NSString::from_str(value);
    let url = NSURL::URLWithString_encodingInvalidCharacters(&value, false)
        .ok_or_else(|| anyhow!("invalid URL: {value}"))?;
    unsafe { reminder.setURL(Some(&url)) };
    Ok(())
}

pub(in crate::reminders) fn add_reminder_notifications(
    reminder: &EKReminder,
    notifications: &[ParsedReminderNotification],
) {
    for notification in notifications {
        let date = nsdate_from_utc(notification.absolute_utc);
        let alarm = unsafe { EKAlarm::alarmWithAbsoluteDate(&date) };
        unsafe { reminder.addAlarm(&alarm) };
    }
}

pub(in crate::reminders) fn add_reminder_geofence(
    reminder: &EKReminder,
    geofence: &ParsedReminderGeofence,
) {
    let title = NSString::from_str(&geofence.title);
    let structured = unsafe { EKStructuredLocation::locationWithTitle(&title) };
    let location = unsafe {
        CLLocation::initWithLatitude_longitude(
            CLLocation::alloc(),
            geofence.latitude,
            geofence.longitude,
        )
    };
    unsafe {
        structured.setGeoLocation(Some(&location));
        structured.setRadius(geofence.radius_meters);
    }
    let alarm = unsafe { EKAlarm::alarmWithRelativeOffset(0.0) };
    unsafe {
        alarm.setStructuredLocation(Some(&structured));
        alarm.setProximity(match geofence.proximity {
            ReminderGeofenceProximityArg::Arrive => EKAlarmProximity::Enter,
            ReminderGeofenceProximityArg::Leave => EKAlarmProximity::Leave,
        });
        reminder.addAlarm(&alarm);
    }
}

pub(in crate::reminders) fn set_reminder_recurrence(
    reminder: &EKReminder,
    recurrence: Option<&ParsedReminderRecurrence>,
) {
    let Some(recurrence) = recurrence else {
        unsafe { reminder.setRecurrenceRules(None) };
        return;
    };
    let end = if let Some(count) = recurrence.count {
        Some(unsafe { EKRecurrenceEnd::recurrenceEndWithOccurrenceCount(count) })
    } else {
        recurrence.until_utc.map(|until| {
            let date = nsdate_from_utc(until);
            unsafe { EKRecurrenceEnd::recurrenceEndWithEndDate(&date) }
        })
    };
    let rule = unsafe {
        EKRecurrenceRule::initRecurrenceWithFrequency_interval_end(
            EKRecurrenceRule::alloc(),
            match recurrence.frequency {
                ReminderRepeatArg::Daily => EKRecurrenceFrequency::Daily,
                ReminderRepeatArg::Weekly => EKRecurrenceFrequency::Weekly,
                ReminderRepeatArg::Monthly => EKRecurrenceFrequency::Monthly,
                ReminderRepeatArg::Yearly => EKRecurrenceFrequency::Yearly,
            },
            recurrence.interval as isize,
            end.as_deref(),
        )
    };
    let rules = NSArray::from_retained_slice(&[rule]);
    unsafe { reminder.setRecurrenceRules(Some(&rules)) };
}

pub(in crate::reminders) fn nsdate_from_utc(value: DateTime<Utc>) -> Retained<NSDate> {
    NSDate::dateWithTimeIntervalSince1970(
        value.timestamp() as f64 + f64::from(value.timestamp_subsec_nanos()) / 1_000_000_000.0,
    )
}

pub(in crate::reminders) fn reminder_list_report(
    list: &EKCalendar,
    is_default: bool,
) -> ReminderListReport {
    let source = unsafe { list.source() };
    ReminderListReport {
        id: unsafe { list.calendarIdentifier() }.to_string(),
        title: unsafe { list.title() }.to_string(),
        source: source
            .as_ref()
            .map(|source| unsafe { source.title() }.to_string()),
        source_id: source
            .as_ref()
            .map(|source| unsafe { source.sourceIdentifier() }.to_string()),
        source_type: source
            .as_ref()
            .map(|source| source_type_name(unsafe { source.sourceType() }).to_string()),
        list_type: calendar_type_name(unsafe { list.r#type() }).to_string(),
        allows_modifications: unsafe { list.allowsContentModifications() },
        is_immutable: unsafe { list.isImmutable() },
        is_subscribed: unsafe { list.isSubscribed() },
        is_default_for_new_reminders: is_default,
    }
}

pub(in crate::reminders) fn reminder_to_report(
    reminder: &EKReminder,
    details: bool,
) -> ReminderReport {
    let list = unsafe { reminder.calendar() };
    let list_report = list.as_ref().map(|list| reminder_list_report(list, false));
    let notes = unsafe { reminder.notes() }.map(|value| value.to_string());
    let url = unsafe { reminder.URL() }
        .as_ref()
        .and_then(|url| url.absoluteString())
        .map(|value| value.to_string());
    let priority_value = unsafe { reminder.priority() };
    let alarms = details.then(|| reminder_alarms(reminder));
    let recurrence_rules = details.then(|| reminder_recurrence_rules(reminder));
    ReminderReport {
        id: unsafe { reminder.calendarItemIdentifier() }.to_string(),
        title: unsafe { reminder.title() }.to_string(),
        parent_id: None,
        child_count: 0,
        child_ids: details.then(Vec::new),
        completed: unsafe { reminder.isCompleted() },
        completion_date: unsafe { reminder.completionDate() }
            .as_deref()
            .map(nsdate_rfc3339),
        priority: priority_name(priority_value),
        priority_value,
        list: list_report.as_ref().map(|list| list.title.clone()),
        list_id: list_report.as_ref().map(|list| list.id.clone()),
        list_source: list_report.as_ref().and_then(|list| list.source.clone()),
        list_source_id: list_report.as_ref().and_then(|list| list.source_id.clone()),
        list_type: list_report.as_ref().map(|list| list.list_type.clone()),
        allows_list_modifications: list_report.as_ref().map(|list| list.allows_modifications),
        list_selection: None,
        write_action: None,
        due: unsafe { reminder.dueDateComponents() }
            .as_deref()
            .and_then(date_components_report),
        due_input: None,
        start: unsafe { reminder.startDateComponents() }
            .as_deref()
            .and_then(date_components_report),
        start_input: None,
        notes: notes.clone(),
        location: unsafe { reminder.location() }.map(|value| value.to_string()),
        url: url.clone(),
        has_notes: unsafe { reminder.hasNotes() },
        has_url: url.is_some(),
        alarm_count: alarms.as_ref().map(Vec::len),
        recurrence_count: recurrence_rules.as_ref().map(Vec::len),
        alarms,
        recurrence_rules,
        creation_date: unsafe { reminder.creationDate() }
            .as_deref()
            .map(nsdate_rfc3339),
        last_modified_date: unsafe { reminder.lastModifiedDate() }
            .as_deref()
            .map(nsdate_rfc3339),
        external_identifier: unsafe { reminder.calendarItemExternalIdentifier() }
            .map(|value| value.to_string()),
        item_time_zone: unsafe { reminder.timeZone() }.map(|zone| zone.name().to_string()),
    }
}

pub(in crate::reminders) fn priority_name(value: usize) -> ReminderPriority {
    match value {
        0 => ReminderPriority::None,
        1..=4 => ReminderPriority::High,
        5 => ReminderPriority::Medium,
        _ => ReminderPriority::Low,
    }
}

pub(in crate::reminders) fn date_components_report(
    components: &NSDateComponents,
) -> Option<ReminderDateReport> {
    let year = component(components.year())?;
    let month = component(components.month())?;
    let day = component(components.day())?;
    let component_time_zone = components.timeZone();
    let time_zone = component_time_zone
        .as_ref()
        .map(|zone| zone.name().to_string());
    let hour = component(components.hour());
    let minute = component(components.minute());
    let second = component(components.second());
    let date = format!("{year:04}-{month:02}-{day:02}");
    if hour.is_none() && minute.is_none() && second.is_none() {
        return Some(ReminderDateReport {
            kind: ReminderDateKind::Date,
            date: Some(date),
            local: None,
            normalized: None,
            utc: None,
            time_zone,
        });
    }

    let local = format!(
        "{date}T{:02}:{:02}:{:02}",
        hour.unwrap_or(0),
        minute.unwrap_or(0),
        second.unwrap_or(0)
    );
    let instant = components.date();
    let utc = instant.as_deref().and_then(nsdate_utc);
    let normalized = utc.as_ref().map(|utc| {
        component_time_zone
            .as_ref()
            .zip(instant.as_ref())
            .and_then(|(zone, date)| FixedOffset::east_opt(zone.secondsFromGMTForDate(date) as i32))
            .map(|offset| utc.with_timezone(&offset).to_rfc3339())
            .unwrap_or_else(|| utc.with_timezone(&Local).to_rfc3339())
    });
    Some(ReminderDateReport {
        kind: ReminderDateKind::Datetime,
        date: None,
        local: Some(local),
        normalized,
        utc: utc.map(|value| value.to_rfc3339()),
        time_zone,
    })
}

pub(in crate::reminders) fn component(value: isize) -> Option<isize> {
    (value != NSDateComponentUndefined).then_some(value)
}

pub(in crate::reminders) fn nsdate_utc(date: &NSDate) -> Option<DateTime<Utc>> {
    let timestamp = date.timeIntervalSince1970();
    let mut seconds = timestamp.floor() as i64;
    let mut nanos = ((timestamp - timestamp.floor()) * 1_000_000_000.0).round() as u32;
    if nanos == 1_000_000_000 {
        seconds += 1;
        nanos = 0;
    }
    Utc.timestamp_opt(seconds, nanos).single()
}

pub(in crate::reminders) fn nsdate_rfc3339(date: &NSDate) -> String {
    nsdate_utc(date)
        .map(|value| value.with_timezone(&Local).to_rfc3339())
        .unwrap_or_else(|| "invalid-date".to_string())
}

pub(in crate::reminders) fn reminder_alarms(reminder: &EKReminder) -> Vec<ReminderAlarmReport> {
    unsafe { reminder.alarms() }
        .map(|alarms| alarms.iter().map(|alarm| alarm_report(&alarm)).collect())
        .unwrap_or_default()
}

pub(in crate::reminders) fn alarm_report(alarm: &EKAlarm) -> ReminderAlarmReport {
    let absolute = unsafe { alarm.absoluteDate() };
    let eventkit_location = unsafe { alarm.structuredLocation() };
    let structured_location =
        eventkit_location
            .as_ref()
            .map(|location| ReminderStructuredLocationReport {
                title: unsafe { location.title() }.map(|title| title.to_string()),
                radius_meters: unsafe { location.radius() },
                latitude: unsafe { location.geoLocation() }
                    .map(|value| unsafe { value.coordinate() }.latitude),
                longitude: unsafe { location.geoLocation() }
                    .map(|value| unsafe { value.coordinate() }.longitude),
            });
    ReminderAlarmReport {
        relative_offset_seconds: (absolute.is_none() && eventkit_location.is_none())
            .then(|| unsafe { alarm.relativeOffset() }),
        absolute_date: absolute.as_deref().map(nsdate_rfc3339),
        proximity: alarm_proximity_name(unsafe { alarm.proximity() }).to_string(),
        alarm_type: alarm_type_name(unsafe { alarm.r#type() }).to_string(),
        structured_location,
    }
}

pub(in crate::reminders) fn reminder_recurrence_rules(
    reminder: &EKReminder,
) -> Vec<ReminderRecurrenceReport> {
    unsafe { reminder.recurrenceRules() }
        .map(|rules| rules.iter().map(|rule| recurrence_report(&rule)).collect())
        .unwrap_or_default()
}

pub(in crate::reminders) fn recurrence_report(rule: &EKRecurrenceRule) -> ReminderRecurrenceReport {
    let end = unsafe { rule.recurrenceEnd() };
    let end = match end {
        Some(end) if unsafe { end.occurrenceCount() } > 0 => ReminderRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(unsafe { end.occurrenceCount() }),
            end_date: None,
        },
        Some(end) => ReminderRecurrenceEndReport {
            kind: "date".to_string(),
            occurrence_count: None,
            end_date: unsafe { end.endDate() }.as_deref().map(nsdate_rfc3339),
        },
        None => ReminderRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        },
    };
    ReminderRecurrenceReport {
        frequency: recurrence_frequency_name(unsafe { rule.frequency() }).to_string(),
        interval: unsafe { rule.interval() } as usize,
        first_day_of_week: unsafe { rule.firstDayOfTheWeek() },
        end,
        days_of_week: unsafe { rule.daysOfTheWeek() }.map(|values| {
            values
                .iter()
                .map(|value| unsafe { value.dayOfTheWeek() }.0)
                .collect()
        }),
        days_of_month: number_values(unsafe { rule.daysOfTheMonth() }),
        months_of_year: number_values(unsafe { rule.monthsOfTheYear() }),
        weeks_of_year: number_values(unsafe { rule.weeksOfTheYear() }),
        days_of_year: number_values(unsafe { rule.daysOfTheYear() }),
        set_positions: number_values(unsafe { rule.setPositions() }),
    }
}

pub(in crate::reminders) fn number_values(
    values: Option<Retained<NSArray<NSNumber>>>,
) -> Option<Vec<i32>> {
    values.map(|values| values.iter().map(|value| value.intValue()).collect())
}

pub(in crate::reminders) fn calendar_type_name(value: EKCalendarType) -> &'static str {
    match value.0 {
        0 => "local",
        1 => "caldav",
        2 => "exchange",
        3 => "subscription",
        4 => "birthday",
        _ => "unknown",
    }
}

pub(in crate::reminders) fn source_type_name(value: EKSourceType) -> &'static str {
    match value.0 {
        0 => "local",
        1 => "exchange",
        2 => "caldav",
        3 => "mobileme",
        4 => "subscribed",
        5 => "birthdays",
        _ => "unknown",
    }
}

pub(in crate::reminders) fn alarm_proximity_name(value: EKAlarmProximity) -> &'static str {
    if value == EKAlarmProximity::Enter {
        "arrive"
    } else if value == EKAlarmProximity::Leave {
        "leave"
    } else {
        "none"
    }
}

pub(in crate::reminders) fn alarm_type_name(value: EKAlarmType) -> &'static str {
    if value == EKAlarmType::Display {
        "display"
    } else if value == EKAlarmType::Audio {
        "audio"
    } else if value == EKAlarmType::Procedure {
        "procedure"
    } else if value == EKAlarmType::Email {
        "email"
    } else {
        "unknown"
    }
}

pub(in crate::reminders) fn recurrence_frequency_name(
    value: EKRecurrenceFrequency,
) -> &'static str {
    if value == EKRecurrenceFrequency::Daily {
        "daily"
    } else if value == EKRecurrenceFrequency::Weekly {
        "weekly"
    } else if value == EKRecurrenceFrequency::Monthly {
        "monthly"
    } else if value == EKRecurrenceFrequency::Yearly {
        "yearly"
    } else {
        "unknown"
    }
}
