use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "icalctl",
    about = "Manage local macOS Apple Calendar and Reminders data through EventKit"
)]
pub struct Cli {
    /// Print compact JSON instead of human-friendly text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct ReadCalendarSelectorArgs {
    /// Calendar title to include. Can be passed more than once.
    #[arg(short, long = "calendar")]
    pub calendars: Vec<String>,

    /// Exact EventKit calendar id to include. Can be passed more than once.
    #[arg(long = "calendar-id")]
    pub calendar_ids: Vec<String>,

    /// Source title that qualifies every --calendar title.
    #[arg(
        long = "calendar-source",
        requires = "calendars",
        conflicts_with = "source_id"
    )]
    pub calendar_source: Option<String>,

    /// Exact EventKit source id that qualifies every --calendar title.
    #[arg(long, requires = "calendars")]
    pub source_id: Option<String>,
}

#[derive(Debug, Args)]
pub struct WriteCalendarSelectorArgs {
    /// Calendar title. Add defaults to EventKit's default; update keeps the current calendar.
    #[arg(short, long)]
    pub calendar: Option<String>,

    /// Exact EventKit calendar id.
    #[arg(
        long = "calendar-id",
        conflicts_with_all = ["calendar", "calendar_source", "source_id"]
    )]
    pub calendar_id: Option<String>,

    /// Source title that qualifies --calendar.
    #[arg(
        long = "calendar-source",
        requires = "calendar",
        conflicts_with = "source_id"
    )]
    pub calendar_source: Option<String>,

    /// Exact EventKit source id that qualifies --calendar.
    #[arg(long, requires = "calendar")]
    pub source_id: Option<String>,
}

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
    /// Reminder list title. Add defaults to EventKit's default; update keeps the current list.
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

#[derive(Clone, Debug, Default, Args)]
pub struct EventRecurrenceArgs {
    /// Recurrence frequency for a newly created event.
    #[arg(long = "repeat", value_enum)]
    pub repeat: Option<EventRepeatArg>,

    /// Positive recurrence interval; defaults to 1.
    #[arg(long = "repeat-interval", requires = "repeat")]
    pub interval: Option<usize>,

    /// Weekday included by a weekly, monthly, or yearly rule. Repeatable.
    #[arg(long = "repeat-weekday", value_enum, requires = "repeat")]
    pub weekdays: Vec<EventWeekdayArg>,

    /// Month day from 1 through 31, or -1 through -31 from the end. Repeatable.
    #[arg(long = "repeat-month-day", requires = "repeat")]
    pub month_days: Vec<i32>,

    /// Stop after this positive number of occurrences.
    #[arg(long = "repeat-count", requires = "repeat", conflicts_with = "until")]
    pub count: Option<usize>,

