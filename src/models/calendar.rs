use crate::dates::{datetime_in_time_zone, utc_datetime};
use eventkit::{AlarmInfo, CalendarInfo, EventItem, ParticipantInfo};
use serde::Serialize;

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
    ConfiguredDefault,
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
    pub write_scope: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub status: String,
    pub availability: String,
    pub has_notes: bool,
    pub has_url: bool,
    pub alarm_count: Option<usize>,
    pub recurrence_count: Option<usize>,
    pub recurrence_rules: Option<Vec<EventRecurrenceReport>>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EventRecurrenceEndReport {
    pub kind: String,
    pub occurrence_count: Option<usize>,
    pub end_date: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EventRecurrenceReport {
    pub frequency: String,
    pub interval: usize,
    pub first_day_of_week: isize,
    pub end: EventRecurrenceEndReport,
    pub days_of_week: Option<Vec<EventRecurrenceWeekdayReport>>,
    pub days_of_month: Option<Vec<i32>>,
    pub months_of_year: Option<Vec<i32>>,
    pub weeks_of_year: Option<Vec<i32>>,
    pub days_of_year: Option<Vec<i32>>,
    pub set_positions: Option<Vec<i32>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct EventRecurrenceWeekdayReport {
    pub weekday: isize,
    pub week_number: isize,
}

#[derive(Debug, Serialize)]
pub struct EventDraftReport {
    pub operation: String,
    pub scope: Option<String>,
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
    pub recurrence: Option<EventRecurrenceReport>,
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
    pub scope: Option<String>,
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
            write_scope: None,
            location: event.location.clone(),
            notes: event.notes.clone(),
            url: event.URL.clone(),
            status: format!("{:?}", event.status),
            availability: format!("{:?}", event.availability),
            has_notes: event.notes.is_some(),
            has_url: event.URL.is_some(),
            alarm_count: None,
            recurrence_count: None,
            recurrence_rules: None,
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
