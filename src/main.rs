#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Days, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone};
use clap::{Parser, Subcommand};
use eventkit::{AuthorizationStatus, CalendarInfo, EventItem, EventsManager, ParticipantInfo};
use serde::Serialize;
use std::io;

#[derive(Debug, Parser)]
#[command(
    name = "icalctl",
    about = "Read local macOS Apple Calendar data through EventKit"
)]
struct Cli {
    /// Print compact JSON instead of human-friendly text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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

    /// Show one event by exact EventKit identifier.
    Show {
        /// EventKit event identifier.
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
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum JsonOutput {
    Status(StatusReport),
    Calendars { calendars: Vec<CalendarReport> },
    Events { events: Vec<EventReport> },
    Event { event: Box<EventReport> },
}

#[derive(Debug, Serialize)]
struct StatusReport {
    authorization: String,
}

#[derive(Debug, Serialize)]
struct CalendarReport {
    id: String,
    title: String,
    source: Option<String>,
    source_id: Option<String>,
    calendar_type: String,
    allows_modifications: bool,
    is_immutable: bool,
    is_subscribed: bool,
    color_rgba: Option<(f64, f64, f64, f64)>,
    allowed_entity_types: Vec<String>,
    supported_event_availabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EventReport {
    id: String,
    title: String,
    start: String,
    end: String,
    all_day: bool,
    calendar: Option<String>,
    calendar_id: Option<String>,
    location: Option<String>,
    notes: Option<String>,
    url: Option<String>,
    status: String,
    availability: String,
    is_detached: bool,
    occurrence_date: Option<String>,
    creation_date: Option<String>,
    last_modified_date: Option<String>,
    external_identifier: Option<String>,
    timezone: Option<String>,
    attachments_count: usize,
    attendees: Vec<ParticipantReport>,
    organizer: Option<ParticipantReport>,
}

#[derive(Debug, Serialize)]
struct ParticipantReport {
    name: Option<String>,
    url: Option<String>,
    role: String,
    status: String,
    is_current_user: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let output = run(cli.command)?;

    if cli.json {
        serde_json::to_writer(io::stdout(), &output).context("failed to write JSON output")?;
        println!();
    } else {
        print_human_output(&output);
    }

    Ok(())
}

fn run(command: Command) -> Result<JsonOutput> {
    match command {
        Command::Status => Ok(JsonOutput::Status(StatusReport {
            authorization: authorization_string(),
        })),
        Command::Calendars => {
            let events = authorized_events_manager()?;
            let calendars = events
                .list_calendars()
                .context("failed to list Calendar calendars through EventKit")?
                .iter()
                .map(CalendarReport::from)
                .collect();
            Ok(JsonOutput::Calendars { calendars })
        }
        Command::List {
            from,
            to,
            calendars,
        } => {
            let events = fetch_range(&from, &to, &calendars)
                .with_context(|| format!("failed to list events from {from} to {to}"))?;
            Ok(JsonOutput::Events { events })
        }
        Command::Today { calendars } => {
            let (start, end) = today_range()?;
            let events = fetch_events(start, end, &calendars).context("failed to list today")?;
            Ok(JsonOutput::Events { events })
        }
        Command::Upcoming { days, calendars } => {
            if days <= 0 {
                bail!("--days must be greater than zero");
            }
            let start = Local::now();
            let end = start + chrono::Duration::days(days);
            let events =
                fetch_events(start, end, &calendars).context("failed to list upcoming events")?;
            Ok(JsonOutput::Events { events })
        }
        Command::Show { id } => {
            let events = authorized_events_manager()?;
            let event = events
                .get_event(&id)
                .with_context(|| format!("failed to show event {id}"))?;
            Ok(JsonOutput::Event {
                event: Box::new(EventReport::from(&event)),
            })
        }
        Command::Search {
            query,
            from,
            to,
            calendars,
        } => {
            let query = query.to_lowercase();
            let events = fetch_range(&from, &to, &calendars)
                .with_context(|| format!("failed to search events from {from} to {to}"))?
                .into_iter()
                .filter(|event| event_matches(event, &query))
                .collect();
            Ok(JsonOutput::Events { events })
        }
    }
}

fn authorized_events_manager() -> Result<EventsManager> {
    match EventsManager::authorization_status() {
        AuthorizationStatus::FullAccess => Ok(EventsManager::new()),
        AuthorizationStatus::NotDetermined => {
            let events = EventsManager::new();
            if events
                .request_access()
                .context("failed to request full Calendar access through EventKit")?
            {
                Ok(events)
            } else {
                bail!(
                    "Calendar access was not granted; run from Terminal/Ghostty and approve the macOS prompt"
                );
            }
        }
        AuthorizationStatus::WriteOnly => {
            bail!("Calendar access is write-only; full access is required for read commands")
        }
        AuthorizationStatus::Denied => {
            bail!("Calendar access is denied; enable Calendar full access in macOS Settings")
        }
        AuthorizationStatus::Restricted => {
            bail!("Calendar access is restricted by system policy")
        }
    }
}

fn fetch_range(from: &str, to: &str, calendars: &[String]) -> Result<Vec<EventReport>> {
    let start = parse_start_datetime(from).with_context(|| format!("invalid --from: {from}"))?;
    let end = parse_end_datetime(to).with_context(|| format!("invalid --to: {to}"))?;
    fetch_events(start, end, calendars)
}

fn fetch_events(
    start: DateTime<Local>,
    end: DateTime<Local>,
    calendars: &[String],
) -> Result<Vec<EventReport>> {
    if start >= end {
        bail!("--from must be before --to");
    }

    let events = authorized_events_manager()?;
    let calendar_refs: Vec<&str> = calendars.iter().map(String::as_str).collect();
    let selected_calendars = if calendar_refs.is_empty() {
        None
    } else {
        Some(calendar_refs.as_slice())
    };

    let events = events
        .fetch_events(start, end, selected_calendars)
        .context("failed to fetch events through EventKit")?
        .iter()
        .map(EventReport::from)
        .collect();

    Ok(events)
}

fn today_range() -> Result<(DateTime<Local>, DateTime<Local>)> {
    let today = Local::now().date_naive();
    let tomorrow = today
        .checked_add_days(Days::new(1))
        .ok_or_else(|| anyhow!("failed to calculate tomorrow"))?;
    Ok((local_start_of_day(today)?, local_start_of_day(tomorrow)?))
}

fn parse_start_datetime(input: &str) -> Result<DateTime<Local>> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return local_start_of_day(date);
    }

    parse_datetime(input)
}

fn parse_end_datetime(input: &str) -> Result<DateTime<Local>> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        let tomorrow = date
            .checked_add_days(Days::new(1))
            .ok_or_else(|| anyhow!("failed to calculate end date"))?;
        return local_start_of_day(tomorrow);
    }

    parse_datetime(input)
}

