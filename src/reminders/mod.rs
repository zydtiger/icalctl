mod eventkit;
mod query;
mod schedule;
mod selection;
mod store;

use self::eventkit::*;
use self::query::*;
use self::schedule::*;
use self::selection::*;
use self::store::*;

pub(crate) use self::eventkit::ReminderAuthorization;
use crate::cache::resolve_reminder_ref;
use crate::cli::{
    IfExistsArg, ReminderAdvancedScheduleArgs, ReminderBatchCommand, ReminderGeofenceProximityArg,
    ReminderPriorityArg, ReminderRepeatArg, RemindersCommand, WriteReminderListSelectorArgs,
};
use crate::dates::validate_time_zone;
use crate::models::{
    BatchErrorReport, BatchSummaryReport, JsonOutput, ReminderBatchItemReport, ReminderBatchReport,
    ReminderDateKind, ReminderDateReport, ReminderDeletedReport, ReminderDraftReport,
    ReminderListReport, ReminderListSelection, ReminderMutationDraftReport, ReminderReport,
    StatusReport,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, Utc};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_event_kit::EKReminder;
use objc2_foundation::{NSError, NSString, NSURL};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write as IoWrite;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::cli::{ReminderReadFilterArgs, ReminderStateArg};
#[cfg(test)]
use crate::models::ReminderPriority;
#[cfg(test)]
use chrono::Local;
#[cfg(test)]
use objc2_event_kit::{EKAuthorizationStatus, EKEventStore};
#[cfg(test)]
use objc2_foundation::NSDateComponents;

pub fn run(command: RemindersCommand) -> Result<JsonOutput> {
    let store = EventKitReminderStore::new();
    run_with_store(&store, command)
}

fn run_with_store(store: &impl ReminderStore, command: RemindersCommand) -> Result<JsonOutput> {
    match command {
        RemindersCommand::Status => Ok(JsonOutput::ReminderStatus(StatusReport {
            authorization: store.authorization_status().as_str().to_string(),
        })),
        RemindersCommand::Lists {
            source,
            writable_only,
        } => {
            store.ensure_authorized()?;
            let default_id = store.default_list().ok().map(|list| list.id);
            let mut lists = filter_list_discovery(store.lists()?, source.as_deref(), writable_only);
            mark_default_list(&mut lists, default_id.as_deref());
            sort_lists(&mut lists);
            Ok(JsonOutput::ReminderLists { lists })
        }
        RemindersCommand::DefaultList => {
            store.ensure_authorized()?;
            let mut list = store
                .default_list()
                .context("no default reminder list is available for new reminders")?;
            list.is_default_for_new_reminders = true;
            Ok(JsonOutput::DefaultReminderList { list })
        }
        RemindersCommand::List { filters } => list_reminders(store, filters, None),
        RemindersCommand::Search { query, filters } => {
            list_reminders(store, filters, Some(query.as_str()))
        }
        RemindersCommand::Show { id } => {
            store.ensure_authorized()?;
            let id = resolve_reminder_ref(&id)?;
            Ok(JsonOutput::Reminder {
                reminder: Box::new(store.get(&id)?),
            })
        }
        RemindersCommand::Add {
            title,
            json_file,
            list_selector,
            parent_id,
            due,
            start,
            time_zone,
            notes,
            notes_file,
            url,
            location,
            priority,
            notify_at_due,
            notify_minutes_before,
            schedule,
            if_exists,
            duplicate_window_seconds,
            dry_run,
        } => add_reminder_from_cli(
            store,
            AddReminderCliCommand {
                title,
                json_file,
                list_selector,
                parent_id,
                due,
                start,
                time_zone,
                notes,
                notes_file,
                url,
                location,
                priority,
                notify_at_due,
                notify_minutes_before,
                schedule,
                if_exists,
                duplicate_window_seconds,
                dry_run,
            },
        ),
        RemindersCommand::Batch { command } => match command {
            ReminderBatchCommand::Add {
                file,
                if_exists,
                dry_run,
                continue_on_error,
            } => reminder_batch_add(store, &file, if_exists, dry_run, continue_on_error),
        },
        RemindersCommand::Update {
            id,
            title,
            list_selector,
            parent_id,
            clear_parent,
            due,
            clear_due,
            start,
            clear_start,
            time_zone,
            clear_time_zone,
            notes,
            notes_file,
            clear_notes,
            url,
            clear_url,
            location,
            clear_location,
            priority,
            notify_at_due,
            notify_minutes_before,
            schedule,
            clear_notifications,
            clear_recurrence,
            dry_run,
        } => update_reminder(
            store,
            UpdateReminderCommand {
                id,
                title,
                list_selector,
                parent_id,
                clear_parent,
                due,
                clear_due,
                start,
                clear_start,
                time_zone,
                clear_time_zone,
                notes,
                notes_file,
                clear_notes,
                url,
                clear_url,
                location,
                clear_location,
                priority,
                notify_at_due,
                notify_minutes_before,
                schedule,
                clear_notifications,
                clear_recurrence,
                dry_run,
            },
        ),
        RemindersCommand::Complete {
            id,
            completed_at,
            dry_run,
        } => complete_reminder(store, &id, completed_at.as_deref(), dry_run),
        RemindersCommand::Uncomplete { id, dry_run } => uncomplete_reminder(store, &id, dry_run),
        RemindersCommand::Delete { id, force } => delete_reminder(store, &id, force),
    }
}

struct AddReminderCliCommand {
    title: Option<String>,
    json_file: Option<PathBuf>,
    list_selector: WriteReminderListSelectorArgs,
    parent_id: Option<String>,
    due: Option<String>,
    start: Option<String>,
    time_zone: Option<String>,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    url: Option<String>,
    location: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: bool,
    notify_minutes_before: Vec<i64>,
    schedule: ReminderAdvancedScheduleArgs,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Clone)]
struct AddReminderCommand {
    title: String,
    list_selector: WriteReminderListSelectorArgs,
    parent_id: Option<String>,
    due: Option<String>,
    start: Option<String>,
    time_zone: Option<String>,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    url: Option<String>,
    location: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: bool,
    notify_minutes_before: Vec<i64>,
    schedule: ReminderAdvancedScheduleArgs,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Clone)]
struct ReminderSaveDraft {
    title: String,
    list_id: String,
    parent_id: Option<String>,
    due: Option<ParsedReminderDate>,
    start: Option<ParsedReminderDate>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    priority_value: usize,
    notifications: Vec<ParsedReminderNotification>,
    geofence: Option<ParsedReminderGeofence>,
    recurrence: Option<ParsedReminderRecurrence>,
}

#[derive(Clone, Default)]
struct ReminderAddPatch {
    parent_id: Option<String>,
    start: Option<ParsedReminderDate>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    priority_value: Option<usize>,
    notifications: Option<Vec<ParsedReminderNotification>>,
    geofence: Option<Option<ParsedReminderGeofence>>,
    recurrence: Option<ParsedReminderRecurrence>,
}

struct UpdateReminderCommand {
    id: String,
    title: Option<String>,
    list_selector: WriteReminderListSelectorArgs,
    parent_id: Option<String>,
    clear_parent: bool,
    due: Option<String>,
    clear_due: bool,
    start: Option<String>,
    clear_start: bool,
    time_zone: Option<String>,
    clear_time_zone: bool,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    clear_notes: bool,
    url: Option<String>,
    clear_url: bool,
    location: Option<String>,
    clear_location: bool,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: bool,
    notify_minutes_before: Vec<i64>,
    schedule: ReminderAdvancedScheduleArgs,
    clear_notifications: bool,
    clear_recurrence: bool,
    dry_run: bool,
}

