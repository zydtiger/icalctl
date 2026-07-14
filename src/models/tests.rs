use super::*;
use crate::dates::{parse_end_datetime, parse_start_datetime};
use eventkit::{CalendarInfo, CalendarType, EventAvailability, EventItem, EventStatus};
use serde_json::json;

fn calendar() -> CalendarInfo {
    CalendarInfo {
        identifier: "CAL-1".to_string(),
        title: "Calendar".to_string(),
        source: Some("iCloud".to_string()),
        source_id: Some("SOURCE-1".to_string()),
        calendar_type: CalendarType::CalDAV,
        allows_modifications: true,
        is_immutable: false,
        is_subscribed: false,
        color: None,
        allowed_entity_types: vec!["event".to_string()],
        supported_event_availabilities: vec!["busy".to_string()],
    }
}

#[test]
fn version_output_has_stable_machine_readable_fields() {
    let output = JsonOutput::Version {
        version: VersionReport {
            name: "icalctl".to_string(),
            version: "0.1.0".to_string(),
            git_commit: Some("abc123".to_string()),
            target: "aarch64-apple-darwin".to_string(),
            profile: "release".to_string(),
        },
    };
    let value = serde_json::to_value(output).unwrap();

    assert_eq!(value["type"], "version");
    assert_eq!(value["version"]["version"], "0.1.0");
    assert_eq!(value["version"]["git_commit"], "abc123");
    assert_eq!(value["version"]["target"], "aarch64-apple-darwin");
    assert_eq!(value["version"]["profile"], "release");
}

#[test]
fn default_calendar_output_includes_identity_and_default_marker() {
    let mut calendar = CalendarReport::from(&calendar());
    calendar.is_default_for_new_events = true;
    let value = serde_json::to_value(JsonOutput::DefaultCalendar { calendar }).unwrap();

    assert_eq!(value["type"], "default_calendar");
    assert_eq!(value["calendar"]["id"], "CAL-1");
    assert_eq!(value["calendar"]["source"], "iCloud");
    assert_eq!(value["calendar"]["source_id"], "SOURCE-1");
    assert_eq!(value["calendar"]["calendar_type"], "CalDAV");
    assert_eq!(value["calendar"]["allows_modifications"], true);
    assert_eq!(value["calendar"]["is_default_for_new_events"], true);
}

#[test]
fn calendar_selection_serializes_for_add_provenance() {
    assert_eq!(
        serde_json::to_value(CalendarSelection::Explicit).unwrap(),
        json!("explicit")
    );
    assert_eq!(
        serde_json::to_value(CalendarSelection::EventkitDefault).unwrap(),
        json!("eventkit_default")
    );
    assert_eq!(
        serde_json::to_value(CalendarSelection::ConfiguredDefault).unwrap(),
        json!("configured_default")
    );
}

#[test]
fn dry_run_output_exposes_resolved_write_plan() {
    let value = serde_json::to_value(JsonOutput::DryRun {
        would_write: false,
        draft: Box::new(EventDraftReport {
            operation: "add".to_string(),
            scope: None,
            event_id: None,
            title: "Meeting".to_string(),
            start: "2026-07-10T09:00:00+08:00".to_string(),
            end: "2026-07-10T10:00:00+08:00".to_string(),
            start_input: Some("2026-07-10T09:00:00+08:00".to_string()),
            end_input: Some("2026-07-10T10:00:00+08:00".to_string()),
            start_utc: "2026-07-10T01:00:00+00:00".to_string(),
            end_utc: "2026-07-10T02:00:00+00:00".to_string(),
            start_local: "2026-07-10T09:00:00+08:00".to_string(),
            end_local: "2026-07-10T10:00:00+08:00".to_string(),
            start_in_event_time_zone: Some("2026-07-10T03:00:00+02:00".to_string()),
            end_in_event_time_zone: Some("2026-07-10T04:00:00+02:00".to_string()),
            duration_seconds: 3600,
            all_day: false,
            timed: true,
            calendar: "Work".to_string(),
            calendar_id: "CAL-1".to_string(),
            calendar_source: Some("iCloud".to_string()),
            calendar_source_id: Some("SOURCE-1".to_string()),
            calendar_selection: Some(CalendarSelection::Explicit),
            time_zone: Some("Europe/Berlin".to_string()),
            availability: "busy".to_string(),
            alarm_count: 1,
            recurrence: None,
            has_notes: true,
            has_location: false,
            has_url: true,
            duplicate_warnings: Vec::new(),
        }),
    })
    .unwrap();

    assert_eq!(value["type"], "dry_run");
    assert_eq!(value["would_write"], false);
    assert_eq!(value["draft"]["calendar_id"], "CAL-1");
    assert_eq!(value["draft"]["timed"], true);
    assert_eq!(value["draft"]["start_input"], "2026-07-10T09:00:00+08:00");
    assert_eq!(value["draft"]["start_utc"], "2026-07-10T01:00:00+00:00");
    assert_eq!(value["draft"]["time_zone"], "Europe/Berlin");
    assert_eq!(value["draft"]["duration_seconds"], 3600);
    assert_eq!(value["draft"]["alarm_count"], 1);
    assert_eq!(value["draft"]["has_notes"], true);
    assert_eq!(value["draft"]["has_url"], true);
}

