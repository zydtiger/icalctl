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
}