#[derive(Clone, Default)]
struct ReminderLifecyclePatch {
    title: Option<String>,
    list_id: Option<String>,
    parent_id: Option<Option<String>>,
    due: Option<Option<ParsedReminderDate>>,
    start: Option<Option<ParsedReminderDate>>,
    notes: Option<Option<String>>,
    location: Option<Option<String>>,
    url: Option<Option<String>>,
    priority_value: Option<usize>,
    notifications: Option<Vec<ParsedReminderNotification>>,
    geofence: Option<Option<ParsedReminderGeofence>>,
    recurrence: Option<Option<ParsedReminderRecurrence>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderJsonGeofence {
    title: String,
    latitude: f64,
    longitude: f64,
    radius_meters: f64,
    proximity: ReminderGeofenceProximityArg,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderJsonRecurrence {
    frequency: ReminderRepeatArg,
    interval: Option<usize>,
    count: Option<usize>,
    until: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderJsonDraft {
    client_id: Option<String>,
    title: String,
    list: Option<String>,
    list_id: Option<String>,
    list_source: Option<String>,
    source_id: Option<String>,
    parent_id: Option<String>,
    due: Option<String>,
    start: Option<String>,
    #[serde(default)]
    time_zone: ReminderJsonOverride<String>,
    notes: Option<String>,
    url: Option<String>,
    location: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: Option<bool>,
    notify_minutes_before: Option<Vec<i64>>,
    notify_at: Option<Vec<String>>,
    #[serde(default)]
    geofence: ReminderJsonOverride<ReminderJsonGeofence>,
    #[serde(default)]
    recurrence: ReminderJsonOverride<ReminderJsonRecurrence>,
}

#[derive(Clone, Debug, Default)]
enum ReminderJsonOverride<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for ReminderJsonOverride<T>
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderBatchDefaults {
    list: Option<String>,
    list_id: Option<String>,
    list_source: Option<String>,
    source_id: Option<String>,
    parent_id: Option<String>,
    time_zone: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: Option<bool>,
    notify_minutes_before: Option<Vec<i64>>,
    notify_at: Option<Vec<String>>,
    geofence: Option<ReminderJsonGeofence>,
    recurrence: Option<ReminderJsonRecurrence>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderBatchEnvelope {
    version: u8,
    #[serde(default)]
    defaults: ReminderBatchDefaults,
    reminders: Vec<Value>,
}

fn add_reminder_from_cli(
    store: &impl ReminderStore,
    input: AddReminderCliCommand,
) -> Result<JsonOutput> {
    let command = if let Some(path) = input.json_file.as_deref() {
        if add_cli_has_individual_fields(&input) {
            bail!("--json-file cannot be combined with a title or individual reminder fields");
        }
        let contents = read_notes_file(path)
            .with_context(|| format!("failed to read reminder JSON file {}", path.display()))?;
        let draft: ReminderJsonDraft = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse reminder JSON file {}", path.display()))?;
        if draft.client_id.is_some() {
            bail!("client_id is only valid in reminder batch files");
        }
        json_draft_to_command(
            &ReminderBatchDefaults::default(),
            draft,
            input.if_exists,
            input.duplicate_window_seconds,
            input.dry_run,
        )?
    } else {
        AddReminderCommand {
            title: input
                .title
                .context("reminder add requires TITLE or --json-file")?,
            list_selector: input.list_selector,
            parent_id: input.parent_id,
            due: input.due,
            start: input.start,
            time_zone: input.time_zone,
            notes: input.notes,
            notes_file: input.notes_file,
            url: input.url,
            location: input.location,
            priority: input.priority,
            notify_at_due: input.notify_at_due,
            notify_minutes_before: input.notify_minutes_before,
            schedule: input.schedule,
            if_exists: input.if_exists,
            duplicate_window_seconds: input.duplicate_window_seconds,
            dry_run: input.dry_run,
        }
    };
    add_reminder(store, command)
}

fn add_cli_has_individual_fields(input: &AddReminderCliCommand) -> bool {
    input.title.is_some()
        || !write_list_selector_is_empty(&input.list_selector)
        || input.parent_id.is_some()
        || input.due.is_some()
        || input.start.is_some()
        || input.time_zone.is_some()
        || input.notes.is_some()
        || input.notes_file.is_some()
        || input.url.is_some()
        || input.location.is_some()
        || input.priority.is_some()
        || input.notify_at_due
        || !input.notify_minutes_before.is_empty()
        || !advanced_schedule_is_empty(&input.schedule)
}

fn advanced_schedule_is_empty(schedule: &ReminderAdvancedScheduleArgs) -> bool {
    schedule.notify_at.is_empty()
        && schedule.geofence_title.is_none()
        && schedule.geofence_latitude.is_none()
        && schedule.geofence_longitude.is_none()
        && schedule.geofence_radius_meters.is_none()
        && schedule.geofence_proximity.is_none()
        && schedule.repeat.is_none()
        && schedule.repeat_interval.is_none()
        && schedule.repeat_count.is_none()
        && schedule.repeat_until.is_none()
}

fn json_draft_to_command(
    defaults: &ReminderBatchDefaults,
    draft: ReminderJsonDraft,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
) -> Result<AddReminderCommand> {
    let has_timezone_less_timed_date = reminder_has_timezone_less_timed_date(&draft);
    let item_has_selector = draft.list.is_some()
        || draft.list_id.is_some()
        || draft.list_source.is_some()
        || draft.source_id.is_some();
    let (list, list_id, list_source, source_id) = if item_has_selector {
        (
            draft.list,
            draft.list_id,
            draft.list_source,
            draft.source_id,
        )
    } else {
        (
            defaults.list.clone(),
            defaults.list_id.clone(),
            defaults.list_source.clone(),
            defaults.source_id.clone(),
        )
    };
    validate_json_list_selector(
        list.as_deref(),
        list_id.as_deref(),
        list_source.as_deref(),
        source_id.as_deref(),
    )?;
    let geofence = reminder_json_override(draft.geofence, &defaults.geofence);
    let recurrence = reminder_json_override(draft.recurrence, &defaults.recurrence);
    let time_zone = match draft.time_zone {
        ReminderJsonOverride::Value(value) => Some(value),
        ReminderJsonOverride::Null => None,
        ReminderJsonOverride::Missing => has_timezone_less_timed_date
            .then(|| defaults.time_zone.clone())
            .flatten(),
    };
    let schedule = ReminderAdvancedScheduleArgs {
        notify_at: draft
            .notify_at
            .or_else(|| defaults.notify_at.clone())
            .unwrap_or_default(),
        geofence_title: geofence.as_ref().map(|value| value.title.clone()),
        geofence_latitude: geofence.as_ref().map(|value| value.latitude),
        geofence_longitude: geofence.as_ref().map(|value| value.longitude),
        geofence_radius_meters: geofence.as_ref().map(|value| value.radius_meters),
        geofence_proximity: geofence.as_ref().map(|value| value.proximity),
        repeat: recurrence.as_ref().map(|value| value.frequency),
        repeat_interval: recurrence.as_ref().and_then(|value| value.interval),
        repeat_count: recurrence.as_ref().and_then(|value| value.count),
        repeat_until: recurrence.and_then(|value| value.until),
    };
    Ok(AddReminderCommand {
        title: draft.title,
        list_selector: WriteReminderListSelectorArgs {
            list,
            list_id,
            list_source,
            source_id,
        },
        parent_id: draft.parent_id.or_else(|| defaults.parent_id.clone()),
        due: draft.due,
        start: draft.start,
        time_zone,
        notes: draft.notes,
        notes_file: None,
        url: draft.url,
        location: draft.location,
        priority: draft.priority.or(defaults.priority),
        notify_at_due: draft
            .notify_at_due
            .or(defaults.notify_at_due)
            .unwrap_or(false),
        notify_minutes_before: draft
            .notify_minutes_before
            .or_else(|| defaults.notify_minutes_before.clone())
            .unwrap_or_default(),
        schedule,
        if_exists,
        duplicate_window_seconds,
        dry_run,
    })
}

fn reminder_json_override<T: Clone>(
    value: ReminderJsonOverride<T>,
    default: &Option<T>,
) -> Option<T> {
    match value {
        ReminderJsonOverride::Missing => default.clone(),
        ReminderJsonOverride::Null => None,
        ReminderJsonOverride::Value(value) => Some(value),
    }
}

fn reminder_has_timezone_less_timed_date(draft: &ReminderJsonDraft) -> bool {
    [draft.due.as_deref(), draft.start.as_deref()]
        .into_iter()
        .flatten()
        .any(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err()
                && DateTime::parse_from_rfc3339(value).is_err()
        })
}

fn validate_json_list_selector(
    list: Option<&str>,
    list_id: Option<&str>,
    list_source: Option<&str>,
    source_id: Option<&str>,
) -> Result<()> {
    if list_id.is_some() && (list.is_some() || list_source.is_some() || source_id.is_some()) {
        bail!("list_id cannot be combined with list, list_source, or source_id");
    }
    if list_source.is_some() && source_id.is_some() {
        bail!("list_source and source_id cannot be combined");
    }
    if (list_source.is_some() || source_id.is_some()) && list.is_none() {
        bail!("list_source and source_id require list");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReminderBatchIdentity {
    list_id: String,
    parent_id: Option<String>,
    title: String,
    due: String,
}

struct ReminderBatchSlot {
    index: usize,
    client_id: Option<String>,
    command: Option<AddReminderCommand>,
    draft: Option<ReminderDraftReport>,
    error: Option<String>,
}

fn reminder_batch_add(
    store: &impl ReminderStore,
    path: &Path,
    if_exists: IfExistsArg,
    dry_run: bool,
    continue_on_error: bool,
) -> Result<JsonOutput> {
    let contents = fs::read(path)
        .with_context(|| format!("failed to read reminder batch file {}", path.display()))?;
    let envelope: ReminderBatchEnvelope = serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse reminder batch file {}", path.display()))?;
    if envelope.version != 1 {
        bail!(
            "unsupported reminder batch version {}; expected 1",
            envelope.version
        );
    }
    if envelope.reminders.is_empty() {
        bail!("reminder batch must contain at least one reminder");
    }
    validate_json_list_selector(
        envelope.defaults.list.as_deref(),
        envelope.defaults.list_id.as_deref(),
        envelope.defaults.list_source.as_deref(),
        envelope.defaults.source_id.as_deref(),
    )?;
    store.ensure_authorized()?;

    let mut slots = envelope
        .reminders
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let client_id = value
                .get("client_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            match serde_json::from_value::<ReminderJsonDraft>(value) {
                Ok(draft) => {
                    let client_id = draft.client_id.clone();
                    match json_draft_to_command(&envelope.defaults, draft, if_exists, 0, true) {
                        Ok(command) => ReminderBatchSlot {
                            index,
                            client_id,
                            command: Some(command),
                            draft: None,
                            error: None,
                        },
                        Err(error) => ReminderBatchSlot {
                            index,
                            client_id,
                            command: None,
                            draft: None,
                            error: Some(format!("invalid reminder: {error:#}")),
                        },
                    }
                }
                Err(error) => ReminderBatchSlot {
                    index,
                    client_id,
                    command: None,
                    draft: None,
                    error: Some(format!("invalid reminder: {error}")),
                },
            }
        })
        .collect::<Vec<_>>();
    mark_duplicate_reminder_client_ids(&mut slots);

    for slot in &mut slots {
        if slot.error.is_some() {
            continue;
        }
        let command = slot
            .command
            .as_ref()
            .expect("batch command is present")
            .clone();
        match add_reminder(store, command) {
            Ok(JsonOutput::ReminderDryRun { draft, .. }) => {
                if let Some(command) = slot.command.as_mut() {
                    command.list_selector = WriteReminderListSelectorArgs {
                        list: None,
                        list_id: Some(draft.list_id.clone()),
                        list_source: None,
                        source_id: None,
                    };
                }
                slot.draft = Some(*draft);
            }
            Ok(_) => slot.error = Some("reminder preflight returned unexpected output".to_string()),
            Err(error) => slot.error = Some(format!("{error:#}")),
        }
    }
    mark_duplicate_reminder_identities(&mut slots);

    let has_preflight_errors = slots.iter().any(|slot| slot.error.is_some());
    let can_write = !has_preflight_errors || continue_on_error;
    let items = if dry_run {
        reminder_batch_dry_reports(&slots)
    } else if has_preflight_errors && !continue_on_error {
        reminder_batch_blocked_reports(&slots)
    } else {
        reminder_batch_execute(store, &slots, continue_on_error)
    };
    let summary = summarize_reminder_batch(&items);
    Ok(JsonOutput::ReminderBatch {
        batch: ReminderBatchReport {
            version: 1,
            dry_run,
            can_write,
            if_exists: if_exists_name(if_exists).to_string(),
            continue_on_error,
            summary,
            items,
        },
    })
}

fn mark_duplicate_reminder_client_ids(slots: &mut [ReminderBatchSlot]) {
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

fn mark_duplicate_reminder_identities(slots: &mut [ReminderBatchSlot]) {
    let mut positions: HashMap<ReminderBatchIdentity, Vec<usize>> = HashMap::new();
    for (position, slot) in slots.iter().enumerate() {
        if slot.error.is_none()
            && let Some(draft) = &slot.draft
        {
            let due = match draft.due.as_ref() {
                None => "undated".to_string(),
                Some(due) if due.kind == ReminderDateKind::Date => {
                    format!("date:{}", due.date.as_deref().unwrap_or("invalid"))
                }
                Some(due) => format!("datetime:{}", due.utc.as_deref().unwrap_or("invalid")),
            };
            positions
                .entry(ReminderBatchIdentity {
                    list_id: draft.list_id.clone(),
                    parent_id: draft.parent_id.clone(),
                    title: draft.title.clone(),
                    due,
                })
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
                    "duplicate reminder identity within batch at rows {rows}"
                ));
            }
        }
    }
}

fn reminder_batch_dry_reports(slots: &[ReminderBatchSlot]) -> Vec<ReminderBatchItemReport> {
    slots
        .iter()
        .map(|slot| {
            if let Some(error) = &slot.error {
                return reminder_batch_error_report(slot, "failed", error.clone());
            }
            let draft = slot.draft.as_ref().expect("batch draft is present");
            let status = match draft.operation.as_str() {
                "create" => "would_create",
                "skip" => "would_skip",
                "update" => "would_update",
                _ => "failed",
            };
            reminder_batch_planned_report(slot, status, None)
        })
        .collect()
}

fn reminder_batch_blocked_reports(slots: &[ReminderBatchSlot]) -> Vec<ReminderBatchItemReport> {
    slots
        .iter()
        .map(|slot| {
            if let Some(error) = &slot.error {
                reminder_batch_error_report(slot, "failed", error.clone())
            } else {
                reminder_batch_planned_report(
                    slot,
                    "not_attempted",
                    Some("reminder batch blocked by preflight errors".to_string()),
                )
            }
        })
        .collect()
}

fn reminder_batch_execute(
    store: &impl ReminderStore,
    slots: &[ReminderBatchSlot],
    continue_on_error: bool,
) -> Vec<ReminderBatchItemReport> {
    let mut stopped = false;
    let mut reports = Vec::with_capacity(slots.len());
    for slot in slots {
        if let Some(error) = &slot.error {
            reports.push(reminder_batch_error_report(slot, "failed", error.clone()));
            continue;
        }
        if stopped {
            reports.push(reminder_batch_planned_report(
                slot,
                "not_attempted",
                Some("not attempted after an earlier reminder write failure".to_string()),
            ));
            continue;
        }
        let mut command = slot
            .command
            .as_ref()
            .expect("batch command is present")
            .clone();
        command.dry_run = false;
        match add_reminder(store, command) {
            Ok(JsonOutput::Reminder { reminder }) => {
                let status = reminder
                    .write_action
                    .as_deref()
                    .unwrap_or("written")
                    .to_string();
                let mut executed_draft = slot.draft.clone();
                if let Some(draft) = executed_draft.as_mut() {
                    draft.operation = match status.as_str() {
                        "created" => "create",
                        "skipped" => "skip",
                        "updated" => "update",
                        other => other,
                    }
                    .to_string();
                    draft.matched_reminder_id = matches!(status.as_str(), "skipped" | "updated")
                        .then(|| reminder.id.clone());
                }
                let matched_reminder_id =
                    matches!(status.as_str(), "skipped" | "updated").then(|| reminder.id.clone());
                reports.push(ReminderBatchItemReport {
                    index: slot.index,
                    client_id: slot.client_id.clone(),
                    status,
                    reminder_id: Some(reminder.id.clone()),
                    matched_reminder_id,
                    draft: executed_draft.map(Box::new),
                    error: None,
                });
            }
            Ok(_) => {
                reports.push(reminder_batch_error_report(
                    slot,
                    "failed",
                    "reminder write returned unexpected output".to_string(),
                ));
                if !continue_on_error {
                    stopped = true;
                }
            }
            Err(error) => {
                reports.push(reminder_batch_error_report(
                    slot,
                    "failed",
                    format!("{error:#}"),
                ));
                if !continue_on_error {
                    stopped = true;
                }
            }
        }
    }
    reports
}

fn reminder_batch_planned_report(
    slot: &ReminderBatchSlot,
    status: &str,
    error: Option<String>,
) -> ReminderBatchItemReport {
    let draft = slot.draft.as_ref();
    ReminderBatchItemReport {
        index: slot.index,
        client_id: slot.client_id.clone(),
        status: status.to_string(),
        reminder_id: None,
        matched_reminder_id: draft.and_then(|draft| draft.matched_reminder_id.clone()),
        draft: draft.cloned().map(Box::new),
        error: error.map(|message| BatchErrorReport { message }),
    }
}

fn reminder_batch_error_report(
    slot: &ReminderBatchSlot,
    status: &str,
    message: String,
) -> ReminderBatchItemReport {
    ReminderBatchItemReport {
        index: slot.index,
        client_id: slot.client_id.clone(),
        status: status.to_string(),
        reminder_id: None,
        matched_reminder_id: slot
            .draft
            .as_ref()
            .and_then(|draft| draft.matched_reminder_id.clone()),
        draft: slot.draft.clone().map(Box::new),
        error: Some(BatchErrorReport { message }),
    }
}

fn summarize_reminder_batch(items: &[ReminderBatchItemReport]) -> BatchSummaryReport {
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
            _ => summary.failed += 1,
        }
    }
    summary
}

