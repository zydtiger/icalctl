use super::*;
use ::eventkit::CalendarType;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_test_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("icalctl-{name}-{nonce}"))
}

#[test]
fn nullable_patch_clear_wins() {
    assert_eq!(nullable_patch(Some("notes"), false), Some(Some("notes")));
    assert_eq!(nullable_patch(Some("notes"), true), Some(None));
    assert_eq!(nullable_patch(None, true), Some(None));
    assert_eq!(nullable_patch(None, false), None);
}

#[test]
fn notes_file_preserves_exact_utf8_contents() {
    let path = temporary_test_path("notes.txt");
    let expected = "First line\nSecond line\n\nFinal line without trimming";
    fs::write(&path, expected).unwrap();

    let actual = read_notes_file(&path).unwrap();
    fs::remove_file(path).unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn json_event_draft_supports_structured_add_fields() {
    let draft: JsonAddDraft = serde_json::from_str(
        r#"{
                "title": "Meeting",
                "start": "2026-07-10T09:00",
                "end": "2026-07-10T10:00",
                "calendar_id": "CAL-1",
                "notes": "Agenda",
                "location": "Room 3",
                "url": "https://example.com",
                "availability": "busy",
                "time_zone": "Asia/Shanghai",
                "alarm_minutes_before": [10],
                "recurrence": {
                    "frequency": "weekly",
                    "interval": 2,
                    "weekdays": ["monday", "wednesday"],
                    "count": 8
                },
                "timed": true
            }"#,
    )
    .unwrap();

    assert_eq!(draft.calendar_id.as_deref(), Some("CAL-1"));
    assert_eq!(draft.notes.as_deref(), Some("Agenda"));
    assert_eq!(draft.alarm_minutes_before, [10]);
    assert_eq!(
        draft.recurrence.as_ref().unwrap().frequency,
        EventRepeatArg::Weekly
    );
    assert_eq!(
        draft.recurrence.as_ref().unwrap().weekdays,
        [EventWeekdayArg::Monday, EventWeekdayArg::Wednesday]
    );
    assert!(draft.timed);
    assert!(!draft.all_day);
}

#[test]
fn availability_arg_maps_to_eventkit_availability() {
    assert_eq!(
        EventAvailability::from(AvailabilityArg::Free),
        EventAvailability::Free
    );
    assert_eq!(
        EventAvailability::from(AvailabilityArg::Unavailable),
        EventAvailability::Unavailable
    );
}

#[test]
fn exactly_one_calendar_is_marked_as_default() {
    let calendars = vec![calendar("A", "Work"), calendar("B", "Personal")];
    let reports = calendar_reports(&calendars, Some("B"));

    assert_eq!(
        reports
            .iter()
            .filter(|calendar| calendar.is_default_for_new_events)
            .count(),
        1
    );
    assert!(reports[1].is_default_for_new_events);
    assert!(reports[1].allows_modifications);
}

#[test]
fn calendar_list_filters_source_and_writability() {
    let icloud = calendar("A", "Personal");
    let mut exchange = calendar("B", "Work");
    exchange.source = Some("Exchange".to_string());
    let mut read_only = calendar("C", "Holidays");
    read_only.allows_modifications = false;
    let calendars = vec![icloud, exchange, read_only];

    let icloud_writable = filter_calendar_list(calendars, Some("iCloud"), true);

    assert_eq!(icloud_writable.len(), 1);
    assert_eq!(icloud_writable[0].identifier, "A");
}

#[test]
fn add_selection_reports_explicit_or_eventkit_default() {
    let implicit = WriteCalendarSelectorArgs {
        calendar: None,
        calendar_id: None,
        calendar_source: None,
        source_id: None,
    };
    let explicit = WriteCalendarSelectorArgs {
        calendar: None,
        calendar_id: Some("B".to_string()),
        calendar_source: None,
        source_id: None,
    };

    assert_eq!(
        calendar_selection_for_write_with_config(&implicit, false),
        CalendarSelection::EventkitDefault,
    );
    assert_eq!(
        calendar_selection_for_write_with_config(&implicit, true),
        CalendarSelection::ConfiguredDefault,
    );
    assert_eq!(
        calendar_selection_for_write_with_config(&explicit, true),
        CalendarSelection::Explicit,
    );
}

