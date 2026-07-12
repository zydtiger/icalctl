use crate::calendar::{
    authorized_events_manager, availability_name, ensure_availability_supported,
    ensure_valid_event_range, parse_event_recurrence, recurrence_rules_match,
    replace_relative_alarms, resolve_target_calendar_with_selection, validate_alarm_minutes,
    validate_recurring_all_day_inputs,
};
use crate::cli::{
    AvailabilityArg, EventJsonRecurrence, EventRecurrenceArgs, IfExistsArg,
    WriteCalendarSelectorArgs,
};
use crate::dates::{
    datetime_in_time_zone, parse_end_datetime_in_time_zone, parse_start_datetime_in_time_zone,
    utc_datetime, validate_time_zone,
};
use crate::eventkit_bridge::{
    create_event_in_calendar, read_event_details, update_event_calendar_metadata,
    validate_event_time_zone, validate_event_url,
};
use crate::models::{
    BatchErrorReport, BatchItemReport, BatchReport, BatchSummaryReport, CalendarSelection,
    EventDraftReport, EventRecurrenceReport,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use eventkit::{CalendarInfo, EventAvailability, EventDraft, EventItem, EventPatch, EventsManager};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const BATCH_VERSION: u8 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchEnvelope {
    version: u8,
    #[serde(default)]
    defaults: BatchDefaults,
    events: Vec<Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchDefaults {
    calendar: Option<String>,
    calendar_id: Option<String>,
    calendar_source: Option<String>,
    source_id: Option<String>,
    time_zone: Option<String>,
    availability: Option<AvailabilityArg>,
    all_day: Option<bool>,
    alarm_minutes_before: Option<Vec<i64>>,
    recurrence: Option<EventJsonRecurrence>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchEvent {
    client_id: Option<String>,
    title: String,
    start: String,
    end: String,
    calendar: Option<String>,
    calendar_id: Option<String>,
    calendar_source: Option<String>,
    source_id: Option<String>,
    #[serde(default)]
    notes: PatchValue<String>,
    #[serde(default)]
    location: PatchValue<String>,
    #[serde(default)]
    url: PatchValue<String>,
    #[serde(default)]
    time_zone: PatchValue<String>,
    availability: Option<AvailabilityArg>,
    all_day: Option<bool>,
    alarm_minutes_before: Option<Vec<i64>>,
    #[serde(default)]
    recurrence: PatchValue<EventJsonRecurrence>,
}

#[derive(Clone, Debug, Default)]
enum PatchValue<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for PatchValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct EventIdentity {
    calendar_id: String,
    title: String,
    start_utc: String,
    end_utc: String,
    all_day: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlannedAction {
    Create,
    Skip,
    Update,
}

struct PreparedEvent {
    identity: EventIdentity,
    client_id: Option<String>,
    title: String,
    start_input: String,
    end_input: String,
    start: DateTime<Local>,
    end: DateTime<Local>,
    calendar: CalendarInfo,
    calendar_selection: CalendarSelection,
    notes: PatchValue<String>,
    location: PatchValue<String>,
    url: PatchValue<String>,
    time_zone: PatchValue<String>,
    availability: Option<EventAvailability>,
    all_day: bool,
    all_day_patch: Option<bool>,
    alarms: Option<Vec<i64>>,
    recurrence: Option<EventRecurrenceReport>,
    matched: Option<EventItem>,
    action: Option<PlannedAction>,
    policy_error: Option<String>,
}

struct BatchSlot {
    index: usize,
    client_id: Option<String>,
    event: Option<BatchEvent>,
    prepared: Option<PreparedEvent>,
    error: Option<String>,
}

pub fn run_batch_add(
    path: &Path,
    if_exists: IfExistsArg,
    dry_run: bool,
    continue_on_error: bool,
) -> Result<BatchReport> {
    let contents =
        fs::read(path).with_context(|| format!("failed to read batch file {}", path.display()))?;
    let envelope: BatchEnvelope = serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse batch file {}", path.display()))?;
    if envelope.version != BATCH_VERSION {
        bail!(
            "unsupported batch version {}; expected {}",
            envelope.version,
            BATCH_VERSION
        );
    }
    validate_defaults(&envelope.defaults)?;

    let mut slots = parse_event_slots(envelope.events);
    mark_duplicate_client_ids(&mut slots);

    let events = authorized_events_manager()?;
    for slot in &mut slots {
        if slot.error.is_some() {
            continue;
        }
        let event = slot.event.take().expect("validated batch event is present");
        slot.client_id = event.client_id.clone();
        match prepare_event(&events, &envelope.defaults, event, if_exists) {
            Ok(prepared) => {
                if let Some(error) = prepared.policy_error.clone() {
                    slot.error = Some(error);
                }
                slot.prepared = Some(prepared);
            }
            Err(error) => slot.error = Some(format!("{error:#}")),
        }
    }
    mark_duplicate_identities(&mut slots);

    let has_preflight_errors = slots.iter().any(|slot| slot.error.is_some());
    let can_write = !has_preflight_errors || continue_on_error;

    let items = if dry_run {
        dry_run_reports(&events, &slots)
    } else if has_preflight_errors && !continue_on_error {
        blocked_reports(&events, &slots)
    } else {
        execute_reports(&events, &slots, continue_on_error)
    };
    let summary = summarize(&items);

    Ok(BatchReport {
        dry_run,
        can_write,
        if_exists: if_exists_name(if_exists).to_string(),
        continue_on_error,
        summary,
        items,
    })
}

fn parse_event_slots(values: Vec<Value>) -> Vec<BatchSlot> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let client_id = value
                .get("client_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            match serde_json::from_value::<BatchEvent>(value) {
                Ok(event) => BatchSlot {
                    index,
                    client_id: event.client_id.clone(),
                    event: Some(event),
                    prepared: None,
                    error: None,
                },
                Err(error) => BatchSlot {
                    index,
                    client_id,
                    event: None,
                    prepared: None,
                    error: Some(format!("invalid event: {error}")),
                },
            }
        })
        .collect()
}

fn mark_duplicate_client_ids(slots: &mut [BatchSlot]) {
    let mut positions: HashMap<String, Vec<usize>> = HashMap::new();
    for (position, slot) in slots.iter().enumerate() {
        if let Some(client_id) = &slot.client_id {
            positions
                .entry(client_id.clone())
                .or_default()
                .push(position);
        }
    }
    for (client_id, positions) in positions {
        if positions.len() > 1 {
            for position in positions {
                slots[position].error = Some(format!("duplicate client_id: {client_id:?}"));
            }
        }
    }
}

fn mark_duplicate_identities(slots: &mut [BatchSlot]) {
    let mut positions: HashMap<EventIdentity, Vec<usize>> = HashMap::new();
    for (position, slot) in slots.iter().enumerate() {
        if slot.error.is_none()
            && let Some(prepared) = &slot.prepared
        {
            positions
                .entry(prepared.identity.clone())
                .or_default()
                .push(position);
        }
    }
    for positions in positions.into_values() {
        if positions.len() > 1 {
            let rows = positions
                .iter()
                .map(|position| (slots[*position].index + 1).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            for position in positions {
                slots[position].error = Some(format!(
                    "duplicate event identity within batch at rows {rows}"
                ));
            }
        }
    }
}

fn prepare_event(
    events: &EventsManager,
    defaults: &BatchDefaults,
    event: BatchEvent,
    if_exists: IfExistsArg,
) -> Result<PreparedEvent> {
    let selector = merged_calendar_selector(defaults, &event)?;
    let (calendar, calendar_selection) =
        resolve_target_calendar_with_selection(events, &selector, true)?;

    let time_zone = merged_time_zone(defaults, &event.time_zone);
    if let PatchValue::Value(value) = &time_zone {
        validate_time_zone(value)?;
        validate_event_time_zone(value)?;
    }
    let parse_time_zone = match &time_zone {
        PatchValue::Value(value) => Some(value.as_str()),
        PatchValue::Missing | PatchValue::Null => None,
    };
    let start = parse_start_datetime_in_time_zone(&event.start, parse_time_zone)
        .with_context(|| format!("invalid start: {}", event.start))?;
    let end = parse_end_datetime_in_time_zone(&event.end, parse_time_zone)
        .with_context(|| format!("invalid end: {}", event.end))?;
    ensure_valid_event_range(start, end)?;

    let alarms = event
        .alarm_minutes_before
        .clone()
        .or_else(|| defaults.alarm_minutes_before.clone());
    if let Some(alarms) = &alarms {
        validate_alarm_minutes(alarms)?;
    }
    if let PatchValue::Value(url) = &event.url {
        validate_event_url(url)?;
    }
    let availability_arg = event.availability.or(defaults.availability);
    let availability = availability_arg.map(EventAvailability::from);
    ensure_availability_supported(&calendar, availability)?;
    let all_day_patch = event.all_day.or(defaults.all_day);
    let all_day = all_day_patch.unwrap_or(false);
    let recurrence_input = merged_recurrence(defaults, &event.recurrence);
    let recurrence = match recurrence_input {
        PatchValue::Value(value) => {
            let args = EventRecurrenceArgs::from(value);
            parse_event_recurrence(&args, &event.start, parse_time_zone)?
        }
        PatchValue::Missing | PatchValue::Null => None,
    };
    validate_recurring_all_day_inputs(all_day, recurrence.is_some(), &event.start, &event.end)?;

    let matches = matching_events(
        events,
        &event.title,
        start,
        end,
        all_day,
        &calendar.identifier,
    )?;
    let (matched, action, policy_error) = match matches.len() {
        0 => (None, Some(PlannedAction::Create), None),
        1 => {
            let matched = matches.into_iter().next();
            let (action, mut policy_error) = existing_match_action(
                if_exists,
                &matched.as_ref().expect("match is present").identifier,
            );
            let existing = matched.as_ref().expect("match is present");
            let details = read_event_details(&existing.identifier, Some(existing.start_date))?;
            let recurrence_matches =
                recurrence_rules_match(recurrence.as_ref(), &details.recurrence_rules);
            if !recurrence_matches {
                policy_error = Some(format!(
                    "matching event [{}] has a different recurrence rule",
                    existing.identifier
                ));
            } else if if_exists == IfExistsArg::Update && !details.recurrence_rules.is_empty() {
                policy_error = Some(
                    "recurring batch update requires explicit series scope; use skip for an identical rule"
                        .to_string(),
                );
            }
            (matched, action, policy_error)
        }
        count => {
            let ids = matches
                .iter()
                .map(|event| event.identifier.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("{count} existing events match this identity; matching ids: {ids}");
        }
    };
    let identity = EventIdentity {
        calendar_id: calendar.identifier.clone(),
        title: event.title.clone(),
        start_utc: utc_datetime(start),
        end_utc: utc_datetime(end),
        all_day,
    };

    Ok(PreparedEvent {
        identity,
        client_id: event.client_id,
        title: event.title,
        start_input: event.start,
        end_input: event.end,
        start,
        end,
        calendar,
        calendar_selection,
        notes: event.notes,
        location: event.location,
        url: event.url,
        time_zone,
        availability,
        all_day,
        all_day_patch,
        alarms,
        recurrence,
        matched,
        action,
        policy_error,
    })
}

fn existing_match_action(
    if_exists: IfExistsArg,
    event_id: &str,
) -> (Option<PlannedAction>, Option<String>) {
    match if_exists {
        IfExistsArg::Skip => (Some(PlannedAction::Skip), None),
        IfExistsArg::Update => (Some(PlannedAction::Update), None),
        IfExistsArg::Error => (
            None,
            Some(format!(
                "matching event already exists [{event_id}]; pass --if-exists skip or --if-exists update"
            )),
        ),
    }
}

fn validate_defaults(defaults: &BatchDefaults) -> Result<()> {
    validate_selector_fields(
        defaults.calendar.as_deref(),
        defaults.calendar_id.as_deref(),
        defaults.calendar_source.as_deref(),
        defaults.source_id.as_deref(),
    )?;
    if let Some(time_zone) = defaults.time_zone.as_deref() {
        validate_time_zone(time_zone)?;
        validate_event_time_zone(time_zone)?;
    }
    if let Some(alarms) = &defaults.alarm_minutes_before {
        validate_alarm_minutes(alarms)?;
    }
    Ok(())
}

fn merged_calendar_selector(
    defaults: &BatchDefaults,
    event: &BatchEvent,
) -> Result<WriteCalendarSelectorArgs> {
    let event_has_selector = event.calendar.is_some()
        || event.calendar_id.is_some()
        || event.calendar_source.is_some()
        || event.source_id.is_some();
    let (calendar, calendar_id, calendar_source, source_id) = if event_has_selector {
        (
            event.calendar.clone(),
            event.calendar_id.clone(),
            event.calendar_source.clone(),
            event.source_id.clone(),
        )
    } else {
        (
            defaults.calendar.clone(),
            defaults.calendar_id.clone(),
            defaults.calendar_source.clone(),
            defaults.source_id.clone(),
        )
    };
    validate_selector_fields(
        calendar.as_deref(),
        calendar_id.as_deref(),
        calendar_source.as_deref(),
        source_id.as_deref(),
    )?;
    Ok(WriteCalendarSelectorArgs {
        calendar,
        calendar_id,
        calendar_source,
        source_id,
    })
}

fn validate_selector_fields(
    calendar: Option<&str>,
    calendar_id: Option<&str>,
    calendar_source: Option<&str>,
    source_id: Option<&str>,
) -> Result<()> {
    if calendar_id.is_some()
        && (calendar.is_some() || calendar_source.is_some() || source_id.is_some())
    {
        bail!("calendar_id cannot be combined with calendar, calendar_source, or source_id");
    }
    if calendar_source.is_some() && source_id.is_some() {
        bail!("calendar_source and source_id cannot be combined");
    }
    if (calendar_source.is_some() || source_id.is_some()) && calendar.is_none() {
        bail!("calendar_source and source_id require calendar");
    }
    Ok(())
}

fn merged_time_zone(defaults: &BatchDefaults, event: &PatchValue<String>) -> PatchValue<String> {
    match event {
        PatchValue::Missing => defaults
            .time_zone
            .clone()
            .map(PatchValue::Value)
            .unwrap_or(PatchValue::Missing),
        PatchValue::Null => PatchValue::Null,
        PatchValue::Value(value) => PatchValue::Value(value.clone()),
    }
}

fn merged_recurrence(
    defaults: &BatchDefaults,
    event: &PatchValue<EventJsonRecurrence>,
) -> PatchValue<EventJsonRecurrence> {
    match event {
        PatchValue::Missing => defaults
            .recurrence
            .clone()
            .map(PatchValue::Value)
            .unwrap_or(PatchValue::Missing),
        PatchValue::Null => PatchValue::Null,
        PatchValue::Value(value) => PatchValue::Value(value.clone()),
    }
}

fn matching_events(
    events: &EventsManager,
    title: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
    all_day: bool,
    calendar_id: &str,
) -> Result<Vec<EventItem>> {
    Ok(events
        .fetch_events(
            start - chrono::Duration::seconds(1),
            end + chrono::Duration::seconds(1),
            None,
        )
        .context("failed to check for existing events")?
        .into_iter()
        .filter(|event| event.title == title)
        .filter(|event| event.start_date == start && event.end_date == end)
        .filter(|event| event.all_day == all_day)
        .filter(|event| event.calendar_id.as_deref() == Some(calendar_id))
        .collect())
}

fn dry_run_reports(events: &EventsManager, slots: &[BatchSlot]) -> Vec<BatchItemReport> {
    slots
        .iter()
        .map(|slot| {
            if let Some(error) = &slot.error {
                return failed_report(events, slot, error.clone());
            }
            let prepared = slot.prepared.as_ref().expect("prepared event is present");
            let status = match prepared.action.expect("planned action is present") {
                PlannedAction::Create => "would_create",
                PlannedAction::Skip => "would_skip",
                PlannedAction::Update => "would_update",
            };
            planned_report(events, slot, status, None)
        })
        .collect()
}

fn blocked_reports(events: &EventsManager, slots: &[BatchSlot]) -> Vec<BatchItemReport> {
    slots
        .iter()
        .map(|slot| {
            if let Some(error) = &slot.error {
                failed_report(events, slot, error.clone())
            } else {
                planned_report(
                    events,
                    slot,
                    "not_attempted",
                    Some("batch blocked by preflight errors".to_string()),
                )
            }
        })
        .collect()
}

fn execute_reports(
    events: &EventsManager,
    slots: &[BatchSlot],
    continue_on_error: bool,
) -> Vec<BatchItemReport> {
    let mut stopped = false;
    let mut reports = Vec::with_capacity(slots.len());
    for slot in slots {
        if let Some(error) = &slot.error {
            reports.push(failed_report(events, slot, error.clone()));
            continue;
        }
        if stopped {
            reports.push(planned_report(
                events,
                slot,
                "not_attempted",
                Some("not attempted after an earlier write failure".to_string()),
            ));
            continue;
        }
        let prepared = slot.prepared.as_ref().expect("prepared event is present");
        let (status, event_id, error) = execute_prepared(events, prepared);
        if error.is_some() && !continue_on_error {
            stopped = true;
        }
        reports.push(BatchItemReport {
            index: slot.index,
            client_id: prepared.client_id.clone(),
            status: status.to_string(),
            event_id,
            matched_event_id: prepared
                .matched
                .as_ref()
                .map(|event| event.identifier.clone()),
            draft: draft_report(events, prepared).ok().map(Box::new),
            error: error.map(|message| BatchErrorReport { message }),
        });
    }
    reports
}

fn execute_prepared(
    events: &EventsManager,
    prepared: &PreparedEvent,
) -> (&'static str, Option<String>, Option<String>) {
    match prepared.action.expect("planned action is present") {
        PlannedAction::Skip => {
            let id = prepared
                .matched
                .as_ref()
                .expect("skip has a match")
                .identifier
                .clone();
            ("skipped", Some(id), None)
        }
        PlannedAction::Create => match create_prepared(prepared) {
            Ok(id) => ("created", Some(id), None),
            Err((id, error)) => ("failed", id, Some(format!("{error:#}"))),
        },
        PlannedAction::Update => {
            let id = prepared
                .matched
                .as_ref()
                .expect("update has a match")
                .identifier
                .clone();
            match update_prepared(events, prepared, &id) {
                Ok(id) => ("updated", Some(id), None),
                Err(error) => ("failed", Some(id), Some(format!("{error:#}"))),
            }
        }
    }
}

fn create_prepared(
    prepared: &PreparedEvent,
) -> std::result::Result<String, (Option<String>, anyhow::Error)> {
    let draft = EventDraft {
        title: &prepared.title,
        start: Some(prepared.start),
        end: Some(prepared.end),
        notes: patch_value(&prepared.notes),
        location: patch_value(&prepared.location),
        calendar_title: None,
        all_day: prepared.all_day,
        URL: patch_value(&prepared.url),
        availability: prepared.availability,
        ..Default::default()
    };
    let time_zone = patch_value(&prepared.time_zone);
    let id = create_event_in_calendar(
        &draft,
        &prepared.calendar.identifier,
        time_zone,
        prepared.recurrence.as_ref(),
        prepared.alarms.as_deref().unwrap_or_default(),
    )
    .context("failed to create event through EventKit")
    .map_err(|error| (None, error))?;
    Ok(id)
}

fn update_prepared(events: &EventsManager, prepared: &PreparedEvent, id: &str) -> Result<String> {
    let notes = patch_ref(&prepared.notes);
    let location = patch_ref(&prepared.location);
    let url = patch_ref(&prepared.url);
    let has_event_patch = notes.is_some()
        || location.is_some()
        || url.is_some()
        || prepared.all_day_patch.is_some()
        || prepared.availability.is_some();
    if has_event_patch {
        let patch = EventPatch {
            notes,
            location,
            URL: url,
            all_day: prepared.all_day_patch,
            availability: prepared.availability,
            ..Default::default()
        };
        events
            .update_event(id, &patch)
            .with_context(|| format!("failed to update event {id}"))?;
    }

    let time_zone = patch_ref(&prepared.time_zone);
    let id = if time_zone.is_some() {
        update_event_calendar_metadata(id, None, time_zone)
            .with_context(|| format!("failed to update timezone for event {id}"))?
    } else {
        id.to_string()
    };
    if let Some(alarms) = &prepared.alarms {
        replace_relative_alarms(events, &id, alarms)?;
    }
    Ok(id)
}

fn planned_report(
    events: &EventsManager,
    slot: &BatchSlot,
    status: &str,
    error: Option<String>,
) -> BatchItemReport {
    let prepared = slot.prepared.as_ref().expect("prepared event is present");
    BatchItemReport {
        index: slot.index,
        client_id: prepared.client_id.clone(),
        status: status.to_string(),
        event_id: prepared
            .matched
            .as_ref()
            .map(|event| event.identifier.clone()),
        matched_event_id: prepared
            .matched
            .as_ref()
            .map(|event| event.identifier.clone()),
        draft: draft_report(events, prepared).ok().map(Box::new),
        error: error.map(|message| BatchErrorReport { message }),
    }
}

fn failed_report(events: &EventsManager, slot: &BatchSlot, error: String) -> BatchItemReport {
    let matched_event_id = slot
        .prepared
        .as_ref()
        .and_then(|prepared| prepared.matched.as_ref())
        .map(|event| event.identifier.clone());
    BatchItemReport {
        index: slot.index,
        client_id: slot.client_id.clone(),
        status: "failed".to_string(),
        event_id: matched_event_id.clone(),
        matched_event_id,
        draft: slot
            .prepared
            .as_ref()
            .and_then(|prepared| draft_report(events, prepared).ok())
            .map(Box::new),
        error: Some(BatchErrorReport { message: error }),
    }
}

fn draft_report(events: &EventsManager, prepared: &PreparedEvent) -> Result<EventDraftReport> {
    let current = prepared.matched.as_ref();
    let effective_time_zone = match &prepared.time_zone {
        PatchValue::Value(value) => Some(value.as_str()),
        PatchValue::Null => None,
        PatchValue::Missing => current.and_then(|event| event.timezone.as_deref()),
    };
    let start_in_event_time_zone = effective_time_zone
        .map(|time_zone| datetime_in_time_zone(prepared.start, time_zone))
        .transpose()?;
    let end_in_event_time_zone = effective_time_zone
        .map(|time_zone| datetime_in_time_zone(prepared.end, time_zone))
        .transpose()?;
    let availability = prepared
        .availability
        .or_else(|| current.map(|event| event.availability));
    let alarm_count = match &prepared.alarms {
        Some(alarms) => alarms.len(),
        None => current
            .map(|event| events.get_event_alarms(&event.identifier))
            .transpose()
            .context("failed to read existing alarms")?
            .map_or(0, |alarms| alarms.len()),
    };

    Ok(EventDraftReport {
        operation: match prepared.action {
            Some(PlannedAction::Create) => "batch_add",
            Some(PlannedAction::Skip) => "batch_skip",
            Some(PlannedAction::Update) => "batch_update",
            None => "batch_error",
        }
        .to_string(),
        scope: None,
        event_id: current.map(|event| event.identifier.clone()),
        title: prepared.title.clone(),
        start: prepared.start.to_rfc3339(),
        end: prepared.end.to_rfc3339(),
        start_input: Some(prepared.start_input.clone()),
        end_input: Some(prepared.end_input.clone()),
        start_utc: utc_datetime(prepared.start),
        end_utc: utc_datetime(prepared.end),
        start_local: prepared.start.to_rfc3339(),
        end_local: prepared.end.to_rfc3339(),
        start_in_event_time_zone,
        end_in_event_time_zone,
        duration_seconds: (prepared.end - prepared.start).num_seconds(),
        all_day: prepared.all_day,
        timed: !prepared.all_day,
        calendar: prepared.calendar.title.clone(),
        calendar_id: prepared.calendar.identifier.clone(),
        calendar_source: prepared.calendar.source.clone(),
        calendar_source_id: prepared.calendar.source_id.clone(),
        calendar_selection: Some(prepared.calendar_selection),
        time_zone: effective_time_zone.map(str::to_string),
        availability: availability
            .map(availability_name)
            .unwrap_or("default")
            .to_string(),
        alarm_count,
        recurrence: prepared.recurrence.clone(),
        has_notes: patched_presence(
            current.and_then(|event| event.notes.as_deref()),
            &prepared.notes,
        ),
        has_location: patched_presence(
            current.and_then(|event| event.location.as_deref()),
            &prepared.location,
        ),
        has_url: patched_presence(
            current.and_then(|event| event.URL.as_deref()),
            &prepared.url,
        ),
        duplicate_warnings: Vec::new(),
    })
}

fn patch_value(value: &PatchValue<String>) -> Option<&str> {
    match value {
        PatchValue::Value(value) => Some(value),
        PatchValue::Missing | PatchValue::Null => None,
    }
}

fn patch_ref(value: &PatchValue<String>) -> Option<Option<&str>> {
    match value {
        PatchValue::Missing => None,
        PatchValue::Null => Some(None),
        PatchValue::Value(value) => Some(Some(value)),
    }
}

fn patched_presence(current: Option<&str>, value: &PatchValue<String>) -> bool {
    match value {
        PatchValue::Missing => current.is_some(),
        PatchValue::Null => false,
        PatchValue::Value(_) => true,
    }
}

fn if_exists_name(value: IfExistsArg) -> &'static str {
    match value {
        IfExistsArg::Skip => "skip",
        IfExistsArg::Update => "update",
        IfExistsArg::Error => "error",
    }
}

fn summarize(items: &[BatchItemReport]) -> BatchSummaryReport {
    let mut summary = BatchSummaryReport {
        total: items.len(),
        ..Default::default()
    };
    for item in items {
        match item.status.as_str() {
            "created" => summary.created += 1,
            "skipped" => summary.skipped += 1,
            "updated" => summary.updated += 1,
            "failed" => summary.failed += 1,
            "not_attempted" => summary.not_attempted += 1,
            "would_create" => summary.would_create += 1,
            "would_skip" => summary.would_skip += 1,
            "would_update" => summary.would_update += 1,
            status => unreachable!("unknown batch status: {status}"),
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::parse_start_datetime;
    use eventkit::CalendarType;
    use serde_json::json;

    fn prepared_event(title: &str) -> PreparedEvent {
        let start = parse_start_datetime("2026-07-10T09:00").unwrap();
        let end = parse_start_datetime("2026-07-10T10:00").unwrap();
        let calendar = CalendarInfo {
            identifier: "CAL-1".to_string(),
            title: "Calendar".to_string(),
            source: Some("iCloud".to_string()),
            source_id: Some("SOURCE-1".to_string()),
            calendar_type: CalendarType::CalDAV,
            allows_modifications: true,
            is_immutable: false,
            is_subscribed: false,
            color: None,
            allowed_entity_types: vec!["event".to_string()],
            supported_event_availabilities: vec!["busy".to_string()],
        };
        PreparedEvent {
            identity: EventIdentity {
                calendar_id: calendar.identifier.clone(),
                title: title.to_string(),
                start_utc: utc_datetime(start),
                end_utc: utc_datetime(end),
                all_day: false,
            },
            client_id: None,
            title: title.to_string(),
            start_input: "2026-07-10T09:00".to_string(),
            end_input: "2026-07-10T10:00".to_string(),
            start,
            end,
            calendar,
            calendar_selection: CalendarSelection::Explicit,
            notes: PatchValue::Missing,
            location: PatchValue::Missing,
            url: PatchValue::Missing,
            time_zone: PatchValue::Missing,
            availability: None,
            all_day: false,
            all_day_patch: None,
            alarms: None,
            recurrence: None,
            matched: None,
            action: Some(PlannedAction::Create),
            policy_error: None,
        }
    }

    #[test]
    fn parses_rows_independently() {
        let slots = parse_event_slots(vec![
            json!({
                "client_id": "valid",
                "title": "Meeting",
                "start": "2026-07-10T09:00",
                "end": "2026-07-10T10:00"
            }),
            json!({"client_id": "invalid", "title": "Missing times"}),
        ]);

        assert!(slots[0].error.is_none());
        assert!(slots[1].error.as_deref().unwrap().contains("missing field"));
        assert_eq!(slots[1].client_id.as_deref(), Some("invalid"));
    }

    #[test]
    fn explicit_null_is_distinct_from_an_omitted_patch_field() {
        let omitted: BatchEvent = serde_json::from_value(json!({
            "title": "Meeting",
            "start": "2026-07-10T09:00",
            "end": "2026-07-10T10:00"
        }))
        .unwrap();
        let cleared: BatchEvent = serde_json::from_value(json!({
            "title": "Meeting",
            "start": "2026-07-10T09:00",
            "end": "2026-07-10T10:00",
            "notes": null
        }))
        .unwrap();

        assert!(matches!(omitted.notes, PatchValue::Missing));
        assert!(matches!(cleared.notes, PatchValue::Null));

        let cleared_recurrence: BatchEvent = serde_json::from_value(json!({
            "title": "Meeting",
            "start": "2026-07-10T09:00",
            "end": "2026-07-10T10:00",
            "recurrence": null
        }))
        .unwrap();
        assert!(matches!(cleared_recurrence.recurrence, PatchValue::Null));
    }

    #[test]
    fn recurring_all_day_validation_uses_effective_batch_defaults_and_overrides() {
        let defaults: BatchDefaults = serde_json::from_value(json!({
            "all_day": true,
            "recurrence": { "frequency": "daily" }
        }))
        .unwrap();
        let event = |extra: Value| {
            let mut value = json!({
                "title": "DST dates",
                "start": "2030-03-30T00:00:00+01:00",
                "end": "2030-03-31T00:00:00+01:00"
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            serde_json::from_value::<BatchEvent>(value).unwrap()
        };
        let validate = |event: &BatchEvent| {
            let all_day = event.all_day.or(defaults.all_day).unwrap_or(false);
            let recurring = matches!(
                merged_recurrence(&defaults, &event.recurrence),
                PatchValue::Value(_)
            );
            validate_recurring_all_day_inputs(all_day, recurring, &event.start, &event.end)
        };

        assert!(validate(&event(json!({}))).is_err());
        assert!(validate(&event(json!({ "recurrence": null }))).is_ok());
        assert!(validate(&event(json!({ "all_day": false }))).is_ok());

        let date_only: BatchEvent = serde_json::from_value(json!({
            "title": "DST dates",
            "start": "2030-03-30",
            "end": "2030-03-30"
        }))
        .unwrap();
        assert!(validate(&date_only).is_ok());

        let ordinary_defaults = BatchDefaults {
            all_day: Some(true),
            ..Default::default()
        };
        assert!(
            validate_recurring_all_day_inputs(
                ordinary_defaults.all_day.unwrap(),
                ordinary_defaults.recurrence.is_some(),
                "2030-03-30T00:00:00+01:00",
                "2030-03-31T00:00:00+01:00"
            )
            .is_ok()
        );
    }

    #[test]
    fn duplicate_client_ids_fail_every_matching_row() {
        let mut slots = parse_event_slots(vec![
            json!({
                "client_id": "same",
                "title": "One",
                "start": "2026-07-10T09:00",
                "end": "2026-07-10T10:00"
            }),
            json!({
                "client_id": "same",
                "title": "Two",
                "start": "2026-07-10T11:00",
                "end": "2026-07-10T12:00"
            }),
        ]);
        mark_duplicate_client_ids(&mut slots);

        assert!(slots.iter().all(|slot| slot.error.is_some()));
    }

    #[test]
    fn duplicate_event_identities_fail_every_matching_row() {
        let mut slots = vec![
            BatchSlot {
                index: 0,
                client_id: None,
                event: None,
                prepared: Some(prepared_event("Meeting")),
                error: None,
            },
            BatchSlot {
                index: 1,
                client_id: None,
                event: None,
                prepared: Some(prepared_event("Meeting")),
                error: None,
            },
        ];
        mark_duplicate_identities(&mut slots);

        assert!(slots.iter().all(|slot| slot.error.is_some()));
        assert!(slots[0].error.as_deref().unwrap().contains("rows 1, 2"));
    }

    #[test]
    fn invalid_shared_defaults_are_rejected_before_writes() {
        let defaults = BatchDefaults {
            alarm_minutes_before: Some(vec![-5]),
            ..Default::default()
        };

        assert!(validate_defaults(&defaults).is_err());
    }

    #[test]
    fn selector_qualifiers_require_a_title() {
        let error = validate_selector_fields(None, None, Some("iCloud"), None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("require calendar"));
    }

    #[test]
    fn summary_counts_each_status() {
        let items = ["created", "skipped", "failed", "would_update"]
            .into_iter()
            .enumerate()
            .map(|(index, status)| BatchItemReport {
                index,
                client_id: None,
                status: status.to_string(),
                event_id: None,
                matched_event_id: None,
                draft: None,
                error: None,
            })
            .collect::<Vec<_>>();
        let summary = summarize(&items);

        assert_eq!(summary.total, 4);
        assert_eq!(summary.created, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.would_update, 1);
    }

    #[test]
    fn existing_match_policy_is_explicit_and_idempotent() {
        let (skip, skip_error) = existing_match_action(IfExistsArg::Skip, "EVENT-1");
        let (update, update_error) = existing_match_action(IfExistsArg::Update, "EVENT-1");
        let (error, error_message) = existing_match_action(IfExistsArg::Error, "EVENT-1");

        assert_eq!(skip, Some(PlannedAction::Skip));
        assert!(skip_error.is_none());
        assert_eq!(update, Some(PlannedAction::Update));
        assert!(update_error.is_none());
        assert!(error.is_none());
        assert!(error_message.unwrap().contains("EVENT-1"));
    }

    #[test]
    fn event_timezone_overrides_or_clears_the_default() {
        let defaults = BatchDefaults {
            time_zone: Some("Europe/Berlin".to_string()),
            ..Default::default()
        };

        assert!(matches!(
            merged_time_zone(&defaults, &PatchValue::Missing),
            PatchValue::Value(value) if value == "Europe/Berlin"
        ));
        assert!(matches!(
            merged_time_zone(&defaults, &PatchValue::Value("Asia/Shanghai".to_string())),
            PatchValue::Value(value) if value == "Asia/Shanghai"
        ));
        assert!(matches!(
            merged_time_zone(&defaults, &PatchValue::Null),
            PatchValue::Null
        ));
    }

    #[test]
    fn unknown_event_fields_are_rejected() {
        let slots = parse_event_slots(vec![json!({
            "title": "Meeting",
            "start": "2026-07-10T09:00",
            "end": "2026-07-10T10:00",
            "surprise": true
        })]);

        assert!(slots[0].error.as_deref().unwrap().contains("unknown field"));
    }
}
