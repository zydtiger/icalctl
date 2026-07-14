use super::IfExistsArg;
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct ReadReminderListSelectorArgs {
    /// Reminder list title to include. Can be passed more than once.
    #[arg(short = 'l', long = "list")]
    pub lists: Vec<String>,

    /// Exact EventKit reminder list id to include. Can be passed more than once.
    #[arg(long = "list-id")]
    pub list_ids: Vec<String>,

    /// Source title that qualifies every --list title.
    #[arg(long = "list-source", requires = "lists", conflicts_with = "source_id")]
    pub list_source: Option<String>,

    /// Exact EventKit source id that qualifies every --list title.
    #[arg(long, requires = "lists")]
    pub source_id: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct WriteReminderListSelectorArgs {
    /// Reminder list title. Add uses the configured or EventKit default; update keeps the current list.
    #[arg(short = 'l', long = "list")]
    pub list: Option<String>,

    /// Exact EventKit reminder list id.
    #[arg(
        long = "list-id",
        conflicts_with_all = ["list", "list_source", "source_id"]
    )]
    pub list_id: Option<String>,

    /// Source title that qualifies --list.
    #[arg(long = "list-source", requires = "list", conflicts_with = "source_id")]
    pub list_source: Option<String>,

    /// Exact EventKit source id that qualifies --list.
    #[arg(long, requires = "list")]
    pub source_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct ReminderReadFilterArgs {
    #[command(flatten)]
    pub list_selector: ReadReminderListSelectorArgs,

    /// Completion state to include.
    #[arg(long, value_enum, default_value_t = ReminderStateArg::Incomplete)]
    pub state: ReminderStateArg,

    /// Inclusive lower bound for due dates or datetimes. Undated reminders are excluded.
    #[arg(long = "due-from", value_name = "VALUE")]
    pub due_from: Option<String>,

    /// Inclusive upper bound for due dates or datetimes. Undated reminders are excluded.
    #[arg(long = "due-to", value_name = "VALUE")]
    pub due_to: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub struct ReminderAdvancedScheduleArgs {
    /// Notify at an arbitrary RFC3339 instant with an explicit UTC offset. Repeatable.
    #[arg(long = "notify-at", value_name = "RFC3339")]
    pub notify_at: Vec<String>,

    /// Label for a single arrival/departure geofence alarm.
    #[arg(
        long = "geofence-title",
        requires_all = ["geofence_latitude", "geofence_longitude", "geofence_radius_meters", "geofence_proximity"]
    )]
    pub geofence_title: Option<String>,

    /// Geofence latitude from -90 through 90.
    #[arg(long = "geofence-latitude", requires = "geofence_title")]
    pub geofence_latitude: Option<f64>,

    /// Geofence longitude from -180 through 180.
    #[arg(long = "geofence-longitude", requires = "geofence_title")]
    pub geofence_longitude: Option<f64>,

    /// Positive geofence radius in meters.
    #[arg(long = "geofence-radius-meters", requires = "geofence_title")]
    pub geofence_radius_meters: Option<f64>,

    /// Trigger when arriving at or leaving the geofence.
    #[arg(long = "geofence-proximity", value_enum, requires = "geofence_title")]
    pub geofence_proximity: Option<ReminderGeofenceProximityArg>,

    /// Simple recurrence frequency.
    #[arg(long = "repeat", value_enum)]
    pub repeat: Option<ReminderRepeatArg>,

    /// Positive recurrence interval; defaults to 1 when --repeat is supplied.
    #[arg(long = "repeat-interval", requires = "repeat")]
    pub repeat_interval: Option<usize>,

    /// Stop after this positive number of occurrences.
    #[arg(
        long = "repeat-count",
        requires = "repeat",
        conflicts_with = "repeat_until"
    )]
    pub repeat_count: Option<usize>,

    /// Stop after an RFC3339 instant with an explicit UTC offset.
    #[arg(long = "repeat-until", value_name = "RFC3339", requires = "repeat")]
    pub repeat_until: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum RemindersCommand {
    /// Print EventKit Reminders authorization status.
    Status,

    /// List reminder lists available in Reminders.app.
    Lists {
        /// Only include lists from this exact source title.
        #[arg(long)]
        source: Option<String>,

        /// Only include lists that allow reminder modifications.
        #[arg(long)]
        writable_only: bool,
    },

    /// Show the system default list for new reminders.
    DefaultList,

    /// List reminders, defaulting to incomplete items.
    List {
        #[command(flatten)]
        filters: ReminderReadFilterArgs,
    },

    /// Search reminder title, notes, location, URL, and list name.
    Search {
        /// Case-insensitive search query.
        query: String,

        #[command(flatten)]
        filters: ReminderReadFilterArgs,
    },

    /// Show one reminder by exact EventKit identifier or cached row number.
    Show {
        /// EventKit reminder identifier, or row number from the last reminder list.
        id: String,
    },

    /// Create a reminder after resolving and validating its exact target list.
    Add {
        /// Reminder title.
        title: Option<String>,

        /// Read a complete structured reminder draft from a strict JSON file.
        #[arg(long, value_name = "PATH")]
        json_file: Option<PathBuf>,

        #[command(flatten)]
        list_selector: WriteReminderListSelectorArgs,

        /// Exact EventKit id of an existing reminder to use as the native parent.
        #[arg(long = "parent-id", value_name = "REMINDER_ID")]
        parent_id: Option<String>,

        /// Due date or datetime. Omit for an undated reminder.
        #[arg(long, value_name = "VALUE")]
        due: Option<String>,

        /// Start date or datetime.
        #[arg(long, value_name = "VALUE")]
        start: Option<String>,

        /// IANA zone for timezone-less timed due/start values.
        #[arg(long = "time-zone", value_name = "TZID")]
        time_zone: Option<String>,

        /// Reminder notes.
        #[arg(long, conflicts_with = "notes_file")]
        notes: Option<String>,

        /// Read exact reminder notes from a UTF-8 file, or stdin with -.
        #[arg(long, value_name = "PATH")]
        notes_file: Option<PathBuf>,

        /// Reminder URL.
        #[arg(long)]
        url: Option<String>,

        /// Free-text reminder location.
        #[arg(long)]
        location: Option<String>,

        /// Reminder priority. Priority does not create a notification.
        #[arg(long, value_enum)]
        priority: Option<ReminderPriorityArg>,

        /// Notify exactly at the timed due instant.
        #[arg(long)]
        notify_at_due: bool,

        /// Notify N minutes before the timed due instant. Repeatable.
        #[arg(long = "notify-minutes-before", value_name = "MINUTES")]
        notify_minutes_before: Vec<i64>,

        #[command(flatten)]
        schedule: ReminderAdvancedScheduleArgs,

        /// Behavior when a matching reminder already exists.
        #[arg(long = "if-exists", value_enum, default_value_t = IfExistsArg::Error)]
        if_exists: IfExistsArg,

        /// Timed-due tolerance in seconds for duplicate matching.
        #[arg(long, default_value_t = 0)]
        duplicate_window_seconds: i64,

        /// Validate and print the resolved reminder without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Safely create or reconcile multiple reminders from versioned JSON.
    Batch {
        #[command(subcommand)]
        command: ReminderBatchCommand,
    },

    /// Patch an existing reminder; omitted fields remain unchanged.
    Update {
        /// EventKit reminder identifier, or row number from the last reminder list.
        id: String,

        /// Replace the reminder title.
        #[arg(long)]
        title: Option<String>,

        #[command(flatten)]
        list_selector: WriteReminderListSelectorArgs,

        /// Reparent under this exact EventKit reminder id.
        #[arg(
            long = "parent-id",
            value_name = "REMINDER_ID",
            conflicts_with = "clear_parent"
        )]
        parent_id: Option<String>,

        /// Remove the reminder from its current parent.
        #[arg(long)]
        clear_parent: bool,

        /// Replace the due date or datetime.
        #[arg(long, value_name = "VALUE", conflicts_with = "clear_due")]
        due: Option<String>,

        /// Remove the due date.
        #[arg(long)]
        clear_due: bool,

        /// Replace the start date or datetime.
        #[arg(long, value_name = "VALUE", conflicts_with = "clear_start")]
        start: Option<String>,

        /// Remove the start date.
        #[arg(long)]
        clear_start: bool,

        /// Set the IANA zone on timezone-less supplied or existing timed due/start values.
        #[arg(
            long = "time-zone",
            value_name = "TZID",
            conflicts_with = "clear_time_zone"
        )]
        time_zone: Option<String>,

        /// Remove timezone metadata from supplied or existing timed due/start values.
        #[arg(long)]
        clear_time_zone: bool,

        /// Replace reminder notes.
        #[arg(long, conflicts_with_all = ["notes_file", "clear_notes"])]
        notes: Option<String>,

        /// Read replacement notes from a UTF-8 file, or stdin with -.
        #[arg(long, value_name = "PATH", conflicts_with = "clear_notes")]
        notes_file: Option<PathBuf>,

        /// Remove reminder notes.
        #[arg(long)]
        clear_notes: bool,

        /// Replace the reminder URL.
        #[arg(long, conflicts_with = "clear_url")]
        url: Option<String>,

        /// Remove the reminder URL.
        #[arg(long)]
        clear_url: bool,

        /// Replace the free-text reminder location.
        #[arg(long, conflicts_with = "clear_location")]
        location: Option<String>,

        /// Remove the reminder location.
        #[arg(long)]
        clear_location: bool,

        /// Replace reminder priority; use none to clear it.
        #[arg(long, value_enum)]
        priority: Option<ReminderPriorityArg>,

        /// Notify exactly at the resulting timed due instant.
        #[arg(long)]
        notify_at_due: bool,

        /// Notify N minutes before the resulting timed due instant. Repeatable.
        #[arg(long = "notify-minutes-before", value_name = "MINUTES")]
        notify_minutes_before: Vec<i64>,

        #[command(flatten)]
        schedule: ReminderAdvancedScheduleArgs,

        /// Remove every existing time and location alarm.
        #[arg(long, conflicts_with_all = ["notify_at_due", "notify_minutes_before", "notify_at", "geofence_title"])]
        clear_notifications: bool,

        /// Remove the existing recurrence rule.
        #[arg(long, conflicts_with = "repeat")]
        clear_recurrence: bool,

        /// Validate and print the patch without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Mark a reminder complete.
    Complete {
        /// EventKit reminder identifier, or row number from the last reminder list.
        id: String,

        /// Completion instant; must be RFC3339 with an explicit UTC offset.
        #[arg(long, value_name = "RFC3339")]
        completed_at: Option<String>,

        /// Validate and print the completion without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Mark a reminder incomplete and clear its completion date.
    Uncomplete {
        /// EventKit reminder identifier, or row number from the last reminder list.
        id: String,

        /// Validate and print the change without writing.
        #[arg(long)]
        dry_run: bool,
    },

    /// Delete a reminder by exact EventKit identifier or cached row number.
    Delete {
        /// EventKit reminder identifier, or row number from the last reminder list.
        id: String,

        /// Delete without an interactive confirmation prompt.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ReminderBatchCommand {
    /// Create reminders from a versioned JSON batch file.
    Add {
        /// Path to the reminder batch file.
        #[arg(long)]
        file: PathBuf,

        /// Behavior when a matching reminder already exists.
        #[arg(long = "if-exists", value_enum, default_value_t = IfExistsArg::Error)]
        if_exists: IfExistsArg,

        /// Validate every row and report planned actions without writing.
        #[arg(long)]
        dry_run: bool,

        /// Process valid rows despite preflight or write failures.
        #[arg(long)]
        continue_on_error: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum ReminderStateArg {
    #[default]
    Incomplete,
    Completed,
    All,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReminderPriorityArg {
    None,
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReminderGeofenceProximityArg {
    Arrive,
    Leave,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ReminderRepeatArg {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}