#[test]
fn event_readback_reports_utc_and_stored_time_zone() {
    let event = EventItem {
        identifier: "EVENT-1".to_string(),
        title: "Flight".to_string(),
        notes: Some("Flight notes".to_string()),
        location: None,
        start_date: parse_start_datetime("2026-07-12T15:55:00+03:00").unwrap(),
        end_date: parse_end_datetime("2026-07-12T15:55:00+02:00").unwrap(),
        all_day: false,
        calendar_title: Some("Travel".to_string()),
        calendar_id: Some("CAL-1".to_string()),
        URL: Some("https://example.com/flight".to_string()),
        availability: EventAvailability::Busy,
        status: EventStatus::Confirmed,
        is_detached: false,
        occurrence_date: None,
        structured_location: None,
        creation_date: None,
        last_modified_date: None,
        external_identifier: None,
        timezone: Some("Europe/Berlin".to_string()),
        attachments_count: 0,
        attendees: Vec::new(),
        organizer: None,
    };

    let report = EventReport::from(&event);

    assert_eq!(report.duration_seconds, 3600);
    assert!(report.has_notes);
    assert!(report.has_url);
    assert_eq!(report.alarm_count, None);
    assert_eq!(report.start_utc, "2026-07-12T12:55:00+00:00");
    assert_eq!(report.end_utc, "2026-07-12T13:55:00+00:00");
    assert_eq!(
        report.start_in_event_time_zone.as_deref(),
        Some("2026-07-12T14:55:00+02:00")
    );
    assert_eq!(
        report.end_in_event_time_zone.as_deref(),
        Some("2026-07-12T15:55:00+02:00")
    );

    let summary = serde_json::to_value(&report).unwrap();
    assert_eq!(summary["recurrence_count"], serde_json::Value::Null);
    assert_eq!(summary["recurrence_rules"], serde_json::Value::Null);

    let mut detail = report;
    detail.is_detached = true;
    detail.occurrence_date = Some("2026-07-05T12:55:00+00:00".to_string());
    detail.recurrence_count = Some(0);
    detail.recurrence_rules = Some(Vec::new());
    let empty_detail = serde_json::to_value(&detail).unwrap();
    assert_eq!(empty_detail["recurrence_count"], 0);
    assert_eq!(empty_detail["recurrence_rules"], serde_json::json!([]));

    detail.recurrence_count = Some(1);
    detail.recurrence_rules = Some(vec![EventRecurrenceReport {
        frequency: "weekly".to_string(),
        interval: 1,
        first_day_of_week: 2,
        end: EventRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        },
        days_of_week: Some(vec![EventRecurrenceWeekdayReport {
            weekday: 2,
            week_number: 0,
        }]),
        days_of_month: None,
        months_of_year: None,
        weeks_of_year: None,
        days_of_year: None,
        set_positions: None,
    }]);
    let value = serde_json::to_value(detail).unwrap();
    assert_eq!(value["recurrence_count"], 1);
    assert_eq!(value["recurrence_rules"][0]["frequency"], "weekly");
    assert_eq!(value["is_detached"], true);
    assert_eq!(value["occurrence_date"], "2026-07-05T12:55:00+00:00");
}

#[test]
fn batch_output_has_per_item_status_id_and_error_shape() {
    let output = JsonOutput::Batch {
        batch: BatchReport {
            dry_run: true,
            can_write: false,
            if_exists: "error".to_string(),
            continue_on_error: false,
            summary: BatchSummaryReport {
                total: 1,
                failed: 1,
                ..Default::default()
            },
            items: vec![BatchItemReport {
                index: 0,
                client_id: Some("flight-1".to_string()),
                status: "failed".to_string(),
                event_id: Some("EVENT-1".to_string()),
                matched_event_id: Some("EVENT-1".to_string()),
                draft: None,
                error: Some(BatchErrorReport {
                    message: "matching event already exists".to_string(),
                }),
            }],
        },
    };
    let value = serde_json::to_value(&output).unwrap();

    assert_eq!(value["type"], "batch");
    assert_eq!(value["batch"]["items"][0]["status"], "failed");
    assert_eq!(value["batch"]["items"][0]["event_id"], "EVENT-1");
    assert_eq!(
        value["batch"]["items"][0]["error"]["message"],
        "matching event already exists"
    );
    assert!(output.has_failures());
}
