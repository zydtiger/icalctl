use crate::cli::{ReadReminderListSelectorArgs, WriteReminderListSelectorArgs};
use crate::models::{ReminderListReport, ReminderListSelection};
use anyhow::{Context, Result, anyhow, bail};
use std::fmt::Write;

pub(super) fn write_list_selector_is_empty(selector: &WriteReminderListSelectorArgs) -> bool {
    selector.list.is_none() && selector.list_id.is_none()
}

pub(super) fn resolve_write_list(
    lists: &[ReminderListReport],
    default_list: Option<&ReminderListReport>,
    configured_default_id: Option<&str>,
    selector: &WriteReminderListSelectorArgs,
) -> Result<(ReminderListReport, ReminderListSelection)> {
    let (list, selection) = if write_list_selector_is_empty(selector) {
        if let Some(list_id) = configured_default_id {
            let resolved = resolve_lists(
                lists,
                &ReadReminderListSelectorArgs {
                    lists: Vec::new(),
                    list_ids: vec![list_id.to_string()],
                    list_source: None,
                    source_id: None,
                },
            )?;
            let [list] = resolved.as_slice() else {
                bail!("configured reminder list must resolve to exactly one list");
            };
            (list.clone(), ReminderListSelection::ConfiguredDefault)
        } else {
            (
                default_list
                    .cloned()
                    .context("EventKit did not return a default reminder list")?,
                ReminderListSelection::EventkitDefault,
            )
        }
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

pub(super) fn resolve_lists(
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
