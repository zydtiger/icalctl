use crate::cache::resolve_reminder_ref;
use crate::cli::{
    IfExistsArg, ReadReminderListSelectorArgs, ReminderPriorityArg, ReminderReadFilterArgs,
    ReminderStateArg, RemindersCommand, WriteReminderListSelectorArgs,
};
use crate::dates::{
    parse_end_datetime, parse_start_datetime, parse_start_datetime_in_time_zone, validate_time_zone,
};
use crate::models::{
    JsonOutput, ReminderAlarmReport, ReminderDateKind, ReminderDateReport, ReminderDeletedReport,
    ReminderDraftReport, ReminderListReport, ReminderListSelection, ReminderMutationDraftReport,
    ReminderNotificationReport, ReminderPriority, ReminderRecurrenceEndReport,
    ReminderRecurrenceReport, ReminderReport, ReminderStructuredLocationReport, StatusReport,
};
use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use chrono::{
    DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc,
};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAlarm, EKAlarmProximity, EKAlarmType, EKAuthorizationStatus, EKCalendar, EKCalendarType,
    EKEntityType, EKEventStore, EKRecurrenceFrequency, EKRecurrenceRule, EKReminder, EKSourceType,
};
use objc2_foundation::{
    NSArray, NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSDateComponentUndefined,
    NSDateComponents, NSError, NSNumber, NSString, NSTimeZone, NSURL,
};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::io::Write as IoWrite;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

pub fn run(command: RemindersCommand) -> Result<JsonOutput> {
    let store = EventKitReminderStore::new();
    run_with_store(&store, command)
}

