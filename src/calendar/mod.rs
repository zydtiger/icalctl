mod duplicates;
mod eventkit;
mod read;
mod recurrence;
mod selection;

pub(crate) use self::eventkit::{
    create_event_in_calendar, read_event_details, update_event_calendar_metadata,
    validate_event_time_zone, validate_event_url,
};
pub(crate) use self::read::{authorized_events_manager, ensure_valid_event_range, fetch_events};
pub(crate) use self::recurrence::{
    parse_event_recurrence, recurrence_rules_match, validate_recurring_all_day_inputs,
};
pub(crate) use self::selection::{resolve_target_calendar, resolve_target_calendar_with_selection};

use self::duplicates::*;
use self::eventkit::*;
use self::read::*;
use self::recurrence::*;
use self::selection::*;

use self::selection::{CalendarSelector, resolve_calendars};
use crate::cache::resolve_event_show_ref;
use crate::cli::{
    AvailabilityArg, BatchCommand, Command, EventJsonRecurrence, EventRecurrenceArgs,
    EventRepeatArg, EventScopeArg, EventWeekdayArg, IfExistsArg, ReadCalendarSelectorArgs,
    TravelCommand, WriteCalendarSelectorArgs,
};
use crate::dates::{
    datetime_in_time_zone, parse_end_datetime, parse_end_datetime_in_time_zone,
    parse_start_datetime, parse_start_datetime_in_time_zone, today_range, utc_datetime,
    validate_time_zone,
};

