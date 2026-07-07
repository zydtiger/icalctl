use crate::cache::resolve_event_ref;
use crate::cli::{AvailabilityArg, Command};
use crate::dates::{parse_end_datetime, parse_start_datetime, today_range};
use crate::models::{
    AlarmReport, CalendarReport, DeletedReport, EventReport, JsonOutput, StatusReport,
};
use crate::output::event_time_range;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local};
use eventkit::{
    AlarmInfo, AlarmProximity, AuthorizationStatus, EventAvailability, EventDraft, EventItem,
    EventPatch, EventsManager,
};
use std::io::{self, Write};

pub fn run(command: Command) -> Result<JsonOutput> {
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
            let id = resolve_event_ref(&id)?;
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
        Command::Add {
            title,
            start,
            end,
            calendar,
            notes,
            location,
            url,
            all_day,
            availability,
            alarm_minutes_before,
        } => {
            let event = add_event(AddEventInput {
                title,
                start,
                end,
                calendar,
                notes,
                location,
                url,
                all_day,
                availability,
                alarm_minutes_before,
            })?;
            Ok(JsonOutput::Event {
                event: Box::new(event),
            })
        }
        Command::Update {
            id,
            title,
            start,
            end,
            calendar,
            notes,
            clear_notes,
            location,
            clear_location,
            url,
            clear_url,
            all_day,
            timed,
            availability,
            add_alarm_minutes_before,
        } => {
            let event = update_event(UpdateEventInput {
                id,
                title,
                start,
                end,
                calendar,
                notes,
                clear_notes,
                location,
                clear_location,
                url,
                clear_url,
                all_day,
                timed,
                availability,
                add_alarm_minutes_before,
            })?;
            Ok(JsonOutput::Event {
                event: Box::new(event),
            })
        }
        Command::Delete { id, force } => {
            let deleted = delete_event(&id, force)?;
            Ok(JsonOutput::Deleted { deleted })
        }
    }
}

struct AddEventInput {
    title: String,
    start: String,
    end: String,
    calendar: Option<String>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    all_day: bool,
    availability: Option<AvailabilityArg>,
    alarm_minutes_before: Vec<i64>,
}

struct UpdateEventInput {
    id: String,
    title: Option<String>,
    start: Option<String>,
    end: Option<String>,
    calendar: Option<String>,
    notes: Option<String>,
    clear_notes: bool,
    location: Option<String>,
    clear_location: bool,
    url: Option<String>,
    clear_url: bool,
    all_day: bool,
    timed: bool,
    availability: Option<AvailabilityArg>,
    add_alarm_minutes_before: Vec<i64>,
}

fn add_event(input: AddEventInput) -> Result<EventReport> {
    let start = parse_start_datetime(&input.start)
        .with_context(|| format!("invalid --start: {}", input.start))?;
    let end =
        parse_end_datetime(&input.end).with_context(|| format!("invalid --end: {}", input.end))?;
    ensure_valid_event_range(start, end)?;

    let events = authorized_events_manager()?;
    ensure_target_calendar_writable(&events, input.calendar.as_deref())?;

    let draft = EventDraft {
        title: &input.title,
        start: Some(start),
        end: Some(end),
        notes: input.notes.as_deref(),
        location: input.location.as_deref(),
        calendar_title: input.calendar.as_deref(),
        all_day: input.all_day,
        URL: input.url.as_deref(),
        availability: input.availability.map(EventAvailability::from),
        ..Default::default()
    };

    let created = events
        .create_event(&draft)
        .context("failed to create event through EventKit")?;

    add_relative_alarms(&events, &created.identifier, &input.alarm_minutes_before)?;
    event_report_with_alarms(&events, &created.identifier)
}

fn update_event(input: UpdateEventInput) -> Result<EventReport> {
    let id = resolve_event_ref(&input.id)?;
    let events = authorized_events_manager()?;
    let current = events
        .get_event(&id)
        .with_context(|| format!("failed to load event {id}"))?;

    ensure_event_calendar_writable(&events, &current, "update")?;
    if let Some(calendar) = &input.calendar {
        ensure_target_calendar_writable(&events, Some(calendar))?;
    }

    let start = input
        .start
        .as_deref()
        .map(parse_start_datetime)
        .transpose()
        .with_context(|| format!("invalid --start for event {id}"))?;
    let end = input
        .end
        .as_deref()
        .map(parse_end_datetime)
        .transpose()
        .with_context(|| format!("invalid --end for event {id}"))?;

    let effective_start = start.unwrap_or(current.start_date);
    let effective_end = end.unwrap_or(current.end_date);
    ensure_valid_event_range(effective_start, effective_end)?;

    let notes = nullable_patch(input.notes.as_deref(), input.clear_notes);
    let location = nullable_patch(input.location.as_deref(), input.clear_location);
    let url = nullable_patch(input.url.as_deref(), input.clear_url);
    let all_day = if input.all_day {
        Some(true)
    } else if input.timed {
        Some(false)
    } else {
        None
    };
    let availability = input.availability.map(EventAvailability::from);

    let has_patch = input.title.is_some()
        || start.is_some()
        || end.is_some()
        || input.calendar.is_some()
        || notes.is_some()
        || location.is_some()
        || url.is_some()
        || all_day.is_some()
        || availability.is_some();

    if !has_patch && input.add_alarm_minutes_before.is_empty() {
        bail!("no update fields provided");
    }

    if has_patch {
        let patch = EventPatch {
            title: input.title.as_deref(),
            notes,
            location,
            start,
            end,
            all_day,
            calendar_title: input.calendar.as_deref(),
            URL: url,
            availability,
            ..Default::default()
        };

        events
            .update_event(&id, &patch)
            .with_context(|| format!("failed to update event {id}"))?;
    }

    add_relative_alarms(&events, &id, &input.add_alarm_minutes_before)?;
    event_report_with_alarms(&events, &id)
}

