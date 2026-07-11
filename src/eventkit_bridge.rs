use crate::models::{
    AlarmReport, EventRecurrenceEndReport, EventRecurrenceReport, EventRecurrenceWeekdayReport,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, TimeZone, Utc};
use eventkit::EventDraft;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_event_kit::{
    EKAlarm, EKAlarmProximity, EKAlarmType, EKCalendarItem, EKEvent, EKEventStore,
    EKRecurrenceDayOfWeek, EKRecurrenceEnd, EKRecurrenceFrequency, EKRecurrenceRule, EKSpan,
    EKWeekday,
};
use objc2_foundation::{NSArray, NSDate, NSNumber, NSString, NSTimeZone, NSURL};

pub fn create_event_in_calendar(
    draft: &EventDraft<'_>,
    calendar_id: &str,
    time_zone: Option<&str>,
    recurrence: Option<&EventRecurrenceReport>,
    alarm_minutes_before: &[i64],
) -> Result<String> {
    let store = unsafe { EKEventStore::new() };
    let calendar_id = NSString::from_str(calendar_id);
    let calendar = unsafe { store.calendarWithIdentifier(&calendar_id) }
        .context("selected calendar is no longer available")?;
    let event = unsafe { EKEvent::eventWithEventStore(&store) };

    let title = NSString::from_str(draft.title);
    unsafe { event.setTitle(Some(&title)) };

    let start = draft
        .start
        .ok_or_else(|| anyhow!("event start is required"))?;
    let end = draft.end.ok_or_else(|| anyhow!("event end is required"))?;
    let start = NSDate::dateWithTimeIntervalSince1970(start.timestamp() as f64);
    let end = NSDate::dateWithTimeIntervalSince1970(end.timestamp() as f64);
    unsafe {
        event.setStartDate(Some(&start));
        event.setEndDate(Some(&end));
        event.setAllDay(draft.all_day);
        event.setCalendar(Some(&calendar));
    }

    if let Some(notes) = draft.notes {
        let notes = NSString::from_str(notes);
        unsafe { event.setNotes(Some(&notes)) };
    }
    if let Some(location) = draft.location {
        let location = NSString::from_str(location);
        unsafe { event.setLocation(Some(&location)) };
    }
    if let Some(url) = draft.URL {
        set_url(&event, url)?;
    }
    if let Some(availability) = draft.availability {
        unsafe { event.setAvailability(availability.to_ek()) };
    }
    if let Some(time_zone) = time_zone {
        set_time_zone(&event, time_zone)?;
    }
    if let Some(recurrence) = recurrence {
        set_event_recurrence(&event, recurrence)?;
    }
    set_relative_alarms(&event, alarm_minutes_before)?;

    unsafe {
        store
            .saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true)
            .map_err(|error| anyhow!("failed to save event: {error:?}"))?;
        store.refreshSourcesIfNecessary();
    }

    unsafe { event.eventIdentifier() }
        .map(|id| id.to_string())
        .ok_or_else(|| anyhow!("EventKit did not return an id for the created event"))
}

fn set_relative_alarms(event: &EKEvent, alarm_minutes_before: &[i64]) -> Result<()> {
    for minutes in alarm_minutes_before {
        let seconds = minutes
            .checked_mul(60)
            .context("alarm minutes overflowed seconds")?;
        let alarm = unsafe { EKAlarm::alarmWithRelativeOffset(-(seconds as f64)) };
        unsafe { event.addAlarm(&alarm) };
    }
    Ok(())
}

