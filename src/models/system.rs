use super::*;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JsonOutput {
    Config {
        config: ConfigReport,
    },
    Version {
        version: VersionReport,
    },
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
    ReminderDryRun {
        would_write: bool,
        draft: Box<ReminderDraftReport>,
    },
    ReminderMutationDryRun {
        would_write: bool,
        draft: Box<ReminderMutationDraftReport>,
    },
    ReminderDeleted {
        deleted: ReminderDeletedReport,
    },
    ReminderBatch {
        batch: ReminderBatchReport,
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

#[derive(Debug, Serialize)]
pub struct ConfigReport {
    pub action: String,
    pub path: String,
    pub key: Option<String>,
    pub value: Option<String>,
    pub contents: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VersionReport {
    pub name: String,
    pub version: String,
    pub git_commit: Option<String>,
    pub target: String,
    pub profile: String,
}

impl JsonOutput {
    pub fn has_failures(&self) -> bool {
        matches!(self, Self::Batch { batch } if batch.summary.failed > 0 || batch.summary.not_attempted > 0)
            || matches!(self, Self::ReminderBatch { batch } if batch.summary.failed > 0 || batch.summary.not_attempted > 0)
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
    pub location_authorization: String,
    pub location_services_enabled: bool,
    pub process: ProcessReport,
    pub info_plist: InfoPlistReport,
    pub recommended_command: String,
    pub recommended_reminders_command: String,
    pub remediation: Vec<String>,
    pub reminders_remediation: Vec<String>,
    pub location_remediation: Vec<String>,
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
    pub has_location_usage_description: bool,
}