trait ReminderStore {
    fn authorization_status(&self) -> ReminderAuthorization;
    fn ensure_authorized(&self) -> Result<()>;
    fn lists(&self) -> Result<Vec<ReminderListReport>>;
    fn default_list(&self) -> Result<ReminderListReport>;
    fn fetch(&self, list_ids: &[String]) -> Result<Vec<ReminderReport>>;
    fn get(&self, id: &str) -> Result<ReminderReport>;
    fn create(&self, draft: &ReminderSaveDraft) -> Result<ReminderReport>;
    fn update_add_fields(&self, id: &str, patch: &ReminderAddPatch) -> Result<ReminderReport>;
    fn update(&self, id: &str, patch: &ReminderLifecyclePatch) -> Result<ReminderReport>;
    fn set_completion(
        &self,
        id: &str,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<ReminderReport>;
    fn delete(&self, id: &str) -> Result<()>;
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
            list_selector,
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
            if_exists,
            duplicate_window_seconds,
            dry_run,
        } => add_reminder(
            store,
            AddReminderCommand {
                title,
                list_selector,
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
                if_exists,
                duplicate_window_seconds,
                dry_run,
            },
        ),
        RemindersCommand::Update {
            id,
            title,
            list_selector,
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
            dry_run,
        } => update_reminder(
            store,
            UpdateReminderCommand {
                id,
                title,
                list_selector,
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

struct AddReminderCommand {
    title: String,
    list_selector: WriteReminderListSelectorArgs,
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
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Clone)]
struct ReminderSaveDraft {
    title: String,
    list_id: String,
    due: Option<ParsedReminderDate>,
    start: Option<ParsedReminderDate>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    priority_value: usize,
    notifications: Vec<ParsedReminderNotification>,
}

#[derive(Clone, Default)]
struct ReminderAddPatch {
    start: Option<ParsedReminderDate>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    priority_value: Option<usize>,
    notifications: Option<Vec<ParsedReminderNotification>>,
}

struct UpdateReminderCommand {
    id: String,
    title: Option<String>,
    list_selector: WriteReminderListSelectorArgs,
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
    dry_run: bool,
}

#[derive(Clone, Default)]
struct ReminderLifecyclePatch {
    title: Option<String>,
    list_id: Option<String>,
    due: Option<Option<ParsedReminderDate>>,
    start: Option<Option<ParsedReminderDate>>,
    notes: Option<Option<String>>,
    location: Option<Option<String>>,
    url: Option<Option<String>>,
    priority_value: Option<usize>,
}

#[derive(Clone)]
struct ParsedReminderNotification {
    minutes_before: i64,
    absolute_utc: DateTime<Utc>,
}

#[derive(Clone)]
struct ParsedReminderDate {
    input: String,
    report: ReminderDateReport,
    components: ReminderDateComponents,
}

#[derive(Clone)]
struct ReminderDateComponents {
    year: i32,
    month: u32,
    day: u32,
    hour: Option<u32>,
    minute: Option<u32>,
    second: Option<u32>,
    time_zone: Option<ComponentTimeZone>,
}

#[derive(Clone)]
enum ComponentTimeZone {
    Named(String),
    FixedOffset(i32),
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
    let notifications_supplied = command.notify_at_due || !command.notify_minutes_before.is_empty();
    let notifications = build_notifications(
        due.as_ref(),
        command.notify_at_due,
        &command.notify_minutes_before,
    )?;

    let lists = store.lists()?;
    let default_list = if write_list_selector_is_empty(&command.list_selector) {
        Some(store.default_list()?)
    } else {
        None
    };
    let (list, list_selection) =
        resolve_write_list(&lists, default_list.as_ref(), &command.list_selector)?;
    let priority_arg = command.priority;
    let priority_value = priority_arg.map(priority_arg_value).unwrap_or(0);

    let save = ReminderSaveDraft {
        title: command.title.clone(),
        list_id: list.id.clone(),
        due: due.clone(),
        start: start.clone(),
        notes: notes.clone(),
        location: command.location.clone(),
        url: command.url.clone(),
        priority_value,
        notifications: notifications.clone(),
    };
    let patch = ReminderAddPatch {
        start: start.clone(),
        notes: notes.clone(),
        location: command.location.clone(),
        url: command.url.clone(),
        priority_value: priority_arg.map(priority_arg_value),
        notifications: notifications_supplied.then_some(notifications.clone()),
    };

    let existing = store.fetch(std::slice::from_ref(&list.id))?;
    let matches = find_duplicates(
        &existing,
        &command.title,
        due.as_ref(),
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
        notification_count: notifications.len(),
        notifications: notification_reports(&notifications, due.as_ref()),
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
        let (list, _) = resolve_write_list(&lists, None, &command.list_selector)?;
        patch.list_id = Some(list.id.clone());
        moved_list = Some(list);
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
        (patch.list_id.is_some(), "list"),
        (patch.due.is_some(), "due"),
        (patch.start.is_some(), "start"),
        (patch.notes.is_some(), "notes"),
        (patch.location.is_some(), "location"),
        (patch.url.is_some(), "url"),
        (patch.priority_value.is_some(), "priority"),
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
    result
}

fn clear_parsed_time_zone(value: ParsedReminderDate) -> Result<ParsedReminderDate> {
    if value.report.kind == ReminderDateKind::Date {
        return Ok(value);
    }
    let local = value
        .report
        .local
        .as_deref()
        .context("timed reminder value has no local components")?;
    floating_reminder_date(local)
}

fn floating_reminder_date(input: &str) -> Result<ParsedReminderDate> {
    let local = parse_naive_datetime(input)?;
    Ok(ParsedReminderDate {
        input: input.to_string(),
        report: ReminderDateReport {
            kind: ReminderDateKind::Datetime,
            date: None,
            local: Some(local.format("%Y-%m-%dT%H:%M:%S").to_string()),
            normalized: None,
            utc: None,
            time_zone: None,
        },
        components: ReminderDateComponents {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: Some(local.hour()),
            minute: Some(local.minute()),
            second: Some(local.second()),
            time_zone: None,
        },
    })
}

fn rezone_unchanged_dates(
    before: &ReminderReport,
    patch: &mut ReminderLifecyclePatch,
    time_zone: Option<&str>,
    clear: bool,
) -> Result<()> {
    if patch.due.is_none()
        && let Some(due) = &before.due
        && due.kind == ReminderDateKind::Datetime
    {
        let local = due
            .local
            .as_deref()
            .context("timed due value has no local components")?;
        patch.due = Some(Some(if clear {
            floating_reminder_date(local)?
        } else {
            parse_reminder_date(local, time_zone)?
        }));
    }
    if patch.start.is_none()
        && let Some(start) = &before.start
        && start.kind == ReminderDateKind::Datetime
    {
        let local = start
            .local
            .as_deref()
            .context("timed start value has no local components")?;
        patch.start = Some(Some(if clear {
            floating_reminder_date(local)?
        } else {
            parse_reminder_date(local, time_zone)?
        }));
    }
    Ok(())
}

fn write_list_selector_is_empty(selector: &WriteReminderListSelectorArgs) -> bool {
    selector.list.is_none() && selector.list_id.is_none()
}

fn resolve_write_list(
    lists: &[ReminderListReport],
    default_list: Option<&ReminderListReport>,
    selector: &WriteReminderListSelectorArgs,
) -> Result<(ReminderListReport, ReminderListSelection)> {
    let (list, selection) = if write_list_selector_is_empty(selector) {
        (
            default_list
                .cloned()
                .context("EventKit did not return a default reminder list")?,
            ReminderListSelection::EventkitDefault,
        )
    } else {
        let titles: Vec<String> = selector.list.iter().cloned().collect();
        let ids: Vec<String> = selector.list_id.iter().cloned().collect();
        let resolved = resolve_lists(
            lists,
            &ReadReminderListSelectorArgs {
                lists: titles,
                list_ids: ids,
                list_source: selector.list_source.clone(),
                source_id: selector.source_id.clone(),
            },
        )?;
        let [list] = resolved.as_slice() else {
            bail!("reminder list selector must resolve to exactly one list");
        };
        (list.clone(), ReminderListSelection::Explicit)
    };
    if !list.allows_modifications {
        bail!(
            "reminder list is read-only: {} [{}] source={}",
            list.title,
            list.id,
            list.source.as_deref().unwrap_or("unknown")
        );
    }
    Ok((list, selection))
}

fn parse_reminder_date(input: &str, time_zone: Option<&str>) -> Result<ParsedReminderDate> {
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Ok(ParsedReminderDate {
            input: input.to_string(),
            report: ReminderDateReport {
                kind: ReminderDateKind::Date,
                date: Some(date.format("%Y-%m-%d").to_string()),
                local: None,
                normalized: None,
                utc: None,
                time_zone: None,
            },
            components: ReminderDateComponents {
                year: date.year(),
                month: date.month(),
                day: date.day(),
                hour: None,
                minute: None,
                second: None,
                time_zone: None,
            },
        });
    }

    if let Ok(value) = DateTime::parse_from_rfc3339(input) {
        let local = value.naive_local();
        let offset_seconds = value.offset().local_minus_utc();
        return Ok(timed_reminder_date(
            input,
            local,
            value.to_rfc3339(),
            value.with_timezone(&Utc).to_rfc3339(),
            value.offset().to_string(),
            ComponentTimeZone::FixedOffset(offset_seconds),
        ));
    }

    let local = parse_naive_datetime(input)?;
    let instant = parse_start_datetime_in_time_zone(input, time_zone)?;
    match time_zone {
        Some(time_zone) => {
            let zone = validate_time_zone(time_zone)?;
            Ok(timed_reminder_date(
                input,
                local,
                instant.with_timezone(&zone).to_rfc3339(),
                instant.with_timezone(&Utc).to_rfc3339(),
                time_zone.to_string(),
                ComponentTimeZone::Named(time_zone.to_string()),
            ))
        }
        None => {
            let local_zone = NSTimeZone::localTimeZone().name().to_string();
            Ok(timed_reminder_date(
                input,
                local,
                instant.to_rfc3339(),
                instant.with_timezone(&Utc).to_rfc3339(),
                local_zone.clone(),
                ComponentTimeZone::Named(local_zone),
            ))
        }
    }
}

fn parse_naive_datetime(input: &str) -> Result<NaiveDateTime> {
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(value) = NaiveDateTime::parse_from_str(input, format) {
            return Ok(value);
        }
    }
    bail!("expected YYYY-MM-DD, YYYY-MM-DDTHH:MM, or RFC3339 datetime")
}

fn timed_reminder_date(
    input: &str,
    local: NaiveDateTime,
    normalized: String,
    utc: String,
    time_zone_name: String,
    component_time_zone: ComponentTimeZone,
) -> ParsedReminderDate {
    ParsedReminderDate {
        input: input.to_string(),
        report: ReminderDateReport {
            kind: ReminderDateKind::Datetime,
            date: None,
            local: Some(local.format("%Y-%m-%dT%H:%M:%S").to_string()),
            normalized: Some(normalized),
            utc: Some(utc),
            time_zone: Some(time_zone_name),
        },
        components: ReminderDateComponents {
            year: local.year(),
            month: local.month(),
            day: local.day(),
            hour: Some(local.hour()),
            minute: Some(local.minute()),
            second: Some(local.second()),
            time_zone: Some(component_time_zone),
        },
    }
}

fn build_notifications(
    due: Option<&ParsedReminderDate>,
    notify_at_due: bool,
    notify_minutes_before: &[i64],
) -> Result<Vec<ParsedReminderNotification>> {
    if !notify_at_due && notify_minutes_before.is_empty() {
        return Ok(Vec::new());
    }
    let due = due.context("notification flags require a timed --due value")?;
    if due.report.kind != ReminderDateKind::Datetime {
        bail!("notification flags require a timed --due value, not a date-only due value");
    }
    let due_utc = due
        .report
        .utc
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
        .context("timed due value did not produce a normalized instant")?;

    let mut minutes = BTreeSet::new();
    if notify_at_due {
        minutes.insert(0);
    }
    for value in notify_minutes_before {
        if *value <= 0 {
            bail!("--notify-minutes-before must be greater than zero");
        }
        minutes.insert(*value);
    }
    minutes
        .into_iter()
        .map(|minutes_before| {
            let duration = chrono::Duration::try_minutes(minutes_before)
                .context("--notify-minutes-before is too large")?;
            let absolute_utc = due_utc
                .checked_sub_signed(duration)
                .context("notification time is outside the supported date range")?;
            Ok(ParsedReminderNotification {
                minutes_before,
                absolute_utc,
            })
        })
        .collect()
}

fn notification_reports(
    notifications: &[ParsedReminderNotification],
    due: Option<&ParsedReminderDate>,
) -> Vec<ReminderNotificationReport> {
    notifications
        .iter()
        .map(|notification| ReminderNotificationReport {
            kind: if notification.minutes_before == 0 {
                "at_due"
            } else {
                "before_due"
            }
            .to_string(),
            minutes_before: notification.minutes_before,
            absolute_utc: notification.absolute_utc.to_rfc3339(),
            absolute_in_due_time_zone: render_notification_in_due_time_zone(
                notification.absolute_utc,
                due,
            ),
        })
        .collect()
}

fn render_notification_in_due_time_zone(
    instant: DateTime<Utc>,
    due: Option<&ParsedReminderDate>,
) -> String {
    let Some(due) = due else {
        return instant.to_rfc3339();
    };
    if let Some(zone) = due
        .report
        .time_zone
        .as_deref()
        .and_then(|value| value.parse::<chrono_tz::Tz>().ok())
    {
        return instant.with_timezone(&zone).to_rfc3339();
    }
    if let Some(offset) = due
        .report
        .normalized
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| *value.offset())
    {
        return instant.with_timezone(&offset).to_rfc3339();
    }
    instant.with_timezone(&Local).to_rfc3339()
}