    /// Stop at this RFC3339 instant with an explicit UTC offset.
    #[arg(long = "repeat-until", value_name = "RFC3339", requires = "repeat")]
    pub until: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventJsonRecurrence {
    pub frequency: EventRepeatArg,
    pub interval: Option<usize>,
    #[serde(default)]
    pub weekdays: Vec<EventWeekdayArg>,
    #[serde(default)]
    pub month_days: Vec<i32>,
    pub count: Option<usize>,
    pub until: Option<String>,
}

impl From<EventJsonRecurrence> for EventRecurrenceArgs {
    fn from(value: EventJsonRecurrence) -> Self {
        Self {
            repeat: Some(value.frequency),
            interval: value.interval,
            weekdays: value.weekdays,
            month_days: value.month_days,
            count: value.count,
            until: value.until,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print EventKit Calendar authorization status.
    Status,

    /// Diagnose Calendar/Reminders permissions and launch-context problems.
    Doctor,

    /// List calendars available in Calendar.app.
    Calendars {
        /// Only include calendars from this exact source title.
        #[arg(long)]
        source: Option<String>,

        /// Only include calendars that allow event modifications.
        #[arg(long)]
        writable_only: bool,
    },

    /// Show the system default calendar for new events.
    DefaultCalendar,

    /// List events in a bounded date range.
    List {
        /// Start date or datetime. Examples: 2026-07-06, 2026-07-06T09:00.
        #[arg(long)]
        from: String,

        /// End date or datetime. Date-only values include the whole day.
        #[arg(long)]
        to: String,

        #[command(flatten)]
        calendar_selector: ReadCalendarSelectorArgs,
    },

    /// List today's events.
    Today {
        #[command(flatten)]
        calendar_selector: ReadCalendarSelectorArgs,
    },

    /// List upcoming events from now through N days from now.
    Upcoming {
        /// Number of days to include.
        #[arg(long, default_value_t = 7)]
        days: i64,

        #[command(flatten)]
        calendar_selector: ReadCalendarSelectorArgs,
    },

    /// Show one event by exact EventKit identifier or cached row number.
    Show {
        /// EventKit event identifier, or row number from the last event list.
        id: String,

        /// Exact occurrence start for a recurring EventKit identifier.
        #[arg(long, value_name = "RFC3339")]
        occurrence_start: Option<String>,
    },

    /// Search event title, notes, location, URL, and calendar name in a bounded range.
    Search {
        /// Case-insensitive search query.
        query: String,

        /// Start date or datetime. Examples: 2026-07-06, 2026-07-06T09:00.
        #[arg(long)]
        from: String,

        /// End date or datetime. Date-only values include the whole day.
        #[arg(long)]
        to: String,

        #[command(flatten)]
        calendar_selector: ReadCalendarSelectorArgs,
    },

    /// Create a calendar event.
    Add {
        /// Event title.
        #[arg(required_unless_present = "json_file")]
        title: Option<String>,

        /// Start date or datetime. Examples: 2026-07-06, 2026-07-06T09:00.
        #[arg(long, required_unless_present = "json_file")]
        start: Option<String>,

        /// End date or datetime. Date-only values include the whole day.
        #[arg(long, required_unless_present = "json_file")]
        end: Option<String>,

        #[command(flatten)]
        calendar_selector: WriteCalendarSelectorArgs,

        /// Event notes.
        #[arg(long, conflicts_with = "notes_file")]
        notes: Option<String>,

        /// Read exact event notes from a UTF-8 file, or stdin with -.
        #[arg(long, value_name = "PATH")]
        notes_file: Option<PathBuf>,

        /// Read the complete event draft from a JSON file.
        #[arg(long, value_name = "PATH")]
        json_file: Option<PathBuf>,

        /// Event location.
        #[arg(long)]
        location: Option<String>,

        /// Event URL.
        #[arg(long)]
        url: Option<String>,

        /// Mark the event as all-day.
        #[arg(long)]
        all_day: bool,

        /// Event availability.
        #[arg(long, value_enum)]
        availability: Option<AvailabilityArg>,

        /// IANA zone for timezone-less inputs and EventKit storage, for example Europe/Berlin.
        #[arg(long = "time-zone", value_name = "TZID")]
        time_zone: Option<String>,

        /// Add a display alarm N minutes before the event. Can be passed more than once.
        #[arg(long = "alarm-minutes-before", value_name = "MINUTES")]
        alarm_minutes_before: Vec<i64>,

        #[command(flatten)]
        recurrence: EventRecurrenceArgs,

        /// Behavior when a matching event already exists.
        #[arg(long = "if-exists", value_enum, default_value_t = IfExistsArg::Error)]
        if_exists: IfExistsArg,

        /// Start/end tolerance in seconds for duplicate matching.
        #[arg(long, default_value_t = 0)]
        duplicate_window_seconds: i64,

        /// Validate and print the resolved event draft without writing to Calendar.
        #[arg(long)]
        dry_run: bool,
    },

    /// Update a calendar event by exact EventKit identifier or cached row number.
    Update {
        /// EventKit event identifier, or row number from the last event list.
        id: String,

        /// Exact occurrence start for a recurring EventKit identifier.
        #[arg(long, value_name = "RFC3339")]
        occurrence_start: Option<String>,

        /// Required mutation scope when the selected event is recurring.
        #[arg(long, value_enum)]
        scope: Option<EventScopeArg>,

        /// New event title.
        #[arg(long)]
        title: Option<String>,

        /// New start date or datetime.
        #[arg(long)]
        start: Option<String>,

        /// New end date or datetime. Date-only values include the whole day.
        #[arg(long)]
        end: Option<String>,

        #[command(flatten)]
        calendar_selector: WriteCalendarSelectorArgs,

        /// Replace event notes.
        #[arg(long, conflicts_with = "clear_notes")]
        notes: Option<String>,

        /// Clear event notes.
        #[arg(long)]
        clear_notes: bool,

        /// Replace event location.
        #[arg(long, conflicts_with = "clear_location")]
        location: Option<String>,

        /// Clear event location.
        #[arg(long)]
        clear_location: bool,

        /// Replace event URL.
        #[arg(long, conflicts_with = "clear_url")]
        url: Option<String>,

        /// Clear event URL.
        #[arg(long)]
        clear_url: bool,

        /// Mark the event as all-day.
        #[arg(long, conflicts_with = "timed")]
        all_day: bool,

        /// Mark the event as timed.
        #[arg(long)]
        timed: bool,

        /// Event availability.
        #[arg(long, value_enum)]
        availability: Option<AvailabilityArg>,

        /// IANA zone for timezone-less updated times and EventKit storage.
        #[arg(
            long = "time-zone",
            value_name = "TZID",
            conflicts_with = "clear_time_zone"
        )]
        time_zone: Option<String>,

        /// Clear the event's stored time zone.
        #[arg(long)]
        clear_time_zone: bool,

        /// Add a display alarm N minutes before the event. Can be passed more than once.
        #[arg(long = "add-alarm-minutes-before", value_name = "MINUTES")]
        add_alarm_minutes_before: Vec<i64>,

        /// Validate and print the resulting event draft without writing to Calendar.
        #[arg(long)]
        dry_run: bool,
    },

