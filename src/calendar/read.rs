use super::*;

pub(crate) fn authorized_events_manager() -> Result<EventsManager> {
    match EventsManager::authorization_status() {
        AuthorizationStatus::FullAccess => Ok(EventsManager::new()),
        AuthorizationStatus::NotDetermined => {
            let events = EventsManager::new();
            match events.request_access() {
                Ok(true) => Ok(events),
                Ok(false) => bail!(
                    "Calendar access was not granted (authorization=NotDetermined); run `icalctl calendars` from Terminal.app, iTerm, or Ghostty and approve the macOS Calendar prompt"
                ),
                Err(error) => bail!(crate::doctor::access_request_error_message(&error)),
            }
        }
        AuthorizationStatus::WriteOnly => {
            bail!(
                "Calendar access is write-only (authorization=WriteOnly); enable Full Calendar Access in System Settings > Privacy & Security > Calendars, then run `icalctl doctor --json`"
            )
        }
        AuthorizationStatus::Denied => {
            bail!(
                "Calendar access is denied (authorization=Denied); enable Full Calendar Access in System Settings > Privacy & Security > Calendars, then run `icalctl doctor --json`"
            )
        }
        AuthorizationStatus::Restricted => {
            bail!(
                "Calendar access is restricted (authorization=Restricted); ask the device administrator to allow Calendar access, then run `icalctl doctor --json`"
            )
        }
    }
}

pub(crate) fn ensure_valid_event_range(start: DateTime<Local>, end: DateTime<Local>) -> Result<()> {
    if start >= end {
        bail!("event start must be before event end");
    }
    Ok(())
}

pub(super) fn event_report_with_alarms(
    events: &EventsManager,
    id: &str,
    occurrence_start: Option<&str>,
) -> Result<EventReport> {
    let occurrence_start = occurrence_start
        .map(parse_occurrence_start)
        .transpose()
        .with_context(|| format!("invalid occurrence start for event {id}"))?;
    let event = match occurrence_start {
        Some(start) => find_event_occurrence(events, id, start)?,
        None => events
            .get_event(id)
            .with_context(|| format!("failed to reload event {id}"))?,
    };
    let details = read_event_details(id, occurrence_start)
        .with_context(|| format!("failed to read alarms and recurrence for event {id}"))?;
    let calendars = list_calendars(events)?;
    let mut report = event_report(&event, &calendars);
    report.alarm_count = Some(details.alarms.len());
    report.alarms = Some(details.alarms);
    report.recurrence_count = Some(details.recurrence_rules.len());
    report.recurrence_rules = Some(details.recurrence_rules);
    Ok(report)
}

pub(super) fn parse_occurrence_start(value: &str) -> Result<DateTime<Local>> {
    let value = DateTime::parse_from_rfc3339(value)
        .context("occurrence start must be RFC3339 with an explicit UTC offset")?;
    let seconds = value
        .timestamp_nanos_opt()
        .map(|value| value / 1_000_000_000)
        .unwrap_or_else(|| value.timestamp_millis() / 1_000);
    Utc.timestamp_opt(seconds, 0)
        .single()
        .map(|value| value.with_timezone(&Local))
        .context("occurrence start is outside the supported date range")
}

pub(super) fn find_event_occurrence(
    events: &EventsManager,
    id: &str,
    occurrence_start: DateTime<Local>,
) -> Result<EventItem> {
    let matches = events
        .fetch_events(
            occurrence_start - chrono::Duration::seconds(1),
            occurrence_start + chrono::Duration::seconds(1),
            None,
        )
        .with_context(|| format!("failed to query occurrence for event {id}"))?
        .into_iter()
        .filter(|event| {
            event.identifier == id && event.start_date.timestamp() == occurrence_start.timestamp()
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [event] => Ok(event.clone()),
        [] => bail!(
            "no occurrence of event {id} starts at {}; refresh the event list or verify --occurrence-start",
            occurrence_start.to_rfc3339()
        ),
        _ => bail!(
            "multiple occurrences of event {id} start at {}; refusing to choose",
            occurrence_start.to_rfc3339()
        ),
    }
}

pub(super) fn fetch_range(
    from: &str,
    to: &str,
    selector: &ReadCalendarSelectorArgs,
) -> Result<Vec<EventReport>> {
    let start = parse_start_datetime(from).with_context(|| format!("invalid --from: {from}"))?;
    let end = parse_end_datetime(to).with_context(|| format!("invalid --to: {to}"))?;
    fetch_events(start, end, selector)
}

pub(crate) fn fetch_events(
    start: DateTime<Local>,
    end: DateTime<Local>,
    selector: &ReadCalendarSelectorArgs,
) -> Result<Vec<EventReport>> {
    if start >= end {
        bail!("--from must be before --to");
    }

    let events = authorized_events_manager()?;
    let calendars = list_calendars(&events)?;
    let selector = CalendarSelector {
        titles: &selector.calendars,
        ids: &selector.calendar_ids,
        source: selector.calendar_source.as_deref(),
        source_id: selector.source_id.as_deref(),
    };
    let selected_ids = if selector.is_empty() {
        None
    } else {
        Some(
            resolve_calendars(&calendars, &selector)?
                .into_iter()
                .map(|calendar| calendar.identifier)
                .collect::<Vec<_>>(),
        )
    };

    let reports = events
        .fetch_events(start, end, None)
        .context("failed to fetch events through EventKit")?
        .into_iter()
        .filter(|event| {
            selected_ids.as_ref().is_none_or(|ids| {
                event
                    .calendar_id
                    .as_ref()
                    .is_some_and(|id| ids.contains(id))
            })
        })
        .map(|event| event_report(&event, &calendars))
        .collect();

    Ok(reports)
}

pub(super) fn list_calendars(events: &EventsManager) -> Result<Vec<CalendarInfo>> {
    events
        .list_calendars()
        .context("failed to list calendars through EventKit")
}

fn event_report(event: &EventItem, calendars: &[CalendarInfo]) -> EventReport {
    let mut report = EventReport::from(event);
    if let Some(calendar) = event
        .calendar_id
        .as_deref()
        .and_then(|id| calendars.iter().find(|calendar| calendar.identifier == id))
    {
        report.calendar_source = calendar.source.clone();
        report.calendar_source_id = calendar.source_id.clone();
        report.calendar_type = Some(format!("{:?}", calendar.calendar_type));
        report.allows_calendar_modifications = Some(calendar.allows_modifications);
    }
    report
}

pub(super) fn event_matches(event: &EventReport, query: &str) -> bool {
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

pub(super) fn authorization_string() -> String {
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