fn delete_event(reference: &str, force: bool) -> Result<DeletedReport> {
    let id = resolve_event_ref(reference)?;
    let events = authorized_events_manager()?;
    let event = events
        .get_event(&id)
        .with_context(|| format!("failed to load event {id}"))?;
    ensure_event_calendar_writable(&events, &event, "delete")?;

    if !force {
        confirm_delete(&event)?;
    }

    events
        .delete_event(&id, false)
        .with_context(|| format!("failed to delete event {id}"))?;

    Ok(DeletedReport {
        id,
        title: event.title,
    })
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

fn ensure_valid_event_range(start: DateTime<Local>, end: DateTime<Local>) -> Result<()> {
    if start >= end {
        bail!("event start must be before event end");
    }
    Ok(())
}

fn nullable_patch(value: Option<&str>, clear: bool) -> Option<Option<&str>> {
    if clear { Some(None) } else { value.map(Some) }
}

fn ensure_target_calendar_writable(
    events: &EventsManager,
    calendar_title: Option<&str>,
) -> Result<()> {
    let calendar = if let Some(title) = calendar_title {
        events
            .list_calendars()
            .context("failed to list calendars before write")?
            .into_iter()
            .find(|calendar| calendar.title == title)
            .ok_or_else(|| anyhow!("calendar not found: {title}"))?
    } else {
        events
            .default_calendar()
            .context("no default calendar is available for new events")?
    };

    if !calendar.allows_modifications {
        bail!("calendar is read-only: {}", calendar.title);
    }

    Ok(())
}

fn ensure_event_calendar_writable(
    events: &EventsManager,
    event: &EventItem,
    action: &str,
) -> Result<()> {
    let calendars = events
        .list_calendars()
        .context("failed to list calendars before write")?;

    let calendar = event
        .calendar_id
        .as_deref()
        .and_then(|id| calendars.iter().find(|calendar| calendar.identifier == id))
        .or_else(|| {
            event
                .calendar_title
                .as_deref()
                .and_then(|title| calendars.iter().find(|calendar| calendar.title == title))
        });

    if let Some(calendar) = calendar
        && !calendar.allows_modifications
    {
        bail!(
            "cannot {action} event on read-only calendar: {}",
            calendar.title
        );
    }

    Ok(())
}

fn add_relative_alarms(events: &EventsManager, id: &str, minutes_before: &[i64]) -> Result<()> {
    for minutes in minutes_before {
        if *minutes < 0 {
            bail!("alarm minutes before must be zero or greater: {minutes}");
        }

        let alarm = AlarmInfo {
            relative_offset: Some(-(*minutes as f64) * 60.0),
            proximity: AlarmProximity::None,
            ..Default::default()
        };

        events
            .add_event_alarm(id, &alarm)
            .with_context(|| format!("failed to add {minutes}-minute alarm to event {id}"))?;
    }

    Ok(())
}

fn event_report_with_alarms(events: &EventsManager, id: &str) -> Result<EventReport> {
    let event = events
        .get_event(id)
        .with_context(|| format!("failed to reload event {id}"))?;
    let alarms = events
        .get_event_alarms(id)
        .with_context(|| format!("failed to read alarms for event {id}"))?
        .iter()
        .map(AlarmReport::from)
        .collect();
    let mut report = EventReport::from(&event);
    report.alarms = Some(alarms);
    Ok(report)
}

fn confirm_delete(event: &EventItem) -> Result<()> {
    let mut stderr = io::stderr();
    writeln!(
        stderr,
        "Delete event \"{}\" ({})?",
        event.title,
        event_time_range(&EventReport::from(event))
    )?;
    write!(stderr, "Type delete to confirm: ")?;
    stderr.flush()?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read delete confirmation")?;

    if input.trim() == "delete" {
        Ok(())
    } else {
        bail!("delete cancelled")
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

impl From<AvailabilityArg> for EventAvailability {
    fn from(value: AvailabilityArg) -> Self {
        match value {
            AvailabilityArg::Busy => EventAvailability::Busy,
            AvailabilityArg::Free => EventAvailability::Free,
            AvailabilityArg::Tentative => EventAvailability::Tentative,
            AvailabilityArg::Unavailable => EventAvailability::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nullable_patch_clear_wins() {
        assert_eq!(nullable_patch(Some("notes"), false), Some(Some("notes")));
        assert_eq!(nullable_patch(Some("notes"), true), Some(None));
        assert_eq!(nullable_patch(None, true), Some(None));
        assert_eq!(nullable_patch(None, false), None);
    }

    #[test]
    fn availability_arg_maps_to_eventkit_availability() {
        assert_eq!(
            EventAvailability::from(AvailabilityArg::Free),
            EventAvailability::Free
        );
        assert_eq!(
            EventAvailability::from(AvailabilityArg::Unavailable),
            EventAvailability::Unavailable
        );
    }
}
