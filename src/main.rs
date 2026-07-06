#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

use anyhow::{Context, Result};
use clap::Parser;
use eventkit::{CalendarInfo, EventItem, EventsManager};
use serde::Serialize;
use std::io;

#[derive(Debug, Parser)]
#[command(
    name = "icalctl",
    about = "Phase 0 EventKit probe for local macOS Apple Calendar access"
)]
struct Cli {
    /// Print compact JSON instead of human-friendly text.
    #[arg(long)]
    json: bool,

    /// Print the current Calendar authorization status without requesting access.
    #[arg(long)]
    status_only: bool,
}

#[derive(Debug, Serialize)]
struct ProbeReport {
    authorization_before: String,
    authorization_after: Option<String>,
    access_granted: Option<bool>,
    calendars: Vec<CalendarReport>,
    today_events: Vec<EventReport>,
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
    url: Option<String>,
    status: String,
    availability: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let report = run_probe(cli.status_only)?;

    if cli.json {
        serde_json::to_writer(io::stdout(), &report).context("failed to write JSON output")?;
        println!();
    } else {
        print_human_report(&report);
    }

    Ok(())
}

fn run_probe(status_only: bool) -> Result<ProbeReport> {
    let authorization_before = format!("{:?}", EventsManager::authorization_status());

    if status_only {
        return Ok(ProbeReport {
            authorization_before,
            authorization_after: None,
            access_granted: None,
            calendars: Vec::new(),
            today_events: Vec::new(),
        });
    }

    let events = EventsManager::new();
    let access_granted = events
        .request_access()
        .context("failed to request full Calendar access through EventKit")?;
    let authorization_after = format!("{:?}", EventsManager::authorization_status());

    if !access_granted {
        return Ok(ProbeReport {
            authorization_before,
            authorization_after: Some(authorization_after),
            access_granted: Some(false),
            calendars: Vec::new(),
            today_events: Vec::new(),
        });
    }

    let calendars = events
        .list_calendars()
        .context("failed to list Calendar calendars through EventKit")?
        .iter()
        .map(CalendarReport::from)
        .collect();

    let today_events = events
        .fetch_today_events()
        .context("failed to fetch today's Calendar events through EventKit")?
        .iter()
        .map(EventReport::from)
        .collect();

    Ok(ProbeReport {
        authorization_before,
        authorization_after: Some(authorization_after),
        access_granted: Some(true),
        calendars,
        today_events,
    })
}

fn print_human_report(report: &ProbeReport) {
    println!("Calendar authorization: {}", report.authorization_before);

    if let Some(granted) = report.access_granted {
        let status_after = report
            .authorization_after
            .as_deref()
            .unwrap_or("unknown after request");
        println!("Requested full Calendar access: {granted}");
        println!("Calendar authorization after request: {status_after}");
    }

    if report.access_granted == Some(false) {
        println!("Calendar access was not granted; skipping calendars and events.");
        return;
    }

    if report.access_granted.is_none() {
        println!("Status-only mode; no permission prompt or Calendar read was attempted.");
        return;
    }

    println!();
    println!("Calendars ({})", report.calendars.len());
    for calendar in &report.calendars {
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

    println!();
    println!("Today ({})", report.today_events.len());
    if report.today_events.is_empty() {
        println!("- no events found");
    } else {
        for event in &report.today_events {
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
}

fn event_time_range(event: &EventReport) -> String {
    if event.all_day {
        return "all-day".to_string();
    }

    let start = event.start.get(11..16).unwrap_or(&event.start);
    let end = event.end.get(11..16).unwrap_or(&event.end);
    format!("{start}-{end}")
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
            url: event.URL.clone(),
            status: format!("{:?}", event.status),
            availability: format!("{:?}", event.availability),
        }
    }
}