fn find_duplicates<'a>(
    reminders: &'a [ReminderReport],
    title: &str,
    due: Option<&ParsedReminderDate>,
    window_seconds: i64,
) -> Vec<&'a ReminderReport> {
    reminders
        .iter()
        .filter(|reminder| {
            reminder.title == title && due_matches(reminder.due.as_ref(), due, window_seconds)
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

fn filter_list_discovery(
    mut lists: Vec<ReminderListReport>,
    source: Option<&str>,
    writable_only: bool,
) -> Vec<ReminderListReport> {
    lists.retain(|list| source.is_none_or(|value| list.source.as_deref() == Some(value)));
    lists.retain(|list| !writable_only || list.allows_modifications);
    lists
}

fn list_reminders(
    store: &impl ReminderStore,
    filters: ReminderReadFilterArgs,
    query: Option<&str>,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let lists = store.lists()?;
    let selected = resolve_lists(&lists, &filters.list_selector)?;
    let list_ids: Vec<String> = selected.iter().map(|list| list.id.clone()).collect();
    let mut reminders = store.fetch(&list_ids)?;
    apply_filters(&mut reminders, &filters, query)?;
    sort_reminders(&mut reminders);
    Ok(JsonOutput::Reminders { reminders })
}

fn apply_filters(
    reminders: &mut Vec<ReminderReport>,
    filters: &ReminderReadFilterArgs,
    query: Option<&str>,
) -> Result<()> {
    let due_from = filters
        .due_from
        .as_deref()
        .map(parse_due_lower_bound)
        .transpose()?;
    let due_to = filters
        .due_to
        .as_deref()
        .map(parse_due_upper_bound)
        .transpose()?;
    if let (Some(from), Some(to)) = (&due_from, &due_to)
        && from.instant > to.instant
    {
        bail!("--due-from must not be after --due-to");
    }

    let query = query.map(str::to_lowercase);
    reminders.retain(|reminder| {
        let state_matches = match filters.state {
            ReminderStateArg::Incomplete => !reminder.completed,
            ReminderStateArg::Completed => reminder.completed,
            ReminderStateArg::All => true,
        };
        let due_matches = if due_from.is_some() || due_to.is_some() {
            reminder
                .due
                .as_ref()
                .and_then(reminder_date_instant)
                .is_some_and(|due| {
                    due_from.as_ref().is_none_or(|from| due >= from.instant)
                        && due_to.as_ref().is_none_or(|to| {
                            if to.exclusive {
                                due < to.instant
                            } else {
                                due <= to.instant
                            }
                        })
                })
        } else {
            true
        };
        let query_matches = query
            .as_deref()
            .is_none_or(|query| reminder_matches(reminder, query));
        state_matches && due_matches && query_matches
    });
    Ok(())
}

#[derive(Clone, Copy)]
struct DueBound {
    instant: DateTime<Utc>,
    exclusive: bool,
}

fn parse_due_lower_bound(value: &str) -> Result<DueBound> {
    Ok(DueBound {
        instant: parse_start_datetime(value)?.with_timezone(&Utc),
        exclusive: false,
    })
}

fn parse_due_upper_bound(value: &str) -> Result<DueBound> {
    let date_only = NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok();
    let instant = if date_only {
        parse_end_datetime(value)?
    } else {
        parse_start_datetime(value)?
    };
    Ok(DueBound {
        instant: instant.with_timezone(&Utc),
        exclusive: date_only,
    })
}

fn reminder_date_instant(value: &ReminderDateReport) -> Option<DateTime<Utc>> {
    if let Some(normalized) = &value.normalized {
        return DateTime::parse_from_rfc3339(normalized)
            .ok()
            .map(|date| date.with_timezone(&Utc));
    }
    value.date.as_deref().and_then(|date| {
        parse_start_datetime(date)
            .ok()
            .map(|date| date.with_timezone(&Utc))
    })
}

fn reminder_matches(reminder: &ReminderReport, query: &str) -> bool {
    [
        Some(reminder.title.as_str()),
        reminder.notes.as_deref(),
        reminder.location.as_deref(),
        reminder.url.as_deref(),
        reminder.list.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(query))
}

fn sort_lists(lists: &mut [ReminderListReport]) {
    lists.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn sort_reminders(reminders: &mut [ReminderReport]) {
    reminders.sort_by(|left, right| {
        match (
            left.due.as_ref().and_then(reminder_date_instant),
            right.due.as_ref().and_then(reminder_date_instant),
        ) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.id.cmp(&right.id))
    });
}

fn mark_default_list(lists: &mut [ReminderListReport], default_id: Option<&str>) {
    for list in lists {
        list.is_default_for_new_reminders = default_id == Some(list.id.as_str());
    }
}

fn resolve_lists(
    lists: &[ReminderListReport],
    selector: &ReadReminderListSelectorArgs,
) -> Result<Vec<ReminderListReport>> {
    if selector.lists.is_empty() && selector.list_ids.is_empty() {
        return Ok(lists.to_vec());
    }

    let mut resolved = Vec::new();
    for id in &selector.list_ids {
        let list = lists
            .iter()
            .find(|list| list.id == *id)
            .ok_or_else(|| anyhow!("reminder list id not found: {id}"))?;
        push_unique_list(&mut resolved, list);
    }
    for title in &selector.lists {
        let candidates: Vec<&ReminderListReport> = lists
            .iter()
            .filter(|list| list.title == *title)
            .filter(|list| {
                selector
                    .list_source
                    .as_ref()
                    .is_none_or(|source| list.source.as_ref() == Some(source))
            })
            .filter(|list| {
                selector
                    .source_id
                    .as_ref()
                    .is_none_or(|source_id| list.source_id.as_ref() == Some(source_id))
            })
            .collect();
        match candidates.as_slice() {
            [] => bail!(missing_list_message(title, selector)),
            [list] => push_unique_list(&mut resolved, list),
            _ => bail!(ambiguous_list_message(title, &candidates)),
        }
    }
    Ok(resolved)
}

fn push_unique_list(resolved: &mut Vec<ReminderListReport>, list: &ReminderListReport) {
    if !resolved.iter().any(|item| item.id == list.id) {
        resolved.push(list.clone());
    }
}

fn missing_list_message(title: &str, selector: &ReadReminderListSelectorArgs) -> String {
    let mut message = format!("reminder list not found: {title:?}");
    if let Some(source) = &selector.list_source {
        let _ = write!(message, " in source {source:?}");
    }
    if let Some(source_id) = &selector.source_id {
        let _ = write!(message, " with source id {source_id:?}");
    }
    message
}

fn ambiguous_list_message(title: &str, candidates: &[&ReminderListReport]) -> String {
    let mut message = format!(
        "reminder list title {title:?} is ambiguous; use --list-id, --list-source, or --source-id. Matches:"
    );
    for list in candidates {
        let _ = write!(
            message,
            "\n- title={:?} source={:?} source_id={:?} list_id={:?} writable={}",
            list.title,
            list.source.as_deref().unwrap_or("unknown"),
            list.source_id.as_deref().unwrap_or("unknown"),
            list.id,
            list.allows_modifications
        );
    }
    message
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReminderAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    FullAccess,
    WriteOnly,
    Unknown,
}

impl ReminderAuthorization {
    pub(crate) fn current() -> Self {
        let status =
            unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder) };
        Self::from_ek(status)
    }

    fn from_ek(status: EKAuthorizationStatus) -> Self {
        if status == EKAuthorizationStatus::NotDetermined {
            Self::NotDetermined
        } else if status == EKAuthorizationStatus::Restricted {
            Self::Restricted
        } else if status == EKAuthorizationStatus::Denied {
            Self::Denied
        } else if status == EKAuthorizationStatus::FullAccess {
            Self::FullAccess
        } else if status == EKAuthorizationStatus::WriteOnly {
            Self::WriteOnly
        } else {
            Self::Unknown
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotDetermined => "NotDetermined",
            Self::Restricted => "Restricted",
            Self::Denied => "Denied",
            Self::FullAccess => "FullAccess",
            Self::WriteOnly => "WriteOnly",
            Self::Unknown => "Unknown",
        }
    }
}

