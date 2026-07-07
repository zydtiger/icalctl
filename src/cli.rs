use clap::{Parser, Subcommand, ValueEnum};

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

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print EventKit Calendar authorization status.
    Status,

    /// List calendars available in Calendar.app.
    Calendars,

    /// List events in a bounded date range.
    List {
        /// Start date or datetime. Examples: 2026-07-06, 2026-07-06T09:00.
        #[arg(long)]
        from: String,

        /// End date or datetime. Date-only values include the whole day.
        #[arg(long)]
        to: String,

        /// Calendar title to include. Can be passed more than once.
        #[arg(short, long = "calendar")]
        calendars: Vec<String>,
    },

    /// List today's events.
    Today {
        /// Calendar title to include. Can be passed more than once.
        #[arg(short, long = "calendar")]
        calendars: Vec<String>,
    },

    /// List upcoming events from now through N days from now.
    Upcoming {
        /// Number of days to include.
        #[arg(long, default_value_t = 7)]
        days: i64,

        /// Calendar title to include. Can be passed more than once.
        #[arg(short, long = "calendar")]
        calendars: Vec<String>,
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

        /// Calendar title to include. Can be passed more than once.
        #[arg(short, long = "calendar")]
        calendars: Vec<String>,
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

        /// Calendar title. Defaults to the system default calendar for new events.
        #[arg(short, long)]
        calendar: Option<String>,

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

        /// Add a display alarm N minutes before the event. Can be passed more than once.
        #[arg(long = "alarm-minutes-before", value_name = "MINUTES")]
        alarm_minutes_before: Vec<i64>,
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

        /// Move the event to another calendar by title.
        #[arg(short, long)]
        calendar: Option<String>,

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

        /// Add a display alarm N minutes before the event. Can be passed more than once.
        #[arg(long = "add-alarm-minutes-before", value_name = "MINUTES")]
        add_alarm_minutes_before: Vec<i64>,
    },

    /// Delete a calendar event by exact EventKit identifier or cached row number.
    Delete {
        /// EventKit event identifier, or row number from the last event list.
        id: String,

        /// Delete without an interactive confirmation prompt.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum AvailabilityArg {
    Busy,
    Free,
    Tentative,
    Unavailable,
}