fn set_event_recurrence(event: &EKEvent, recurrence: &EventRecurrenceReport) -> Result<()> {
    let frequency = match recurrence.frequency.as_str() {
        "daily" => EKRecurrenceFrequency::Daily,
        "weekly" => EKRecurrenceFrequency::Weekly,
        "monthly" => EKRecurrenceFrequency::Monthly,
        "yearly" => EKRecurrenceFrequency::Yearly,
        value => bail!("unsupported recurrence frequency: {value}"),
    };
    let end = match recurrence.end.kind.as_str() {
        "never" => None,
        "count" => Some(unsafe {
            EKRecurrenceEnd::recurrenceEndWithOccurrenceCount(
                recurrence
                    .end
                    .occurrence_count
                    .context("recurrence count is missing")?,
            )
        }),
        "date" => {
            let value = recurrence
                .end
                .end_date
                .as_deref()
                .context("recurrence end date is missing")?;
            let value = DateTime::parse_from_rfc3339(value)
                .context("recurrence end date must be RFC3339")?;
            let date = NSDate::dateWithTimeIntervalSince1970(
                value.timestamp() as f64
                    + f64::from(value.timestamp_subsec_nanos()) / 1_000_000_000.0,
            );
            Some(unsafe { EKRecurrenceEnd::recurrenceEndWithEndDate(&date) })
        }
        value => bail!("unsupported recurrence end kind: {value}"),
    };
    let weekdays = recurrence.days_of_week.as_ref().map(|values| {
        values
            .iter()
            .map(|value| unsafe {
                EKRecurrenceDayOfWeek::dayOfWeek_weekNumber(
                    EKWeekday(value.weekday),
                    value.week_number,
                )
            })
            .collect::<Vec<_>>()
    });
    let month_days = recurrence.days_of_month.as_ref().map(|values| {
        values
            .iter()
            .map(|value| NSNumber::new_i32(*value))
            .collect::<Vec<_>>()
    });
    let weekday_array = weekdays
        .as_ref()
        .map(|values| NSArray::from_retained_slice(values));
    let month_day_array = month_days
        .as_ref()
        .map(|values| NSArray::from_retained_slice(values));
    let rule = unsafe {
        EKRecurrenceRule::initRecurrenceWithFrequency_interval_daysOfTheWeek_daysOfTheMonth_monthsOfTheYear_weeksOfTheYear_daysOfTheYear_setPositions_end(
            EKRecurrenceRule::alloc(),
            frequency,
            recurrence.interval as isize,
            weekday_array.as_deref(),
            month_day_array.as_deref(),
            None,
            None,
            None,
            None,
            end.as_deref(),
        )
    };
    let rules = NSArray::from_retained_slice(&[rule]);
    unsafe { event.setRecurrenceRules(Some(&rules)) };
    Ok(())
}

pub fn update_event_calendar_metadata(
    event_id: &str,
    calendar_id: Option<&str>,
    time_zone: Option<Option<&str>>,
) -> Result<String> {
    let store = unsafe { EKEventStore::new() };
    unsafe { store.refreshSourcesIfNecessary() };

    let event_id = NSString::from_str(event_id);
    let event =
        unsafe { store.eventWithIdentifier(&event_id) }.context("event is no longer available")?;

    unsafe {
        if let Some(calendar_id) = calendar_id {
            let calendar_id = NSString::from_str(calendar_id);
            let calendar = store
                .calendarWithIdentifier(&calendar_id)
                .context("selected calendar is no longer available")?;
            event.setCalendar(Some(&calendar));
        }
        match time_zone {
            Some(Some(value)) => set_time_zone(&event, value)?,
            Some(None) => event.setTimeZone(None),
            None => {}
        }
        store
            .saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true)
            .map_err(|error| anyhow!("failed to update event calendar metadata: {error:?}"))?;
        store.refreshSourcesIfNecessary();
    }

    unsafe { event.eventIdentifier() }
        .map(|id| id.to_string())
        .ok_or_else(|| anyhow!("EventKit did not return an id for the moved event"))
}

pub struct EventReadDetails {
    pub alarms: Vec<AlarmReport>,
    pub recurrence_rules: Vec<EventRecurrenceReport>,
}