struct EventKitReminderStore {
    store: Retained<EKEventStore>,
}

impl EventKitReminderStore {
    fn new() -> Self {
        Self {
            store: unsafe { EKEventStore::new() },
        }
    }

    fn request_access(&self) -> Result<bool> {
        let result = Arc::new((Mutex::new(None::<(bool, Option<String>)>), Condvar::new()));
        let callback_result = Arc::clone(&result);
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let error = if error.is_null() {
                None
            } else {
                Some(format!("{:?}", unsafe { &*error }))
            };
            let (lock, cvar) = &*callback_result;
            let mut state = lock.lock().expect("reminder authorization lock poisoned");
            *state = Some((granted.as_bool(), error));
            cvar.notify_one();
        });
        unsafe {
            let block_ptr = &*completion as *const _ as *mut _;
            self.store
                .requestFullAccessToRemindersWithCompletion(block_ptr);
        }
        let (lock, cvar) = &*result;
        let mut state = lock
            .lock()
            .map_err(|_| anyhow!("reminder authorization lock poisoned"))?;
        while state.is_none() {
            state = cvar
                .wait(state)
                .map_err(|_| anyhow!("reminder authorization lock poisoned"))?;
        }
        match state.take() {
            Some((granted, None)) => Ok(granted),
            Some((_, Some(error))) => bail!("failed to request full Reminders access: {error}"),
            None => bail!("failed to request full Reminders access"),
        }
    }

    fn ek_lists(&self) -> Vec<Retained<EKCalendar>> {
        unsafe { self.store.calendarsForEntityType(EKEntityType::Reminder) }
            .iter()
            .map(|list| list.retain())
            .collect()
    }

    fn fetch_reports(&self, list_ids: &[String]) -> Result<Vec<ReminderReport>> {
        let selected: Vec<Retained<EKCalendar>> = if list_ids.is_empty() {
            self.ek_lists()
        } else {
            let all = self.ek_lists();
            let mut selected = Vec::new();
            for id in list_ids {
                let list = all
                    .iter()
                    .find(|list| unsafe { list.calendarIdentifier() }.to_string() == *id)
                    .with_context(|| format!("reminder list is no longer available: {id}"))?;
                selected.push(list.clone());
            }
            selected
        };
        let array = NSArray::from_retained_slice(&selected);
        let predicate = unsafe { self.store.predicateForRemindersInCalendars(Some(&array)) };
        let result = Arc::new((Mutex::new(None::<Vec<ReminderReport>>), Condvar::new()));
        let callback_result = Arc::clone(&result);
        let completion = RcBlock::new(move |reminders: *mut NSArray<EKReminder>| {
            let reports = if reminders.is_null() {
                Vec::new()
            } else {
                let reminders = unsafe { Retained::retain(reminders) }
                    .expect("EventKit returned a dangling reminders array");
                reminders
                    .iter()
                    .map(|reminder| reminder_to_report(&reminder, false))
                    .collect()
            };
            let (lock, cvar) = &*callback_result;
            let mut state = lock.lock().expect("reminder fetch lock poisoned");
            *state = Some(reports);
            cvar.notify_one();
        });
        unsafe {
            self.store
                .fetchRemindersMatchingPredicate_completion(&predicate, &completion);
        }
        let (lock, cvar) = &*result;
        let mut state = lock
            .lock()
            .map_err(|_| anyhow!("reminder fetch lock poisoned"))?;
        while state.is_none() {
            state = cvar
                .wait(state)
                .map_err(|_| anyhow!("reminder fetch lock poisoned"))?;
        }
        state
            .take()
            .ok_or_else(|| anyhow!("EventKit reminder fetch returned no result"))
    }
}