    /// Safely create or reconcile multiple events from a JSON file.
    Batch {
        #[command(subcommand)]
        command: BatchCommand,
    },

    /// Deterministic travel convenience helpers.
    Travel {
        #[command(subcommand)]
        command: TravelCommand,
    },

    /// Read local Apple Reminders through EventKit.
    Reminders {
        #[command(subcommand)]
        command: RemindersCommand,
    },

    /// Delete a calendar event by exact EventKit identifier or cached row number.
    Delete {
        /// EventKit event identifier, or row number from the last event list.
        id: String,

        /// Exact occurrence start for a recurring EventKit identifier.
        #[arg(long, value_name = "RFC3339")]
        occurrence_start: Option<String>,

        /// Required mutation scope when the selected event is recurring.
        #[arg(long, value_enum)]
        scope: Option<EventScopeArg>,

        /// Delete without an interactive confirmation prompt.
        #[arg(long)]
        force: bool,
    },

    /// Print a shell completion script to stdout.
    Completions {
        /// Shell to generate completions for.
        shell: Shell,
    },
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

#[derive(Debug, Subcommand)]
pub enum BatchCommand {
    /// Create events from a versioned JSON batch file.
    Add {
        /// Path to the JSON batch file.
        #[arg(long)]
        file: PathBuf,

        /// Behavior when an exact matching event already exists.
        #[arg(long = "if-exists", value_enum, default_value_t = IfExistsArg::Error)]
        if_exists: IfExistsArg,

        /// Validate and report every planned action without writing to Calendar.
        #[arg(long)]
        dry_run: bool,

        /// Process valid items and continue after individual failures.
        #[arg(long)]
        continue_on_error: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum TravelCommand {
    /// Format one flight leg and route it through normal event creation.
    Flight {
        /// Flight number, normalized to uppercase.
        flight_number: String,

        /// Departure airport code (3-4 ASCII letters).
        #[arg(long = "from")]
        from_airport: String,

        /// Arrival airport code (3-4 ASCII letters).
        #[arg(long = "to")]
        to_airport: String,

        /// RFC3339 departure timestamp with an explicit UTC offset.
        #[arg(long)]
        departure: String,

        /// RFC3339 arrival timestamp with an explicit UTC offset.
        #[arg(long)]
        arrival: String,

        #[command(flatten)]
        calendar_selector: WriteCalendarSelectorArgs,

        /// Extra notes appended after the generated flight details.
        #[arg(long, conflicts_with = "notes_file")]
        notes: Option<String>,

        /// Read exact extra notes from a UTF-8 file, or stdin with -.
        #[arg(long, value_name = "PATH")]
        notes_file: Option<PathBuf>,

        /// Event URL.
        #[arg(long)]
        url: Option<String>,

        /// Event availability.
        #[arg(long, value_enum, default_value = "busy")]
        availability: AvailabilityArg,

        /// Add a display alarm N minutes before the flight. Repeatable.
        #[arg(long = "alarm-minutes-before", value_name = "MINUTES")]
        alarm_minutes_before: Vec<i64>,

        /// Behavior when a matching flight event already exists.
        #[arg(long = "if-exists", value_enum, default_value_t = IfExistsArg::Error)]
        if_exists: IfExistsArg,

        /// Start/end tolerance in seconds for duplicate matching.
        #[arg(long, default_value_t = 0)]
        duplicate_window_seconds: i64,

        /// Validate and print the resolved event without writing to Calendar.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityArg {
    Busy,
    Free,
    Tentative,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum IfExistsArg {
    Skip,
    Update,
    #[default]
    Error,
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
pub enum EventRepeatArg {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum EventWeekdayArg {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum EventScopeArg {
    Occurrence,
    Future,
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

pub fn print_completions(shell: Shell) {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    generate(shell, &mut command, name, &mut io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_qualifier_requires_calendar_title() {
        let result = Cli::try_parse_from(["icalctl", "today", "--calendar-source", "iCloud"]);

        assert!(result.is_err());
    }

    #[test]
    fn default_calendar_command_parses() {
        let cli = Cli::try_parse_from(["icalctl", "default-calendar"]).unwrap();

        assert!(matches!(cli.command, Command::DefaultCalendar));
    }

    #[test]
    fn doctor_command_parses() {
        let cli = Cli::try_parse_from(["icalctl", "doctor"]).unwrap();

        assert!(matches!(cli.command, Command::Doctor));
    }

    #[test]
    fn calendars_accepts_source_and_writable_filters() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "calendars",
            "--source",
            "iCloud",
            "--writable-only",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Calendars {
                source: Some(source),
                writable_only: true,
            } if source == "iCloud"
        ));
    }

    #[test]
    fn read_commands_accept_multiple_exact_calendar_ids() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "list",
            "--from",
            "2026-07-10",
            "--to",
            "2026-07-10",
            "--calendar-id",
            "A",
            "--calendar-id",
            "B",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::List {
                calendar_selector: ReadCalendarSelectorArgs { calendar_ids, .. },
                ..
            } if calendar_ids == ["A", "B"]
        ));
    }

    #[test]
    fn write_calendar_id_conflicts_with_title() {
        let result = Cli::try_parse_from([
            "icalctl",
            "add",
            "Meeting",
            "--start",
            "2026-07-10T09:00",
            "--end",
            "2026-07-10T10:00",
            "--calendar",
            "Calendar",
            "--calendar-id",
            "ABC",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn add_and_update_accept_dry_run() {
        let add = Cli::try_parse_from([
            "icalctl",
            "add",
            "Meeting",
            "--start",
            "2026-07-10T09:00",
            "--end",
            "2026-07-10T10:00",
            "--dry-run",
        ]);
        let update = Cli::try_parse_from([
            "icalctl",
            "update",
            "EVENT-ID",
            "--title",
            "Meeting",
            "--dry-run",
        ]);

        assert!(add.is_ok());
        assert!(update.is_ok());
    }

    #[test]
    fn add_accepts_duplicate_policy_and_window() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "add",
            "Meeting",
            "--start",
            "2026-07-10T09:00",
            "--end",
            "2026-07-10T10:00",
            "--if-exists",
            "skip",
            "--duplicate-window-seconds",
            "30",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Add {
                if_exists: IfExistsArg::Skip,
                duplicate_window_seconds: 30,
                ..
            }
        ));
    }

    #[test]
    fn add_accepts_notes_file_or_complete_json_file() {
        let notes = Cli::try_parse_from([
            "icalctl",
            "add",
            "Meeting",
            "--start",
            "2026-07-10T09:00",
            "--end",
            "2026-07-10T10:00",
            "--notes-file",
            "notes.txt",
        ]);
        let json =
            Cli::try_parse_from(["icalctl", "add", "--json-file", "event.json", "--dry-run"]);

        assert!(notes.is_ok());
        assert!(json.is_ok());
    }

    #[test]
    fn add_rejects_notes_and_notes_file_together() {
        let result = Cli::try_parse_from([
            "icalctl",
            "add",
            "Meeting",
            "--start",
            "2026-07-10T09:00",
            "--end",
            "2026-07-10T10:00",
            "--notes",
            "inline",
            "--notes-file",
            "notes.txt",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn add_duplicate_policy_defaults_to_error_and_exact_times() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "add",
            "Meeting",
            "--start",
            "2026-07-10T09:00",
            "--end",
            "2026-07-10T10:00",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Add {
                if_exists: IfExistsArg::Error,
                duplicate_window_seconds: 0,
                ..
            }
        ));
    }

    #[test]
    fn add_and_update_accept_event_time_zone() {
        let add = Cli::try_parse_from([
            "icalctl",
            "add",
            "Flight",
            "--start",
            "2026-07-12T15:55:00+03:00",
            "--end",
            "2026-07-12T15:55:00+02:00",
            "--time-zone",
            "Europe/Berlin",
        ]);
        let update = Cli::try_parse_from(["icalctl", "update", "EVENT-ID", "--clear-time-zone"]);

        assert!(add.is_ok());
        assert!(update.is_ok());
    }

    #[test]
    fn update_rejects_set_and_clear_time_zone_together() {
        let result = Cli::try_parse_from([
            "icalctl",
            "update",
            "EVENT-ID",
            "--time-zone",
            "Europe/Berlin",
            "--clear-time-zone",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn batch_add_parses_safety_flags() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "batch",
            "add",
            "--file",
            "events.json",
            "--if-exists",
            "skip",
            "--dry-run",
            "--continue-on-error",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Batch {
                command: BatchCommand::Add {
                    if_exists: IfExistsArg::Skip,
                    dry_run: true,
                    continue_on_error: true,
                    ..
                }
            }
        ));
    }

    #[test]
    fn travel_flight_forwards_add_options() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "travel",
            "flight",
            "ho1607",
            "--from",
            "pvg",
            "--to",
            "hel",
            "--departure",
            "2026-07-11T09:25:00+08:00",
            "--arrival",
            "2026-07-11T14:00:00+03:00",
            "--calendar",
            "Travel",
            "--calendar-source",
            "iCloud",
            "--notes",
            "Booking confirmed",
            "--url",
            "https://example.com/flight",
            "--availability",
            "free",
            "--alarm-minutes-before",
            "30",
            "--if-exists",
            "skip",
            "--duplicate-window-seconds",
            "60",
            "--dry-run",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Travel {
                command: TravelCommand::Flight {
                    flight_number,
                    from_airport,
                    to_airport,
                    calendar_selector: WriteCalendarSelectorArgs {
                        calendar: Some(calendar),
                        calendar_source: Some(source),
                        ..
                    },
                    notes: Some(notes),
                    url: Some(url),
                    availability: AvailabilityArg::Free,
                    alarm_minutes_before,
                    if_exists: IfExistsArg::Skip,
                    duplicate_window_seconds: 60,
                    dry_run: true,
                    ..
                }
            } if flight_number == "ho1607"
                && from_airport == "pvg"
                && to_airport == "hel"
                && calendar == "Travel"
                && source == "iCloud"
                && notes == "Booking confirmed"
                && url == "https://example.com/flight"
                && alarm_minutes_before == [30]
        ));
    }