fn parse_datetime(input: &str) -> Result<DateTime<Local>> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(input) {
        return Ok(datetime.with_timezone(&Local));
    }

    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(datetime) = NaiveDateTime::parse_from_str(input, format) {
            return local_datetime(datetime);
        }
    }

    bail!("expected YYYY-MM-DD, YYYY-MM-DDTHH:MM, or RFC3339 datetime")
}

fn local_start_of_day(date: NaiveDate) -> Result<DateTime<Local>> {
    let datetime = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("invalid start of day"))?;
    local_datetime(datetime)
}

fn local_datetime(datetime: NaiveDateTime) -> Result<DateTime<Local>> {
    match Local.from_local_datetime(&datetime) {
        LocalResult::Single(datetime) => Ok(datetime),
        LocalResult::Ambiguous(first, _) => Ok(first),
        LocalResult::None => bail!("local datetime does not exist in the current timezone"),
    }
}

fn event_matches(event: &EventReport, query: &str) -> bool {
    contains_query(&event.title, query)
        || event
            .notes
            .as_deref()
            .is_some_and(|value| contains_query(value, query))
        || event
            .location
            .as_deref()
            .is_some_and(|value| contains_query(value, query))
        || event
            .url
            .as_deref()
            .is_some_and(|value| contains_query(value, query))
        || event
            .calendar
            .as_deref()
            .is_some_and(|value| contains_query(value, query))
}

fn contains_query(value: &str, query: &str) -> bool {
    value.to_lowercase().contains(query)
}

fn authorization_string() -> String {
    format!("{:?}", EventsManager::authorization_status())
}

fn print_human_output(output: &JsonOutput) {
    match output {
        JsonOutput::Status(status) => {
            println!("Calendar authorization: {}", status.authorization);
        }
        JsonOutput::Calendars { calendars } => {
            println!("Calendars ({})", calendars.len());
            for calendar in calendars {
                print_calendar(calendar);
            }
        }
        JsonOutput::Events { events } => print_events(events),
        JsonOutput::Event { event } => print_event_detail(event),
    }
}

fn print_calendar(calendar: &CalendarReport) {
    let source = calendar.source.as_deref().unwrap_or("unknown source");
    println!(
        "- {} [{}] source={} writable={} subscribed={} id={}",
        calendar.title,
        calendar.calendar_type,
        source,
        calendar.allows_modifications,
        calendar.is_subscribed,
        calendar.id
    );
}