fn add_reminder(store: &impl ReminderStore, command: AddReminderCommand) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    if command.title.trim().is_empty() {
        bail!("reminder title must not be empty");
    }
    if command.duplicate_window_seconds < 0 {
        bail!("--duplicate-window-seconds must not be negative");
    }
    if let Some(time_zone) = &command.time_zone {
        validate_time_zone(time_zone)?;
    }

    let notes = match command.notes_file.as_deref() {
        Some(path) => Some(read_notes_file(path)?),
        None => command.notes.clone(),
    };
    if let Some(url) = &command.url {
        validate_reminder_url(url)?;
    }

    let due = command
        .due
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder due value")?;
    let start = command
        .start
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder start value")?;
    if command.time_zone.is_some()
        && [due.as_ref(), start.as_ref()]
            .into_iter()
            .flatten()
            .all(|value| value.report.kind == ReminderDateKind::Date)
    {
        bail!("--time-zone requires at least one timed --due or --start value");
    }
    let notifications_supplied = command.notify_at_due
        || !command.notify_minutes_before.is_empty()
        || !command.schedule.notify_at.is_empty()
        || command.schedule.geofence_title.is_some();
    let mut notifications = build_notifications(
        due.as_ref().map(|value| &value.report),
        command.notify_at_due,
        &command.notify_minutes_before,
    )?;
    notifications.extend(parse_absolute_notifications(&command.schedule.notify_at)?);
    let geofence = parse_geofence(&command.schedule)?;
    let recurrence = parse_recurrence(&command.schedule)?;

    let parent = command
        .parent_id
        .as_deref()
        .map(|id| resolve_parent(store, id))
        .transpose()?;
    let lists = store.lists()?;
    let parent_list = parent
        .as_ref()
        .map(|parent| reminder_parent_list(&lists, parent))
        .transpose()?;
    let configured_default_id =
        if write_list_selector_is_empty(&command.list_selector) && parent_list.is_none() {
            crate::config::default_reminder_list_id()?
        } else {
            None
        };
    let (list, list_selection) = if write_list_selector_is_empty(&command.list_selector) {
        if let Some(parent_list) = parent_list.as_ref() {
            (parent_list.clone(), ReminderListSelection::Explicit)
        } else {
            let default_list = if configured_default_id.is_some() {
                None
            } else {
                Some(store.default_list()?)
            };
            resolve_write_list(
                &lists,
                default_list.as_ref(),
                configured_default_id.as_deref(),
                &command.list_selector,
            )?
        }
    } else {
        resolve_write_list(&lists, None, None, &command.list_selector)?
    };
    if let Some(parent_list) = parent_list.as_ref()
        && parent_list.id != list.id
    {
        bail!(
            "parent reminder {} [{}] is in {} [{}], but the child target is {} [{}]; parent and child must use the same reminder list",
            parent.as_ref().expect("parent is present").title,
            parent.as_ref().expect("parent is present").id,
            parent_list.title,
            parent_list.id,
            list.title,
            list.id
        );
    }
    let priority_arg = command.priority;
    let priority_value = priority_arg.map(priority_arg_value).unwrap_or(0);

    let save = ReminderSaveDraft {
        title: command.title.clone(),
        list_id: list.id.clone(),
        parent_id: command.parent_id.clone(),
        due: due.clone(),
        start: start.clone(),
        notes: notes.clone(),
        location: command.location.clone(),
        url: command.url.clone(),
        priority_value,
        notifications: notifications.clone(),
        geofence: geofence.clone(),
        recurrence: recurrence.clone(),
    };
    let patch = ReminderAddPatch {
        parent_id: command.parent_id.clone(),
        start: start.clone(),
        notes: notes.clone(),
        location: command.location.clone(),
        url: command.url.clone(),
        priority_value: priority_arg.map(priority_arg_value),
        notifications: notifications_supplied.then_some(notifications.clone()),
        geofence: notifications_supplied.then_some(geofence.clone()),
        recurrence: recurrence.clone(),
    };

    let existing = store.fetch(std::slice::from_ref(&list.id))?;
    let matches = find_duplicates(
        &existing,
        &command.title,
        due.as_ref(),
        command.parent_id.as_deref(),
        command.duplicate_window_seconds,
    );
    if matches.len() > 1 {
        let ids = matches
            .iter()
            .map(|reminder| reminder.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "duplicate policy matched multiple reminders in {} [{}]: {ids}; use an exact due value or a smaller --duplicate-window-seconds",
            list.title,
            list.id
        );
    }
    let matched = matches.into_iter().next();
    if let (Some(matched), Some(parent)) = (matched.as_ref(), parent.as_ref()) {
        validate_parent_assignment(store, &matched.id, &list.id, parent)?;
    }
    if let Some(matched) = matched.as_ref()
        && command.if_exists == IfExistsArg::Error
    {
        bail!(
            "matching reminder already exists: {} [{}] in {} [{}]; use --if-exists skip or update",
            matched.title,
            matched.id,
            list.title,
            list.id
        );
    }

    let operation = match (&matched, command.if_exists) {
        (Some(_), IfExistsArg::Skip) => "skip",
        (Some(_), IfExistsArg::Update) => "update",
        _ => "create",
    };
    if let Some(recurrence) = &recurrence {
        let effective_anchor = if operation == "update" {
            matched
                .as_ref()
                .and_then(|reminder| reminder.due.as_ref())
                .or_else(|| start.as_ref().map(|value| &value.report))
                .or_else(|| {
                    matched
                        .as_ref()
                        .and_then(|reminder| reminder.start.as_ref())
                })
        } else {
            due.as_ref()
                .map(|value| &value.report)
                .or_else(|| start.as_ref().map(|value| &value.report))
        };
        let effective_anchor =
            effective_anchor.context("recurrence requires a due or start date")?;
        validate_recurrence_end_after_anchor(
            &recurrence_report_from_parsed(recurrence),
            Some(effective_anchor),
        )?;
    }
    let duplicate_warnings = (command.duplicate_window_seconds > 0)
        .then(|| {
            format!(
                "timed due duplicate matching uses a ±{} second window; date-only and undated identities remain exact",
                command.duplicate_window_seconds
            )
        })
        .into_iter()
        .collect();
    let draft = ReminderDraftReport {
        operation: operation.to_string(),
        matched_reminder_id: matched.as_ref().map(|reminder| reminder.id.clone()),
        title: command.title.clone(),
        parent_id: command.parent_id.clone(),
        list: list.title.clone(),
        list_id: list.id.clone(),
        list_source: list.source.clone(),
        list_source_id: list.source_id.clone(),
        list_selection,
        due: due.as_ref().map(|value| value.report.clone()),
        due_input: due.as_ref().map(|value| value.input.clone()),
        start: start.as_ref().map(|value| value.report.clone()),
        start_input: start.as_ref().map(|value| value.input.clone()),
        priority: priority_name(priority_value),
        priority_value,
        has_notes: notes.is_some(),
        has_location: command.location.is_some(),
        has_url: command.url.is_some(),
        notification_count: notifications
            .iter()
            .filter(|notification| notification.minutes_before.is_some())
            .count(),
        notifications: notification_reports(
            &notifications,
            due.as_ref().map(|value| &value.report),
        ),
        planned_alarm_count: notifications.len() + usize::from(geofence.is_some()),
        planned_alarms: planned_alarm_reports(&notifications, geofence.as_ref()),
        recurrence: recurrence.as_ref().map(recurrence_report_from_parsed),
        if_exists: if_exists_name(command.if_exists).to_string(),
        duplicate_window_seconds: command.duplicate_window_seconds,
        duplicate_warnings,
    };
    if command.dry_run {
        return Ok(JsonOutput::ReminderDryRun {
            would_write: false,
            draft: Box::new(draft),
        });
    }

    let mut reminder = match (operation, matched.as_ref()) {
        ("skip", Some(matched)) => store.get(&matched.id)?,
        ("update", Some(matched)) => store.update_add_fields(&matched.id, &patch)?,
        _ => store.create(&save)?,
    };
    reminder.list_selection = Some(list_selection);
    reminder.write_action = Some(
        match operation {
            "skip" => "skipped",
            "update" => "updated",
            _ => "created",
        }
        .to_string(),
    );
    reminder.due_input = due.map(|value| value.input);
    reminder.start_input = start.map(|value| value.input);
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

fn update_reminder(
    store: &impl ReminderStore,
    command: UpdateReminderCommand,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(&command.id)?;
    let before = store.get(&id)?;
    ensure_reminder_writable(&before, "update")?;

    if command
        .title
        .as_ref()
        .is_some_and(|title| title.trim().is_empty())
    {
        bail!("reminder title must not be empty");
    }
    if let Some(time_zone) = &command.time_zone {
        validate_time_zone(time_zone)?;
    }
    if let Some(url) = &command.url {
        validate_reminder_url(url)?;
    }

    let notes = match command.notes_file.as_deref() {
        Some(path) => Some(read_notes_file(path)?),
        None => command.notes.clone(),
    };
    let mut due = command
        .due
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder due value")?;
    let mut start = command
        .start
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder start value")?;
    if command.clear_time_zone {
        due = due.map(clear_parsed_time_zone).transpose()?;
        start = start.map(clear_parsed_time_zone).transpose()?;
    }

    let mut patch = ReminderLifecyclePatch {
        title: command.title.clone(),
        parent_id: if command.clear_parent {
            Some(None)
        } else {
            command.parent_id.clone().map(Some)
        },
        due: if command.clear_due {
            Some(None)
        } else {
            due.map(Some)
        },
        start: if command.clear_start {
            Some(None)
        } else {
            start.map(Some)
        },
        notes: if command.clear_notes {
            Some(None)
        } else {
            notes.map(Some)
        },
        location: if command.clear_location {
            Some(None)
        } else {
            command.location.clone().map(Some)
        },
        url: if command.clear_url {
            Some(None)
        } else {
            command.url.clone().map(Some)
        },
        priority_value: command.priority.map(priority_arg_value),
        ..Default::default()
    };
    let mut moved_list = None;
    if !write_list_selector_is_empty(&command.list_selector) {
        let lists = store.lists()?;
        let (list, _) = resolve_write_list(&lists, None, None, &command.list_selector)?;
        patch.list_id = Some(list.id.clone());
        moved_list = Some(list);
    }

    let resulting_list_id = moved_list
        .as_ref()
        .map(|list| list.id.as_str())
        .or(before.list_id.as_deref())
        .context("reminder list is unavailable")?;
    if let Some(parent_id) = command.parent_id.as_deref() {
        let parent = resolve_parent(store, parent_id)?;
        validate_parent_assignment(store, &id, resulting_list_id, &parent)?;
    } else if !command.clear_parent
        && let Some(parent_id) = before.parent_id.as_deref()
    {
        let parent = resolve_parent(store, parent_id)?;
        let parent_list_id = parent
            .list_id
            .as_deref()
            .context("parent reminder list is unavailable")?;
        if parent_list_id != resulting_list_id {
            bail!(
                "moving child reminder {} [{}] away from parent {} [{}] requires --clear-parent or --parent-id for a parent in the destination list",
                before.title,
                before.id,
                parent.title,
                parent.id
            );
        }
    }
    if moved_list.is_some() && before.child_count > 0 {
        bail!(
            "cannot move parent reminder {} [{}] while it has {} direct child reminder(s); reparent or clear those children first",
            before.title,
            before.id,
            before.child_count
        );
    }

    if command.time_zone.is_some() || command.clear_time_zone {
        rezone_unchanged_dates(
            &before,
            &mut patch,
            command.time_zone.as_deref(),
            command.clear_time_zone,
        )?;
        let result = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
        let has_timed_value = [result.due.as_ref(), result.start.as_ref()]
            .into_iter()
            .flatten()
            .any(|value| value.kind == ReminderDateKind::Datetime);
        if !has_timed_value {
            bail!("--time-zone and --clear-time-zone require a timed due or start value");
        }
    }

    let alarm_flags_supplied = command.notify_at_due
        || !command.notify_minutes_before.is_empty()
        || !command.schedule.notify_at.is_empty()
        || command.schedule.geofence_title.is_some();
    if command.clear_notifications && alarm_flags_supplied {
        bail!("--clear-notifications conflicts with notification and geofence options");
    }
    if command.clear_notifications {
        patch.notifications = Some(Vec::new());
        patch.geofence = Some(None);
    } else if alarm_flags_supplied {
        let result = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
        let mut notifications = build_notifications(
            result.due.as_ref(),
            command.notify_at_due,
            &command.notify_minutes_before,
        )?;
        notifications.extend(parse_absolute_notifications(&command.schedule.notify_at)?);
        let geofence = parse_geofence(&command.schedule)?;
        patch.notifications = Some(notifications);
        patch.geofence = Some(geofence);
    }

    if command.clear_recurrence && command.schedule.repeat.is_some() {
        bail!("--clear-recurrence conflicts with --repeat");
    }
    if command.clear_recurrence {
        patch.recurrence = Some(None);
    } else if let Some(recurrence) = parse_recurrence(&command.schedule)? {
        patch.recurrence = Some(Some(recurrence));
    }

    let final_preview = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
    let recurrence_rules = match &patch.recurrence {
        Some(Some(recurrence)) => vec![recurrence_report_from_parsed(recurrence)],
        Some(None) => Vec::new(),
        None => before.recurrence_rules.clone().unwrap_or_default(),
    };
    let recurrence_remains = !recurrence_rules.is_empty()
        || (patch.recurrence.is_none() && before.recurrence_count.unwrap_or_default() > 0);
    if recurrence_remains {
        let anchor = final_preview
            .due
            .as_ref()
            .or(final_preview.start.as_ref())
            .context(
                "recurrence requires a due or start date; clear recurrence when removing its final anchor",
            )?;
        for recurrence in &recurrence_rules {
            validate_recurrence_end_after_anchor(recurrence, Some(anchor))?;
        }
    }

    let changed_fields = lifecycle_changed_fields(&patch);
    if changed_fields.is_empty() {
        bail!("reminder update requires at least one field change");
    }
    let mut result = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
    result.due_input = command.due.clone();
    result.start_input = command.start.clone();
    result.write_action = Some("would_update".to_string());
    if command.dry_run {
        return Ok(reminder_mutation_dry_run(
            "update",
            changed_fields,
            before,
            result,
        ));
    }

    let mut reminder = store.update(&id, &patch)?;
    reminder.write_action = Some("updated".to_string());
    reminder.due_input = command.due;
    reminder.start_input = command.start;
    if moved_list.is_some() {
        reminder.list_selection = Some(ReminderListSelection::Explicit);
    }
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

fn complete_reminder(
    store: &impl ReminderStore,
    reference: &str,
    completed_at: Option<&str>,
    dry_run: bool,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(reference)?;
    let before = store.get(&id)?;
    ensure_reminder_writable(&before, "complete")?;
    if before.child_count > 0 {
        bail!(
            "cannot complete parent reminder {} [{}] while it has {} direct child reminder(s); complete or reparent the children first",
            before.title,
            before.id,
            before.child_count
        );
    }
    let completed_at = match completed_at {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .with_context(|| {
                "--completed-at must be RFC3339 with an explicit UTC offset, for example 2026-07-11T14:00:00+03:00"
            })?
            .with_timezone(&Utc),
        None => Utc::now(),
    };
    let mut result = before.clone();
    result.completed = true;
    result.completion_date = Some(completed_at.to_rfc3339());
    result.write_action = Some("would_complete".to_string());
    if dry_run {
        return Ok(reminder_mutation_dry_run(
            "complete",
            vec!["completed".to_string(), "completion_date".to_string()],
            before,
            result,
        ));
    }
    let mut reminder = store.set_completion(&id, Some(completed_at))?;
    reminder.write_action = Some("completed".to_string());
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

fn uncomplete_reminder(
    store: &impl ReminderStore,
    reference: &str,
    dry_run: bool,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(reference)?;
    let before = store.get(&id)?;
    ensure_reminder_writable(&before, "uncomplete")?;
    let mut result = before.clone();
    result.completed = false;
    result.completion_date = None;
    result.write_action = Some("would_uncomplete".to_string());
    if dry_run {
        return Ok(reminder_mutation_dry_run(
            "uncomplete",
            vec!["completed".to_string(), "completion_date".to_string()],
            before,
            result,
        ));
    }
    let mut reminder = store.set_completion(&id, None)?;
    reminder.write_action = Some("uncompleted".to_string());
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

fn delete_reminder(store: &impl ReminderStore, reference: &str, force: bool) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(reference)?;
    let reminder = store.get(&id)?;
    ensure_reminder_writable(&reminder, "delete")?;
    if reminder.child_count > 0 {
        bail!(
            "cannot delete parent reminder {} [{}] while it has {} direct child reminder(s); reparent or delete the children first",
            reminder.title,
            reminder.id,
            reminder.child_count
        );
    }
    if !force {
        confirm_reminder_delete(&reminder)?;
    }
    store.delete(&id)?;
    Ok(JsonOutput::ReminderDeleted {
        deleted: ReminderDeletedReport {
            id,
            title: reminder.title,
            list: reminder.list,
            list_id: reminder.list_id,
        },
    })
}

fn reminder_mutation_dry_run(
    operation: &str,
    changed_fields: Vec<String>,
    before: ReminderReport,
    result: ReminderReport,
) -> JsonOutput {
    JsonOutput::ReminderMutationDryRun {
        would_write: false,
        draft: Box::new(ReminderMutationDraftReport {
            operation: operation.to_string(),
            reminder_id: before.id.clone(),
            changed_fields,
            before: Box::new(before),
            result: Box::new(result),
        }),
    }
}

fn ensure_reminder_writable(reminder: &ReminderReport, operation: &str) -> Result<()> {
    let list = reminder.list.as_deref().unwrap_or("unknown list");
    let list_id = reminder.list_id.as_deref().unwrap_or("unknown id");
    match reminder.allows_list_modifications {
        Some(true) => Ok(()),
        Some(false) => bail!("cannot {operation} reminder in read-only list {list} [{list_id}]"),
        None => bail!(
            "cannot {operation} reminder because its list is unavailable or writability is unknown: {list} [{list_id}]"
        ),
    }
}

fn resolve_parent(store: &impl ReminderStore, parent_id: &str) -> Result<ReminderReport> {
    let parent = store
        .get(parent_id)
        .with_context(|| format!("parent reminder not found: {parent_id}"))?;
    ensure_reminder_writable(&parent, "use as a parent")?;
    if parent.completed {
        bail!(
            "completed reminder {} [{}] cannot be used as a parent",
            parent.title,
            parent.id
        );
    }
    Ok(parent)
}

fn reminder_parent_list(
    lists: &[ReminderListReport],
    parent: &ReminderReport,
) -> Result<ReminderListReport> {
    let list_id = parent
        .list_id
        .as_deref()
        .context("parent reminder list is unavailable")?;
    lists
        .iter()
        .find(|list| list.id == list_id)
        .cloned()
        .with_context(|| format!("parent reminder list is no longer available: {list_id}"))
}

fn validate_parent_assignment(
    store: &impl ReminderStore,
    child_id: &str,
    child_list_id: &str,
    parent: &ReminderReport,
) -> Result<()> {
    if parent.id == child_id {
        bail!("a reminder cannot be its own parent: {child_id}");
    }
    let parent_list_id = parent
        .list_id
        .as_deref()
        .context("parent reminder list is unavailable")?;
    if parent_list_id != child_list_id {
        bail!(
            "parent reminder {} [{}] is in list {}, but child {} is in list {}; parent and child must use the same reminder list",
            parent.title,
            parent.id,
            parent_list_id,
            child_id,
            child_list_id
        );
    }

    let mut seen = BTreeSet::new();
    let mut current = Some(parent.clone());
    while let Some(reminder) = current {
        if reminder.id == child_id {
            bail!("parent assignment would create a reminder hierarchy cycle involving {child_id}");
        }
        if !seen.insert(reminder.id.clone()) {
            bail!(
                "existing reminder hierarchy contains a cycle at {} [{}]",
                reminder.title,
                reminder.id
            );
        }
        current = reminder
            .parent_id
            .as_deref()
            .map(|id| resolve_parent(store, id))
            .transpose()?;
    }
    Ok(())
}

fn confirm_reminder_delete(reminder: &ReminderReport) -> Result<()> {
    let mut stderr = io::stderr();
    let list = reminder.list.as_deref().unwrap_or("unknown list");
    writeln!(
        stderr,
        "Delete reminder \"{}\" from {} [{}]?",
        reminder.title,
        list,
        reminder.list_id.as_deref().unwrap_or("unknown id")
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

fn lifecycle_changed_fields(patch: &ReminderLifecyclePatch) -> Vec<String> {
    [
        (patch.title.is_some(), "title"),
        (patch.parent_id.is_some(), "parent"),
        (patch.list_id.is_some(), "list"),
        (patch.due.is_some(), "due"),
        (patch.start.is_some(), "start"),
        (patch.notes.is_some(), "notes"),
        (patch.location.is_some(), "location"),
        (patch.url.is_some(), "url"),
        (patch.priority_value.is_some(), "priority"),
        (
            patch.notifications.is_some() || patch.geofence.is_some(),
            "alarms",
        ),
        (patch.recurrence.is_some(), "recurrence"),
    ]
    .into_iter()
    .filter(|(changed, _)| *changed)
    .map(|(_, name)| name.to_string())
    .collect()
}

fn preview_lifecycle_patch(
    before: &ReminderReport,
    patch: &ReminderLifecyclePatch,
    moved_list: Option<&ReminderListReport>,
) -> ReminderReport {
    let mut result = before.clone();
    if let Some(title) = &patch.title {
        result.title = title.clone();
    }
    if let Some(parent_id) = &patch.parent_id {
        result.parent_id = parent_id.clone();
    }
    if let Some(list) = moved_list {
        result.list = Some(list.title.clone());
        result.list_id = Some(list.id.clone());
        result.list_source = list.source.clone();
        result.list_source_id = list.source_id.clone();
        result.list_type = Some(list.list_type.clone());
        result.allows_list_modifications = Some(list.allows_modifications);
        result.list_selection = Some(ReminderListSelection::Explicit);
    }
    if let Some(due) = &patch.due {
        result.due = due.as_ref().map(|value| value.report.clone());
        result.due_input = due.as_ref().map(|value| value.input.clone());
    }
    if let Some(start) = &patch.start {
        result.start = start.as_ref().map(|value| value.report.clone());
        result.start_input = start.as_ref().map(|value| value.input.clone());
    }
    if let Some(notes) = &patch.notes {
        result.notes = notes.clone();
        result.has_notes = notes.is_some();
    }
    if let Some(location) = &patch.location {
        result.location = location.clone();
    }
    if let Some(url) = &patch.url {
        result.url = url.clone();
        result.has_url = url.is_some();
    }
    if let Some(priority) = patch.priority_value {
        result.priority_value = priority;
        result.priority = priority_name(priority);
    }
    if patch.notifications.is_some() || patch.geofence.is_some() {
        let notifications = patch.notifications.as_deref().unwrap_or_default();
        let geofence = patch.geofence.as_ref().and_then(Option::as_ref);
        let alarms = alarm_reports_from_parsed(notifications, geofence);
        result.alarm_count = Some(alarms.len());
        result.alarms = Some(alarms);
    }
    if let Some(recurrence) = &patch.recurrence {
        let rules = recurrence
            .as_ref()
            .map(recurrence_report_from_parsed)
            .into_iter()
            .collect::<Vec<_>>();
        result.recurrence_count = Some(rules.len());
        result.recurrence_rules = Some(rules);
    }
    result
}

fn find_duplicates<'a>(
    reminders: &'a [ReminderReport],
    title: &str,
    due: Option<&ParsedReminderDate>,
    parent_id: Option<&str>,
    window_seconds: i64,
) -> Vec<&'a ReminderReport> {
    reminders
        .iter()
        .filter(|reminder| {
            reminder.title == title
                && reminder.parent_id.as_deref() == parent_id
                && due_matches(reminder.due.as_ref(), due, window_seconds)
        })
        .collect()
}

fn due_matches(
    existing: Option<&ReminderDateReport>,
    requested: Option<&ParsedReminderDate>,
    window_seconds: i64,
) -> bool {
    match (existing, requested) {
        (None, None) => true,
        (Some(existing), Some(requested))
            if existing.kind == ReminderDateKind::Date
                && requested.report.kind == ReminderDateKind::Date =>
        {
            existing.date == requested.report.date
        }
        (Some(existing), Some(requested))
            if existing.kind == ReminderDateKind::Datetime
                && requested.report.kind == ReminderDateKind::Datetime =>
        {
            match (
                existing
                    .normalized
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
                requested
                    .report
                    .normalized
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
            ) {
                (Some(existing), Some(requested)) => {
                    (existing - requested).num_seconds().abs() <= window_seconds
                }
                _ => {
                    existing.local == requested.report.local
                        && existing.time_zone == requested.report.time_zone
                }
            }
        }
        _ => false,
    }
}

fn priority_arg_value(priority: ReminderPriorityArg) -> usize {
    match priority {
        ReminderPriorityArg::None => 0,
        ReminderPriorityArg::High => 1,
        ReminderPriorityArg::Medium => 5,
        ReminderPriorityArg::Low => 9,
    }
}

fn if_exists_name(value: IfExistsArg) -> &'static str {
    match value {
        IfExistsArg::Error => "error",
        IfExistsArg::Skip => "skip",
        IfExistsArg::Update => "update",
    }
}

fn validate_reminder_url(value: &str) -> Result<()> {
    let value = NSString::from_str(value);
    NSURL::URLWithString_encodingInvalidCharacters(&value, false)
        .map(|_| ())
        .ok_or_else(|| anyhow!("invalid URL: {value}"))
}

fn read_notes_file(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut notes = String::new();
        io::stdin()
            .read_to_string(&mut notes)
            .context("failed to read reminder notes from stdin")?;
        return Ok(notes);
    }
    fs::read_to_string(path)
        .with_context(|| format!("failed to read reminder notes from {}", path.display()))
}