pub fn read_event_details(
    event_id: &str,
    occurrence_start: Option<DateTime<Local>>,
) -> Result<EventReadDetails> {
    let store = unsafe { EKEventStore::new() };
    unsafe { store.refreshSourcesIfNecessary() };
    let event = find_event_occurrence(&store, event_id, occurrence_start)?;
    let alarms = unsafe { event.alarms() }
        .map(|alarms| {
            alarms
                .iter()
                .map(|alarm| event_alarm_report(&alarm))
                .collect()
        })
        .unwrap_or_default();
    let recurrence_rules = unsafe { event.recurrenceRules() }
        .map(|rules| {
            rules
                .iter()
                .map(|rule| event_recurrence_report(&rule))
                .collect()
        })
        .unwrap_or_default();
    Ok(EventReadDetails {
        alarms,
        recurrence_rules,
    })
}

fn event_alarm_report(alarm: &objc2_event_kit::EKAlarm) -> AlarmReport {
    let absolute_date = unsafe { alarm.absoluteDate() };
    AlarmReport {
        relative_offset_seconds: Some(unsafe { alarm.relativeOffset() }),
        absolute_date: absolute_date
            .as_deref()
            .map(nsdate_dependency_compatible_rfc3339),
        proximity: alarm_proximity_name(unsafe { alarm.proximity() }).to_string(),
        alarm_type: alarm_type_name(unsafe { alarm.r#type() }).to_string(),
    }
}

fn alarm_proximity_name(value: EKAlarmProximity) -> &'static str {
    if value == EKAlarmProximity::Enter {
        "Enter"
    } else if value == EKAlarmProximity::Leave {
        "Leave"
    } else {
        "None"
    }
}

fn alarm_type_name(value: EKAlarmType) -> &'static str {
    if value == EKAlarmType::Display {
        "Display"
    } else if value == EKAlarmType::Audio {
        "Audio"
    } else if value == EKAlarmType::Procedure {
        "Procedure"
    } else if value == EKAlarmType::Email {
        "Email"
    } else {
        "Unknown"
    }
}

fn find_event_occurrence(
    store: &EKEventStore,
    event_id: &str,
    occurrence_start: Option<DateTime<Local>>,
) -> Result<Retained<EKEvent>> {
    let event_id_string = NSString::from_str(event_id);
    let Some(occurrence_start) = occurrence_start else {
        return unsafe { store.eventWithIdentifier(&event_id_string) }
            .context("event is no longer available while reading recurrence");
    };
    let center = occurrence_start.timestamp() as f64;
    let from = NSDate::dateWithTimeIntervalSince1970(center - 1.0);
    let to = NSDate::dateWithTimeIntervalSince1970(center + 1.0);
    let predicate =
        unsafe { store.predicateForEventsWithStartDate_endDate_calendars(&from, &to, None) };
    let mut matches = unsafe { store.eventsMatchingPredicate(&predicate) }
        .iter()
        .filter(|event| {
            let id_matches = unsafe { event.eventIdentifier() }
                .is_some_and(|value| value.to_string() == event_id);
            let start_matches = occurrence_second_matches(
                unsafe { event.startDate() }.timeIntervalSince1970(),
                occurrence_start,
            );
            id_matches && start_matches
        })
        .collect::<Vec<_>>();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => bail!(
            "no occurrence of event {event_id} starts at {} while reading recurrence",
            occurrence_start.to_rfc3339()
        ),
        _ => bail!(
            "multiple occurrences of event {event_id} start at {} while reading recurrence",
            occurrence_start.to_rfc3339()
        ),
    }
}

fn occurrence_second_matches(event_start: f64, occurrence_start: DateTime<Local>) -> bool {
    event_start as i64 == occurrence_start.timestamp()
}