use crate::models::{
    CalendarReport, CalendarSelection, DeletedReport, EventDraftReport, EventRecurrenceEndReport,
    EventRecurrenceReport, EventRecurrenceWeekdayReport, EventReport, JsonOutput, StatusReport,
};
use crate::output::event_time_range;
use ::eventkit::{
    AlarmInfo, AlarmProximity, AuthorizationStatus, CalendarInfo, EventAvailability, EventDraft,
    EventItem, EventKitError, EventPatch, EventSpan, EventsManager,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub fn run(command: Command) -> Result<JsonOutput> {
    match command {
        Command::Config { command } => crate::config::run(command),
        Command::Version => Ok(JsonOutput::Version {
            version: crate::version::report(),
        }),
        Command::Status => Ok(JsonOutput::Status(StatusReport {
            authorization: authorization_string(),
        })),
        Command::Doctor => Ok(JsonOutput::Doctor {
            doctor: crate::doctor::doctor_report(),
        }),
        Command::Calendars {
            source,
            writable_only,
        } => {
            let events = authorized_events_manager()?;
            let default_id = match events.default_calendar() {
                Ok(calendar) => Some(calendar.identifier),
                Err(EventKitError::NoDefaultCalendar) => None,
                Err(error) => return Err(error).context("failed to read default calendar"),
            };
            let calendars = events
                .list_calendars()
                .context("failed to list Calendar calendars through EventKit")?;
            let calendars = filter_calendar_list(calendars, source.as_deref(), writable_only);
            let calendars = calendar_reports(&calendars, default_id.as_deref());
            Ok(JsonOutput::Calendars { calendars })
        }
        Command::DefaultCalendar => {
            let events = authorized_events_manager()?;
            let calendar = events
                .default_calendar()
                .context("no default calendar is available for new events")?;
            let mut calendar = CalendarReport::from(&calendar);
            calendar.is_default_for_new_events = true;
            Ok(JsonOutput::DefaultCalendar { calendar })
        }
        Command::List {
            from,
            to,
            calendar_selector,
        } => {
            let events = fetch_range(&from, &to, &calendar_selector)
                .with_context(|| format!("failed to list events from {from} to {to}"))?;
            Ok(JsonOutput::Events { events })
        }
        Command::Today { calendar_selector } => {
            let (start, end) = today_range()?;
            let events =
                fetch_events(start, end, &calendar_selector).context("failed to list today")?;
            Ok(JsonOutput::Events { events })
        }
        Command::Upcoming {
            days,
            calendar_selector,
        } => {
            if days <= 0 {
                bail!("--days must be greater than zero");
            }
            let start = Local::now();
            let end = start + chrono::Duration::days(days);
            let events = fetch_events(start, end, &calendar_selector)
                .context("failed to list upcoming events")?;
            Ok(JsonOutput::Events { events })
        }
        Command::Show {
            id,
            occurrence_start,
        } => {
            let reference = resolve_event_show_ref(&id, occurrence_start)?;
            let events = authorized_events_manager()?;
            Ok(JsonOutput::Event {
                event: Box::new(event_report_with_alarms(
                    &events,
                    &reference.id,
                    reference.occurrence_start.as_deref(),
                )?),
            })
        }
        Command::Search {
            query,
            from,
            to,
            calendar_selector,
        } => {
            let query = query.to_lowercase();
            let events = fetch_range(&from, &to, &calendar_selector)
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
            calendar_selector,
            notes,
            notes_file,
            json_file,
            location,
            url,
            all_day,
            availability,
            time_zone,
            alarm_minutes_before,
            recurrence,
            if_exists,
            duplicate_window_seconds,
            dry_run,
        } => {
            let input = resolve_add_command(AddCommandInput {
                title,
                start,
                end,
                calendar_selector,
                notes,
                notes_file,
                json_file,
                location,
                url,
                all_day,
                availability,
                time_zone,
                alarm_minutes_before,
                recurrence,
                if_exists,
                duplicate_window_seconds,
                dry_run,
            })?;
            let result = add_event(input)?;
            Ok(write_result_output(result))
        }
        Command::Update {
            id,
            occurrence_start,
            scope,
            title,
            start,
            end,
            calendar_selector,
            notes,
            clear_notes,
            location,
            clear_location,
            url,
            clear_url,
            all_day,
            timed,
            availability,
            time_zone,
            clear_time_zone,
            add_alarm_minutes_before,
            dry_run,
        } => {
            let result = update_event(UpdateEventInput {
                id,
                occurrence_start,
                scope,
                title,
                start,
                end,
                calendar_selector,
                notes,
                clear_notes,
                location,
                clear_location,
                url,
                clear_url,
                all_day,
                timed,
                availability,
                time_zone,
                clear_time_zone,
                add_alarm_minutes_before,
                dry_run,
            })?;
            Ok(write_result_output(result))
        }
        Command::Batch { command } => match command {
            BatchCommand::Add {
                file,
                if_exists,
                dry_run,
                continue_on_error,
            } => Ok(JsonOutput::Batch {
                batch: crate::batch::run_batch_add(&file, if_exists, dry_run, continue_on_error)?,
            }),
        },
        Command::Travel { command } => match command {
            TravelCommand::Serve { .. } => {
                unreachable!("travel serve is handled before calendar command dispatch")
            }
            TravelCommand::Flight {
                flight_number,
                from_airport,
                to_airport,
                departure,
                arrival,
                calendar_selector,
                notes,
                notes_file,
                url,
                availability,
                alarm_minutes_before,
                if_exists,
                duplicate_window_seconds,
                dry_run,
            } => {
                let extra_notes = match notes_file {
                    Some(path) => Some(read_notes_file(&path)?),
                    None => notes,
                };
                let flight = crate::travel::format_flight(crate::travel::FlightInput {
                    flight_number: &flight_number,
                    from_airport: &from_airport,
                    to_airport: &to_airport,
                    departure: &departure,
                    arrival: &arrival,
                    extra_notes: extra_notes.as_deref(),
                })?;
                let result = add_event(AddEventInput {
                    title: flight.title,
                    start: flight.start,
                    end: flight.end,
                    calendar_selector,
                    notes: Some(flight.notes),
                    location: Some(flight.location),
                    url,
                    all_day: false,
                    availability: Some(availability),
                    time_zone: None,
                    alarm_minutes_before,
                    recurrence: None,
                    if_exists,
                    duplicate_window_seconds,
                    dry_run,
                })?;
                Ok(write_result_output(result))
            }
        },
        Command::Reminders { command } => crate::reminders::run(command),
        Command::Delete {
            id,
            occurrence_start,
            scope,
            force,
        } => {
            let deleted = delete_event(&id, occurrence_start, scope, force)?;
            Ok(JsonOutput::Deleted { deleted })
        }
        Command::Completions { .. } => unreachable!("completions are handled before calendar run"),
    }
}

struct AddCommandInput {
    title: Option<String>,
    start: Option<String>,
    end: Option<String>,
    calendar_selector: WriteCalendarSelectorArgs,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    json_file: Option<PathBuf>,
    location: Option<String>,
    url: Option<String>,
    all_day: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    alarm_minutes_before: Vec<i64>,
    recurrence: EventRecurrenceArgs,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

struct AddEventInput {
    title: String,
    start: String,
    end: String,
    calendar_selector: WriteCalendarSelectorArgs,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    all_day: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    alarm_minutes_before: Vec<i64>,
    recurrence: Option<EventRecurrenceReport>,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonAddDraft {
    title: String,
    start: String,
    end: String,
    calendar: Option<String>,
    calendar_id: Option<String>,
    calendar_source: Option<String>,
    source_id: Option<String>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    #[serde(default)]
    all_day: bool,
    #[serde(default)]
    timed: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    #[serde(default)]
    alarm_minutes_before: Vec<i64>,
    recurrence: Option<EventJsonRecurrence>,
}

struct UpdateEventInput {
    id: String,
    occurrence_start: Option<String>,
    scope: Option<EventScopeArg>,
    title: Option<String>,
    start: Option<String>,
    end: Option<String>,
    calendar_selector: WriteCalendarSelectorArgs,
    notes: Option<String>,
    clear_notes: bool,
    location: Option<String>,
    clear_location: bool,
    url: Option<String>,
    clear_url: bool,
    all_day: bool,
    timed: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    clear_time_zone: bool,
    add_alarm_minutes_before: Vec<i64>,
    dry_run: bool,
}

enum WriteEventResult {
    Written(Box<EventReport>),
    DryRun(Box<EventDraftReport>),
}

fn write_result_output(result: WriteEventResult) -> JsonOutput {
    match result {
        WriteEventResult::Written(event) => JsonOutput::Event { event },
        WriteEventResult::DryRun(draft) => JsonOutput::DryRun {
            would_write: false,
            draft,
        },
    }
}

fn resolve_add_command(input: AddCommandInput) -> Result<AddEventInput> {
    let AddCommandInput {
        title,
        start,
        end,
        calendar_selector,
        notes,
        notes_file,
        json_file,
        location,
        url,
        all_day,
        availability,
        time_zone,
        alarm_minutes_before,
        recurrence,
        if_exists,
        duplicate_window_seconds,
        dry_run,
    } = input;

    if let Some(path) = json_file {
        if title.is_some()
            || start.is_some()
            || end.is_some()
            || !write_selector_is_empty(&calendar_selector)
            || notes.is_some()
            || notes_file.is_some()
            || location.is_some()
            || url.is_some()
            || all_day
            || availability.is_some()
            || time_zone.is_some()
            || !alarm_minutes_before.is_empty()
            || !event_recurrence_args_is_empty(&recurrence)
        {
            bail!(
                "--json-file cannot be combined with individual event fields; keep duplicate policy, tolerance, and dry-run flags on the command"
            );
        }
        let draft = read_json_add_draft(&path)?;
        if draft.all_day && draft.timed {
            bail!("JSON event cannot set both all_day and timed to true");
        }
        validate_json_selector(&draft)?;
        let recurrence = draft
            .recurrence
            .clone()
            .map(EventRecurrenceArgs::from)
            .map(|args| parse_event_recurrence(&args, &draft.start, draft.time_zone.as_deref()))
            .transpose()?
            .flatten();
        return Ok(AddEventInput {
            title: draft.title,
            start: draft.start,
            end: draft.end,
            calendar_selector: WriteCalendarSelectorArgs {
                calendar: draft.calendar,
                calendar_id: draft.calendar_id,
                calendar_source: draft.calendar_source,
                source_id: draft.source_id,
            },
            notes: draft.notes,
            location: draft.location,
            url: draft.url,
            all_day: draft.all_day && !draft.timed,
            availability: draft.availability,
            time_zone: draft.time_zone,
            alarm_minutes_before: draft.alarm_minutes_before,
            recurrence,
            if_exists,
            duplicate_window_seconds,
            dry_run,
        });
    }

    let notes = match notes_file {
        Some(path) => Some(read_notes_file(&path)?),
        None => notes,
    };
    let title = title.context("event title is required unless --json-file is used")?;
    let start = start.context("--start is required unless --json-file is used")?;
    let end = end.context("--end is required unless --json-file is used")?;
    let recurrence = parse_event_recurrence(&recurrence, &start, time_zone.as_deref())?;
    Ok(AddEventInput {
        title,
        start,
        end,
        calendar_selector,
        notes,
        location,
        url,
        all_day,
        availability,
        time_zone,
        alarm_minutes_before,
        recurrence,
        if_exists,
        duplicate_window_seconds,
        dry_run,
    })
}

fn read_notes_file(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut notes = String::new();
        io::stdin()
            .read_to_string(&mut notes)
            .context("failed to read event notes from stdin")?;
        return Ok(notes);
    }
    fs::read_to_string(path)
        .with_context(|| format!("failed to read notes file {}", path.display()))
}

fn read_json_add_draft(path: &Path) -> Result<JsonAddDraft> {
    let contents = fs::read(path)
        .with_context(|| format!("failed to read JSON event file {}", path.display()))?;
    serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse JSON event file {}", path.display()))
}

fn validate_json_selector(draft: &JsonAddDraft) -> Result<()> {
    if draft.calendar_id.is_some()
        && (draft.calendar.is_some()
            || draft.calendar_source.is_some()
            || draft.source_id.is_some())
    {
        bail!("JSON calendar_id cannot be combined with calendar, calendar_source, or source_id");
    }
    if draft.calendar_source.is_some() && draft.source_id.is_some() {
        bail!("JSON calendar_source and source_id cannot be combined");
    }
    if (draft.calendar_source.is_some() || draft.source_id.is_some()) && draft.calendar.is_none() {
        bail!("JSON calendar_source and source_id require calendar");
    }
    Ok(())
}

fn add_event(input: AddEventInput) -> Result<WriteEventResult> {
    if input.duplicate_window_seconds < 0 {
        bail!("--duplicate-window-seconds must be zero or greater");
    }
    if let Some(time_zone) = input.time_zone.as_deref() {
        validate_time_zone(time_zone)?;
        validate_event_time_zone(time_zone)?;
    }
    validate_recurring_all_day_inputs(
        input.all_day,
        input.recurrence.is_some(),
        &input.start,
        &input.end,
    )?;
    let start = parse_start_datetime_in_time_zone(&input.start, input.time_zone.as_deref())
        .with_context(|| format!("invalid --start: {}", input.start))?;
    let end = parse_end_datetime_in_time_zone(&input.end, input.time_zone.as_deref())
        .with_context(|| format!("invalid --end: {}", input.end))?;
    ensure_valid_event_range(start, end)?;
    validate_alarm_minutes(&input.alarm_minutes_before)?;
    if let Some(url) = input.url.as_deref() {
        validate_event_url(url)?;
    }

    let events = authorized_events_manager()?;
    let (target, selection) =
        resolve_target_calendar_with_selection(&events, &input.calendar_selector, true)?;
    let availability = input.availability.map(EventAvailability::from);
    ensure_availability_supported(&target, availability)?;

    let duplicates = matching_events(
        &events,
        &DuplicateQuery {
            title: &input.title,
            start,
            end,
            all_day: input.all_day,
            calendar_id: &target.identifier,
            excluded_event_id: None,
            window_seconds: input.duplicate_window_seconds,
        },
    )?;
    let recurrence_policy_error = duplicate_recurrence_policy_error(&input, &duplicates)?;

    if input.dry_run {
        let mut duplicate_warnings =
            duplicate_warnings(&duplicates, input.duplicate_window_seconds);
        if let Some(error) = &recurrence_policy_error {
            duplicate_warnings.push(error.clone());
        }
        let operation = if recurrence_policy_error.is_some() {
            "error_recurrence_policy"
        } else {
            match duplicates.len() {
                0 => "add",
                1 => match input.if_exists {
                    IfExistsArg::Skip => "skip_existing",
                    IfExistsArg::Update => "update_existing",
                    IfExistsArg::Error => "error_existing",
                },
                _ => "error_ambiguous_duplicates",
            }
        };
        let start_in_event_time_zone = input
            .time_zone
            .as_deref()
            .map(|time_zone| datetime_in_time_zone(start, time_zone))
            .transpose()?;
        let end_in_event_time_zone = input
            .time_zone
            .as_deref()
            .map(|time_zone| datetime_in_time_zone(end, time_zone))
            .transpose()?;
        return Ok(WriteEventResult::DryRun(Box::new(EventDraftReport {
            operation: operation.to_string(),
            scope: None,
            event_id: duplicates.first().map(|event| event.identifier.clone()),
            title: input.title,
            start: start.to_rfc3339(),
            end: end.to_rfc3339(),
            start_input: Some(input.start),
            end_input: Some(input.end),
            start_utc: utc_datetime(start),
            end_utc: utc_datetime(end),
            start_local: start.to_rfc3339(),
            end_local: end.to_rfc3339(),
            start_in_event_time_zone,
            end_in_event_time_zone,
            duration_seconds: (end - start).num_seconds(),
            all_day: input.all_day,
            timed: !input.all_day,
            calendar: target.title,
            calendar_id: target.identifier,
            calendar_source: target.source,
            calendar_source_id: target.source_id,
            calendar_selection: Some(selection),
            time_zone: input.time_zone,
            availability: availability
                .map_or("default", availability_name)
                .to_string(),
            alarm_count: input.alarm_minutes_before.len(),
            recurrence: input.recurrence.clone(),
            has_notes: input.notes.is_some(),
            has_location: input.location.is_some(),
            has_url: input.url.is_some(),
            duplicate_warnings,
        })));
    }

    if let Some(error) = recurrence_policy_error {
        bail!(error);
    }

    if duplicates.len() > 1 {
        let ids = duplicates
            .iter()
            .map(|event| event.identifier.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "{} matching events found; refusing to choose between ids: {ids}",
            duplicates.len()
        );
    }
    if let Some(existing) = duplicates.into_iter().next() {
        return match input.if_exists {
            IfExistsArg::Error => bail!(
                "matching event already exists [{}]; pass --if-exists skip or --if-exists update",
                existing.identifier
            ),
            IfExistsArg::Skip => {
                let mut report = event_report_with_alarms(&events, &existing.identifier, None)?;
                report.calendar_selection = Some(selection);
                report.write_action = Some("skipped".to_string());
                report.start_input = Some(input.start);
                report.end_input = Some(input.end);
                Ok(WriteEventResult::Written(Box::new(report)))
            }
            IfExistsArg::Update => update_existing_from_add(
                &events,
                &existing.identifier,
                &input,
                selection,
                availability,
            ),
        };
    }

    let draft = EventDraft {
        title: &input.title,
        start: Some(start),
        end: Some(end),
        notes: input.notes.as_deref(),
        location: input.location.as_deref(),
        calendar_title: None,
        all_day: input.all_day,
        URL: input.url.as_deref(),
        availability,
        ..Default::default()
    };

    let id = create_event_in_calendar(
        &draft,
        &target.identifier,
        input.time_zone.as_deref(),
        input.recurrence.as_ref(),
        &input.alarm_minutes_before,
    )
    .context("failed to create event through EventKit")?;
    let mut report = event_report_with_alarms(&events, &id, None)?;
    report.calendar_selection = Some(selection);
    report.write_action = Some("created".to_string());
    report.start_input = Some(input.start);
    report.end_input = Some(input.end);
    Ok(WriteEventResult::Written(Box::new(report)))
}

fn update_existing_from_add(
    events: &EventsManager,
    event_id: &str,
    input: &AddEventInput,
    selection: CalendarSelection,
    availability: Option<EventAvailability>,
) -> Result<WriteEventResult> {
    let notes = input.notes.as_deref().map(Some);
    let location = input.location.as_deref().map(Some);
    let url = input.url.as_deref().map(Some);
    if notes.is_some() || location.is_some() || url.is_some() || availability.is_some() {
        let patch = EventPatch {
            notes,
            location,
            URL: url,
            availability,
            ..Default::default()
        };
        events
            .update_event(event_id, &patch)
            .with_context(|| format!("failed to update matching event {event_id}"))?;
    }

    let event_id = if let Some(time_zone) = input.time_zone.as_deref() {
        update_event_calendar_metadata(event_id, None, Some(Some(time_zone)))
            .with_context(|| format!("failed to update timezone for matching event {event_id}"))?
    } else {
        event_id.to_string()
    };
    if !input.alarm_minutes_before.is_empty() {
        replace_relative_alarms(events, &event_id, &input.alarm_minutes_before)?;
    }

    let mut report = event_report_with_alarms(events, &event_id, None)?;
    report.calendar_selection = Some(selection);
    report.write_action = Some("updated".to_string());
    report.start_input = Some(input.start.clone());
    report.end_input = Some(input.end.clone());
    Ok(WriteEventResult::Written(Box::new(report)))
}

fn update_event(input: UpdateEventInput) -> Result<WriteEventResult> {
    let reference = resolve_event_show_ref(&input.id, input.occurrence_start.clone())?;
    let id = reference.id;
    let occurrence_start = reference
        .occurrence_start
        .as_deref()
        .map(parse_occurrence_start)
        .transpose()
        .with_context(|| format!("invalid occurrence start for event {id}"))?;
    let events = authorized_events_manager()?;
    let current = match occurrence_start {
        Some(start) => find_event_occurrence(&events, &id, start)?,
        None => events
            .get_event(&id)
            .with_context(|| format!("failed to load event {id}"))?,
    };
    let details = read_event_details(&id, occurrence_start)
        .with_context(|| format!("failed to read recurrence for event {id}"))?;
    let recurring = current.occurrence_date.is_some()
        || current.is_detached
        || !details.recurrence_rules.is_empty();
    let (span, scope) =
        resolve_event_mutation_scope(recurring, occurrence_start.is_some(), input.scope)?;

    ensure_event_calendar_writable(&events, &current, "update")?;
    let target = if write_selector_is_empty(&input.calendar_selector) {
        None
    } else {
        Some(resolve_target_calendar(
            &events,
            &input.calendar_selector,
            false,
        )?)
    };

    if let Some(time_zone) = input.time_zone.as_deref() {
        validate_time_zone(time_zone)?;
        validate_event_time_zone(time_zone)?;
    }
    // Parsing is controlled only by this invocation's --time-zone. Without
    // the flag, naive inputs always mean the Mac's local time, even when the
    // existing event already carries EventKit timezone metadata.
    let parse_time_zone = input.time_zone.as_deref();

    let start = input
        .start
        .as_deref()
        .map(|value| parse_start_datetime_in_time_zone(value, parse_time_zone))
        .transpose()
        .with_context(|| format!("invalid --start for event {id}"))?;
    let end = input
        .end
        .as_deref()
        .map(|value| parse_end_datetime_in_time_zone(value, parse_time_zone))
        .transpose()
        .with_context(|| format!("invalid --end for event {id}"))?;

    let effective_start = start.unwrap_or(current.start_date);
    let effective_end = end.unwrap_or(current.end_date);
    ensure_valid_event_range(effective_start, effective_end)?;
    validate_alarm_minutes(&input.add_alarm_minutes_before)?;
    if let Some(url) = input.url.as_deref() {
        validate_event_url(url)?;
    }

    let notes = nullable_patch(input.notes.as_deref(), input.clear_notes);
    let location = nullable_patch(input.location.as_deref(), input.clear_location);
    let url = nullable_patch(input.url.as_deref(), input.clear_url);
    let time_zone = nullable_patch(input.time_zone.as_deref(), input.clear_time_zone);
    let all_day = if input.all_day {
        Some(true)
    } else if input.timed {
        Some(false)
    } else {
        None
    };
    let availability = input.availability.map(EventAvailability::from);
    let current_calendar = event_calendar(&events, &current)?;
    let effective_calendar = target.as_ref().unwrap_or(&current_calendar);
    ensure_availability_supported(effective_calendar, availability)?;

    let has_patch = input.title.is_some()
        || start.is_some()
        || end.is_some()
        || target.is_some()
        || notes.is_some()
        || location.is_some()
        || url.is_some()
        || all_day.is_some()
        || availability.is_some()
        || time_zone.is_some();

    if !has_patch && input.add_alarm_minutes_before.is_empty() {
        bail!("no update fields provided");
    }

    if input.dry_run {
        let effective_title = input.title.as_deref().unwrap_or(&current.title);
        let effective_all_day = all_day.unwrap_or(current.all_day);
        let effective_availability = availability.unwrap_or(current.availability);
        let effective_time_zone = match time_zone {
            Some(value) => value,
            None => current.timezone.as_deref(),
        };
        let existing_alarm_count = details.alarms.len();
        let duplicates = matching_events(
            &events,
            &DuplicateQuery {
                title: effective_title,
                start: effective_start,
                end: effective_end,
                all_day: effective_all_day,
                calendar_id: &effective_calendar.identifier,
                excluded_event_id: Some(&id),
                window_seconds: 0,
            },
        )?;
        let duplicate_warnings = duplicate_warnings(&duplicates, 0);
        let start_in_event_time_zone = effective_time_zone
            .map(|time_zone| datetime_in_time_zone(effective_start, time_zone))
            .transpose()?;
        let end_in_event_time_zone = effective_time_zone
            .map(|time_zone| datetime_in_time_zone(effective_end, time_zone))
            .transpose()?;

        return Ok(WriteEventResult::DryRun(Box::new(EventDraftReport {
            operation: "update".to_string(),
            scope: scope.map(str::to_string),
            event_id: Some(id),
            title: effective_title.to_string(),
            start: effective_start.to_rfc3339(),
            end: effective_end.to_rfc3339(),
            start_input: input.start,
            end_input: input.end,
            start_utc: utc_datetime(effective_start),
            end_utc: utc_datetime(effective_end),
            start_local: effective_start.to_rfc3339(),
            end_local: effective_end.to_rfc3339(),
            start_in_event_time_zone,
            end_in_event_time_zone,
            duration_seconds: (effective_end - effective_start).num_seconds(),
            all_day: effective_all_day,
            timed: !effective_all_day,
            calendar: effective_calendar.title.clone(),
            calendar_id: effective_calendar.identifier.clone(),
            calendar_source: effective_calendar.source.clone(),
            calendar_source_id: effective_calendar.source_id.clone(),
            calendar_selection: target.as_ref().map(|_| CalendarSelection::Explicit),
            time_zone: effective_time_zone.map(str::to_string),
            availability: availability_name(effective_availability).to_string(),
            alarm_count: existing_alarm_count + input.add_alarm_minutes_before.len(),
            recurrence: details.recurrence_rules.first().cloned(),
            has_notes: patched_field_present(current.notes.as_deref(), notes),
            has_location: patched_field_present(current.location.as_deref(), location),
            has_url: patched_field_present(current.URL.as_deref(), url),
            duplicate_warnings,
        })));
    }

    let has_event_patch = input.title.is_some()
        || start.is_some()
        || end.is_some()
        || notes.is_some()
        || location.is_some()
        || url.is_some()
        || all_day.is_some()
        || availability.is_some();

    let patch = EventPatch {
        title: input.title.as_deref(),
        notes,
        location,
        start,
        end,
        all_day,
        calendar_title: None,
        URL: url,
        availability,
        span,
        ..Default::default()
    };
    debug_assert!(
        has_event_patch
            || target.is_some()
            || time_zone.is_some()
            || !input.add_alarm_minutes_before.is_empty()
    );
    let updated = update_event_scoped(
        &id,
        occurrence_start,
        &patch,
        target.as_ref().map(|target| target.identifier.as_str()),
        time_zone,
        &input.add_alarm_minutes_before,
        span,
    )
    .with_context(|| format!("failed to update event {id}"))?;
    let readback_events = authorized_events_manager()?;
    let mut report = event_report_with_alarms(
        &readback_events,
        &updated.id,
        recurring.then_some(updated.occurrence_start.as_str()),
    )?;
    report.write_action = Some("updated".to_string());
    report.write_scope = scope.map(str::to_string);
    report.start_input = input.start;
    report.end_input = input.end;
    Ok(WriteEventResult::Written(Box::new(report)))
}

fn delete_event(
    reference: &str,
    occurrence_start: Option<String>,
    requested_scope: Option<EventScopeArg>,
    force: bool,
) -> Result<DeletedReport> {
    let reference = resolve_event_show_ref(reference, occurrence_start)?;
    let id = reference.id;
    let occurrence_start = reference
        .occurrence_start
        .as_deref()
        .map(parse_occurrence_start)
        .transpose()
        .with_context(|| format!("invalid occurrence start for event {id}"))?;
    let events = authorized_events_manager()?;
    let event = match occurrence_start {
        Some(start) => find_event_occurrence(&events, &id, start)?,
        None => events
            .get_event(&id)
            .with_context(|| format!("failed to load event {id}"))?,
    };
    let details = read_event_details(&id, occurrence_start)
        .with_context(|| format!("failed to read recurrence for event {id}"))?;
    let recurring = event.occurrence_date.is_some()
        || event.is_detached
        || !details.recurrence_rules.is_empty();
    let (span, scope) =
        resolve_event_mutation_scope(recurring, occurrence_start.is_some(), requested_scope)?;
    ensure_event_calendar_writable(&events, &event, "delete")?;

    if !force {
        confirm_delete(&event, scope)?;
    }

    delete_event_scoped(&id, occurrence_start, span)
        .with_context(|| format!("failed to delete event {id}"))?;

    Ok(DeletedReport {
        id,
        title: event.title,
        scope: scope.map(str::to_string),
    })
}

fn resolve_event_mutation_scope(
    recurring: bool,
    has_occurrence_start: bool,
    requested: Option<EventScopeArg>,
) -> Result<(EventSpan, Option<&'static str>)> {
    if !recurring {
        if requested.is_some() {
            bail!("--scope is only valid for recurring events");
        }
        return Ok((EventSpan::This, None));
    }
    if !has_occurrence_start {
        bail!(
            "recurring event mutation requires an exact occurrence; pass --occurrence-start or use a cached row number"
        );
    }
    match requested {
        Some(EventScopeArg::Occurrence) => Ok((EventSpan::This, Some("occurrence"))),
        Some(EventScopeArg::Future) => Ok((EventSpan::Future, Some("future"))),
        None => bail!("recurring event mutation requires --scope occurrence or --scope future"),
    }
}

fn nullable_patch(value: Option<&str>, clear: bool) -> Option<Option<&str>> {
    if clear { Some(None) } else { value.map(Some) }
}

pub(crate) fn ensure_availability_supported(
    calendar: &CalendarInfo,
    availability: Option<EventAvailability>,
) -> Result<()> {
    let Some(availability) = availability else {
        return Ok(());
    };
    let availability = availability_name(availability);
    if !calendar
        .supported_event_availabilities
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(availability))
    {
        bail!(
            "calendar {:?} does not support availability {:?}; supported values: {}",
            calendar.title,
            availability,
            if calendar.supported_event_availabilities.is_empty() {
                "none".to_string()
            } else {
                calendar.supported_event_availabilities.join(", ")
            }
        );
    }
    Ok(())
}

pub(crate) fn availability_name(availability: EventAvailability) -> &'static str {
    match availability {
        EventAvailability::NotSupported => "not_supported",
        EventAvailability::Busy => "busy",
        EventAvailability::Free => "free",
        EventAvailability::Tentative => "tentative",
        EventAvailability::Unavailable => "unavailable",
    }
}