fn print_events(events: &[EventReport]) {
    println!("Events ({})", events.len());
    if events.is_empty() {
        println!("- no events found");
        return;
    }

    for event in events {
        let calendar = event.calendar.as_deref().unwrap_or("unknown calendar");
        println!(
            "- {} {} ({}) [{}]",
            event_time_range(event),
            event.title,
            calendar,
            event.id
        );
    }
}

fn print_event_detail(event: &EventReport) {
    println!("{}", event.title);
    println!("id: {}", event.id);
    println!("time: {}", event_time_range(event));

    if let Some(calendar) = &event.calendar {
        println!("calendar: {calendar}");
    }
    if let Some(location) = &event.location {
        println!("location: {location}");
    }
    if let Some(url) = &event.url {
        println!("url: {url}");
    }
    if let Some(notes) = &event.notes {
        println!();
        println!("{notes}");
    }
}

fn event_time_range(event: &EventReport) -> String {
    if event.all_day {
        return format!("{} all-day", date_part(&event.start));
    }

    let start_date = date_part(&event.start);
    let end_date = date_part(&event.end);
    let start_time = time_part(&event.start);
    let end_time = time_part(&event.end);

    if start_date == end_date {
        format!("{start_date} {start_time}-{end_time}")
    } else {
        format!("{start_date} {start_time} to {end_date} {end_time}")
    }
}

fn date_part(value: &str) -> &str {
    value.get(0..10).unwrap_or(value)
}

fn time_part(value: &str) -> &str {
    value.get(11..16).unwrap_or(value)
}

impl From<&CalendarInfo> for CalendarReport {
    fn from(calendar: &CalendarInfo) -> Self {
        Self {
            id: calendar.identifier.clone(),
            title: calendar.title.clone(),
            source: calendar.source.clone(),
            source_id: calendar.source_id.clone(),
            calendar_type: format!("{:?}", calendar.calendar_type),
            allows_modifications: calendar.allows_modifications,
            is_immutable: calendar.is_immutable,
            is_subscribed: calendar.is_subscribed,
            color_rgba: calendar.color,
            allowed_entity_types: calendar.allowed_entity_types.clone(),
            supported_event_availabilities: calendar.supported_event_availabilities.clone(),
        }
    }
}

impl From<&EventItem> for EventReport {
    fn from(event: &EventItem) -> Self {
        Self {
            id: event.identifier.clone(),
            title: event.title.clone(),
            start: event.start_date.to_rfc3339(),
            end: event.end_date.to_rfc3339(),
            all_day: event.all_day,
            calendar: event.calendar_title.clone(),
            calendar_id: event.calendar_id.clone(),
            location: event.location.clone(),
            notes: event.notes.clone(),
            url: event.URL.clone(),
            status: format!("{:?}", event.status),
            availability: format!("{:?}", event.availability),
            is_detached: event.is_detached,
            occurrence_date: event.occurrence_date.map(|value| value.to_rfc3339()),
            creation_date: event.creation_date.map(|value| value.to_rfc3339()),
            last_modified_date: event.last_modified_date.map(|value| value.to_rfc3339()),
            external_identifier: event.external_identifier.clone(),
            timezone: event.timezone.clone(),
            attachments_count: event.attachments_count,
            attendees: event
                .attendees
                .iter()
                .map(ParticipantReport::from)
                .collect(),
            organizer: event.organizer.as_ref().map(ParticipantReport::from),
        }
    }
}

impl From<&ParticipantInfo> for ParticipantReport {
    fn from(participant: &ParticipantInfo) -> Self {
        Self {
            name: participant.name.clone(),
            url: participant.URL.clone(),
            role: format!("{:?}", participant.role),
            status: format!("{:?}", participant.status),
            is_current_user: participant.is_current_user,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_only_start_uses_start_of_day() {
        let parsed = parse_start_datetime("2026-07-06").unwrap();

        assert!(parsed.to_rfc3339().starts_with("2026-07-06T00:00:00"));
    }

    #[test]
    fn date_only_end_is_exclusive_next_day() {
        let parsed = parse_end_datetime("2026-07-06").unwrap();

        assert!(parsed.to_rfc3339().starts_with("2026-07-07T00:00:00"));
    }

    #[test]
    fn local_datetime_accepts_minute_precision() {
        let parsed = parse_start_datetime("2026-07-06T09:30").unwrap();

        assert!(parsed.to_rfc3339().starts_with("2026-07-06T09:30:00"));
    }
}