impl ReminderStore for EventKitReminderStore {
    fn authorization_status(&self) -> ReminderAuthorization {
        ReminderAuthorization::current()
    }

    fn ensure_authorized(&self) -> Result<()> {
        match self.authorization_status() {
            ReminderAuthorization::FullAccess => Ok(()),
            ReminderAuthorization::NotDetermined => {
                if self.request_access()? {
                    Ok(())
                } else {
                    bail!("full Reminders access was denied")
                }
            }
            ReminderAuthorization::Denied => bail!(
                "Reminders access is denied; enable it in System Settings > Privacy & Security > Reminders"
            ),
            ReminderAuthorization::Restricted => {
                bail!("Reminders access is restricted by system policy")
            }
            ReminderAuthorization::WriteOnly => {
                bail!("full Reminders access is required for reminder reads")
            }
            ReminderAuthorization::Unknown => bail!("unknown Reminders authorization status"),
        }
    }

    fn lists(&self) -> Result<Vec<ReminderListReport>> {
        Ok(self
            .ek_lists()
            .iter()
            .map(|list| reminder_list_report(list, false))
            .collect())
    }

    fn default_list(&self) -> Result<ReminderListReport> {
        let list = unsafe { self.store.defaultCalendarForNewReminders() }
            .context("EventKit did not return a default reminder list")?;
        Ok(reminder_list_report(&list, true))
    }

    fn fetch(&self, list_ids: &[String]) -> Result<Vec<ReminderReport>> {
        self.fetch_reports(list_ids)
            .context("failed to fetch reminders through EventKit")
    }

    fn get(&self, id: &str) -> Result<ReminderReport> {
        let reminder = self.find_reminder(id)?;
        Ok(reminder_to_report(&reminder, true))
    }

