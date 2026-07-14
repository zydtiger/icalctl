mod calendar;
mod config;
mod reminders;
mod shared;
#[cfg(test)]
mod tests;
mod travel;

pub(crate) use self::calendar::{
    BatchCommand, EventJsonRecurrence, EventRecurrenceArgs, ReadCalendarSelectorArgs,
    WriteCalendarSelectorArgs,
};
pub(crate) use self::config::ConfigCommand;
pub(crate) use self::reminders::{
    ReadReminderListSelectorArgs, ReminderAdvancedScheduleArgs, ReminderBatchCommand,
    ReminderGeofenceProximityArg, ReminderPriorityArg, ReminderReadFilterArgs, ReminderRepeatArg,
    ReminderStateArg, RemindersCommand, WriteReminderListSelectorArgs,
};
pub(crate) use self::shared::{
    AvailabilityArg, EventRepeatArg, EventScopeArg, EventWeekdayArg, IfExistsArg,
};
pub(crate) use self::travel::TravelCommand;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "icalctl",
    about = "Manage local macOS Apple Calendar and Reminders data through EventKit",
    version
)]
pub struct Cli {
    /// Print compact JSON instead of human-friendly text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect and change persistent icalctl configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },

    /// Print semantic version and build provenance.
    Version,

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

pub fn print_completions(shell: Shell) {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    generate(shell, &mut command, name, &mut io::stdout());
}