fn reminders_with_hierarchy(
    reminders: &[Retained<EKReminder>],
    details: bool,
) -> Vec<ReminderReport> {
    let public_ids = reminders
        .iter()
        .map(|reminder| unsafe { reminder.calendarItemIdentifier() }.to_string())
        .collect::<Vec<_>>();
    let parent_ids = public_ids
        .iter()
        .map(|id| reminderkit_parent_id(id).ok().flatten())
        .collect::<Vec<_>>();
    let mut children = vec![Vec::<String>::new(); reminders.len()];
    for (child_index, parent_id) in parent_ids.iter().enumerate() {
        if let Some(parent_index) = parent_id
            .as_deref()
            .and_then(|parent_id| public_ids.iter().position(|id| id == parent_id))
        {
            children[parent_index].push(public_ids[child_index].clone());
        }
    }
    for child_ids in &mut children {
        child_ids.sort();
    }

    reminders
        .iter()
        .enumerate()
        .map(|(index, reminder)| {
            let mut report = reminder_to_report(reminder, details);
            report.parent_id = parent_ids[index].clone();
            report.child_count = children[index].len();
            report.child_ids = details.then(|| children[index].clone());
            report
        })
        .collect()
}

fn private_class(name: &str) -> Result<&'static AnyClass> {
    let class = match name {
        "NSUUID" => AnyClass::get(c"NSUUID"),
        "REMReminder" => AnyClass::get(c"REMReminder"),
        "REMSaveRequest" => AnyClass::get(c"REMSaveRequest"),
        "REMStore" => AnyClass::get(c"REMStore"),
        _ => None,
    };
    class.with_context(|| format!("private macOS class {name} is unavailable"))
}

fn reminderkit_store() -> Result<Retained<AnyObject>> {
    let class = private_class("REMStore")?;
    Ok(unsafe { msg_send![class, new] })
}

fn reminderkit_reminder(store: &AnyObject, id: &str) -> Result<Retained<AnyObject>> {
    let uuid_class = private_class("NSUUID")?;
    let id = NSString::from_str(id);
    let uuid: Retained<AnyObject> =
        unsafe { msg_send![msg_send![uuid_class, alloc], initWithUUIDString: &*id] };
    let reminder_class = private_class("REMReminder")?;
    let object_id: Retained<AnyObject> =
        unsafe { msg_send![reminder_class, objectIDWithUUID: &*uuid] };
    let reminder: Option<Retained<AnyObject>> = unsafe {
        msg_send![store, fetchReminderWithObjectID: &*object_id, error: std::ptr::null_mut::<*mut NSError>()]
    };
    reminder.with_context(|| format!("ReminderKit could not find reminder {id}"))
}

fn reminderkit_parent_id(id: &str) -> Result<Option<String>> {
    let store = reminderkit_store()?;
    let reminder = reminderkit_reminder(&store, id)?;
    let parent: Option<Retained<AnyObject>> = unsafe { msg_send![&*reminder, parentReminder] };
    let Some(parent) = parent else {
        return Ok(None);
    };
    let storage: Retained<AnyObject> = unsafe { msg_send![&*parent, storage] };
    let object_id: Retained<AnyObject> = unsafe { msg_send![&*storage, objectID] };
    let uuid: Retained<AnyObject> = unsafe { msg_send![&*object_id, uuid] };
    let uuid_string: Retained<NSString> = unsafe { msg_send![&*uuid, UUIDString] };
    Ok(Some(uuid_string.to_string()))
}