    fn create(&self, draft: &ReminderSaveDraft) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let list_id = NSString::from_str(&draft.list_id);
        let list = unsafe { self.store.calendarWithIdentifier(&list_id) }
            .context("selected reminder list is no longer available")?;
        if !unsafe { list.allowsContentModifications() } {
            bail!("selected reminder list is read-only");
        }
        let reminder = unsafe { EKReminder::reminderWithEventStore(&self.store) };
        let title = NSString::from_str(&draft.title);
        unsafe {
            reminder.setTitle(Some(&title));
            reminder.setCalendar(Some(&list));
            reminder.setPriority(draft.priority_value);
        }
        if let Some(due) = &draft.due {
            let components = reminder_date_components(&due.components)?;
            unsafe { reminder.setDueDateComponents(Some(&components)) };
        }
        if let Some(start) = &draft.start {
            let components = reminder_date_components(&start.components)?;
            unsafe { reminder.setStartDateComponents(Some(&components)) };
        }
        if let Some(notes) = &draft.notes {
            let notes = NSString::from_str(notes);
            unsafe { reminder.setNotes(Some(&notes)) };
        }
        if let Some(location) = &draft.location {
            let location = NSString::from_str(location);
            unsafe { reminder.setLocation(Some(&location)) };
        }
        if let Some(url) = &draft.url {
            set_reminder_url(&reminder, url)?;
        }
        add_reminder_notifications(&reminder, &draft.notifications);
        self.save_reminder(&reminder)?;
        Ok(reminder_to_report(&reminder, true))
    }

    fn update_add_fields(&self, id: &str, patch: &ReminderAddPatch) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if let Some(start) = &patch.start {
            let components = reminder_date_components(&start.components)?;
            unsafe { reminder.setStartDateComponents(Some(&components)) };
        }
        if let Some(notes) = &patch.notes {
            let notes = NSString::from_str(notes);
            unsafe { reminder.setNotes(Some(&notes)) };
        }
        if let Some(location) = &patch.location {
            let location = NSString::from_str(location);
            unsafe { reminder.setLocation(Some(&location)) };
        }
        if let Some(url) = &patch.url {
            set_reminder_url(&reminder, url)?;
        }
        if let Some(priority) = patch.priority_value {
            unsafe { reminder.setPriority(priority) };
        }
        if let Some(notifications) = &patch.notifications {
            unsafe { reminder.setAlarms(None) };
            add_reminder_notifications(&reminder, notifications);
        }
        self.save_reminder(&reminder)?;
        Ok(reminder_to_report(&reminder, true))
    }

    fn update(&self, id: &str, patch: &ReminderLifecyclePatch) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if !unsafe { reminder.calendar() }
            .as_ref()
            .is_some_and(|list| unsafe { list.allowsContentModifications() })
        {
            bail!("the reminder's current list is read-only");
        }
        if let Some(title) = &patch.title {
            let title = NSString::from_str(title);
            unsafe { reminder.setTitle(Some(&title)) };
        }
        if let Some(list_id) = &patch.list_id {
            let list_id = NSString::from_str(list_id);
            let list = unsafe { self.store.calendarWithIdentifier(&list_id) }
                .context("selected reminder list is no longer available")?;
            if !unsafe { list.allowsContentModifications() } {
                bail!("selected reminder list is read-only");
            }
            unsafe { reminder.setCalendar(Some(&list)) };
        }
        if let Some(due) = &patch.due {
            let components = due
                .as_ref()
                .map(|value| reminder_date_components(&value.components))
                .transpose()?;
            unsafe { reminder.setDueDateComponents(components.as_deref()) };
        }
        if let Some(start) = &patch.start {
            let components = start
                .as_ref()
                .map(|value| reminder_date_components(&value.components))
                .transpose()?;
            unsafe { reminder.setStartDateComponents(components.as_deref()) };
        }
        if let Some(notes) = &patch.notes {
            let notes = notes.as_ref().map(|value| NSString::from_str(value));
            unsafe { reminder.setNotes(notes.as_deref()) };
        }
        if let Some(location) = &patch.location {
            let location = location.as_ref().map(|value| NSString::from_str(value));
            unsafe { reminder.setLocation(location.as_deref()) };
        }
        if let Some(url) = &patch.url {
            match url {
                Some(url) => set_reminder_url(&reminder, url)?,
                None => unsafe { reminder.setURL(None) },
            }
        }
        if let Some(priority) = patch.priority_value {
            unsafe { reminder.setPriority(priority) };
        }
        self.save_reminder(&reminder)?;
        Ok(reminder_to_report(&reminder, true))
    }

    fn set_completion(
        &self,
        id: &str,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if !unsafe { reminder.calendar() }
            .as_ref()
            .is_some_and(|list| unsafe { list.allowsContentModifications() })
        {
            bail!("the reminder's current list is read-only");
        }
        match completed_at {
            Some(completed_at) => {
                let date = NSDate::dateWithTimeIntervalSince1970(
                    completed_at.timestamp_millis() as f64 / 1_000.0,
                );
                unsafe { reminder.setCompletionDate(Some(&date)) };
            }
            None => unsafe { reminder.setCompletionDate(None) },
        }
        self.save_reminder(&reminder)?;
        Ok(reminder_to_report(&reminder, true))
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if !unsafe { reminder.calendar() }
            .as_ref()
            .is_some_and(|list| unsafe { list.allowsContentModifications() })
        {
            bail!("the reminder's current list is read-only");
        }
        unsafe {
            self.store
                .removeReminder_commit_error(&reminder, true)
                .map_err(|error| anyhow!("failed to delete reminder: {error:?}"))?;
            self.store.refreshSourcesIfNecessary();
        }
        Ok(())
    }
}

impl EventKitReminderStore {
    fn find_reminder(&self, id: &str) -> Result<Retained<EKReminder>> {
        unsafe { self.store.refreshSourcesIfNecessary() };
        let id = NSString::from_str(id);
        let item = unsafe { self.store.calendarItemWithIdentifier(&id) }
            .context("reminder is no longer available")?;
        item.downcast_ref::<EKReminder>()
            .map(Message::retain)
            .context("the selected EventKit item is an event, not a reminder")
    }

    fn save_reminder(&self, reminder: &EKReminder) -> Result<()> {
        unsafe {
            self.store
                .saveReminder_commit_error(reminder, true)
                .map_err(|error| anyhow!("failed to save reminder: {error:?}"))?;
            self.store.refreshSourcesIfNecessary();
        }
        Ok(())
    }
}

fn reminder_date_components(value: &ReminderDateComponents) -> Result<Retained<NSDateComponents>> {
    let components = NSDateComponents::new();
    let calendar = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
        .context("Gregorian calendar is unavailable")?;
    components.setCalendar(Some(&calendar));
    components.setYear(value.year as isize);
    components.setMonth(value.month as isize);
    components.setDay(value.day as isize);
    if let Some(hour) = value.hour {
        components.setHour(hour as isize);
    }
    if let Some(minute) = value.minute {
        components.setMinute(minute as isize);
    }
    if let Some(second) = value.second {
        components.setSecond(second as isize);
    }
    if let Some(time_zone) = &value.time_zone {
        let time_zone = match time_zone {
            ComponentTimeZone::Named(name) => {
                NSTimeZone::timeZoneWithName(&NSString::from_str(name))
                    .with_context(|| format!("unknown EventKit time zone: {name}"))?
            }
            ComponentTimeZone::FixedOffset(seconds) => {
                NSTimeZone::timeZoneForSecondsFromGMT(*seconds as isize)
            }
        };
        components.setTimeZone(Some(&time_zone));
    }
    Ok(components)
}

fn set_reminder_url(reminder: &EKReminder, value: &str) -> Result<()> {
    let value = NSString::from_str(value);
    let url = NSURL::URLWithString_encodingInvalidCharacters(&value, false)
        .ok_or_else(|| anyhow!("invalid URL: {value}"))?;
    unsafe { reminder.setURL(Some(&url)) };
    Ok(())
}

