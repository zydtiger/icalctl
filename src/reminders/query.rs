use super::ReminderStore;
use super::selection::resolve_lists;
use crate::cli::{ReminderReadFilterArgs, ReminderStateArg};
use crate::dates::{parse_end_datetime, parse_start_datetime};
use crate::models::{JsonOutput, ReminderDateReport, ReminderListReport, ReminderReport};
use anyhow::{Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use std::cmp::Ordering;

pub(super) fn filter_list_discovery(
    mut lists: Vec<ReminderListReport>,
    source: Option<&str>,
    writable_only: bool,
) -> Vec<ReminderListReport> {
    lists.retain(|list| source.is_none_or(|value| list.source.as_deref() == Some(value)));
    lists.retain(|list| !writable_only || list.allows_modifications);
    lists
}

pub(super) fn list_reminders(
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

pub(super) fn apply_filters(
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
pub(in crate::reminders) struct DueBound {
    pub(in crate::reminders) instant: DateTime<Utc>,
    pub(in crate::reminders) exclusive: bool,
}

fn parse_due_lower_bound(value: &str) -> Result<DueBound> {
    Ok(DueBound {
        instant: parse_start_datetime(value)?.with_timezone(&Utc),
        exclusive: false,
    })
}

pub(in crate::reminders) fn parse_due_upper_bound(value: &str) -> Result<DueBound> {
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

pub(super) fn sort_lists(lists: &mut [ReminderListReport]) {
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

pub(super) fn mark_default_list(lists: &mut [ReminderListReport], default_id: Option<&str>) {
    for list in lists {
        list.is_default_for_new_reminders = default_id == Some(list.id.as_str());
    }
}