fn event_recurrence_report(rule: &EKRecurrenceRule) -> EventRecurrenceReport {
    let end = match unsafe { rule.recurrenceEnd() } {
        Some(end) if unsafe { end.occurrenceCount() } > 0 => EventRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(unsafe { end.occurrenceCount() }),
            end_date: None,
        },
        Some(end) => match unsafe { end.endDate() } {
            Some(date) => EventRecurrenceEndReport {
                kind: "date".to_string(),
                occurrence_count: None,
                end_date: Some(nsdate_rfc3339(&date)),
            },
            None => EventRecurrenceEndReport {
                kind: "never".to_string(),
                occurrence_count: None,
                end_date: None,
            },
        },
        None => EventRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        },
    };
    EventRecurrenceReport {
        frequency: recurrence_frequency_name(unsafe { rule.frequency() }).to_string(),
        interval: unsafe { rule.interval() } as usize,
        first_day_of_week: unsafe { rule.firstDayOfTheWeek() },
        end,
        days_of_week: unsafe { rule.daysOfTheWeek() }.map(|values| {
            values
                .iter()
                .map(|value| EventRecurrenceWeekdayReport {
                    weekday: unsafe { value.dayOfTheWeek() }.0,
                    week_number: unsafe { value.weekNumber() },
                })
                .collect()
        }),
        days_of_month: number_values(unsafe { rule.daysOfTheMonth() }),
        months_of_year: number_values(unsafe { rule.monthsOfTheYear() }),
        weeks_of_year: number_values(unsafe { rule.weeksOfTheYear() }),
        days_of_year: number_values(unsafe { rule.daysOfTheYear() }),
        set_positions: number_values(unsafe { rule.setPositions() }),
    }
}

fn recurrence_frequency_name(value: EKRecurrenceFrequency) -> &'static str {
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

fn number_values(values: Option<Retained<NSArray<NSNumber>>>) -> Option<Vec<i32>> {
    values.map(|values| values.iter().map(|value| value.intValue()).collect())
}

fn nsdate_rfc3339(date: &NSDate) -> String {
    let timestamp = date.timeIntervalSince1970();
    let mut seconds = timestamp.floor() as i64;
    let mut nanos = ((timestamp - timestamp.floor()) * 1_000_000_000.0).round() as u32;
    if nanos == 1_000_000_000 {
        seconds += 1;
        nanos = 0;
    }
    Utc.timestamp_opt(seconds, nanos)
        .single()
        .map(|value| value.with_timezone(&Local).to_rfc3339())
        .unwrap_or_else(|| format!("invalid EventKit date ({timestamp})"))
}

pub(crate) fn canonical_eventkit_recurrence_end_utc(value: &str) -> Result<String> {
    let value = DateTime::parse_from_rfc3339(value).context("date must be RFC3339")?;
    Utc.timestamp_opt(value.timestamp(), 0)
        .single()
        .map(|value| value.to_rfc3339())
        .context("date is outside the supported EventKit range")
}

fn nsdate_dependency_compatible_rfc3339(date: &NSDate) -> String {
    Local
        .timestamp_opt(date.timeIntervalSince1970() as i64, 0)
        .single()
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| format!("invalid EventKit date ({})", date.timeIntervalSince1970()))
}

fn set_url(item: &EKCalendarItem, value: &str) -> Result<()> {
    let ns_value = NSString::from_str(value);
    let url = NSURL::URLWithString_encodingInvalidCharacters(&ns_value, false)
        .ok_or_else(|| anyhow!("invalid URL: {value}"))?;
    unsafe { item.setURL(Some(&url)) };
    Ok(())
}

pub fn validate_event_url(value: &str) -> Result<()> {
    let ns_value = NSString::from_str(value);
    NSURL::URLWithString_encodingInvalidCharacters(&ns_value, false)
        .map(|_| ())
        .ok_or_else(|| anyhow!("invalid URL: {value}"))
}

pub fn validate_event_time_zone(value: &str) -> Result<()> {
    let value = NSString::from_str(value);
    NSTimeZone::timeZoneWithName(&value)
        .map(|_| ())
        .ok_or_else(|| anyhow!("unknown IANA time zone: {value}"))
}

