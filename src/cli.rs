use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "icalctl",
    about = "Read local macOS Apple Calendar data through EventKit"
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

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print EventKit Calendar authorization status.
    Status,

    /// Diagnose Calendar permission and launch-context problems.
    Doctor,

    /// List calendars available in Calendar.app.
    Calendars,

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
        title: String,

        /// Start date or datetime. Examples: 2026-07-06, 2026-07-06T09:00.
        #[arg(long)]
        start: String,

        /// End date or datetime. Date-only values include the whole day.
        #[arg(long)]
        end: String,

        #[command(flatten)]
        calendar_selector: WriteCalendarSelectorArgs,

        /// Event notes.
        #[arg(long)]
        notes: Option<String>,

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

        /// Validate and print the resolved event draft without writing to Calendar.
        #[arg(long)]
        dry_run: bool,
    },

    /// Update a calendar event by exact EventKit identifier or cached row number.
    Update {
        /// EventKit event identifier, or row number from the last event list.
        id: String,

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

    /// Delete a calendar event by exact EventKit identifier or cached row number.
    Delete {
        /// EventKit event identifier, or row number from the last event list.
        id: String,

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
}
