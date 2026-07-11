use crate::cache::{resolve_event_ref, resolve_event_show_ref};
use crate::calendar_selector::{
    CalendarSelector, require_single_writable_calendar, resolve_calendars,
};
use crate::cli::{
    AvailabilityArg, BatchCommand, Command, EventJsonRecurrence, EventRecurrenceArgs,
    EventRepeatArg, EventWeekdayArg, IfExistsArg, ReadCalendarSelectorArgs, TravelCommand,
    WriteCalendarSelectorArgs,
};
use crate::dates::{
    datetime_in_time_zone, parse_end_datetime, parse_end_datetime_in_time_zone,
    parse_start_datetime, parse_start_datetime_in_time_zone, today_range, utc_datetime,
    validate_time_zone,
};
use crate::eventkit_bridge::{
    canonical_eventkit_recurrence_end_utc, create_event_in_calendar, read_event_details,
    update_event_calendar_metadata, validate_event_time_zone, validate_event_url,
};
use crate::models::{
    CalendarReport, CalendarSelection, DeletedReport, EventDraftReport, EventRecurrenceEndReport,
    EventRecurrenceReport, EventRecurrenceWeekdayReport, EventReport, JsonOutput, StatusReport,
};
use crate::output::event_time_range;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, TimeZone, Utc};
use eventkit::{
    AlarmInfo, AlarmProximity, AuthorizationStatus, CalendarInfo, EventAvailability, EventDraft,
    EventItem, EventKitError, EventPatch, EventsManager,
};
use serde::Deserialize;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub fn run(command: Command) -> Result<JsonOutput> {
    match command {
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
        Command::Delete { id, force } => {
            let deleted = delete_event(&id, force)?;
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

fn event_recurrence_args_is_empty(args: &EventRecurrenceArgs) -> bool {
    args.repeat.is_none()
        && args.interval.is_none()
        && args.weekdays.is_empty()
        && args.month_days.is_empty()
        && args.count.is_none()
        && args.until.is_none()
}

pub(crate) fn parse_event_recurrence(
    args: &EventRecurrenceArgs,
    start_input: &str,
    time_zone: Option<&str>,
) -> Result<Option<EventRecurrenceReport>> {
    let Some(frequency) = args.repeat else {
        if !event_recurrence_args_is_empty(args) {
            bail!("event recurrence options require --repeat");
        }
        return Ok(None);
    };
    let interval = args.interval.unwrap_or(1);
    if interval == 0 {
        bail!("--repeat-interval must be greater than zero");
    }
    isize::try_from(interval).context("--repeat-interval is too large for EventKit")?;
    if args.count == Some(0) {
        bail!("--repeat-count must be greater than zero");
    }
    if frequency == EventRepeatArg::Daily
        && (!args.weekdays.is_empty() || !args.month_days.is_empty())
    {
        bail!("daily recurrence cannot use --repeat-weekday or --repeat-month-day");
    }
    if frequency != EventRepeatArg::Monthly && !args.month_days.is_empty() {
        bail!("--repeat-month-day requires --repeat monthly");
    }
    if !args.weekdays.is_empty() && !args.month_days.is_empty() {
        bail!("--repeat-weekday and --repeat-month-day cannot be combined");
    }
    let mut month_days = args.month_days.clone();
    month_days.sort_unstable();
    month_days.dedup();
    if let Some(value) = month_days
        .iter()
        .find(|value| **value == 0 || value.unsigned_abs() > 31)
    {
        bail!("--repeat-month-day must be from 1 through 31 or -1 through -31: {value}");
    }
    let mut weekdays = args
        .weekdays
        .iter()
        .map(|value| EventRecurrenceWeekdayReport {
            weekday: event_weekday_number(*value),
            week_number: 0,
        })
        .collect::<Vec<_>>();
    weekdays.sort_by_key(|value| value.weekday);
    weekdays.dedup_by_key(|value| value.weekday);
    let start = parse_start_datetime_in_time_zone(start_input, time_zone)
        .context("invalid recurrence anchor")?;
    let end = if let Some(count) = args.count {
        EventRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(count),
            end_date: None,
        }
    } else if let Some(until) = args.until.as_deref() {
        let until = DateTime::parse_from_rfc3339(until)
            .context("--repeat-until must be RFC3339 with an explicit UTC offset")?
            .with_timezone(&Local);
        let stored_until = canonical_eventkit_recurrence_end_utc(&until.to_rfc3339())?;
        let stored_until_local = DateTime::parse_from_rfc3339(&stored_until)
            .context("canonical EventKit recurrence end must be RFC3339")?
            .with_timezone(&Local);
        if stored_until_local < start {
            bail!("--repeat-until must not be before the event start");
        }
        EventRecurrenceEndReport {
            kind: "date".to_string(),
            occurrence_count: None,
            end_date: Some(stored_until),
        }
    } else {
        EventRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        }
    };
    Ok(Some(EventRecurrenceReport {
        frequency: match frequency {
            EventRepeatArg::Daily => "daily",
            EventRepeatArg::Weekly => "weekly",
            EventRepeatArg::Monthly => "monthly",
            EventRepeatArg::Yearly => "yearly",
        }
        .to_string(),
        interval,
        first_day_of_week: if frequency == EventRepeatArg::Weekly && interval > 1 {
            2
        } else {
            0
        },
        end,
        days_of_week: (!weekdays.is_empty()).then_some(weekdays),
        days_of_month: (!month_days.is_empty()).then_some(month_days),
        months_of_year: None,
        weeks_of_year: None,
        days_of_year: None,
        set_positions: None,
    }))
}

fn event_weekday_number(value: EventWeekdayArg) -> isize {
    match value {
        EventWeekdayArg::Sunday => 1,
        EventWeekdayArg::Monday => 2,
        EventWeekdayArg::Tuesday => 3,
        EventWeekdayArg::Wednesday => 4,
        EventWeekdayArg::Thursday => 5,
        EventWeekdayArg::Friday => 6,
        EventWeekdayArg::Saturday => 7,
    }
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
    let start = parse_start_datetime_in_time_zone(&input.start, input.time_zone.as_deref())
        .with_context(|| format!("invalid --start: {}", input.start))?;
    let end = parse_end_datetime_in_time_zone(&input.end, input.time_zone.as_deref())
        .with_context(|| format!("invalid --end: {}", input.end))?;
    ensure_valid_event_range(start, end)?;
    validate_alarm_minutes(&input.alarm_minutes_before)?;
    if let Some(url) = input.url.as_deref() {
        validate_event_url(url)?;
    }

    let selection = calendar_selection_for_write(&input.calendar_selector);
    let events = authorized_events_manager()?;
    let target = resolve_target_calendar(&events, &input.calendar_selector, true)?;
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

fn duplicate_recurrence_policy_error(
    input: &AddEventInput,
    duplicates: &[EventItem],
) -> Result<Option<String>> {
    let [existing] = duplicates else {
        return Ok(None);
    };
    let details = read_event_details(&existing.identifier, Some(existing.start_date))?;
    let recurrence_matches =
        recurrence_rules_match(input.recurrence.as_ref(), &details.recurrence_rules);
    if !recurrence_matches {
        return Ok(Some(format!(
            "matching event [{}] has a different recurrence rule; refusing to treat it as the same event",
            existing.identifier
        )));
    }
    if input.if_exists == IfExistsArg::Update && !details.recurrence_rules.is_empty() {
        return Ok(Some(
            "--if-exists update for a recurring match requires explicit series scope; use skip for an identical rule or wait for Issue 14 Phase 4"
                .to_string(),
        ));
    }
    Ok(None)
}

pub(crate) fn recurrence_rules_match(
    requested: Option<&EventRecurrenceReport>,
    existing: &[EventRecurrenceReport],
) -> bool {
    match requested {
        Some(requested) => {
            existing.len() == 1
                && existing.first().is_some_and(|existing| {
                    canonical_recurrence(existing) == canonical_recurrence(requested)
                })
        }
        None => existing.is_empty(),
    }
}

fn canonical_recurrence(rule: &EventRecurrenceReport) -> EventRecurrenceReport {
    let mut rule = rule.clone();
    rule.first_day_of_week = if rule.frequency == "weekly" && rule.interval > 1 {
        if rule.first_day_of_week == 0 {
            2
        } else {
            rule.first_day_of_week
        }
    } else {
        0
    };
    if let Some(end_date) = rule.end.end_date.as_deref()
        && let Ok(value) = canonical_eventkit_recurrence_end_utc(end_date)
    {
        rule.end.end_date = Some(value);
    }
    normalize_optional_vec(&mut rule.days_of_week);
    normalize_optional_vec(&mut rule.days_of_month);
    normalize_optional_vec(&mut rule.months_of_year);
    normalize_optional_vec(&mut rule.weeks_of_year);
    normalize_optional_vec(&mut rule.days_of_year);
    normalize_optional_vec(&mut rule.set_positions);
    rule
}

fn normalize_optional_vec<T: Ord>(values: &mut Option<Vec<T>>) {
    if let Some(values) = values {
        values.sort_unstable();
        values.dedup();
    }
    if values.as_ref().is_some_and(Vec::is_empty) {
        *values = None;
    }
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
    let id = resolve_event_ref(&input.id)?;
    let events = authorized_events_manager()?;
    let current = events
        .get_event(&id)
        .with_context(|| format!("failed to load event {id}"))?;

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
        let existing_alarm_count = events
            .get_event_alarms(&id)
            .with_context(|| format!("failed to read alarms for event {id}"))?
            .len();
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
            recurrence: None,
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

    if has_event_patch {
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
            ..Default::default()
        };

        events
            .update_event(&id, &patch)
            .with_context(|| format!("failed to update event {id}"))?;
    }

    let id = if target.is_some() || time_zone.is_some() {
        update_event_calendar_metadata(
            &id,
            target.as_ref().map(|target| target.identifier.as_str()),
            time_zone,
        )
        .with_context(|| format!("failed to update calendar metadata for event {id}"))?
    } else {
        id
    };

    add_relative_alarms(&events, &id, &input.add_alarm_minutes_before)?;
    let mut report = event_report_with_alarms(&events, &id, None)?;
    report.write_action = Some("updated".to_string());
    report.start_input = input.start;
    report.end_input = input.end;
    Ok(WriteEventResult::Written(Box::new(report)))
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

fn nullable_patch(value: Option<&str>, clear: bool) -> Option<Option<&str>> {
    if clear { Some(None) } else { value.map(Some) }
}

pub(crate) fn resolve_target_calendar(
    events: &EventsManager,
    args: &WriteCalendarSelectorArgs,
    allow_default: bool,
) -> Result<CalendarInfo> {
    let calendar = if write_selector_is_empty(args) {
        if !allow_default {
            bail!("a calendar selector is required");
        }
        events
            .default_calendar()
            .context("no default calendar is available for new events")?
    } else {
        let calendars = list_calendars(events)?;
        let titles: Vec<String> = args.calendar.iter().cloned().collect();
        let ids: Vec<String> = args.calendar_id.iter().cloned().collect();
        require_single_writable_calendar(
            &calendars,
            &CalendarSelector {
                titles: &titles,
                ids: &ids,
                source: args.calendar_source.as_deref(),
                source_id: args.source_id.as_deref(),
            },
        )?
    };

    Ok(calendar)
}

fn write_selector_is_empty(args: &WriteCalendarSelectorArgs) -> bool {
    args.calendar.is_none() && args.calendar_id.is_none()
}

fn calendar_selection_for_write(args: &WriteCalendarSelectorArgs) -> CalendarSelection {
    if write_selector_is_empty(args) {
        CalendarSelection::EventkitDefault
    } else {
        CalendarSelection::Explicit
    }
}

fn calendar_reports(calendars: &[CalendarInfo], default_id: Option<&str>) -> Vec<CalendarReport> {
    calendars
        .iter()
        .map(|calendar| {
            let mut report = CalendarReport::from(calendar);
            report.is_default_for_new_events = default_id == Some(calendar.identifier.as_str());
            report
        })
        .collect()
}

fn filter_calendar_list(
    calendars: Vec<CalendarInfo>,
    source: Option<&str>,
    writable_only: bool,
) -> Vec<CalendarInfo> {
    calendars
        .into_iter()
        .filter(|calendar| source.is_none_or(|source| calendar.source.as_deref() == Some(source)))
        .filter(|calendar| !writable_only || calendar.allows_modifications)
        .collect()
}

fn event_calendar(events: &EventsManager, event: &EventItem) -> Result<CalendarInfo> {
    let calendars = list_calendars(events)?;
    event
        .calendar_id
        .as_deref()
        .and_then(|id| calendars.iter().find(|calendar| calendar.identifier == id))
        .or_else(|| {
            event
                .calendar_title
                .as_deref()
                .and_then(|title| calendars.iter().find(|calendar| calendar.title == title))
        })
        .cloned()
        .context("event calendar is no longer available")
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

struct DuplicateQuery<'a> {
    title: &'a str,
    start: DateTime<Local>,
    end: DateTime<Local>,
    all_day: bool,
    calendar_id: &'a str,
    excluded_event_id: Option<&'a str>,
    window_seconds: i64,
}

fn matching_events(events: &EventsManager, query: &DuplicateQuery<'_>) -> Result<Vec<EventItem>> {
    let fetch_padding = query.window_seconds.max(1);
    let candidates = events
        .fetch_events(
            query.start - chrono::Duration::seconds(fetch_padding),
            query.end + chrono::Duration::seconds(fetch_padding),
            None,
        )
        .context("failed to check for duplicate events")?;

    Ok(candidates
        .into_iter()
        .filter(|event| query.excluded_event_id != Some(event.identifier.as_str()))
        .filter(|event| event.title == query.title)
        .filter(|event| event.all_day == query.all_day)
        .filter(|event| {
            datetime_within_window(event.start_date, query.start, query.window_seconds)
                && datetime_within_window(event.end_date, query.end, query.window_seconds)
        })
        .filter(|event| event.calendar_id.as_deref() == Some(query.calendar_id))
        .collect())
}

fn datetime_within_window(
    candidate: DateTime<Local>,
    expected: DateTime<Local>,
    window_seconds: i64,
) -> bool {
    let limit_milliseconds = window_seconds.checked_mul(1_000).unwrap_or(i64::MAX);
    (candidate - expected).num_milliseconds().abs() <= limit_milliseconds
}

fn duplicate_warnings(events: &[EventItem], window_seconds: i64) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            format!(
                "possible duplicate within {window_seconds} seconds: {:?} at {} [{}]",
                event.title,
                event.start_date.to_rfc3339(),
                event.identifier
            )
        })
        .collect()
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

fn event_report_with_alarms(
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

fn parse_occurrence_start(value: &str) -> Result<DateTime<Local>> {
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

fn find_event_occurrence(
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

fn fetch_range(
    from: &str,
    to: &str,
    selector: &ReadCalendarSelectorArgs,
) -> Result<Vec<EventReport>> {
    let start = parse_start_datetime(from).with_context(|| format!("invalid --from: {from}"))?;
    let end = parse_end_datetime(to).with_context(|| format!("invalid --to: {to}"))?;
    fetch_events(start, end, selector)
}

fn fetch_events(
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

fn list_calendars(events: &EventsManager) -> Result<Vec<CalendarInfo>> {
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
    use eventkit::CalendarType;
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
            calendar_selection_for_write(&implicit),
            CalendarSelection::EventkitDefault
        );
        assert_eq!(
            calendar_selection_for_write(&explicit),
            CalendarSelection::Explicit
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