fn set_time_zone(item: &EKCalendarItem, value: &str) -> Result<()> {
    let time_zone = NSString::from_str(value);
    let time_zone = NSTimeZone::timeZoneWithName(&time_zone)
        .ok_or_else(|| anyhow!("unknown IANA time zone: {value}"))?;
    unsafe { item.setTimeZone(Some(&time_zone)) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::recurrence_rules_match;
    use objc2::AnyThread;
    use objc2_event_kit::{
        EKAlarm, EKRecurrenceDayOfWeek, EKRecurrenceEnd, EKRecurrenceRule, EKWeekday,
    };

    #[test]
    fn recurrence_reader_preserves_positive_and_negative_weekday_ordinals() {
        let first_monday = unsafe { EKRecurrenceDayOfWeek::dayOfWeek_weekNumber(EKWeekday(2), 1) };
        let last_friday = unsafe { EKRecurrenceDayOfWeek::dayOfWeek_weekNumber(EKWeekday(6), -1) };
        let days = NSArray::from_retained_slice(&[first_monday, last_friday]);
        let end = unsafe { EKRecurrenceEnd::recurrenceEndWithOccurrenceCount(6) };
        let rule = unsafe {
            EKRecurrenceRule::initRecurrenceWithFrequency_interval_daysOfTheWeek_daysOfTheMonth_monthsOfTheYear_weeksOfTheYear_daysOfTheYear_setPositions_end(
                EKRecurrenceRule::alloc(),
                EKRecurrenceFrequency::Monthly,
                2,
                Some(&days),
                None,
                None,
                None,
                None,
                None,
                Some(&end),
            )
        };

        let report = event_recurrence_report(&rule);

        assert_eq!(report.frequency, "monthly");
        assert_eq!(report.interval, 2);
        assert_eq!(report.end.occurrence_count, Some(6));
        assert_eq!(
            report.days_of_week,
            Some(vec![
                EventRecurrenceWeekdayReport {
                    weekday: 2,
                    week_number: 1,
                },
                EventRecurrenceWeekdayReport {
                    weekday: 6,
                    week_number: -1,
                },
            ])
        );
    }

    #[test]
    fn event_alarm_reader_preserves_relative_alarm_shape() {
        let alarm = unsafe { EKAlarm::alarmWithRelativeOffset(-600.0) };
        let report = event_alarm_report(&alarm);

        assert_eq!(report.relative_offset_seconds, Some(-600.0));
        assert_eq!(report.absolute_date, None);
        assert_eq!(report.proximity, "None");
        assert_eq!(report.alarm_type, "Display");
    }

    #[test]
    fn absolute_alarm_reader_matches_existing_whole_second_json_contract() {
        let date = NSDate::dateWithTimeIntervalSince1970(1_800_000_000.75);
        let alarm = unsafe { EKAlarm::alarmWithAbsoluteDate(&date) };
        let report = event_alarm_report(&alarm);

        assert_eq!(report.relative_offset_seconds, Some(0.0));
        assert_eq!(
            report.absolute_date,
            Some(
                Local
                    .timestamp_opt(1_800_000_000, 0)
                    .single()
                    .unwrap()
                    .to_rfc3339()
            )
        );
    }

    #[test]
    fn occurrence_matching_uses_dependency_compatible_whole_seconds() {
        let requested = DateTime::parse_from_rfc3339("2026-07-20T09:00:00+03:00")
            .unwrap()
            .with_timezone(&Local);

        assert!(occurrence_second_matches(
            requested.timestamp() as f64 + 0.75,
            requested
        ));
        assert!(!occurrence_second_matches(
            requested.timestamp() as f64 + 1.0,
            requested
        ));
        let epoch = Utc
            .timestamp_opt(0, 0)
            .single()
            .unwrap()
            .with_timezone(&Local);
        assert!(occurrence_second_matches(-0.25, epoch));
        assert!(!occurrence_second_matches(-1.25, epoch));
    }

    #[test]
    fn recurring_creation_builds_an_eventkit_rule_without_saving() {
        let store = unsafe { EKEventStore::new() };
        let event = unsafe { EKEvent::eventWithEventStore(&store) };
        let recurrence = EventRecurrenceReport {
            frequency: "monthly".to_string(),
            interval: 1,
            first_day_of_week: 2,
            end: EventRecurrenceEndReport {
                kind: "count".to_string(),
                occurrence_count: Some(4),
                end_date: None,
            },
            days_of_week: None,
            days_of_month: Some(vec![1, -1]),
            months_of_year: None,
            weeks_of_year: None,
            days_of_year: None,
            set_positions: None,
        };

        set_event_recurrence(&event, &recurrence).unwrap();
        let rules = unsafe { event.recurrenceRules() }.unwrap();
        let round_trip = event_recurrence_report(&rules.objectAtIndex(0));

        assert_eq!(round_trip.frequency, "monthly");
        assert_eq!(round_trip.interval, 1);
        assert_eq!(round_trip.end.occurrence_count, Some(4));
        assert_eq!(round_trip.days_of_month, Some(vec![1, -1]));
    }

    #[test]
    fn every_supported_frequency_round_trips_to_the_same_semantic_rule() {
        for (frequency, interval) in [
            ("daily", 1),
            ("weekly", 1),
            ("weekly", 2),
            ("monthly", 1),
            ("yearly", 1),
        ] {
            let store = unsafe { EKEventStore::new() };
            let event = unsafe { EKEvent::eventWithEventStore(&store) };
            let requested = EventRecurrenceReport {
                frequency: frequency.to_string(),
                interval,
                first_day_of_week: if frequency == "weekly" && interval > 1 {
                    2
                } else {
                    0
                },
                end: EventRecurrenceEndReport {
                    kind: "never".to_string(),
                    occurrence_count: None,
                    end_date: None,
                },
                days_of_week: (frequency == "weekly").then_some(vec![
                    EventRecurrenceWeekdayReport {
                        weekday: 2,
                        week_number: 0,
                    },
                ]),
                days_of_month: None,
                months_of_year: None,
                weeks_of_year: None,
                days_of_year: None,
                set_positions: None,
            };

            set_event_recurrence(&event, &requested).unwrap();
            let rules = unsafe { event.recurrenceRules() }.unwrap();
            let rule = rules.objectAtIndex(0);
            let read_back = event_recurrence_report(&rule);

            assert!(
                recurrence_rules_match(Some(&requested), &[read_back]),
                "{frequency} interval {interval} did not round-trip semantically"
            );
        }
    }

    #[test]
    fn fractional_recurrence_end_round_trips_semantically_through_nsdate() {
        let store = unsafe { EKEventStore::new() };
        let event = unsafe { EKEvent::eventWithEventStore(&store) };
        let requested = EventRecurrenceReport {
            frequency: "daily".to_string(),
            interval: 1,
            first_day_of_week: 0,
            end: EventRecurrenceEndReport {
                kind: "date".to_string(),
                occurrence_count: None,
                end_date: Some("2026-12-31T23:59:59.123456789+03:00".to_string()),
            },
            days_of_week: None,
            days_of_month: None,
            months_of_year: None,
            weeks_of_year: None,
            days_of_year: None,
            set_positions: None,
        };

        set_event_recurrence(&event, &requested).unwrap();
        let rules = unsafe { event.recurrenceRules() }.unwrap();
        let read_back = event_recurrence_report(&rules.objectAtIndex(0));

        assert!(
            recurrence_rules_match(Some(&requested), std::slice::from_ref(&read_back)),
            "requested canonical={} read={} read canonical={}",
            canonical_eventkit_recurrence_end_utc(requested.end.end_date.as_deref().unwrap())
                .unwrap(),
            read_back.end.end_date.as_deref().unwrap(),
            canonical_eventkit_recurrence_end_utc(read_back.end.end_date.as_deref().unwrap())
                .unwrap()
        );
    }

    #[test]
    fn creation_attaches_all_alarms_before_any_save() {
        let store = unsafe { EKEventStore::new() };
        let event = unsafe { EKEvent::eventWithEventStore(&store) };

        set_relative_alarms(&event, &[30, 10]).unwrap();

        let alarms = unsafe { event.alarms() }.unwrap();
        assert_eq!(alarms.len(), 2);
        let mut offsets = alarms
            .iter()
            .map(|alarm| unsafe { alarm.relativeOffset() } as i64)
            .collect::<Vec<_>>();
        offsets.sort_unstable();
        assert_eq!(offsets, [-1800, -600]);
    }
}