#[test]
fn alarm_validation_rejects_negative_values_before_writes() {
    assert!(validate_alarm_minutes(&[0, 10]).is_ok());
    assert_eq!(
        validate_alarm_minutes(&[-1]).unwrap_err().to_string(),
        "alarm minutes before must be zero or greater: -1"
    );
}

#[test]
fn url_validation_runs_without_writing() {
    assert!(validate_event_url("https://example.com/event").is_ok());
    assert!(validate_event_url("https://exa mple.com/event").is_err());
}

#[test]
fn eventkit_time_zone_validation_runs_without_writing() {
    assert!(validate_event_time_zone("Europe/Berlin").is_ok());
    assert!(validate_event_time_zone("Mars/Olympus_Mons").is_err());
}

#[test]
fn event_range_validation_rejects_non_positive_duration() {
    let start = parse_start_datetime("2026-07-10T10:00").unwrap();
    let same = parse_end_datetime("2026-07-10T10:00").unwrap();
    let earlier = parse_end_datetime("2026-07-10T09:00").unwrap();

    assert!(ensure_valid_event_range(start, same).is_err());
    assert!(ensure_valid_event_range(start, earlier).is_err());
}

#[test]
fn duplicate_window_is_exact_by_default_and_tolerant_when_requested() {
    let expected = parse_start_datetime("2026-07-10T10:00:00+08:00").unwrap();
    let thirty_seconds_later = expected + chrono::Duration::seconds(30);

    assert!(!datetime_within_window(thirty_seconds_later, expected, 0));
    assert!(datetime_within_window(thirty_seconds_later, expected, 30));
    assert!(!datetime_within_window(thirty_seconds_later, expected, 29));
}

#[test]
fn availability_validation_uses_calendar_capabilities() {
    let mut calendar = calendar("A", "Work");
    calendar.supported_event_availabilities = vec!["busy".to_string(), "free".to_string()];

    assert!(ensure_availability_supported(&calendar, Some(EventAvailability::Free)).is_ok());
    assert!(ensure_availability_supported(&calendar, Some(EventAvailability::Tentative)).is_err());
}

#[test]
fn nullable_patch_presence_matches_resulting_field() {
    assert!(patched_field_present(Some("old"), None));
    assert!(patched_field_present(None, Some(Some("new"))));
    assert!(!patched_field_present(Some("old"), Some(None)));
}

#[test]
fn occurrence_start_requires_an_explicit_rfc3339_offset() {
    assert!(parse_occurrence_start("2026-07-20T09:00:00+03:00").is_ok());
    assert!(parse_occurrence_start("2026-07-20T09:00:00").is_err());
    assert_eq!(
        parse_occurrence_start("1969-12-31T23:59:59.750+00:00")
            .unwrap()
            .timestamp(),
        0
    );
}

#[test]
fn recurring_creation_normalizes_weekdays_and_count() {
    let recurrence = parse_event_recurrence(
        &EventRecurrenceArgs {
            repeat: Some(EventRepeatArg::Weekly),
            interval: Some(2),
            weekdays: vec![
                EventWeekdayArg::Wednesday,
                EventWeekdayArg::Monday,
                EventWeekdayArg::Monday,
            ],
            month_days: Vec::new(),
            count: Some(8),
            until: None,
        },
        "2026-07-20T09:00:00+03:00",
        None,
    )
    .unwrap()
    .unwrap();

    assert_eq!(recurrence.frequency, "weekly");
    assert_eq!(recurrence.interval, 2);
    assert_eq!(recurrence.end.occurrence_count, Some(8));
    assert_eq!(
        recurrence.days_of_week.unwrap(),
        vec![
            EventRecurrenceWeekdayReport {
                weekday: 2,
                week_number: 0,
            },
            EventRecurrenceWeekdayReport {
                weekday: 4,
                week_number: 0,
            },
        ]
    );
}

