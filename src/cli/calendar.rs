use super::{EventRepeatArg, EventWeekdayArg, IfExistsArg};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    /// Calendar title. Add uses the configured or EventKit default; update keeps the current calendar.
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