fn set_reminderkit_parent(child_id: &str, parent_id: Option<&str>) -> Result<()> {
    let store = reminderkit_store()?;
    let child = reminderkit_reminder(&store, child_id)?;
    let save_class = private_class("REMSaveRequest")?;
    let request: Retained<AnyObject> =
        unsafe { msg_send![msg_send![save_class, alloc], initWithStore: &*store] };
    let child_change: Retained<AnyObject> =
        unsafe { msg_send![&*request, updateReminder: &*child] };
    if let Some(parent_id) = parent_id {
        let parent = reminderkit_reminder(&store, parent_id)?;
        let parent_change: Retained<AnyObject> =
            unsafe { msg_send![&*request, updateReminder: &*parent] };
        let context: Retained<AnyObject> = unsafe { msg_send![&*parent_change, subtaskContext] };
        let _: () = unsafe { msg_send![&*context, addReminderChangeItem: &*child_change] };
    } else {
        let _: () = unsafe { msg_send![&*child_change, removeFromParentReminder] };
    }
    let saved: Bool = unsafe {
        msg_send![&*request, saveSynchronouslyWithError: std::ptr::null_mut::<*mut NSError>()]
    };
    if !saved.as_bool() {
        bail!("ReminderKit failed to save the reminder hierarchy");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ReadReminderListSelectorArgs;
    use objc2_foundation::{NSCalendar, NSTimeZone};
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("icalctl-reminders-{name}-{nonce}.json"))
    }

    fn list(id: &str, title: &str, source: &str, source_id: &str) -> ReminderListReport {
        ReminderListReport {
            id: id.to_string(),
            title: title.to_string(),
            source: Some(source.to_string()),
            source_id: Some(source_id.to_string()),
            source_type: Some("caldav".to_string()),
            list_type: "caldav".to_string(),
            allows_modifications: true,
            is_immutable: false,
            is_subscribed: false,
            is_default_for_new_reminders: false,
        }
    }

    fn selector() -> ReadReminderListSelectorArgs {
        ReadReminderListSelectorArgs {
            lists: Vec::new(),
            list_ids: Vec::new(),
            list_source: None,
            source_id: None,
        }
    }

    fn reminder(id: &str, title: &str, completed: bool, due: Option<&str>) -> ReminderReport {
        ReminderReport {
            id: id.to_string(),
            title: title.to_string(),
            parent_id: None,
            child_count: 0,
            child_ids: None,
            completed,
            completion_date: None,
            priority: ReminderPriority::None,
            priority_value: 0,
            list: Some("Tasks".to_string()),
            list_id: Some("A".to_string()),
            list_source: Some("iCloud".to_string()),
            list_source_id: Some("S1".to_string()),
            list_type: Some("caldav".to_string()),
            allows_list_modifications: Some(true),
            list_selection: None,
            write_action: None,
            due: due.map(|date| ReminderDateReport {
                kind: ReminderDateKind::Date,
                date: Some(date.to_string()),
                local: None,
                normalized: None,
                utc: None,
                time_zone: None,
            }),
            due_input: None,
            start: None,
            start_input: None,
            notes: None,
            location: None,
            url: None,
            has_notes: false,
            has_url: false,
            alarm_count: None,
            recurrence_count: None,
            alarms: None,
            recurrence_rules: None,
            creation_date: None,
            last_modified_date: None,
            external_identifier: None,
            item_time_zone: None,
        }
    }

    fn filters(state: ReminderStateArg) -> ReminderReadFilterArgs {
        ReminderReadFilterArgs {
            list_selector: selector(),
            state,
            due_from: None,
            due_to: None,
        }
    }

    struct FakeStore {
        lists: Vec<ReminderListReport>,
        default_list: ReminderListReport,
        alternate_default_list: Option<ReminderListReport>,
        default_list_calls: Cell<usize>,
        reminders: RefCell<Vec<ReminderReport>>,
        creates: Cell<usize>,
        updates: Cell<usize>,
        lifecycle_updates: Cell<usize>,
        completion_updates: Cell<usize>,
        deletes: Cell<usize>,
        last_patch: RefCell<Option<ReminderAddPatch>>,
    }

    impl FakeStore {
        fn new() -> Self {
            let mut default_list = list("A", "Tasks", "iCloud", "S1");
            default_list.is_default_for_new_reminders = true;
            Self {
                lists: vec![default_list.clone()],
                default_list,
                alternate_default_list: None,
                default_list_calls: Cell::new(0),
                reminders: RefCell::new(Vec::new()),
                creates: Cell::new(0),
                updates: Cell::new(0),
                lifecycle_updates: Cell::new(0),
                completion_updates: Cell::new(0),
                deletes: Cell::new(0),
                last_patch: RefCell::new(None),
            }
        }
    }

    impl ReminderStore for FakeStore {
        fn authorization_status(&self) -> ReminderAuthorization {
            ReminderAuthorization::FullAccess
        }

        fn ensure_authorized(&self) -> Result<()> {
            Ok(())
        }

        fn lists(&self) -> Result<Vec<ReminderListReport>> {
            Ok(self.lists.clone())
        }

        fn default_list(&self) -> Result<ReminderListReport> {
            let calls = self.default_list_calls.get();
            self.default_list_calls.set(calls + 1);
            if calls > 0
                && let Some(alternate) = &self.alternate_default_list
            {
                return Ok(alternate.clone());
            }
            Ok(self.default_list.clone())
        }

        fn fetch(&self, list_ids: &[String]) -> Result<Vec<ReminderReport>> {
            Ok(self
                .reminders
                .borrow()
                .iter()
                .filter(|reminder| {
                    reminder
                        .list_id
                        .as_ref()
                        .is_some_and(|id| list_ids.contains(id))
                })
                .cloned()
                .collect())
        }

        fn get(&self, id: &str) -> Result<ReminderReport> {
            self.reminders
                .borrow()
                .iter()
                .find(|reminder| reminder.id == id)
                .cloned()
                .context("fake reminder not found")
        }

        fn create(&self, draft: &ReminderSaveDraft) -> Result<ReminderReport> {
            self.creates.set(self.creates.get() + 1);
            let mut report = reminder("CREATED", &draft.title, false, None);
            report.list_id = Some(draft.list_id.clone());
            report.parent_id = draft.parent_id.clone();
            report.due = draft.due.as_ref().map(|value| value.report.clone());
            report.start = draft.start.as_ref().map(|value| value.report.clone());
            report.notes = draft.notes.clone();
            report.location = draft.location.clone();
            report.url = draft.url.clone();
            report.priority_value = draft.priority_value;
            report.priority = priority_name(draft.priority_value);
            report.has_notes = report.notes.is_some();
            report.has_url = report.url.is_some();
            report.alarm_count =
                Some(draft.notifications.len() + usize::from(draft.geofence.is_some()));
            report.alarms = Some(alarm_reports_from_parsed(
                &draft.notifications,
                draft.geofence.as_ref(),
            ));
            report.recurrence_count = Some(usize::from(draft.recurrence.is_some()));
            report.recurrence_rules = Some(
                draft
                    .recurrence
                    .as_ref()
                    .map(recurrence_report_from_parsed)
                    .into_iter()
                    .collect(),
            );
            Ok(report)
        }

        fn update_add_fields(&self, id: &str, patch: &ReminderAddPatch) -> Result<ReminderReport> {
            self.updates.set(self.updates.get() + 1);
            self.last_patch.replace(Some(patch.clone()));
            let mut report = self.get(id)?;
            if let Some(parent_id) = &patch.parent_id {
                report.parent_id = Some(parent_id.clone());
            }
            if let Some(start) = &patch.start {
                report.start = Some(start.report.clone());
            }
            if let Some(notes) = &patch.notes {
                report.notes = Some(notes.clone());
                report.has_notes = true;
            }
            if let Some(location) = &patch.location {
                report.location = Some(location.clone());
            }
            if let Some(url) = &patch.url {
                report.url = Some(url.clone());
                report.has_url = true;
            }
            if let Some(priority) = patch.priority_value {
                report.priority_value = priority;
                report.priority = priority_name(priority);
            }
            if let Some(notifications) = &patch.notifications {
                let geofence = patch.geofence.as_ref().and_then(Option::as_ref);
                report.alarm_count = Some(notifications.len() + usize::from(geofence.is_some()));
                report.alarms = Some(alarm_reports_from_parsed(notifications, geofence));
            }
            if let Some(recurrence) = &patch.recurrence {
                report.recurrence_count = Some(1);
                report.recurrence_rules = Some(vec![recurrence_report_from_parsed(recurrence)]);
            }
            Ok(report)
        }

        fn update(&self, id: &str, patch: &ReminderLifecyclePatch) -> Result<ReminderReport> {
            self.lifecycle_updates.set(self.lifecycle_updates.get() + 1);
            let mut reminders = self.reminders.borrow_mut();
            let reminder = reminders
                .iter_mut()
                .find(|reminder| reminder.id == id)
                .context("fake reminder not found")?;
            let moved_list = patch
                .list_id
                .as_ref()
                .and_then(|id| self.lists.iter().find(|list| &list.id == id));
            *reminder = preview_lifecycle_patch(reminder, patch, moved_list);
            Ok(reminder.clone())
        }

        fn set_completion(
            &self,
            id: &str,
            completed_at: Option<DateTime<Utc>>,
        ) -> Result<ReminderReport> {
            self.completion_updates
                .set(self.completion_updates.get() + 1);
            let mut reminders = self.reminders.borrow_mut();
            let reminder = reminders
                .iter_mut()
                .find(|reminder| reminder.id == id)
                .context("fake reminder not found")?;
            reminder.completed = completed_at.is_some();
            reminder.completion_date = completed_at.map(|value| value.to_rfc3339());
            Ok(reminder.clone())
        }

        fn delete(&self, id: &str) -> Result<()> {
            self.deletes.set(self.deletes.get() + 1);
            let mut reminders = self.reminders.borrow_mut();
            let initial = reminders.len();
            reminders.retain(|reminder| reminder.id != id);
            if reminders.len() == initial {
                bail!("fake reminder not found");
            }
            Ok(())
        }
    }

    fn write_selector(list_id: Option<&str>) -> WriteReminderListSelectorArgs {
        WriteReminderListSelectorArgs {
            list: None,
            list_id: list_id.map(str::to_string),
            list_source: None,
            source_id: None,
        }
    }

    fn advanced_schedule() -> ReminderAdvancedScheduleArgs {
        ReminderAdvancedScheduleArgs {
            notify_at: Vec::new(),
            geofence_title: None,
            geofence_latitude: None,
            geofence_longitude: None,
            geofence_radius_meters: None,
            geofence_proximity: None,
            repeat: None,
            repeat_interval: None,
            repeat_count: None,
            repeat_until: None,
        }
    }

    fn add_command(title: &str) -> AddReminderCommand {
        AddReminderCommand {
            title: title.to_string(),
            list_selector: write_selector(Some("A")),
            parent_id: None,
            due: None,
            start: None,
            time_zone: None,
            notes: None,
            notes_file: None,
            url: None,
            location: None,
            priority: None,
            notify_at_due: false,
            notify_minutes_before: Vec::new(),
            schedule: advanced_schedule(),
            if_exists: IfExistsArg::Error,
            duplicate_window_seconds: 0,
            dry_run: true,
        }
    }

    #[test]
    fn exact_ids_can_select_multiple_lists() {
        let lists = vec![
            list("A", "Tasks", "iCloud", "S1"),
            list("B", "Work", "Exchange", "S2"),
        ];
        let mut selector = selector();
        selector.list_ids = vec!["B".to_string(), "A".to_string()];

        let resolved = resolve_lists(&lists, &selector).unwrap();

        assert_eq!(
            resolved
                .iter()
                .map(|list| list.id.as_str())
                .collect::<Vec<_>>(),
            ["B", "A"]
        );
    }

    #[test]
    fn duplicate_titles_fail_with_stable_candidates() {
        let lists = vec![
            list("A", "Tasks", "iCloud", "S1"),
            list("B", "Tasks", "Exchange", "S2"),
        ];
        let mut selector = selector();
        selector.lists = vec!["Tasks".to_string()];

        let error = resolve_lists(&lists, &selector).unwrap_err().to_string();

        assert!(error.contains("reminder list title \"Tasks\" is ambiguous"));
        assert!(error.contains("list_id=\"A\""));
        assert!(error.contains("source=\"Exchange\""));
        assert!(error.contains("writable=true"));
    }

    #[test]
    fn source_id_qualifies_duplicate_title() {
        let lists = vec![
            list("A", "Tasks", "iCloud", "S1"),
            list("B", "Tasks", "Exchange", "S2"),
        ];
        let mut selector = selector();
        selector.lists = vec!["Tasks".to_string()];
        selector.source_id = Some("S2".to_string());

        let resolved = resolve_lists(&lists, &selector).unwrap();

        assert_eq!(resolved[0].id, "B");
    }

    #[test]
    fn list_discovery_filters_exact_source_and_writability() {
        let mut readonly = list("B", "Read only", "Exchange", "S2");
        readonly.allows_modifications = false;
        let lists = vec![list("A", "Tasks", "iCloud", "S1"), readonly];

        let filtered = filter_list_discovery(lists, Some("iCloud"), true);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "A");
    }

    #[test]
    fn authorization_mapping_keeps_reminders_separate() {
        assert_eq!(
            ReminderAuthorization::from_ek(EKAuthorizationStatus::NotDetermined),
            ReminderAuthorization::NotDetermined
        );
        assert_eq!(
            ReminderAuthorization::from_ek(EKAuthorizationStatus::FullAccess),
            ReminderAuthorization::FullAccess
        );
        assert_eq!(
            ReminderAuthorization::from_ek(EKAuthorizationStatus::Denied),
            ReminderAuthorization::Denied
        );
    }

    #[test]
    fn priority_buckets_follow_rfc_5545() {
        assert_eq!(priority_name(0), ReminderPriority::None);
        assert_eq!(priority_name(1), ReminderPriority::High);
        assert_eq!(priority_name(4), ReminderPriority::High);
        assert_eq!(priority_name(5), ReminderPriority::Medium);
        assert_eq!(priority_name(6), ReminderPriority::Low);
        assert_eq!(priority_name(9), ReminderPriority::Low);
    }

    #[test]
    fn date_only_due_bound_includes_the_whole_day() {
        let bound = parse_due_upper_bound("2026-07-15").unwrap();
        assert!(bound.exclusive);
        assert_eq!(
            bound.instant.with_timezone(&Local).date_naive(),
            NaiveDate::from_ymd_opt(2026, 7, 16).unwrap()
        );
    }

    #[test]
    fn date_components_preserve_date_only_values() {
        let components = NSDateComponents::new();
        components.setYear(2026);
        components.setMonth(7);
        components.setDay(15);

        let report = date_components_report(&components).unwrap();

        assert_eq!(report.kind, ReminderDateKind::Date);
        assert_eq!(report.date.as_deref(), Some("2026-07-15"));
        assert!(report.normalized.is_none());
        assert!(report.utc.is_none());
    }

    #[test]
    fn private_parent_bridge_runtime_classes_are_available() {
        let _store = unsafe { EKEventStore::new() };
        for name in ["NSUUID", "REMReminder", "REMSaveRequest", "REMStore"] {
            assert!(private_class(name).is_ok(), "missing {name}");
        }
    }

    #[test]
    fn timed_components_expose_local_timezone_and_normalized_values() {
        let components = NSDateComponents::new();
        components.setCalendar(Some(&NSCalendar::currentCalendar()));
        let zone = NSTimeZone::timeZoneWithName(&NSString::from_str("Europe/Helsinki")).unwrap();
        components.setTimeZone(Some(&zone));
        components.setYear(2026);
        components.setMonth(7);
        components.setDay(15);
        components.setHour(14);
        components.setMinute(30);
        components.setSecond(0);

        let report = date_components_report(&components).unwrap();

        assert_eq!(report.kind, ReminderDateKind::Datetime);
        assert_eq!(report.local.as_deref(), Some("2026-07-15T14:30:00"));
        assert_eq!(report.time_zone.as_deref(), Some("Europe/Helsinki"));
        assert_eq!(
            report.normalized.as_deref(),
            Some("2026-07-15T14:30:00+03:00")
        );
        assert_eq!(report.utc.as_deref(), Some("2026-07-15T11:30:00+00:00"));
    }

    #[test]
    fn list_filters_default_state_due_range_and_undated_behavior() {
        let mut reminders = vec![
            reminder("A", "Before", false, Some("2026-07-09")),
            reminder("B", "Inside", false, Some("2026-07-15")),
            reminder("C", "Undated", false, None),
            reminder("D", "Completed", true, Some("2026-07-15")),
        ];
        let mut filters = filters(ReminderStateArg::Incomplete);
        filters.due_from = Some("2026-07-10".to_string());
        filters.due_to = Some("2026-07-15".to_string());

        apply_filters(&mut reminders, &filters, None).unwrap();

        assert_eq!(
            reminders
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["B"]
        );
    }

    #[test]
    fn search_is_case_insensitive_and_all_state_includes_completed() {
        let mut reminders = vec![
            reminder("A", "Submit Report", false, None),
            reminder("B", "Old REPORT", true, None),
            reminder("C", "Call dentist", false, None),
        ];

        apply_filters(
            &mut reminders,
            &filters(ReminderStateArg::All),
            Some("report"),
        )
        .unwrap();

        assert_eq!(
            reminders
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
    }

    #[test]
    fn reminder_json_uses_dedicated_top_level_type() {
        let output = JsonOutput::Reminders {
            reminders: vec![reminder("A", "Task", false, Some("2026-07-15"))],
        };
        let value = serde_json::to_value(output).unwrap();

        assert_eq!(value["type"], json!("reminders"));
        assert_eq!(value["reminders"][0]["due"]["kind"], json!("date"));
        assert_eq!(value["reminders"][0]["due"]["date"], json!("2026-07-15"));
    }

    #[test]
    fn add_dry_run_resolves_exact_list_and_never_calls_writes() {
        let store = FakeStore::new();
        let mut command = add_command("Submit report");
        command.due = Some("2026-07-15".to_string());
        command.notes = Some("Final version".to_string());
        command.location = Some("Office".to_string());
        command.url = Some("https://example.com/report".to_string());
        command.priority = Some(ReminderPriorityArg::High);

        let output = add_reminder(&store, command).unwrap();

        let JsonOutput::ReminderDryRun { would_write, draft } = output else {
            panic!("expected reminder dry run");
        };
        assert!(!would_write);
        assert_eq!(draft.operation, "create");
        assert_eq!(draft.list_id, "A");
        assert_eq!(draft.list_selection, ReminderListSelection::Explicit);
        assert_eq!(draft.due.as_ref().unwrap().kind, ReminderDateKind::Date);
        assert_eq!(draft.priority, ReminderPriority::High);
        assert_eq!(draft.priority_value, 1);
        assert_eq!(draft.notification_count, 0);
        assert!(draft.notifications.is_empty());
        assert!(draft.has_notes && draft.has_location && draft.has_url);
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);
    }

    #[test]
    fn add_dry_run_reports_default_list_provenance() {
        let store = FakeStore::new();
        let mut command = add_command("Undated task");
        command.list_selector = write_selector(None);

        let output = add_reminder(&store, command).unwrap();

        let JsonOutput::ReminderDryRun { draft, .. } = output else {
            panic!("expected reminder dry run");
        };
        assert_eq!(draft.list, "Tasks");
        assert_eq!(draft.list_id, "A");
        assert_eq!(draft.list_selection, ReminderListSelection::EventkitDefault);
        assert!(draft.due.is_none());
    }

    #[test]
    fn configured_default_list_resolves_by_exact_id() {
        let store = FakeStore::new();
        let selector = write_selector(None);

        let (list, selection) =
            resolve_write_list(&store.lists, None, Some("A"), &selector).unwrap();

        assert_eq!(list.id, "A");
        assert_eq!(selection, ReminderListSelection::ConfiguredDefault);
    }

    #[test]
    fn add_uses_configured_default_list_without_eventkit_default_lookup() {
        let mut store = FakeStore::new();
        store.lists.push(list("B", "Work", "iCloud", "S1"));
        let mut command = add_command("Configured task");
        command.list_selector = write_selector(None);

        let output = crate::config::with_test_config_contents(
            "reminder-default",
            "config_version = 1\n[reminders]\ndefault_list_id = \"B\"\n",
            || add_reminder(&store, command),
        )
        .unwrap();

        let JsonOutput::ReminderDryRun { draft, .. } = output else {
            panic!("expected reminder dry run");
        };
        assert_eq!(draft.list_id, "B");
        assert_eq!(
            draft.list_selection,
            ReminderListSelection::ConfiguredDefault
        );
        assert_eq!(store.default_list_calls.get(), 0);
    }

    #[test]
    fn parented_add_inherits_exact_parent_list_without_default_lookup() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("PARENT", "Large task", false, None));
        let mut command = add_command("Child task");
        command.list_selector = write_selector(None);
        command.parent_id = Some("PARENT".to_string());

        let output = add_reminder(&store, command).unwrap();
        let JsonOutput::ReminderDryRun { draft, .. } = output else {
            panic!("expected reminder dry run");
        };
        assert_eq!(draft.parent_id.as_deref(), Some("PARENT"));
        assert_eq!(draft.list_id, "A");
        assert_eq!(draft.list_selection, ReminderListSelection::Explicit);
        assert_eq!(store.default_list_calls.get(), 0);
    }

    #[test]
    fn parented_add_rejects_a_different_explicit_list() {
        let mut store = FakeStore::new();
        store.lists.push(list("B", "Work", "iCloud", "S1"));
        store
            .reminders
            .borrow_mut()
            .push(reminder("PARENT", "Large task", false, None));
        let mut command = add_command("Child task");
        command.list_selector = write_selector(Some("B"));
        command.parent_id = Some("PARENT".to_string());

        let error = add_reminder(&store, command).unwrap_err().to_string();
        assert!(error.contains("parent and child must use the same reminder list"));
        assert_eq!(store.creates.get(), 0);
    }

    #[test]
    fn notification_flags_resolve_absolute_instants_without_writing() {
        let store = FakeStore::new();
        let mut command = add_command("Timed task");
        command.due = Some("2099-12-31T14:30".to_string());
        command.time_zone = Some("Europe/Helsinki".to_string());
        command.notify_at_due = true;
        command.notify_minutes_before = vec![30, 10, 30];

        let output = add_reminder(&store, command).unwrap();

        let JsonOutput::ReminderDryRun { draft, .. } = output else {
            panic!("expected reminder dry run");
        };
        assert_eq!(draft.notification_count, 3);
        assert_eq!(
            draft
                .notifications
                .iter()
                .map(|value| value.minutes_before)
                .collect::<Vec<_>>(),
            [0, 10, 30]
        );
        assert_eq!(
            draft.notifications[0].absolute_utc,
            "2099-12-31T12:30:00+00:00"
        );
        assert_eq!(
            draft.notifications[1].absolute_in_due_time_zone,
            "2099-12-31T14:20:00+02:00"
        );
        assert_eq!(
            draft.notifications[2].absolute_in_due_time_zone,
            "2099-12-31T14:00:00+02:00"
        );
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);
    }

    #[test]
    fn notification_flags_require_timed_due_and_positive_minutes() {
        let store = FakeStore::new();

        let mut undated = add_command("Task");
        undated.notify_at_due = true;
        assert!(
            add_reminder(&store, undated)
                .unwrap_err()
                .to_string()
                .contains("require a timed --due")
        );

        let mut date_only = add_command("Task");
        date_only.due = Some("2026-07-15".to_string());
        date_only.notify_at_due = true;
        assert!(
            add_reminder(&store, date_only)
                .unwrap_err()
                .to_string()
                .contains("not a date-only due value")
        );

        let mut zero = add_command("Task");
        zero.due = Some("2026-07-15T14:30:00+03:00".to_string());
        zero.notify_minutes_before = vec![0];
        assert!(
            add_reminder(&store, zero)
                .unwrap_err()
                .to_string()
                .contains("must be greater than zero")
        );
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);
    }

    #[test]
    fn reminder_datetime_precedence_preserves_named_zone_or_explicit_offset() {
        let named = parse_reminder_date("2026-07-15T14:30", Some("Europe/Helsinki")).unwrap();
        assert_eq!(
            named.report.normalized.as_deref(),
            Some("2026-07-15T14:30:00+03:00")
        );
        assert_eq!(
            named.report.utc.as_deref(),
            Some("2026-07-15T11:30:00+00:00")
        );
        assert_eq!(named.report.time_zone.as_deref(), Some("Europe/Helsinki"));

        let offset =
            parse_reminder_date("2026-07-15T14:30:00+08:00", Some("Europe/Helsinki")).unwrap();
        assert_eq!(
            offset.report.normalized.as_deref(),
            Some("2026-07-15T14:30:00+08:00")
        );
        assert_eq!(
            offset.report.utc.as_deref(),
            Some("2026-07-15T06:30:00+00:00")
        );
        assert_eq!(offset.report.time_zone.as_deref(), Some("+08:00"));

        let components = reminder_date_components(&offset.components).unwrap();
        let read_back = date_components_report(&components).unwrap();
        assert_eq!(
            read_back.normalized.as_deref(),
            Some("2026-07-15T14:30:00+08:00")
        );
        assert_eq!(read_back.utc.as_deref(), Some("2026-07-15T06:30:00+00:00"));
    }

    #[test]
    fn duplicate_policies_error_skip_or_patch_non_identity_fields() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("EXISTING", "Task", false, Some("2026-07-15")));
        let mut error_command = add_command("Task");
        error_command.due = Some("2026-07-15".to_string());
        assert!(
            add_reminder(&store, error_command)
                .unwrap_err()
                .to_string()
                .contains("matching reminder already exists")
        );

        let mut skip_command = add_command("Task");
        skip_command.due = Some("2026-07-15".to_string());
        skip_command.if_exists = IfExistsArg::Skip;
        skip_command.dry_run = false;
        let JsonOutput::Reminder { reminder } = add_reminder(&store, skip_command).unwrap() else {
            panic!("expected reminder result");
        };
        assert_eq!(reminder.write_action.as_deref(), Some("skipped"));
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);

        let mut update_command = add_command("Task");
        update_command.due = Some("2026-07-15".to_string());
        update_command.notes = Some("Changed".to_string());
        update_command.priority = Some(ReminderPriorityArg::Low);
        update_command.if_exists = IfExistsArg::Update;
        update_command.dry_run = false;
        let JsonOutput::Reminder { reminder } = add_reminder(&store, update_command).unwrap()
        else {
            panic!("expected reminder result");
        };
        assert_eq!(reminder.write_action.as_deref(), Some("updated"));
        assert_eq!(reminder.notes.as_deref(), Some("Changed"));
        assert_eq!(reminder.priority, ReminderPriority::Low);
        assert_eq!(store.updates.get(), 1);
        let patch = store.last_patch.borrow();
        assert_eq!(patch.as_ref().unwrap().priority_value, Some(9));
        assert!(patch.as_ref().unwrap().notifications.is_none());
    }

    #[test]
    fn duplicate_update_replaces_notifications_only_when_supplied() {
        let store = FakeStore::new();
        let parsed = parse_reminder_date("2026-07-15T14:30", Some("Europe/Helsinki")).unwrap();
        let mut existing = reminder("EXISTING", "Task", false, None);
        existing.due = Some(parsed.report.clone());
        existing.alarm_count = Some(2);
        store.reminders.borrow_mut().push(existing);

        let mut command = add_command("Task");
        command.due = Some("2026-07-15T14:30".to_string());
        command.time_zone = Some("Europe/Helsinki".to_string());
        command.notify_minutes_before = vec![15];
        command.if_exists = IfExistsArg::Update;
        command.dry_run = false;

        let JsonOutput::Reminder { reminder } = add_reminder(&store, command).unwrap() else {
            panic!("expected reminder result");
        };
        assert_eq!(reminder.write_action.as_deref(), Some("updated"));
        assert_eq!(reminder.alarm_count, Some(1));
        let patch = store.last_patch.borrow();
        let notifications = patch.as_ref().unwrap().notifications.as_ref().unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].minutes_before, Some(15));
    }

    #[test]
    fn timed_duplicate_window_is_opt_in_and_date_only_stays_exact() {
        let existing = ReminderDateReport {
            kind: ReminderDateKind::Datetime,
            date: None,
            local: Some("2026-07-15T14:30:20".to_string()),
            normalized: Some("2026-07-15T14:30:20+03:00".to_string()),
            utc: Some("2026-07-15T11:30:20+00:00".to_string()),
            time_zone: Some("Europe/Helsinki".to_string()),
        };
        let requested = parse_reminder_date("2026-07-15T14:30", Some("Europe/Helsinki")).unwrap();
        assert!(!due_matches(Some(&existing), Some(&requested), 0));
        assert!(due_matches(Some(&existing), Some(&requested), 30));

        let date_existing = ReminderDateReport {
            kind: ReminderDateKind::Date,
            date: Some("2026-07-15".to_string()),
            local: None,
            normalized: None,
            utc: None,
            time_zone: None,
        };
        let date_requested = parse_reminder_date("2026-07-16", None).unwrap();
        assert!(!due_matches(
            Some(&date_existing),
            Some(&date_requested),
            86_400
        ));
    }

    #[test]
    fn add_validation_rejects_timezone_without_timed_fields_before_writes() {
        let store = FakeStore::new();
        let mut command = add_command("Task");
        command.due = Some("2026-07-15".to_string());
        command.time_zone = Some("Europe/Helsinki".to_string());

        let error = add_reminder(&store, command).unwrap_err().to_string();

        assert!(error.contains("--time-zone requires at least one timed"));
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);
    }

    #[test]
    fn add_validation_rejects_bad_url_window_and_title_before_writes() {
        let store = FakeStore::new();

        let mut bad_url = add_command("Task");
        bad_url.url = Some("not a valid URL []".to_string());
        assert!(
            add_reminder(&store, bad_url)
                .unwrap_err()
                .to_string()
                .contains("invalid URL")
        );

        let mut bad_window = add_command("Task");
        bad_window.duplicate_window_seconds = -1;
        assert!(
            add_reminder(&store, bad_window)
                .unwrap_err()
                .to_string()
                .contains("must not be negative")
        );

        assert!(
            add_reminder(&store, add_command("   "))
                .unwrap_err()
                .to_string()
                .contains("title must not be empty")
        );
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);
    }

    #[test]
    fn add_rejects_read_only_and_multiple_duplicate_targets() {
        let mut store = FakeStore::new();
        store.lists[0].allows_modifications = false;
        let error = add_reminder(&store, add_command("Task"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("reminder list is read-only"));

        let store = FakeStore::new();
        store.reminders.borrow_mut().extend([
            reminder("ONE", "Task", false, None),
            reminder("TWO", "Task", false, None),
        ]);
        let mut command = add_command("Task");
        command.if_exists = IfExistsArg::Skip;
        let error = add_reminder(&store, command).unwrap_err().to_string();
        assert!(error.contains("matched multiple reminders"));
        assert!(error.contains("ONE, TWO"));
        assert_eq!(store.creates.get(), 0);
        assert_eq!(store.updates.get(), 0);
    }

    fn update_command(id: &str) -> UpdateReminderCommand {
        UpdateReminderCommand {
            id: id.to_string(),
            title: None,
            list_selector: write_selector(None),
            parent_id: None,
            clear_parent: false,
            due: None,
            clear_due: false,
            start: None,
            clear_start: false,
            time_zone: None,
            clear_time_zone: false,
            notes: None,
            notes_file: None,
            clear_notes: false,
            url: None,
            clear_url: false,
            location: None,
            clear_location: false,
            priority: None,
            notify_at_due: false,
            notify_minutes_before: Vec::new(),
            schedule: advanced_schedule(),
            clear_notifications: false,
            clear_recurrence: false,
            dry_run: true,
        }
    }

    #[test]
    fn lifecycle_update_dry_run_previews_patch_and_performs_zero_writes() {
        let mut store = FakeStore::new();
        store.lists.push(list("B", "Work", "Exchange", "S2"));
        let mut existing = reminder("R1", "Old", false, Some("2026-07-15"));
        existing.notes = Some("old notes".to_string());
        existing.has_notes = true;
        existing.url = Some("https://example.com/old".to_string());
        existing.has_url = true;
        existing.alarm_count = Some(2);
        store.reminders.borrow_mut().push(existing);

        let mut command = update_command("R1");
        command.title = Some("New".to_string());
        command.list_selector = write_selector(Some("B"));
        command.due = Some("2026-07-16T09:30".to_string());
        command.time_zone = Some("Europe/Helsinki".to_string());
        command.clear_notes = true;
        command.clear_url = true;
        command.priority = Some(ReminderPriorityArg::High);

        let output = update_reminder(&store, command).unwrap();
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["type"], "reminder_mutation_dry_run");
        assert_eq!(json["would_write"], false);
        assert_eq!(json["draft"]["result"]["list_id"], "B");
        let JsonOutput::ReminderMutationDryRun { would_write, draft } = output else {
            panic!("expected reminder mutation dry run");
        };
        assert!(!would_write);
        assert_eq!(draft.operation, "update");
        assert_eq!(draft.result.title, "New");
        assert_eq!(draft.result.list_id.as_deref(), Some("B"));
        assert_eq!(draft.result.list.as_deref(), Some("Work"));
        assert!(draft.result.notes.is_none());
        assert!(draft.result.url.is_none());
        assert_eq!(draft.result.priority, ReminderPriority::High);
        assert_eq!(draft.result.alarm_count, Some(2));
        assert_eq!(
            draft
                .result
                .due
                .as_ref()
                .and_then(|due| due.time_zone.as_deref()),
            Some("Europe/Helsinki")
        );
        assert_eq!(store.lifecycle_updates.get(), 0);
        assert_eq!(store.completion_updates.get(), 0);
        assert_eq!(store.deletes.get(), 0);
    }

    #[test]
    fn lifecycle_update_can_reparent_clear_parent_and_reject_cycles() {
        let store = FakeStore::new();
        let child = reminder("CHILD", "Child", false, None);
        let parent = reminder("PARENT", "Parent", false, None);
        store.reminders.borrow_mut().extend([child, parent]);

        let mut reparent = update_command("CHILD");
        reparent.parent_id = Some("PARENT".to_string());
        let output = update_reminder(&store, reparent).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected mutation dry run");
        };
        assert_eq!(draft.changed_fields, ["parent"]);
        assert_eq!(draft.result.parent_id.as_deref(), Some("PARENT"));

        store.reminders.borrow_mut()[0].parent_id = Some("PARENT".to_string());
        let mut clear = update_command("CHILD");
        clear.clear_parent = true;
        let output = update_reminder(&store, clear).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected mutation dry run");
        };
        assert!(draft.result.parent_id.is_none());

        store.reminders.borrow_mut()[1].parent_id = Some("CHILD".to_string());
        let mut cycle = update_command("CHILD");
        cycle.parent_id = Some("PARENT".to_string());
        let error = update_reminder(&store, cycle).unwrap_err().to_string();
        assert!(error.contains("would create a reminder hierarchy cycle"));
        assert_eq!(store.lifecycle_updates.get(), 0);
    }

    #[test]
    fn parent_mutations_that_may_cascade_are_blocked() {
        let store = FakeStore::new();
        let mut parent = reminder("PARENT", "Parent", false, None);
        parent.child_count = 1;
        parent.child_ids = Some(vec!["CHILD".to_string()]);
        store.reminders.borrow_mut().push(parent);

        let complete = complete_reminder(&store, "PARENT", None, true)
            .unwrap_err()
            .to_string();
        assert!(complete.contains("cannot complete parent reminder"));
        let delete = delete_reminder(&store, "PARENT", true)
            .unwrap_err()
            .to_string();
        assert!(delete.contains("cannot delete parent reminder"));
        assert_eq!(store.completion_updates.get(), 0);
        assert_eq!(store.deletes.get(), 0);
    }

    #[test]
    fn lifecycle_update_live_clears_dates_and_preserves_omitted_fields() {
        let store = FakeStore::new();
        let mut existing = reminder("R1", "Task", false, Some("2026-07-15"));
        existing.start = Some(parse_reminder_date("2026-07-14", None).unwrap().report);
        existing.notes = Some("keep me".to_string());
        existing.has_notes = true;
        store.reminders.borrow_mut().push(existing);

        let mut command = update_command("R1");
        command.clear_due = true;
        command.clear_start = true;
        command.location = Some("Office".to_string());
        command.dry_run = false;
        let output = update_reminder(&store, command).unwrap();
        let JsonOutput::Reminder { reminder } = output else {
            panic!("expected reminder result");
        };
        assert!(reminder.due.is_none());
        assert!(reminder.start.is_none());
        assert_eq!(reminder.notes.as_deref(), Some("keep me"));
        assert_eq!(reminder.location.as_deref(), Some("Office"));
        assert_eq!(reminder.write_action.as_deref(), Some("updated"));
        assert_eq!(store.lifecycle_updates.get(), 1);
    }

    #[test]
    fn lifecycle_update_can_set_and_clear_existing_timezone_metadata() {
        let store = FakeStore::new();
        let mut existing = reminder("R1", "Task", false, None);
        existing.due = Some(
            parse_reminder_date("2026-07-15T14:30", None)
                .unwrap()
                .report,
        );
        store.reminders.borrow_mut().push(existing);

        let mut set_zone = update_command("R1");
        set_zone.time_zone = Some("Europe/Helsinki".to_string());
        set_zone.dry_run = false;
        let output = update_reminder(&store, set_zone).unwrap();
        let JsonOutput::Reminder { reminder } = output else {
            panic!("expected reminder result");
        };
        assert_eq!(reminder.due_input, None);
        assert_eq!(
            store.reminders.borrow()[0]
                .due
                .as_ref()
                .and_then(|due| due.time_zone.as_deref()),
            Some("Europe/Helsinki")
        );

        let mut clear_zone = update_command("R1");
        clear_zone.clear_time_zone = true;
        clear_zone.dry_run = false;
        update_reminder(&store, clear_zone).unwrap();
        assert!(
            store.reminders.borrow()[0]
                .due
                .as_ref()
                .and_then(|due| due.time_zone.as_ref())
                .is_none()
        );
    }

    #[test]
    fn lifecycle_timezone_clear_does_not_resolve_floating_wall_clock_through_local_dst() {
        let store = FakeStore::new();
        let mut existing = reminder("R1", "Task", false, None);
        existing.due = Some(ReminderDateReport {
            kind: ReminderDateKind::Datetime,
            date: None,
            local: Some("2026-03-29T03:30:00".to_string()),
            normalized: Some("2026-03-29T03:30:00+09:00".to_string()),
            utc: Some("2026-03-28T18:30:00+00:00".to_string()),
            time_zone: Some("Asia/Tokyo".to_string()),
        });
        store.reminders.borrow_mut().push(existing);

        let mut command = update_command("R1");
        command.clear_time_zone = true;
        let output = update_reminder(&store, command).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected reminder mutation dry run");
        };
        let due = draft.result.due.as_ref().unwrap();
        assert_eq!(due.local.as_deref(), Some("2026-03-29T03:30:00"));
        assert!(due.time_zone.is_none());
        assert!(due.normalized.is_none());
        assert!(due.utc.is_none());
        assert!(draft.result.due_input.is_none());
        assert_eq!(store.lifecycle_updates.get(), 0);
    }

    #[test]
    fn lifecycle_update_explicit_offset_wins_and_live_echoes_user_input() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Task", false, None));
        let input = "2026-07-16T09:30:00+08:00";
        let mut dry_run = update_command("R1");
        dry_run.due = Some(input.to_string());
        dry_run.time_zone = Some("Europe/Helsinki".to_string());
        let output = update_reminder(&store, dry_run).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected reminder mutation dry run");
        };
        assert_eq!(draft.result.due_input.as_deref(), Some(input));

        let mut command = update_command("R1");
        command.due = Some(input.to_string());
        command.time_zone = Some("Europe/Helsinki".to_string());
        command.dry_run = false;

        let output = update_reminder(&store, command).unwrap();
        let JsonOutput::Reminder { reminder } = output else {
            panic!("expected reminder result");
        };
        assert_eq!(reminder.due_input.as_deref(), Some(input));
        let due = reminder.due.as_ref().unwrap();
        assert_eq!(due.time_zone.as_deref(), Some("+08:00"));
        assert_eq!(due.utc.as_deref(), Some("2026-07-16T01:30:00+00:00"));
    }

    #[test]
    fn complete_and_uncomplete_preserve_explicit_timestamp_and_dry_run_is_safe() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Task", false, None));

        let dry_run =
            complete_reminder(&store, "R1", Some("2026-07-11T14:00:00+03:00"), true).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = dry_run else {
            panic!("expected completion dry run");
        };
        assert!(draft.result.completed);
        assert_eq!(
            draft.result.completion_date.as_deref(),
            Some("2026-07-11T11:00:00+00:00")
        );
        assert_eq!(store.completion_updates.get(), 0);

        complete_reminder(&store, "R1", Some("2026-07-11T14:00:00+03:00"), false).unwrap();
        assert!(store.reminders.borrow()[0].completed);
        assert_eq!(store.completion_updates.get(), 1);

        uncomplete_reminder(&store, "R1", false).unwrap();
        assert!(!store.reminders.borrow()[0].completed);
        assert!(store.reminders.borrow()[0].completion_date.is_none());
        assert_eq!(store.completion_updates.get(), 2);
    }

    #[test]
    fn completion_rejects_timestamp_without_offset_before_writing() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Task", false, None));
        let error = complete_reminder(&store, "R1", Some("2026-07-11T14:00:00"), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("explicit UTC offset"));
        assert_eq!(store.completion_updates.get(), 0);
    }

    #[test]
    fn forced_delete_removes_exact_reminder() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Task", false, None));
        let output = delete_reminder(&store, "R1", true).unwrap();
        let JsonOutput::ReminderDeleted { deleted } = output else {
            panic!("expected reminder deletion report");
        };
        assert_eq!(deleted.id, "R1");
        assert_eq!(deleted.list_id.as_deref(), Some("A"));
        assert!(store.reminders.borrow().is_empty());
        assert_eq!(store.deletes.get(), 1);
    }

    #[test]
    fn lifecycle_rejects_noop_and_read_only_mutations() {
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Task", false, None));
        assert!(
            update_reminder(&store, update_command("R1"))
                .unwrap_err()
                .to_string()
                .contains("at least one field change")
        );

        store.reminders.borrow_mut()[0].allows_list_modifications = Some(false);
        assert!(
            uncomplete_reminder(&store, "R1", true)
                .unwrap_err()
                .to_string()
                .contains("read-only")
        );
        assert_eq!(store.lifecycle_updates.get(), 0);
        assert_eq!(store.completion_updates.get(), 0);

        store.reminders.borrow_mut()[0].allows_list_modifications = None;
        assert!(
            complete_reminder(&store, "R1", None, true)
                .unwrap_err()
                .to_string()
                .contains("writability is unknown")
        );
        assert_eq!(store.completion_updates.get(), 0);
    }

    #[test]
    fn absolute_alarm_and_recurrence_dry_run_are_deterministic() {
        let store = FakeStore::new();
        let mut command = add_command("Recurring task");
        command.due = Some("2026-07-15T09:00:00+03:00".to_string());
        command.schedule.notify_at = vec![
            "2026-07-15T07:00:00+03:00".to_string(),
            "2026-07-15T04:00:00+00:00".to_string(),
        ];
        command.schedule.repeat = Some(ReminderRepeatArg::Weekly);
        command.schedule.repeat_interval = Some(2);
        command.schedule.repeat_count = Some(6);

        let output = add_reminder(&store, command).unwrap();
        let JsonOutput::ReminderDryRun { draft, .. } = output else {
            panic!("expected reminder dry run");
        };
        assert_eq!(draft.notification_count, 0);
        assert!(draft.notifications.is_empty());
        assert_eq!(draft.planned_alarm_count, 1);
        assert_eq!(draft.planned_alarms[0].kind, "absolute");
        assert_eq!(
            draft.planned_alarms[0].absolute_utc.as_deref(),
            Some("2026-07-15T04:00:00+00:00")
        );
        let recurrence = draft.recurrence.as_ref().unwrap();
        assert_eq!(recurrence.frequency, "weekly");
        assert_eq!(recurrence.interval, 2);
        assert_eq!(recurrence.end.occurrence_count, Some(6));
        assert_eq!(store.creates.get(), 0);
    }

    #[test]
    fn advanced_schedule_validation_rejects_bad_offsets_geofences_and_recurrence() {
        assert!(
            parse_absolute_notifications(&["2026-07-15T09:00:00".to_string()])
                .unwrap_err()
                .to_string()
                .contains("explicit UTC offset")
        );

        let mut schedule = advanced_schedule();
        schedule.geofence_title = Some("Office".to_string());
        schedule.geofence_latitude = Some(91.0);
        schedule.geofence_longitude = Some(24.9384);
        schedule.geofence_radius_meters = Some(100.0);
        schedule.geofence_proximity = Some(ReminderGeofenceProximityArg::Arrive);
        assert!(
            parse_geofence(&schedule)
                .unwrap_err()
                .to_string()
                .contains("-90 through 90")
        );

        let mut schedule = advanced_schedule();
        schedule.repeat = Some(ReminderRepeatArg::Daily);
        schedule.repeat_interval = Some(0);
        assert!(
            parse_recurrence(&schedule)
                .unwrap_err()
                .to_string()
                .contains("greater than zero")
        );
    }

    #[test]
    fn geofence_plan_preserves_coordinates_radius_and_proximity() {
        let mut schedule = advanced_schedule();
        schedule.geofence_title = Some("Office".to_string());
        schedule.geofence_latitude = Some(60.1699);
        schedule.geofence_longitude = Some(24.9384);
        schedule.geofence_radius_meters = Some(150.0);
        schedule.geofence_proximity = Some(ReminderGeofenceProximityArg::Leave);
        let geofence = parse_geofence(&schedule).unwrap().unwrap();
        let reports = planned_alarm_reports(&[], Some(&geofence));

        assert_eq!(reports[0].kind, "geofence");
        assert_eq!(reports[0].proximity.as_deref(), Some("leave"));
        let location = reports[0].structured_location.as_ref().unwrap();
        assert_eq!(location.title.as_deref(), Some("Office"));
        assert_eq!(location.latitude, Some(60.1699));
        assert_eq!(location.longitude, Some(24.9384));
        assert_eq!(location.radius_meters, 150.0);
    }

    #[test]
    fn lifecycle_can_replace_or_clear_alarms_and_recurrence_without_writes() {
        let store = FakeStore::new();
        let mut existing = reminder("R1", "Task", false, Some("2026-07-15"));
        existing.alarm_count = Some(2);
        existing.alarms = Some(vec![]);
        existing.recurrence_count = Some(1);
        existing.recurrence_rules = Some(vec![]);
        store.reminders.borrow_mut().push(existing);

        let mut replace = update_command("R1");
        replace.schedule.notify_at = vec!["2026-07-14T09:00:00+03:00".to_string()];
        replace.schedule.repeat = Some(ReminderRepeatArg::Monthly);
        replace.schedule.repeat_until = Some("2026-12-31T23:59:00+02:00".to_string());
        let output = update_reminder(&store, replace).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected mutation dry run");
        };
        assert_eq!(draft.result.alarm_count, Some(1));
        assert_eq!(draft.result.alarms.as_ref().unwrap()[0].proximity, "none");
        assert_eq!(draft.result.recurrence_count, Some(1));
        assert_eq!(
            draft.result.recurrence_rules.as_ref().unwrap()[0].frequency,
            "monthly"
        );
        assert_eq!(store.lifecycle_updates.get(), 0);

        let mut clear = update_command("R1");
        clear.clear_notifications = true;
        clear.clear_recurrence = true;
        let output = update_reminder(&store, clear).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected mutation dry run");
        };
        assert_eq!(draft.result.alarm_count, Some(0));
        assert!(draft.result.alarms.as_ref().unwrap().is_empty());
        assert_eq!(draft.result.recurrence_count, Some(0));
        assert!(draft.result.recurrence_rules.as_ref().unwrap().is_empty());
        assert_eq!(store.lifecycle_updates.get(), 0);
    }

    #[test]
    fn recurrence_requires_a_due_or_start_anchor() {
        let store = FakeStore::new();
        let mut command = add_command("Task");
        command.schedule.repeat = Some(ReminderRepeatArg::Daily);
        assert!(
            add_reminder(&store, command)
                .unwrap_err()
                .to_string()
                .contains("requires a due or start")
        );
        assert_eq!(store.creates.get(), 0);
    }

    #[test]
    fn existing_recurrence_cannot_lose_its_final_anchor_without_clear() {
        let store = FakeStore::new();
        let mut existing = reminder("R1", "Task", false, Some("2026-07-15"));
        existing.recurrence_count = Some(1);
        existing.recurrence_rules = Some(vec![recurrence_report_from_parsed(
            &ParsedReminderRecurrence {
                frequency: ReminderRepeatArg::Daily,
                interval: 1,
                count: None,
                until_utc: None,
            },
        )]);
        store.reminders.borrow_mut().push(existing);

        let mut invalid = update_command("R1");
        invalid.clear_due = true;
        assert!(
            update_reminder(&store, invalid)
                .unwrap_err()
                .to_string()
                .contains("clear recurrence")
        );
        assert_eq!(store.lifecycle_updates.get(), 0);

        let mut valid = update_command("R1");
        valid.clear_due = true;
        valid.clear_recurrence = true;
        let output = update_reminder(&store, valid).unwrap();
        let JsonOutput::ReminderMutationDryRun { draft, .. } = output else {
            panic!("expected mutation dry run");
        };
        assert!(draft.result.due.is_none());
        assert_eq!(draft.result.recurrence_count, Some(0));
    }

    #[test]
    fn recurrence_until_must_not_precede_resulting_anchor() {
        let store = FakeStore::new();
        let mut add = add_command("Task");
        add.due = Some("2026-07-15T09:00:00+03:00".to_string());
        add.schedule.repeat = Some(ReminderRepeatArg::Daily);
        add.schedule.repeat_until = Some("2026-07-14T09:00:00+03:00".to_string());
        assert!(
            add_reminder(&store, add)
                .unwrap_err()
                .to_string()
                .contains("must not be before")
        );

        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Task", false, Some("2026-07-15")));
        let mut update = update_command("R1");
        update.schedule.repeat = Some(ReminderRepeatArg::Weekly);
        update.schedule.repeat_until = Some("2026-07-14T23:59:00+03:00".to_string());
        assert!(
            update_reminder(&store, update)
                .unwrap_err()
                .to_string()
                .contains("must not be before")
        );
        assert_eq!(store.lifecycle_updates.get(), 0);
    }

    #[test]
    fn duplicate_update_validates_recurrence_against_preserved_matched_due() {
        let store = FakeStore::new();
        let existing_due = parse_reminder_date("2026-07-15T09:00:00+03:00", None).unwrap();
        let mut existing = reminder("R1", "Task", false, None);
        existing.due = Some(existing_due.report);
        store.reminders.borrow_mut().push(existing);

        let mut command = add_command("Task");
        command.due = Some("2026-07-15T08:59:00+03:00".to_string());
        command.duplicate_window_seconds = 120;
        command.if_exists = IfExistsArg::Update;
        command.schedule.repeat = Some(ReminderRepeatArg::Daily);
        command.schedule.repeat_until = Some("2026-07-15T08:59:30+03:00".to_string());

        assert!(
            add_reminder(&store, command)
                .unwrap_err()
                .to_string()
                .contains("must not be before")
        );
        assert_eq!(store.updates.get(), 0);
    }

    #[test]
    fn absolute_alarms_keep_distinct_submillisecond_instants() {
        let notifications = parse_absolute_notifications(&[
            "2026-07-15T09:00:00.000100+03:00".to_string(),
            "2026-07-15T09:00:00.000900+03:00".to_string(),
        ])
        .unwrap();
        assert_eq!(notifications.len(), 2);
        let first = nsdate_from_utc(notifications[0].absolute_utc);
        let second = nsdate_from_utc(notifications[1].absolute_utc);
        assert!(second.timeIntervalSince1970() > first.timeIntervalSince1970());
    }

    fn json_cli_command(path: PathBuf) -> AddReminderCliCommand {
        AddReminderCliCommand {
            title: None,
            json_file: Some(path),
            list_selector: write_selector(None),
            parent_id: None,
            due: None,
            start: None,
            time_zone: None,
            notes: None,
            notes_file: None,
            url: None,
            location: None,
            priority: None,
            notify_at_due: false,
            notify_minutes_before: Vec::new(),
            schedule: advanced_schedule(),
            if_exists: IfExistsArg::Error,
            duplicate_window_seconds: 0,
            dry_run: true,
        }
    }

    #[test]
    fn structured_json_add_is_strict_and_uses_the_normal_dry_run_pipeline() {
        let path = temporary_test_path("single");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "title": "JSON task",
                "list_id": "A",
                "parent_id": "PARENT",
                "due": "2026-07-15T09:00:00+03:00",
                "notes": "Exact notes",
                "priority": "high",
                "notify_at": ["2026-07-15T07:00:00+03:00"],
                "recurrence": {"frequency": "weekly", "count": 4}
            }))
            .unwrap(),
        )
        .unwrap();
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("PARENT", "Parent task", false, None));
        let output = add_reminder_from_cli(&store, json_cli_command(path.clone())).unwrap();
        fs::remove_file(&path).unwrap();

        let JsonOutput::ReminderDryRun { draft, .. } = output else {
            panic!("expected reminder dry run");
        };
        assert_eq!(draft.title, "JSON task");
        assert_eq!(draft.list_id, "A");
        assert_eq!(draft.parent_id.as_deref(), Some("PARENT"));
        assert_eq!(draft.priority, ReminderPriority::High);
        assert_eq!(draft.planned_alarm_count, 1);
        assert_eq!(
            draft.recurrence.as_ref().unwrap().end.occurrence_count,
            Some(4)
        );
        assert_eq!(store.creates.get(), 0);

        let path = temporary_test_path("unknown");
        fs::write(&path, r#"{"title":"Task","unknown":true}"#).unwrap();
        let error = format!(
            "{:#}",
            add_reminder_from_cli(&store, json_cli_command(path.clone())).unwrap_err()
        );
        fs::remove_file(path).unwrap();
        assert!(error.contains("unknown field"));
    }

    #[test]
    fn structured_json_add_rejects_mixed_cli_fields() {
        let store = FakeStore::new();
        let mut command = json_cli_command(PathBuf::from("unused.json"));
        command.title = Some("CLI task".to_string());
        assert!(
            add_reminder_from_cli(&store, command)
                .unwrap_err()
                .to_string()
                .contains("cannot be combined")
        );
    }

    #[test]
    fn reminder_batch_dry_run_merges_defaults_and_reports_plans() {
        let path = temporary_test_path("batch-valid");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "defaults": {"list_id": "A", "parent_id": "PARENT", "priority": "medium"},
                "reminders": [
                    {"client_id": "one", "title": "First", "due": "2026-07-15"},
                    {
                        "client_id": "two",
                        "title": "Second",
                        "due": "2026-07-16T09:00:00+03:00",
                        "notify_at_due": true,
                        "recurrence": {"frequency": "daily", "interval": 2, "count": 3}
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("PARENT", "Parent task", false, None));
        let output = reminder_batch_add(&store, &path, IfExistsArg::Error, true, false).unwrap();
        fs::remove_file(path).unwrap();

        let value = serde_json::to_value(&output).unwrap();
        assert_eq!(value["type"], "reminder_batch");
        assert_eq!(value["batch"]["version"], 1);
        assert_eq!(value["batch"]["summary"]["would_create"], 2);

        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert!(batch.can_write);
        assert_eq!(batch.summary.total, 2);
        assert!(
            batch
                .items
                .iter()
                .all(|item| item.draft.as_ref().unwrap().parent_id.as_deref() == Some("PARENT"))
        );
        assert_eq!(batch.summary.would_create, 2);
        assert_eq!(batch.items[0].client_id.as_deref(), Some("one"));
        assert_eq!(
            batch.items[0].draft.as_ref().unwrap().priority,
            ReminderPriority::Medium
        );
        assert_eq!(batch.items[1].draft.as_ref().unwrap().notification_count, 1);
        assert_eq!(store.creates.get(), 0);
    }

    #[test]
    fn reminder_batch_timezone_default_only_applies_to_timed_rows() {
        let path = temporary_test_path("batch-mixed-dates");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "defaults": {"list_id": "A", "time_zone": "Europe/Helsinki"},
                "reminders": [
                    {"title": "Undated"},
                    {"title": "Date only", "due": "2026-07-15"},
                    {"title": "Timed", "due": "2026-07-16T09:00"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let store = FakeStore::new();
        let output = reminder_batch_add(&store, &path, IfExistsArg::Error, true, false).unwrap();
        fs::remove_file(path).unwrap();

        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert!(batch.can_write);
        assert_eq!(batch.summary.would_create, 3);
        assert_eq!(batch.items[0].draft.as_ref().unwrap().due, None);
        assert_eq!(
            batch.items[1]
                .draft
                .as_ref()
                .unwrap()
                .due
                .as_ref()
                .unwrap()
                .kind,
            ReminderDateKind::Date
        );
        assert_eq!(
            batch.items[2]
                .draft
                .as_ref()
                .unwrap()
                .due
                .as_ref()
                .unwrap()
                .time_zone
                .as_deref(),
            Some("Europe/Helsinki")
        );
    }

    #[test]
    fn reminder_batch_null_clears_inherited_timezone_geofence_and_recurrence() {
        let defaults: ReminderBatchDefaults = serde_json::from_value(json!({
            "time_zone": "Europe/Helsinki",
            "geofence": {
                "title": "Office",
                "latitude": 60.17,
                "longitude": 24.94,
                "radius_meters": 100.0,
                "proximity": "arrive"
            },
            "recurrence": {"frequency": "daily"}
        }))
        .unwrap();
        let draft: ReminderJsonDraft = serde_json::from_value(json!({
            "title": "One-off",
            "due": "2026-07-15T09:00",
            "time_zone": null,
            "geofence": null,
            "recurrence": null
        }))
        .unwrap();
        let command = json_draft_to_command(&defaults, draft, IfExistsArg::Error, 0, true).unwrap();

        assert_eq!(command.time_zone, None);
        assert!(advanced_schedule_is_empty(&command.schedule));

        let explicit_offset: ReminderJsonDraft = serde_json::from_value(json!({
            "title": "Offset",
            "due": "2026-07-15T09:00:00+03:00"
        }))
        .unwrap();
        let command =
            json_draft_to_command(&defaults, explicit_offset, IfExistsArg::Error, 0, true).unwrap();
        assert_eq!(command.time_zone, None);
    }

    #[test]
    fn reminder_batch_pins_default_list_resolved_during_preflight() {
        let path = temporary_test_path("batch-pinned-list");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "reminders": [{"title": "Pinned target", "due": "2026-07-15"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let mut store = FakeStore::new();
        let alternate = list("B", "Other", "iCloud", "S1");
        store.lists.push(alternate.clone());
        store.alternate_default_list = Some(alternate);

        let output = reminder_batch_add(&store, &path, IfExistsArg::Error, false, false).unwrap();
        fs::remove_file(path).unwrap();
        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert_eq!(batch.summary.created, 1);
        assert_eq!(batch.items[0].draft.as_ref().unwrap().list_id, "A");
        assert_eq!(store.default_list_calls.get(), 1);
    }

    #[test]
    fn reminder_batch_preflight_blocks_all_or_continues_explicitly() {
        let contents = serde_json::to_vec(&json!({
            "version": 1,
            "defaults": {"list_id": "A"},
            "reminders": [
                {"client_id": "bad", "title": "Bad", "unknown": true},
                {"client_id": "good", "title": "Good", "due": "2026-07-15"}
            ]
        }))
        .unwrap();

        let path = temporary_test_path("batch-blocked");
        fs::write(&path, &contents).unwrap();
        let store = FakeStore::new();
        let output = reminder_batch_add(&store, &path, IfExistsArg::Error, false, false).unwrap();
        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert!(!batch.can_write);
        assert_eq!(batch.summary.failed, 1);
        assert_eq!(batch.summary.not_attempted, 1);
        assert_eq!(store.creates.get(), 0);

        let store = FakeStore::new();
        let output = reminder_batch_add(&store, &path, IfExistsArg::Error, false, true).unwrap();
        fs::remove_file(path).unwrap();
        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert!(batch.can_write);
        assert_eq!(batch.summary.failed, 1);
        assert_eq!(batch.summary.created, 1);
        assert_eq!(store.creates.get(), 1);
    }

    #[test]
    fn reminder_batch_rejects_duplicate_client_ids_and_identities() {
        let path = temporary_test_path("batch-duplicates");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "defaults": {"list_id": "A"},
                "reminders": [
                    {"client_id": "same", "title": "Task", "due": "2026-07-15"},
                    {"client_id": "same", "title": "Other", "due": "2026-07-16"},
                    {"client_id": "three", "title": "Duplicate", "due": "2026-07-17T09:00:00+03:00"},
                    {"client_id": "four", "title": "Duplicate", "due": "2026-07-17T08:00:00+02:00"}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let store = FakeStore::new();
        let output = reminder_batch_add(&store, &path, IfExistsArg::Skip, true, false).unwrap();
        fs::remove_file(path).unwrap();
        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert_eq!(batch.summary.failed, 4);
        assert!(
            batch.items[0]
                .error
                .as_ref()
                .unwrap()
                .message
                .contains("client_id")
        );
        assert!(
            batch.items[2]
                .error
                .as_ref()
                .unwrap()
                .message
                .contains("rows 3, 4")
        );
        assert_eq!(store.creates.get(), 0);
    }

    #[test]
    fn reminder_batch_reports_existing_skip_and_update_actions() {
        let path = temporary_test_path("batch-existing");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "defaults": {"list_id": "A"},
                "reminders": [{"client_id": "one", "title": "Existing", "due": "2026-07-15", "notes": "Changed"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let store = FakeStore::new();
        store
            .reminders
            .borrow_mut()
            .push(reminder("R1", "Existing", false, Some("2026-07-15")));
        let output = reminder_batch_add(&store, &path, IfExistsArg::Skip, true, false).unwrap();
        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert_eq!(batch.summary.would_skip, 1);
        assert_eq!(batch.items[0].matched_reminder_id.as_deref(), Some("R1"));

        let output = reminder_batch_add(&store, &path, IfExistsArg::Update, false, false).unwrap();
        fs::remove_file(path).unwrap();
        let JsonOutput::ReminderBatch { batch } = output else {
            panic!("expected reminder batch");
        };
        assert_eq!(batch.summary.updated, 1);
        assert_eq!(batch.items[0].status, "updated");
        assert_eq!(batch.items[0].reminder_id.as_deref(), Some("R1"));
        assert_eq!(batch.items[0].matched_reminder_id.as_deref(), Some("R1"));
        let draft = batch.items[0].draft.as_ref().unwrap();
        assert_eq!(draft.operation, "update");
        assert_eq!(draft.matched_reminder_id.as_deref(), Some("R1"));
        assert_eq!(store.updates.get(), 1);
    }
}
