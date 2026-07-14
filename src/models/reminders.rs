use super::{BatchErrorReport, BatchSummaryReport};
use serde::Serialize;

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
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
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
    pub parent_id: Option<String>,
    pub child_count: usize,
    pub child_ids: Option<Vec<String>>,
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
    pub list_selection: Option<ReminderListSelection>,
    pub write_action: Option<String>,
    pub due: Option<ReminderDateReport>,
    pub due_input: Option<String>,
    pub start: Option<ReminderDateReport>,
    pub start_input: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReminderDraftReport {
    pub operation: String,
    pub matched_reminder_id: Option<String>,
    pub title: String,
    pub parent_id: Option<String>,
    pub list: String,
    pub list_id: String,
    pub list_source: Option<String>,
    pub list_source_id: Option<String>,
    pub list_selection: ReminderListSelection,
    pub due: Option<ReminderDateReport>,
    pub due_input: Option<String>,
    pub start: Option<ReminderDateReport>,
    pub start_input: Option<String>,
    pub priority: ReminderPriority,
    pub priority_value: usize,
    pub has_notes: bool,
    pub has_location: bool,
    pub has_url: bool,
    pub notification_count: usize,
    pub notifications: Vec<ReminderNotificationReport>,
    pub planned_alarm_count: usize,
    pub planned_alarms: Vec<ReminderPlannedAlarmReport>,
    pub recurrence: Option<ReminderRecurrenceReport>,
    pub if_exists: String,
    pub duplicate_window_seconds: i64,
    pub duplicate_warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReminderPlannedAlarmReport {
    pub kind: String,
    pub input: Option<String>,
    pub absolute_utc: Option<String>,
    pub minutes_before_due: Option<i64>,
    pub proximity: Option<String>,
    pub structured_location: Option<ReminderStructuredLocationReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReminderMutationDraftReport {
    pub operation: String,
    pub reminder_id: String,
    pub changed_fields: Vec<String>,
    pub before: Box<ReminderReport>,
    pub result: Box<ReminderReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReminderDeletedReport {
    pub id: String,
    pub title: String,
    pub list: Option<String>,
    pub list_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReminderBatchReport {
    pub version: u8,
    pub dry_run: bool,
    pub can_write: bool,
    pub if_exists: String,
    pub continue_on_error: bool,
    pub summary: BatchSummaryReport,
    pub items: Vec<ReminderBatchItemReport>,
}

#[derive(Debug, Serialize)]
pub struct ReminderBatchItemReport {
    pub index: usize,
    pub client_id: Option<String>,
    pub status: String,
    pub reminder_id: Option<String>,
    pub matched_reminder_id: Option<String>,
    pub draft: Option<Box<ReminderDraftReport>>,
    pub error: Option<BatchErrorReport>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReminderNotificationReport {
    pub kind: String,
    pub minutes_before: i64,
    pub absolute_utc: String,
    pub absolute_in_due_time_zone: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderListSelection {
    Explicit,
    ConfiguredDefault,
    EventkitDefault,
}