pub(crate) fn validate_alarm_minutes(minutes_before: &[i64]) -> Result<()> {
    if let Some(minutes) = minutes_before.iter().find(|minutes| **minutes < 0) {
        bail!("alarm minutes before must be zero or greater: {minutes}");
    }
    Ok(())
}

fn patched_field_present(current: Option<&str>, patch: Option<Option<&str>>) -> bool {
    match patch {
        Some(value) => value.is_some(),
        None => current.is_some(),
    }
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

pub(crate) fn add_relative_alarms(
    events: &EventsManager,
    id: &str,
    minutes_before: &[i64],
) -> Result<()> {
    validate_alarm_minutes(minutes_before)?;
    for minutes in minutes_before {
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

pub(crate) fn replace_relative_alarms(
    events: &EventsManager,
    id: &str,
    minutes_before: &[i64],
) -> Result<()> {
    validate_alarm_minutes(minutes_before)?;
    let existing = events
        .get_event_alarms(id)
        .with_context(|| format!("failed to read alarms for event {id}"))?;
    for index in (0..existing.len()).rev() {
        events
            .remove_event_alarm(id, index)
            .with_context(|| format!("failed to remove alarm {index} from event {id}"))?;
    }
    add_relative_alarms(events, id, minutes_before)
}

fn confirm_delete(event: &EventItem, scope: Option<&str>) -> Result<()> {
    let mut stderr = io::stderr();
    let scope = match scope {
        Some("occurrence") => "only this recurring occurrence",
        Some("future") => "this recurring occurrence and all future occurrences",
        _ => "this event",
    };
    writeln!(
        stderr,
        "Delete {scope}: \"{}\" ({})?",
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

#[cfg(test)]
mod tests {
    use super::*;
    use ::eventkit::CalendarType;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("icalctl-{name}-{nonce}"))
    }

    #[test]
    fn nullable_patch_clear_wins() {
        assert_eq!(nullable_patch(Some("notes"), false), Some(Some("notes")));
        assert_eq!(nullable_patch(Some("notes"), true), Some(None));
        assert_eq!(nullable_patch(None, true), Some(None));
        assert_eq!(nullable_patch(None, false), None);
    }

    #[test]
    fn notes_file_preserves_exact_utf8_contents() {
        let path = temporary_test_path("notes.txt");
        let expected = "First line\nSecond line\n\nFinal line without trimming";
        fs::write(&path, expected).unwrap();

        let actual = read_notes_file(&path).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn json_event_draft_supports_structured_add_fields() {
        let draft: JsonAddDraft = serde_json::from_str(
            r#"{
                "title": "Meeting",
                "start": "2026-07-10T09:00",
                "end": "2026-07-10T10:00",
                "calendar_id": "CAL-1",
                "notes": "Agenda",
                "location": "Room 3",
                "url": "https://example.com",
                "availability": "busy",
                "time_zone": "Asia/Shanghai",
                "alarm_minutes_before": [10],
                "recurrence": {
                    "frequency": "weekly",
                    "interval": 2,
                    "weekdays": ["monday", "wednesday"],
                    "count": 8
                },
                "timed": true
            }"#,
        )
        .unwrap();

        assert_eq!(draft.calendar_id.as_deref(), Some("CAL-1"));
        assert_eq!(draft.notes.as_deref(), Some("Agenda"));
        assert_eq!(draft.alarm_minutes_before, [10]);
        assert_eq!(
            draft.recurrence.as_ref().unwrap().frequency,
            EventRepeatArg::Weekly
        );
        assert_eq!(
            draft.recurrence.as_ref().unwrap().weekdays,
            [EventWeekdayArg::Monday, EventWeekdayArg::Wednesday]
        );
        assert!(draft.timed);
        assert!(!draft.all_day);
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

    #[test]
    fn exactly_one_calendar_is_marked_as_default() {
        let calendars = vec![calendar("A", "Work"), calendar("B", "Personal")];
        let reports = calendar_reports(&calendars, Some("B"));

        assert_eq!(
            reports
                .iter()
                .filter(|calendar| calendar.is_default_for_new_events)
                .count(),
            1
        );
        assert!(reports[1].is_default_for_new_events);
        assert!(reports[1].allows_modifications);
    }

    #[test]
    fn calendar_list_filters_source_and_writability() {
        let icloud = calendar("A", "Personal");
        let mut exchange = calendar("B", "Work");
        exchange.source = Some("Exchange".to_string());
        let mut read_only = calendar("C", "Holidays");
        read_only.allows_modifications = false;
        let calendars = vec![icloud, exchange, read_only];

        let icloud_writable = filter_calendar_list(calendars, Some("iCloud"), true);

        assert_eq!(icloud_writable.len(), 1);
        assert_eq!(icloud_writable[0].identifier, "A");
    }

    #[test]
    fn add_selection_reports_explicit_or_eventkit_default() {
        let implicit = WriteCalendarSelectorArgs {
            calendar: None,
            calendar_id: None,
            calendar_source: None,
            source_id: None,
        };
        let explicit = WriteCalendarSelectorArgs {
            calendar: None,
            calendar_id: Some("B".to_string()),
            calendar_source: None,
            source_id: None,
        };

        assert_eq!(
            calendar_selection_for_write_with_config(&implicit, false),
            CalendarSelection::EventkitDefault,
        );
        assert_eq!(
            calendar_selection_for_write_with_config(&implicit, true),
            CalendarSelection::ConfiguredDefault,
        );
        assert_eq!(
            calendar_selection_for_write_with_config(&explicit, true),
            CalendarSelection::Explicit,
        );
    }

    #[test]
    fn alarm_validation_rejects_negative_values_before_writes() {
        assert!(validate_alarm_minutes(&[0, 10]).is_ok());
        assert_eq!(
            validate_alarm_minutes(&[-1]).unwrap_err().to_string(),
            "alarm minutes before must be zero or greater: -1"
        );
    }

    #[test]
    fn url_validation_runs_without_writing() {
        assert!(validate_event_url("https://example.com/event").is_ok());
        assert!(validate_event_url("https://exa mple.com/event").is_err());
    }

    #[test]
    fn eventkit_time_zone_validation_runs_without_writing() {
        assert!(validate_event_time_zone("Europe/Berlin").is_ok());
        assert!(validate_event_time_zone("Mars/Olympus_Mons").is_err());
    }

    #[test]
    fn event_range_validation_rejects_non_positive_duration() {
        let start = parse_start_datetime("2026-07-10T10:00").unwrap();
        let same = parse_end_datetime("2026-07-10T10:00").unwrap();
        let earlier = parse_end_datetime("2026-07-10T09:00").unwrap();

        assert!(ensure_valid_event_range(start, same).is_err());
        assert!(ensure_valid_event_range(start, earlier).is_err());
    }

    #[test]
    fn duplicate_window_is_exact_by_default_and_tolerant_when_requested() {
        let expected = parse_start_datetime("2026-07-10T10:00:00+08:00").unwrap();
        let thirty_seconds_later = expected + chrono::Duration::seconds(30);

        assert!(!datetime_within_window(thirty_seconds_later, expected, 0));
        assert!(datetime_within_window(thirty_seconds_later, expected, 30));
        assert!(!datetime_within_window(thirty_seconds_later, expected, 29));
    }

    #[test]
    fn availability_validation_uses_calendar_capabilities() {
        let mut calendar = calendar("A", "Work");
        calendar.supported_event_availabilities = vec!["busy".to_string(), "free".to_string()];

        assert!(ensure_availability_supported(&calendar, Some(EventAvailability::Free)).is_ok());
        assert!(
            ensure_availability_supported(&calendar, Some(EventAvailability::Tentative)).is_err()
        );
    }

    #[test]
    fn nullable_patch_presence_matches_resulting_field() {
        assert!(patched_field_present(Some("old"), None));
        assert!(patched_field_present(None, Some(Some("new"))));
        assert!(!patched_field_present(Some("old"), Some(None)));
    }

    #[test]
    fn occurrence_start_requires_an_explicit_rfc3339_offset() {
        assert!(parse_occurrence_start("2026-07-20T09:00:00+03:00").is_ok());
        assert!(parse_occurrence_start("2026-07-20T09:00:00").is_err());
        assert_eq!(
            parse_occurrence_start("1969-12-31T23:59:59.750+00:00")
                .unwrap()
                .timestamp(),
            0
        );
    }

    #[test]
    fn recurring_creation_normalizes_weekdays_and_count() {
        let recurrence = parse_event_recurrence(
            &EventRecurrenceArgs {
                repeat: Some(EventRepeatArg::Weekly),
                interval: Some(2),
                weekdays: vec![
                    EventWeekdayArg::Wednesday,
                    EventWeekdayArg::Monday,
                    EventWeekdayArg::Monday,
                ],
                month_days: Vec::new(),
                count: Some(8),
                until: None,
            },
            "2026-07-20T09:00:00+03:00",
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(recurrence.frequency, "weekly");
        assert_eq!(recurrence.interval, 2);
        assert_eq!(recurrence.end.occurrence_count, Some(8));
        assert_eq!(
            recurrence.days_of_week.unwrap(),
            vec![
                EventRecurrenceWeekdayReport {
                    weekday: 2,
                    week_number: 0,
                },
                EventRecurrenceWeekdayReport {
                    weekday: 4,
                    week_number: 0,
                },
            ]
        );
    }

    #[test]
    fn recurring_creation_rejects_invalid_combinations_and_end() {
        let mut args = EventRecurrenceArgs {
            repeat: Some(EventRepeatArg::Daily),
            weekdays: vec![EventWeekdayArg::Monday],
            ..Default::default()
        };
        assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
        args = EventRecurrenceArgs {
            repeat: Some(EventRepeatArg::Monthly),
            month_days: vec![0],
            ..Default::default()
        };
        assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
        args = EventRecurrenceArgs {
            repeat: Some(EventRepeatArg::Yearly),
            until: Some("2026-07-19T09:00:00+03:00".to_string()),
            ..Default::default()
        };
        assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
        args = EventRecurrenceArgs {
            repeat: Some(EventRepeatArg::Yearly),
            month_days: vec![1],
            ..Default::default()
        };
        assert!(parse_event_recurrence(&args, "2026-07-20T09:00:00+03:00", None).is_err());
    }

    #[test]
    fn recurring_duplicate_identity_requires_the_exact_normalized_rule() {
        let requested = parse_event_recurrence(
            &EventRecurrenceArgs {
                repeat: Some(EventRepeatArg::Weekly),
                weekdays: vec![EventWeekdayArg::Monday],
                ..Default::default()
            },
            "2026-07-20T09:00:00+03:00",
            None,
        )
        .unwrap()
        .unwrap();
        let mut different = requested.clone();
        different.interval = 2;

        assert!(recurrence_rules_match(
            Some(&requested),
            std::slice::from_ref(&requested)
        ));
        assert!(!recurrence_rules_match(Some(&requested), &[different]));
        assert!(!recurrence_rules_match(
            None,
            std::slice::from_ref(&requested)
        ));
        assert!(recurrence_rules_match(None, &[]));
    }

    #[test]
    fn recurring_mutations_require_occurrence_and_explicit_scope() {
        assert!(resolve_event_mutation_scope(true, false, Some(EventScopeArg::Future)).is_err());
        assert!(resolve_event_mutation_scope(true, true, None).is_err());
        assert_eq!(
            resolve_event_mutation_scope(true, true, Some(EventScopeArg::Occurrence)).unwrap(),
            (EventSpan::This, Some("occurrence"))
        );
        assert_eq!(
            resolve_event_mutation_scope(true, true, Some(EventScopeArg::Future)).unwrap(),
            (EventSpan::Future, Some("future"))
        );
    }

    #[test]
    fn nonrecurring_mutations_reject_series_scope() {
        assert_eq!(
            resolve_event_mutation_scope(false, false, None).unwrap(),
            (EventSpan::This, None)
        );
        assert!(
            resolve_event_mutation_scope(false, true, Some(EventScopeArg::Occurrence)).is_err()
        );
    }

    #[test]
    fn recurring_all_day_events_require_date_only_boundaries() {
        assert!(validate_recurring_all_day_inputs(true, true, "2030-03-30", "2030-03-30").is_ok());
        assert!(
            validate_recurring_all_day_inputs(
                true,
                true,
                "2030-03-30T00:00:00+01:00",
                "2030-03-31T00:00:00+01:00"
            )
            .is_err()
        );
        assert!(
            validate_recurring_all_day_inputs(
                false,
                true,
                "2030-03-30T09:00:00+01:00",
                "2030-03-30T10:00:00+01:00"
            )
            .is_ok()
        );
    }

    fn calendar(id: &str, title: &str) -> CalendarInfo {
        CalendarInfo {
            identifier: id.to_string(),
            title: title.to_string(),
            source: Some("iCloud".to_string()),
            source_id: Some("SOURCE".to_string()),
            calendar_type: CalendarType::CalDAV,
            allows_modifications: true,
            is_immutable: false,
            is_subscribed: false,
            color: None,
            allowed_entity_types: vec!["event".to_string()],
            supported_event_availabilities: Vec::new(),
        }
    }
}