fn add_reminder_notifications(reminder: &EKReminder, notifications: &[ParsedReminderNotification]) {
    for notification in notifications {
        let date = NSDate::dateWithTimeIntervalSince1970(
            notification.absolute_utc.timestamp_millis() as f64 / 1_000.0,
        );
        let alarm = unsafe { EKAlarm::alarmWithAbsoluteDate(&date) };
        unsafe { reminder.addAlarm(&alarm) };
    }
}

fn reminder_list_report(list: &EKCalendar, is_default: bool) -> ReminderListReport {
    let source = unsafe { list.source() };
    ReminderListReport {
        id: unsafe { list.calendarIdentifier() }.to_string(),
        title: unsafe { list.title() }.to_string(),
        source: source
            .as_ref()
            .map(|source| unsafe { source.title() }.to_string()),
        source_id: source
            .as_ref()
            .map(|source| unsafe { source.sourceIdentifier() }.to_string()),
        source_type: source
            .as_ref()
            .map(|source| source_type_name(unsafe { source.sourceType() }).to_string()),
        list_type: calendar_type_name(unsafe { list.r#type() }).to_string(),
        allows_modifications: unsafe { list.allowsContentModifications() },
        is_immutable: unsafe { list.isImmutable() },
        is_subscribed: unsafe { list.isSubscribed() },
        is_default_for_new_reminders: is_default,
    }
}

fn reminder_to_report(reminder: &EKReminder, details: bool) -> ReminderReport {
    let list = unsafe { reminder.calendar() };
    let list_report = list.as_ref().map(|list| reminder_list_report(list, false));
    let notes = unsafe { reminder.notes() }.map(|value| value.to_string());
    let url = unsafe { reminder.URL() }
        .as_ref()
        .and_then(|url| url.absoluteString())
        .map(|value| value.to_string());
    let priority_value = unsafe { reminder.priority() };
    let alarms = details.then(|| reminder_alarms(reminder));
    let recurrence_rules = details.then(|| reminder_recurrence_rules(reminder));
    ReminderReport {
        id: unsafe { reminder.calendarItemIdentifier() }.to_string(),
        title: unsafe { reminder.title() }.to_string(),
        completed: unsafe { reminder.isCompleted() },
        completion_date: unsafe { reminder.completionDate() }
            .as_deref()
            .map(nsdate_rfc3339),
        priority: priority_name(priority_value),
        priority_value,
        list: list_report.as_ref().map(|list| list.title.clone()),
        list_id: list_report.as_ref().map(|list| list.id.clone()),
        list_source: list_report.as_ref().and_then(|list| list.source.clone()),
        list_source_id: list_report.as_ref().and_then(|list| list.source_id.clone()),
        list_type: list_report.as_ref().map(|list| list.list_type.clone()),
        allows_list_modifications: list_report.as_ref().map(|list| list.allows_modifications),
        list_selection: None,
        write_action: None,
        due: unsafe { reminder.dueDateComponents() }
            .as_deref()
            .and_then(date_components_report),
        due_input: None,
        start: unsafe { reminder.startDateComponents() }
            .as_deref()
            .and_then(date_components_report),
        start_input: None,
        notes: notes.clone(),
        location: unsafe { reminder.location() }.map(|value| value.to_string()),
        url: url.clone(),
        has_notes: unsafe { reminder.hasNotes() },
        has_url: url.is_some(),
        alarm_count: alarms.as_ref().map(Vec::len),
        recurrence_count: recurrence_rules.as_ref().map(Vec::len),
        alarms,
        recurrence_rules,
        creation_date: unsafe { reminder.creationDate() }
            .as_deref()
            .map(nsdate_rfc3339),
        last_modified_date: unsafe { reminder.lastModifiedDate() }
            .as_deref()
            .map(nsdate_rfc3339),
        external_identifier: unsafe { reminder.calendarItemExternalIdentifier() }
            .map(|value| value.to_string()),
        item_time_zone: unsafe { reminder.timeZone() }.map(|zone| zone.name().to_string()),
    }
}

fn priority_name(value: usize) -> ReminderPriority {
    match value {
        0 => ReminderPriority::None,
        1..=4 => ReminderPriority::High,
        5 => ReminderPriority::Medium,
        _ => ReminderPriority::Low,
    }
}

fn date_components_report(components: &NSDateComponents) -> Option<ReminderDateReport> {
    let year = component(components.year())?;
    let month = component(components.month())?;
    let day = component(components.day())?;
    let component_time_zone = components.timeZone();
    let time_zone = component_time_zone
        .as_ref()
        .map(|zone| zone.name().to_string());
    let hour = component(components.hour());
    let minute = component(components.minute());
    let second = component(components.second());
    let date = format!("{year:04}-{month:02}-{day:02}");
    if hour.is_none() && minute.is_none() && second.is_none() {
        return Some(ReminderDateReport {
            kind: ReminderDateKind::Date,
            date: Some(date),
            local: None,
            normalized: None,
            utc: None,
            time_zone,
        });
    }

    let local = format!(
        "{date}T{:02}:{:02}:{:02}",
        hour.unwrap_or(0),
        minute.unwrap_or(0),
        second.unwrap_or(0)
    );
    let instant = components.date();
    let utc = instant.as_deref().and_then(nsdate_utc);
    let normalized = utc.as_ref().map(|utc| {
        component_time_zone
            .as_ref()
            .zip(instant.as_ref())
            .and_then(|(zone, date)| FixedOffset::east_opt(zone.secondsFromGMTForDate(date) as i32))
            .map(|offset| utc.with_timezone(&offset).to_rfc3339())
            .unwrap_or_else(|| utc.with_timezone(&Local).to_rfc3339())
    });
    Some(ReminderDateReport {
        kind: ReminderDateKind::Datetime,
        date: None,
        local: Some(local),
        normalized,
        utc: utc.map(|value| value.to_rfc3339()),
        time_zone,
    })
}

fn component(value: isize) -> Option<isize> {
    (value != NSDateComponentUndefined).then_some(value)
}

fn nsdate_utc(date: &NSDate) -> Option<DateTime<Utc>> {
    let timestamp = date.timeIntervalSince1970();
    let mut seconds = timestamp.floor() as i64;
    let mut nanos = ((timestamp - timestamp.floor()) * 1_000_000_000.0).round() as u32;
    if nanos == 1_000_000_000 {
        seconds += 1;
        nanos = 0;
    }
    Utc.timestamp_opt(seconds, nanos).single()
}

fn nsdate_rfc3339(date: &NSDate) -> String {
    nsdate_utc(date)
        .map(|value| value.with_timezone(&Local).to_rfc3339())
        .unwrap_or_else(|| "invalid-date".to_string())
}

fn reminder_alarms(reminder: &EKReminder) -> Vec<ReminderAlarmReport> {
    unsafe { reminder.alarms() }
        .map(|alarms| alarms.iter().map(|alarm| alarm_report(&alarm)).collect())
        .unwrap_or_default()
}

fn alarm_report(alarm: &EKAlarm) -> ReminderAlarmReport {
    let absolute = unsafe { alarm.absoluteDate() };
    let structured_location =
        unsafe { alarm.structuredLocation() }.map(|location| ReminderStructuredLocationReport {
            title: unsafe { location.title() }.map(|title| title.to_string()),
            radius_meters: unsafe { location.radius() },
        });
    ReminderAlarmReport {
        relative_offset_seconds: absolute
            .is_none()
            .then(|| unsafe { alarm.relativeOffset() }),
        absolute_date: absolute.as_deref().map(nsdate_rfc3339),
        proximity: alarm_proximity_name(unsafe { alarm.proximity() }).to_string(),
        alarm_type: alarm_type_name(unsafe { alarm.r#type() }).to_string(),
        structured_location,
    }
}

fn reminder_recurrence_rules(reminder: &EKReminder) -> Vec<ReminderRecurrenceReport> {
    unsafe { reminder.recurrenceRules() }
        .map(|rules| rules.iter().map(|rule| recurrence_report(&rule)).collect())
        .unwrap_or_default()
}

fn recurrence_report(rule: &EKRecurrenceRule) -> ReminderRecurrenceReport {
    let end = unsafe { rule.recurrenceEnd() };
    let end = match end {
        Some(end) if unsafe { end.occurrenceCount() } > 0 => ReminderRecurrenceEndReport {
            kind: "count".to_string(),
            occurrence_count: Some(unsafe { end.occurrenceCount() }),
            end_date: None,
        },
        Some(end) => ReminderRecurrenceEndReport {
            kind: "date".to_string(),
            occurrence_count: None,
            end_date: unsafe { end.endDate() }.as_deref().map(nsdate_rfc3339),
        },
        None => ReminderRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        },
    };
    ReminderRecurrenceReport {
        frequency: recurrence_frequency_name(unsafe { rule.frequency() }).to_string(),
        interval: unsafe { rule.interval() } as usize,
        first_day_of_week: unsafe { rule.firstDayOfTheWeek() },
        end,
        days_of_week: unsafe { rule.daysOfTheWeek() }.map(|values| {
            values
                .iter()
                .map(|value| unsafe { value.dayOfTheWeek() }.0)
                .collect()
        }),
        days_of_month: number_values(unsafe { rule.daysOfTheMonth() }),
        months_of_year: number_values(unsafe { rule.monthsOfTheYear() }),
        weeks_of_year: number_values(unsafe { rule.weeksOfTheYear() }),
        days_of_year: number_values(unsafe { rule.daysOfTheYear() }),
        set_positions: number_values(unsafe { rule.setPositions() }),
    }
}

fn number_values(values: Option<Retained<NSArray<NSNumber>>>) -> Option<Vec<i32>> {
    values.map(|values| values.iter().map(|value| value.intValue()).collect())
}

fn calendar_type_name(value: EKCalendarType) -> &'static str {
    match value.0 {
        0 => "local",
        1 => "caldav",
        2 => "exchange",
        3 => "subscription",
        4 => "birthday",
        _ => "unknown",
    }
}

fn source_type_name(value: EKSourceType) -> &'static str {
    match value.0 {
        0 => "local",
        1 => "exchange",
        2 => "caldav",
        3 => "mobileme",
        4 => "subscribed",
        5 => "birthdays",
        _ => "unknown",
    }
}

fn alarm_proximity_name(value: EKAlarmProximity) -> &'static str {
    if value == EKAlarmProximity::Enter {
        "arrive"
    } else if value == EKAlarmProximity::Leave {
        "leave"
    } else {
        "none"
    }
}