    #[test]
    fn travel_flight_defaults_to_busy_with_no_alarms() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "travel",
            "flight",
            "HO1607",
            "--from",
            "PVG",
            "--to",
            "HEL",
            "--departure",
            "2026-07-11T09:25:00+08:00",
            "--arrival",
            "2026-07-11T14:00:00+03:00",
            "--notes-file",
            "flight-notes.txt",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Travel {
                command: TravelCommand::Flight {
                    availability: AvailabilityArg::Busy,
                    alarm_minutes_before,
                    notes_file: Some(notes_file),
                    ..
                }
            } if alarm_minutes_before.is_empty()
                && notes_file.to_str() == Some("flight-notes.txt")
        ));
    }

    #[test]
    fn reminder_status_lists_default_list_and_show_parse() {
        assert!(matches!(
            Cli::try_parse_from(["icalctl", "reminders", "status"])
                .unwrap()
                .command,
            Command::Reminders {
                command: RemindersCommand::Status
            }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "lists",
                "--source",
                "iCloud",
                "--writable-only"
            ])
            .unwrap()
            .command,
            Command::Reminders {
                command: RemindersCommand::Lists {
                    source: Some(source),
                    writable_only: true,
                }
            } if source == "iCloud"
        ));
        assert!(matches!(
            Cli::try_parse_from(["icalctl", "reminders", "default-list"])
                .unwrap()
                .command,
            Command::Reminders {
                command: RemindersCommand::DefaultList
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["icalctl", "reminders", "show", "2"])
                .unwrap()
                .command,
            Command::Reminders {
                command: RemindersCommand::Show { id }
            } if id == "2"
        ));
    }

    #[test]
    fn reminder_list_defaults_to_incomplete_and_accepts_repeated_selectors() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "list",
            "--list-id",
            "A",
            "--list-id",
            "B",
            "--due-from",
            "2026-07-10",
            "--due-to",
            "2026-07-15",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::List {
                    filters: ReminderReadFilterArgs {
                        state: ReminderStateArg::Incomplete,
                        list_selector: ReadReminderListSelectorArgs { list_ids, .. },
                        due_from: Some(from),
                        due_to: Some(to),
                    }
                }
            } if list_ids == ["A", "B"] && from == "2026-07-10" && to == "2026-07-15"
        ));
    }

    #[test]
    fn reminder_search_accepts_completed_and_source_qualified_title() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "search",
            "report",
            "--list",
            "Work",
            "--list-source",
            "Exchange",
            "--state",
            "completed",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Search {
                    query,
                    filters: ReminderReadFilterArgs {
                        state: ReminderStateArg::Completed,
                        list_selector: ReadReminderListSelectorArgs {
                            lists,
                            list_source: Some(source),
                            ..
                        },
                        ..
                    }
                }
            } if query == "report" && lists == ["Work"] && source == "Exchange"
        ));
    }

    #[test]
    fn reminder_source_qualifier_requires_list_title() {
        let result =
            Cli::try_parse_from(["icalctl", "reminders", "list", "--list-source", "iCloud"]);

        assert!(result.is_err());
    }

    #[test]
    fn reminder_add_forwards_safe_creation_options() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "Submit report",
            "--list-id",
            "LIST-1",
            "--due",
            "2026-07-15T14:30",
            "--start",
            "2026-07-15",
            "--time-zone",
            "Europe/Helsinki",
            "--notes",
            "Final version",
            "--url",
            "https://example.com/report",
            "--location",
            "Office",
            "--priority",
            "high",
            "--notify-at-due",
            "--notify-minutes-before",
            "30",
            "--if-exists",
            "skip",
            "--duplicate-window-seconds",
            "30",
            "--dry-run",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Add {
                    title: Some(title),
                    list_selector: WriteReminderListSelectorArgs {
                        list_id: Some(list_id),
                        ..
                    },
                    due: Some(due),
                    start: Some(start),
                    time_zone: Some(time_zone),
                    notes: Some(notes),
                    url: Some(url),
                    location: Some(location),
                    priority: Some(ReminderPriorityArg::High),
                    notify_at_due: true,
                    notify_minutes_before,
                    if_exists: IfExistsArg::Skip,
                    duplicate_window_seconds: 30,
                    dry_run: true,
                    ..
                }
            } if title == "Submit report"
                && list_id == "LIST-1"
                && due == "2026-07-15T14:30"
                && start == "2026-07-15"
                && time_zone == "Europe/Helsinki"
                && notes == "Final version"
                && url == "https://example.com/report"
                && location == "Office"
                && notify_minutes_before == [30]
        ));
    }

    #[test]
    fn reminder_add_allows_undated_default_list_with_priority_enum() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "Call dentist",
            "--priority",
            "high",
            "--notes-file",
            "notes.txt",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Add {
                    list_selector: WriteReminderListSelectorArgs {
                        list: None,
                        list_id: None,
                        ..
                    },
                    due: None,
                    priority: Some(ReminderPriorityArg::High),
                    notes_file: Some(path),
                    ..
                }
            } if path.to_str() == Some("notes.txt")
        ));
    }

    #[test]
    fn reminder_add_rejects_conflicting_list_and_notes_inputs() {
        assert!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "add",
                "Task",
                "--list",
                "Tasks",
                "--list-id",
                "A",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "add",
                "Task",
                "--notes",
                "inline",
                "--notes-file",
                "notes.txt",
            ])
            .is_err()
        );
    }

    #[test]
    fn reminder_update_parses_patch_clear_move_and_dry_run_flags() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "update",
            "2",
            "--title",
            "Revised task",
            "--list-id",
            "LIST-2",
            "--due",
            "2026-07-15T14:30",
            "--time-zone",
            "Europe/Helsinki",
            "--clear-start",
            "--clear-notes",
            "--clear-url",
            "--location",
            "Office",
            "--priority",
            "none",
            "--dry-run",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Update {
                    id,
                    title: Some(title),
                    list_selector: WriteReminderListSelectorArgs {
                        list_id: Some(list_id),
                        ..
                    },
                    due: Some(due),
                    time_zone: Some(time_zone),
                    clear_start: true,
                    clear_notes: true,
                    clear_url: true,
                    location: Some(location),
                    priority: Some(ReminderPriorityArg::None),
                    dry_run: true,
                    ..
                }
            } if id == "2"
                && title == "Revised task"
                && list_id == "LIST-2"
                && due == "2026-07-15T14:30"
                && time_zone == "Europe/Helsinki"
                && location == "Office"
        ));
    }

    #[test]
    fn reminder_lifecycle_commands_parse() {
        assert!(matches!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "complete",
                "ID",
                "--completed-at",
                "2026-07-11T14:00:00+03:00",
                "--dry-run",
            ])
            .unwrap()
            .command,
            Command::Reminders {
                command: RemindersCommand::Complete {
                    id,
                    completed_at: Some(completed_at),
                    dry_run: true,
                }
            } if id == "ID" && completed_at == "2026-07-11T14:00:00+03:00"
        ));
        assert!(matches!(
            Cli::try_parse_from(["icalctl", "reminders", "uncomplete", "ID", "--dry-run"])
                .unwrap()
                .command,
            Command::Reminders {
                command: RemindersCommand::Uncomplete { id, dry_run: true }
            } if id == "ID"
        ));
        assert!(matches!(
            Cli::try_parse_from(["icalctl", "reminders", "delete", "ID", "--force"])
                .unwrap()
                .command,
            Command::Reminders {
                command: RemindersCommand::Delete { id, force: true }
            } if id == "ID"
        ));
    }

    #[test]
    fn reminder_update_rejects_conflicting_set_and_clear_flags() {
        assert!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "update",
                "ID",
                "--due",
                "2026-07-15",
                "--clear-due",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "update",
                "ID",
                "--notes",
                "text",
                "--clear-notes",
            ])
            .is_err()
        );
    }

    #[test]
    fn reminder_add_parses_absolute_geofence_and_recurrence_options() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "Visit office",
            "--due",
            "2026-07-15T09:00:00+03:00",
            "--notify-at",
            "2026-07-15T07:00:00+03:00",
            "--geofence-title",
            "Office",
            "--geofence-latitude",
            "60.1699",
            "--geofence-longitude",
            "24.9384",
            "--geofence-radius-meters",
            "150",
            "--geofence-proximity",
            "arrive",
            "--repeat",
            "weekly",
            "--repeat-interval",
            "2",
            "--repeat-count",
            "6",
            "--dry-run",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Add {
                    schedule: ReminderAdvancedScheduleArgs {
                        notify_at,
                        geofence_title: Some(title),
                        geofence_proximity: Some(ReminderGeofenceProximityArg::Arrive),
                        repeat: Some(ReminderRepeatArg::Weekly),
                        repeat_interval: Some(2),
                        repeat_count: Some(6),
                        ..
                    },
                    dry_run: true,
                    ..
                }
            } if notify_at == ["2026-07-15T07:00:00+03:00"] && title == "Office"
        ));
    }

    #[test]
    fn reminder_update_parses_schedule_clear_flags_and_rejects_partial_geofence() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "update",
            "ID",
            "--clear-notifications",
            "--clear-recurrence",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Update {
                    clear_notifications: true,
                    clear_recurrence: true,
                    dry_run: true,
                    ..
                }
            }
        ));
        assert!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "add",
                "Task",
                "--geofence-title",
                "Office",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "icalctl",
                "reminders",
                "update",
                "ID",
                "--clear-notifications",
                "--notify-at",
                "2026-07-15T09:00:00+03:00",
            ])
            .is_err()
        );
    }

    #[test]
    fn reminder_add_accepts_json_file_without_positional_title() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "--json-file",
            "reminder.json",
            "--if-exists",
            "skip",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Add {
                    title: None,
                    json_file: Some(path),
                    if_exists: IfExistsArg::Skip,
                    dry_run: true,
                    ..
                }
            } if path.to_str() == Some("reminder.json")
        ));
    }

    #[test]
    fn reminder_batch_add_parses_safety_flags() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "reminders",
            "batch",
            "add",
            "--file",
            "reminders.json",
            "--if-exists",
            "update",
            "--dry-run",
            "--continue-on-error",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Reminders {
                command: RemindersCommand::Batch {
                    command: ReminderBatchCommand::Add {
                        file,
                        if_exists: IfExistsArg::Update,
                        dry_run: true,
                        continue_on_error: true,
                    }
                }
            } if file.to_str() == Some("reminders.json")
        ));
    }

    #[test]
    fn event_show_accepts_an_exact_occurrence_start() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "show",
            "SERIES-ID",
            "--occurrence-start",
            "2026-07-20T09:00:00+03:00",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Show {
                id,
                occurrence_start: Some(start),
            } if id == "SERIES-ID" && start == "2026-07-20T09:00:00+03:00"
        ));
    }

    #[test]
    fn event_update_parses_recurring_occurrence_scope() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "update",
            "SERIES-ID",
            "--occurrence-start",
            "2026-07-20T09:00:00+03:00",
            "--scope",
            "future",
            "--title",
            "Moved standup",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Update {
                id,
                occurrence_start: Some(start),
                scope: Some(EventScopeArg::Future),
                ..
            } if id == "SERIES-ID" && start == "2026-07-20T09:00:00+03:00"
        ));
    }

    #[test]
    fn event_delete_parses_recurring_occurrence_scope() {
        let cli =
            Cli::try_parse_from(["icalctl", "delete", "3", "--scope", "occurrence", "--force"])
                .unwrap();
        assert!(matches!(
            cli.command,
            Command::Delete {
                id,
                occurrence_start: None,
                scope: Some(EventScopeArg::Occurrence),
                force: true,
            } if id == "3"
        ));
    }

    #[test]
    fn event_add_parses_recurrence_creation_flags() {
        let cli = Cli::try_parse_from([
            "icalctl",
            "add",
            "Standup",
            "--start",
            "2026-07-20T09:00:00+03:00",
            "--end",
            "2026-07-20T09:30:00+03:00",
            "--repeat",
            "weekly",
            "--repeat-interval",
            "2",
            "--repeat-weekday",
            "monday",
            "--repeat-weekday",
            "wednesday",
            "--repeat-count",
            "8",
            "--dry-run",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::Add {
                recurrence: EventRecurrenceArgs {
                    repeat: Some(EventRepeatArg::Weekly),
                    interval: Some(2),
                    weekdays,
                    count: Some(8),
                    ..
                },
                dry_run: true,
                ..
            } if weekdays == [EventWeekdayArg::Monday, EventWeekdayArg::Wednesday]
        ));
    }
}
