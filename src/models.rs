use crate::dates::{datetime_in_time_zone, utc_datetime};
use eventkit::{AlarmInfo, CalendarInfo, EventItem, ParticipantInfo};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JsonOutput {
    Status(StatusReport),
    Doctor {
        doctor: DoctorReport,
    },
    Calendars {
        calendars: Vec<CalendarReport>,
    },
    DefaultCalendar {
        calendar: CalendarReport,
    },
    ReminderStatus(StatusReport),
    ReminderLists {
        lists: Vec<ReminderListReport>,
    },
    DefaultReminderList {
        list: ReminderListReport,
    },
    Reminders {
        reminders: Vec<ReminderReport>,
    },
    Reminder {
        reminder: Box<ReminderReport>,
    },
    Events {
        events: Vec<EventReport>,
    },
    Event {
        event: Box<EventReport>,
    },
    DryRun {
        would_write: bool,
        draft: Box<EventDraftReport>,
    },
    Batch {
        batch: BatchReport,
    },
    Deleted {
        deleted: DeletedReport,
    },
}

impl JsonOutput {
    pub fn has_failures(&self) -> bool {
        matches!(self, Self::Batch { batch } if batch.summary.failed > 0 || batch.summary.not_attempted > 0)
    }
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub authorization: String,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub authorization: String,
    pub reminders_authorization: String,
    pub process: ProcessReport,
    pub info_plist: InfoPlistReport,
    pub recommended_command: String,
    pub recommended_reminders_command: String,
    pub remediation: Vec<String>,
    pub reminders_remediation: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ProcessReport {
    pub pid: u32,
    pub executable: String,
    pub terminal_program: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InfoPlistReport {
    pub embedded: bool,
    pub bundle_identifier: String,
    pub has_full_access_usage_description: bool,
    pub has_legacy_usage_description: bool,
    pub has_write_only_usage_description: bool,
    pub has_reminders_full_access_usage_description: bool,
    pub has_reminders_legacy_usage_description: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReminderListReport {
    pub id: String,
    pub title: String,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub source_type: Option<String>,
    pub list_type: String,
    pub allows_modifications: bool,
    pub is_immutable: bool,
    pub is_subscribed: bool,
    pub is_default_for_new_reminders: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderPriority {
    None,
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderDateKind {
    Date,
    Datetime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReminderDateReport {
    pub kind: ReminderDateKind,
    pub date: Option<String>,
    pub local: Option<String>,
    pub normalized: Option<String>,
    pub utc: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReminderStructuredLocationReport {
    pub title: Option<String>,
    pub radius_meters: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReminderAlarmReport {
    pub relative_offset_seconds: Option<f64>,
    pub absolute_date: Option<String>,
    pub proximity: String,
    pub alarm_type: String,
    pub structured_location: Option<ReminderStructuredLocationReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReminderRecurrenceEndReport {
    pub kind: String,
    pub occurrence_count: Option<usize>,
    pub end_date: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReminderRecurrenceReport {
    pub frequency: String,
    pub interval: usize,
    pub first_day_of_week: isize,
    pub end: ReminderRecurrenceEndReport,
    pub days_of_week: Option<Vec<isize>>,
    pub days_of_month: Option<Vec<i32>>,
    pub months_of_year: Option<Vec<i32>>,
    pub weeks_of_year: Option<Vec<i32>>,
    pub days_of_year: Option<Vec<i32>>,
    pub set_positions: Option<Vec<i32>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReminderReport {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub completion_date: Option<String>,
    pub priority: ReminderPriority,
    pub priority_value: usize,
    pub list: Option<String>,
    pub list_id: Option<String>,
    pub list_source: Option<String>,
    pub list_source_id: Option<String>,
    pub list_type: Option<String>,
    pub allows_list_modifications: Option<bool>,
    pub due: Option<ReminderDateReport>,
    pub start: Option<ReminderDateReport>,
    pub notes: Option<String>,
    pub location: Option<String>,
    pub url: Option<String>,
    pub has_notes: bool,
    pub has_url: bool,
    pub alarm_count: Option<usize>,
    pub recurrence_count: Option<usize>,
    pub alarms: Option<Vec<ReminderAlarmReport>>,
    pub recurrence_rules: Option<Vec<ReminderRecurrenceReport>>,
    pub creation_date: Option<String>,
    pub last_modified_date: Option<String>,
    pub external_identifier: Option<String>,
    pub item_time_zone: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CalendarReport {
    pub id: String,
    pub title: String,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub calendar_type: String,
    pub allows_modifications: bool,
    pub is_immutable: bool,
    pub is_subscribed: bool,
    pub color_rgba: Option<(f64, f64, f64, f64)>,
    pub allowed_entity_types: Vec<String>,
    pub supported_event_availabilities: Vec<String>,
    pub is_default_for_new_events: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarSelection {
    Explicit,
    EventkitDefault,
}

#[derive(Debug, Serialize)]
pub struct EventReport {
    pub id: String,
    pub title: String,
    pub start: String,
    pub end: String,
    pub start_input: Option<String>,
    pub end_input: Option<String>,
    pub start_utc: String,
    pub end_utc: String,
    pub start_local: String,
    pub end_local: String,
    pub start_in_event_time_zone: Option<String>,
    pub end_in_event_time_zone: Option<String>,
    pub duration_seconds: i64,
    pub all_day: bool,
    pub calendar: Option<String>,
    pub calendar_id: Option<String>,
    pub calendar_source: Option<String>,
    pub calendar_source_id: Option<String>,
    pub calendar_type: Option<String>,
    pub allows_calendar_modifications: Option<bool>,
    pub calendar_selection: Option<CalendarSelection>,
    pub write_action: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub status: String,
    pub availability: String,
    pub has_notes: bool,
    pub has_url: bool,
    pub alarm_count: Option<usize>,
    pub is_detached: bool,
    pub occurrence_date: Option<String>,
    pub creation_date: Option<String>,
    pub last_modified_date: Option<String>,
    pub external_identifier: Option<String>,
    pub timezone: Option<String>,
    pub attachments_count: usize,
    pub attendees: Vec<ParticipantReport>,
    pub organizer: Option<ParticipantReport>,
    pub alarms: Option<Vec<AlarmReport>>,
}

#[derive(Debug, Serialize)]
pub struct EventDraftReport {
    pub operation: String,
    pub event_id: Option<String>,
    pub title: String,
    pub start: String,
    pub end: String,
    pub start_input: Option<String>,
    pub end_input: Option<String>,
    pub start_utc: String,
    pub end_utc: String,
    pub start_local: String,
    pub end_local: String,
    pub start_in_event_time_zone: Option<String>,
    pub end_in_event_time_zone: Option<String>,
    pub duration_seconds: i64,
    pub all_day: bool,
    pub timed: bool,
    pub calendar: String,
    pub calendar_id: String,
    pub calendar_source: Option<String>,
    pub calendar_source_id: Option<String>,
    pub calendar_selection: Option<CalendarSelection>,
    pub time_zone: Option<String>,
    pub availability: String,
    pub alarm_count: usize,
    pub has_notes: bool,
    pub has_location: bool,
    pub has_url: bool,
    pub duplicate_warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ParticipantReport {
    pub name: Option<String>,
    pub url: Option<String>,
    pub role: String,
    pub status: String,
    pub is_current_user: bool,
}

#[derive(Debug, Serialize)]
pub struct AlarmReport {
    pub relative_offset_seconds: Option<f64>,
    pub absolute_date: Option<String>,
    pub proximity: String,
    pub alarm_type: String,
}

#[derive(Debug, Serialize)]
pub struct DeletedReport {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct BatchReport {
    pub dry_run: bool,
    pub can_write: bool,
    pub if_exists: String,
    pub continue_on_error: bool,
    pub summary: BatchSummaryReport,
    pub items: Vec<BatchItemReport>,
}

#[derive(Debug, Default, Serialize)]
pub struct BatchSummaryReport {
    pub total: usize,
    pub created: usize,
    pub skipped: usize,
    pub updated: usize,
    pub failed: usize,
    pub not_attempted: usize,
    pub would_create: usize,
    pub would_skip: usize,
    pub would_update: usize,
}

#[derive(Debug, Serialize)]
pub struct BatchItemReport {
    pub index: usize,
    pub client_id: Option<String>,
    pub status: String,
    pub event_id: Option<String>,
    pub matched_event_id: Option<String>,
    pub draft: Option<Box<EventDraftReport>>,
    pub error: Option<BatchErrorReport>,
}

#[derive(Debug, Serialize)]
pub struct BatchErrorReport {
    pub message: String,
}

impl From<&CalendarInfo> for CalendarReport {
    fn from(calendar: &CalendarInfo) -> Self {
        Self {
            id: calendar.identifier.clone(),
            title: calendar.title.clone(),
            source: calendar.source.clone(),
            source_id: calendar.source_id.clone(),
            calendar_type: format!("{:?}", calendar.calendar_type),
            allows_modifications: calendar.allows_modifications,
            is_immutable: calendar.is_immutable,
            is_subscribed: calendar.is_subscribed,
            color_rgba: calendar.color,
            allowed_entity_types: calendar.allowed_entity_types.clone(),
            supported_event_availabilities: calendar.supported_event_availabilities.clone(),
            is_default_for_new_events: false,
        }
    }
}

impl From<&EventItem> for EventReport {
    fn from(event: &EventItem) -> Self {
        let start_local = event.start_date.to_rfc3339();
        let end_local = event.end_date.to_rfc3339();
        let start_in_event_time_zone = event
            .timezone
            .as_deref()
            .and_then(|time_zone| datetime_in_time_zone(event.start_date, time_zone).ok());
        let end_in_event_time_zone = event
            .timezone
            .as_deref()
            .and_then(|time_zone| datetime_in_time_zone(event.end_date, time_zone).ok());
        Self {
            id: event.identifier.clone(),
            title: event.title.clone(),
            start: start_local.clone(),
            end: end_local.clone(),
            start_input: None,
            end_input: None,
            start_utc: utc_datetime(event.start_date),
            end_utc: utc_datetime(event.end_date),
            start_local,
            end_local,
            start_in_event_time_zone,
            end_in_event_time_zone,
            duration_seconds: (event.end_date - event.start_date).num_seconds(),
            all_day: event.all_day,
            calendar: event.calendar_title.clone(),
            calendar_id: event.calendar_id.clone(),
            calendar_source: None,
            calendar_source_id: None,
            calendar_type: None,
            allows_calendar_modifications: None,
            calendar_selection: None,
            write_action: None,
            location: event.location.clone(),
            notes: event.notes.clone(),
            url: event.URL.clone(),
            status: format!("{:?}", event.status),
            availability: format!("{:?}", event.availability),
            has_notes: event.notes.is_some(),
            has_url: event.URL.is_some(),
            alarm_count: None,
            is_detached: event.is_detached,
            occurrence_date: event.occurrence_date.map(|value| value.to_rfc3339()),
            creation_date: event.creation_date.map(|value| value.to_rfc3339()),
            last_modified_date: event.last_modified_date.map(|value| value.to_rfc3339()),
            external_identifier: event.external_identifier.clone(),
            timezone: event.timezone.clone(),
            attachments_count: event.attachments_count,
            attendees: event
                .attendees
                .iter()
                .map(ParticipantReport::from)
                .collect(),
            organizer: event.organizer.as_ref().map(ParticipantReport::from),
            alarms: None,
        }
    }
}

impl From<&ParticipantInfo> for ParticipantReport {
    fn from(participant: &ParticipantInfo) -> Self {
        Self {
            name: participant.name.clone(),
            url: participant.URL.clone(),
            role: format!("{:?}", participant.role),
            status: format!("{:?}", participant.status),
            is_current_user: participant.is_current_user,
        }
    }
}

impl From<&AlarmInfo> for AlarmReport {
    fn from(alarm: &AlarmInfo) -> Self {
        Self {
            relative_offset_seconds: alarm.relative_offset,
            absolute_date: alarm.absolute_date.map(|value| value.to_rfc3339()),
            proximity: format!("{:?}", alarm.proximity),
            alarm_type: format!("{:?}", alarm.alarm_type),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::{parse_end_datetime, parse_start_datetime};
    use eventkit::{CalendarType, EventAvailability, EventStatus};
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
    }

    #[test]
    fn dry_run_output_exposes_resolved_write_plan() {
        let value = serde_json::to_value(JsonOutput::DryRun {
            would_write: false,
            draft: Box::new(EventDraftReport {
                operation: "add".to_string(),
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
}