fn alarm_type_name(value: EKAlarmType) -> &'static str {
    if value == EKAlarmType::Display {
        "display"
    } else if value == EKAlarmType::Audio {
        "audio"
    } else if value == EKAlarmType::Procedure {
        "procedure"
    } else if value == EKAlarmType::Email {
        "email"
    } else {
        "unknown"
    }
}

fn recurrence_frequency_name(value: EKRecurrenceFrequency) -> &'static str {
    if value == EKRecurrenceFrequency::Daily {
        "daily"
    } else if value == EKRecurrenceFrequency::Weekly {
        "weekly"
    } else if value == EKRecurrenceFrequency::Monthly {
        "monthly"
    } else if value == EKRecurrenceFrequency::Yearly {
        "yearly"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::ReadReminderListSelectorArgs;
    use objc2_foundation::{NSCalendar, NSTimeZone};
    use serde_json::json;
    use std::cell::{Cell, RefCell};

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
            report.due = draft.due.as_ref().map(|value| value.report.clone());
            report.start = draft.start.as_ref().map(|value| value.report.clone());
            report.notes = draft.notes.clone();
            report.location = draft.location.clone();
            report.url = draft.url.clone();
            report.priority_value = draft.priority_value;
            report.priority = priority_name(draft.priority_value);
            report.has_notes = report.notes.is_some();
            report.has_url = report.url.is_some();
            report.alarm_count = Some(draft.notifications.len());
            Ok(report)
        }

        fn update_add_fields(&self, id: &str, patch: &ReminderAddPatch) -> Result<ReminderReport> {
            self.updates.set(self.updates.get() + 1);
            self.last_patch.replace(Some(patch.clone()));
            let mut report = self.get(id)?;
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
                report.alarm_count = Some(notifications.len());
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

    fn add_command(title: &str) -> AddReminderCommand {
        AddReminderCommand {
            title: title.to_string(),
            list_selector: write_selector(Some("A")),
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
        assert_eq!(notifications[0].minutes_before, 15);
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
}
