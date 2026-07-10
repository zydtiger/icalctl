use eventkit::{AlarmInfo, CalendarInfo, EventItem, ParticipantInfo};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JsonOutput {
    Status(StatusReport),
    Calendars { calendars: Vec<CalendarReport> },
    Events { events: Vec<EventReport> },
    Event { event: Box<EventReport> },
    Deleted { deleted: DeletedReport },
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub authorization: String,
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
}

#[derive(Debug, Serialize)]
pub struct EventReport {
    pub id: String,
    pub title: String,
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub calendar: Option<String>,
    pub calendar_id: Option<String>,
    pub calendar_source: Option<String>,
    pub calendar_source_id: Option<String>,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    pub status: String,
    pub availability: String,
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
        }
    }
}

impl From<&EventItem> for EventReport {
    fn from(event: &EventItem) -> Self {
        Self {
            id: event.identifier.clone(),
            title: event.title.clone(),
            start: event.start_date.to_rfc3339(),
            end: event.end_date.to_rfc3339(),
            all_day: event.all_day,
            calendar: event.calendar_title.clone(),
            calendar_id: event.calendar_id.clone(),
            calendar_source: None,
            calendar_source_id: None,
            location: event.location.clone(),
            notes: event.notes.clone(),
            url: event.URL.clone(),
            status: format!("{:?}", event.status),
            availability: format!("{:?}", event.availability),
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