#[test]
fn recurring_creation_rejects_invalid_combinations_and_end() {
    let mut args = EventRecurrenceArgs {
        repeat: Some(EventRepeatArg::Daily),
        weekdays: vec![EventWeekdayArg::Monday],
        ..Default::default()
    };
    assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
    args = EventRecurrenceArgs {
        repeat: Some(EventRepeatArg::Monthly),
        month_days: vec![0],
        ..Default::default()
    };
    assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
    args = EventRecurrenceArgs {
        repeat: Some(EventRepeatArg::Yearly),
        until: Some("2026-07-19T09:00:00+03:00".to_string()),
        ..Default::default()
    };
    assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
    args = EventRecurrenceArgs {
        repeat: Some(EventRepeatArg::Yearly),
        month_days: vec![1],
        ..Default::default()
    };
    assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
}

#[test]
fn recurring_duplicate_identity_requires_the_exact_normalized_rule() {
    let requested = parse_event_recurrence(
        &EventRecurrenceArgs {
            repeat: Some(EventRepeatArg::Weekly),
            weekdays: vec![EventWeekdayArg::Monday],
            ..Default::default()
        },
        "2026-07-20T09:00:00+03:00",
        None,
    )
    .unwrap()
    .unwrap();
    let mut different = requested.clone();
    different.interval = 2;

    assert!(recurrence_rules_match(
        Some(&requested),
        std::slice::from_ref(&requested)
    ));
    assert!(!recurrence_rules_match(Some(&requested), &[different]));
    assert!(!recurrence_rules_match(
        None,
        std::slice::from_ref(&requested)
    ));
    assert!(recurrence_rules_match(None, &[]));
}

#[test]
fn recurring_mutations_require_occurrence_and_explicit_scope() {
    assert!(resolve_event_mutation_scope(true, false, Some(EventScopeArg::Future)).is_err());
    assert!(resolve_event_mutation_scope(true, true, None).is_err());
    assert_eq!(
        resolve_event_mutation_scope(true, true, Some(EventScopeArg::Occurrence)).unwrap(),
        (EventSpan::This, Some("occurrence"))
    );
    assert_eq!(
        resolve_event_mutation_scope(true, true, Some(EventScopeArg::Future)).unwrap(),
        (EventSpan::Future, Some("future"))
    );
}

#[test]
fn nonrecurring_mutations_reject_series_scope() {
    assert_eq!(
        resolve_event_mutation_scope(false, false, None).unwrap(),
        (EventSpan::This, None)
    );
    assert!(resolve_event_mutation_scope(false, true, Some(EventScopeArg::Occurrence)).is_err());
}

#[test]
fn recurring_all_day_events_require_date_only_boundaries() {
    assert!(validate_recurring_all_day_inputs(true, true, "2030-03-30", "2030-03-30").is_ok());
    assert!(
        validate_recurring_all_day_inputs(
            true,
            true,
            "2030-03-30T00:00:00+01:00",
            "2030-03-31T00:00:00+01:00"
        )
        .is_err()
    );
    assert!(
        validate_recurring_all_day_inputs(
            false,
            true,
            "2030-03-30T09:00:00+01:00",
            "2030-03-30T10:00:00+01:00"
        )
        .is_ok()
    );
}

fn calendar(id: &str, title: &str) -> CalendarInfo {
    CalendarInfo {
        identifier: id.to_string(),
        title: title.to_string(),
        source: Some("iCloud".to_string()),
        source_id: Some("SOURCE".to_string()),
        calendar_type: CalendarType::CalDAV,
        allows_modifications: true,
        is_immutable: false,
        is_subscribed: false,
        color: None,
        allowed_entity_types: vec!["event".to_string()],
        supported_event_availabilities: Vec::new(),
    }
}
