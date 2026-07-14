use super::*;

#[test]
fn alarm_label_formats_relative_offsets() {
    let before = AlarmReport {
        relative_offset_seconds: Some(-600.0),
        absolute_date: None,
        proximity: "None".to_string(),
        alarm_type: "Display".to_string(),
    };
    let after = AlarmReport {
        relative_offset_seconds: Some(300.0),
        absolute_date: None,
        proximity: "None".to_string(),
        alarm_type: "Display".to_string(),
    };

    assert_eq!(alarm_label(&before), "10 minutes before");
    assert_eq!(alarm_label(&after), "5 minutes after");
}

#[test]
fn time_format_includes_rfc3339_offset() {
    assert_eq!(time_with_offset("2026-07-12T15:55:00+03:00"), "15:55+03:00");
    assert_eq!(time_with_offset("2026-07-12T15:55:00+02:00"), "15:55+02:00");
}

#[test]
fn reminder_recurrence_label_includes_termination() {
    let mut rule = ReminderRecurrenceReport {
        frequency: "monthly".to_string(),
        interval: 2,
        first_day_of_week: 0,
        end: ReminderRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(12),
            end_date: None,
        },
        days_of_week: None,
        days_of_month: None,
        months_of_year: None,
        weeks_of_year: None,
        days_of_year: None,
        set_positions: None,
    };
    assert_eq!(
        reminder_recurrence_label(&rule),
        "every 2 monthly (count=12)"
    );
    rule.end = ReminderRecurrenceEndReport {
        kind: "date".to_string(),
        occurrence_count: None,
        end_date: Some("2026-12-31T21:59:00+00:00".to_string()),
    };
    assert_eq!(
        reminder_recurrence_label(&rule),
        "every 2 monthly (until=2026-12-31T21:59:00+00:00)"
    );
    rule.end = ReminderRecurrenceEndReport {
        kind: "never".to_string(),
        occurrence_count: None,
        end_date: None,
    };
    assert_eq!(reminder_recurrence_label(&rule), "every 2 monthly (never)");
}

#[test]
fn event_recurrence_label_includes_components_and_termination() {
    let rule = EventRecurrenceReport {
        frequency: "monthly".to_string(),
        interval: 2,
        first_day_of_week: 2,
        end: crate::models::EventRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(6),
            end_date: None,
        },
        days_of_week: Some(vec![
            crate::models::EventRecurrenceWeekdayReport {
                weekday: 2,
                week_number: 1,
            },
            crate::models::EventRecurrenceWeekdayReport {
                weekday: 4,
                week_number: -1,
            },
        ]),
        days_of_month: Some(vec![1, -1]),
        months_of_year: None,
        weeks_of_year: None,
        days_of_year: None,
        set_positions: Some(vec![1]),
    };

    assert_eq!(
        event_recurrence_label(&rule),
        "every 2 monthly; week starts mon; weekdays mon(week=1),wed(week=-1); month days 1,-1; set positions 1; ends after 6 occurrences"
    );
}
