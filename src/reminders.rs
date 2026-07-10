use crate::cache::resolve_reminder_ref;
use crate::cli::{
    ReadReminderListSelectorArgs, ReminderReadFilterArgs, ReminderStateArg, RemindersCommand,
};
use crate::dates::{parse_end_datetime, parse_start_datetime};
use crate::models::{
    JsonOutput, ReminderAlarmReport, ReminderDateKind, ReminderDateReport, ReminderListReport,
    ReminderPriority, ReminderRecurrenceEndReport, ReminderRecurrenceReport, ReminderReport,
    ReminderStructuredLocationReport, StatusReport,
};
use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAlarm, EKAlarmProximity, EKAlarmType, EKAuthorizationStatus, EKCalendar, EKCalendarType,
    EKEntityType, EKEventStore, EKRecurrenceFrequency, EKRecurrenceRule, EKReminder, EKSourceType,
};
use objc2_foundation::{
    NSArray, NSDate, NSDateComponentUndefined, NSDateComponents, NSError, NSNumber, NSString,
};
use std::cmp::Ordering;
use std::fmt::Write;
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
    }
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
        unsafe { self.store.refreshSourcesIfNecessary() };
        let id = NSString::from_str(id);
        let item = unsafe { self.store.calendarItemWithIdentifier(&id) }
            .context("reminder is no longer available")?;
        let reminder = item
            .downcast_ref::<EKReminder>()
            .context("the selected EventKit item is an event, not a reminder")?;
        Ok(reminder_to_report(reminder, true))
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
        due: unsafe { reminder.dueDateComponents() }
            .as_deref()
            .and_then(date_components_report),
        start: unsafe { reminder.startDateComponents() }
            .as_deref()
            .and_then(date_components_report),
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
    let time_zone = components.timeZone().map(|zone| zone.name().to_string());
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
        time_zone
            .as_deref()
            .and_then(|zone| zone.parse::<chrono_tz::Tz>().ok())
            .map(|zone| utc.with_timezone(&zone).to_rfc3339())
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
            due: due.map(|date| ReminderDateReport {
                kind: ReminderDateKind::Date,
                date: Some(date.to_string()),
                local: None,
                normalized: None,
                utc: None,
                time_zone: None,
            }),
            start: None,
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
}
