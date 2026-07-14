use super::{AvailabilityArg, IfExistsArg, WriteCalendarSelectorArgs};
use clap::Subcommand;
use std::path::PathBuf;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum TravelCommand {
    /// Serve the read-only local travel visualization.
    Serve {
        /// Exact EventKit calendar id to include. Repeat to include multiple calendars.
        /// Configured travel.calendar_ids are used when this option is omitted.
        #[arg(long = "calendar-id")]
        calendar_ids: Vec<String>,
    },

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
