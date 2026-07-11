use crate::models::{EventReport, JsonOutput, ReminderDateKind, ReminderReport};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const CACHE_VERSION: u8 = 1;

#[derive(Debug, Deserialize, Serialize)]
struct EventCache {
    version: u8,
    events: Vec<CachedEvent>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CachedEvent {
    row: usize,
    id: String,
    title: String,
    start: String,
    end: String,
    calendar: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventReference {
    pub id: String,
    pub occurrence_start: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ReminderCache {
    version: u8,
    reminders: Vec<CachedReminder>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CachedReminder {
    row: usize,
    id: String,
    title: String,
    due: Option<String>,
    list: Option<String>,
}

pub fn update_from_output(output: &JsonOutput) -> Result<()> {
    match output {
        JsonOutput::Events { events } => write_events(events),
        JsonOutput::Reminders { reminders } => write_reminders(reminders),
        _ => Ok(()),
    }
}

pub fn resolve_event_show_ref(
    reference: &str,
    occurrence_start: Option<String>,
) -> Result<EventReference> {
    let Ok(row) = reference.parse::<usize>() else {
        return Ok(EventReference {
            id: reference.to_string(),
            occurrence_start,
        });
    };
    if occurrence_start.is_some() {
        bail!("--occurrence-start cannot be combined with a cached row number");
    }
    if row == 0 {
        bail!("row numbers start at 1");
    }
    let cache = read_cache()?;
    let event = cache
        .events
        .iter()
        .find(|event| event.row == row)
        .ok_or_else(|| anyhow!("no cached event at row {row}; run a list command first"))?;
    Ok(cached_event_reference(event))
}

fn cached_event_reference(event: &CachedEvent) -> EventReference {
    EventReference {
        id: event.id.clone(),
        occurrence_start: Some(event.start.clone()),
    }
}

pub fn resolve_reminder_ref(reference: &str) -> Result<String> {
    let Ok(row) = reference.parse::<usize>() else {
        return Ok(reference.to_string());
    };

    if row == 0 {
        bail!("row numbers start at 1");
    }

    let cache = read_reminder_cache()?;
    let reminder = cache
        .reminders
        .iter()
        .find(|reminder| reminder.row == row)
        .ok_or_else(|| {
            anyhow!("no cached reminder at row {row}; run `icalctl reminders list` or `icalctl reminders search` first")
        })?;
    Ok(reminder.id.clone())
}

fn write_events(events: &[EventReport]) -> Result<()> {
    let path = cache_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }

    let cache = EventCache {
        version: CACHE_VERSION,
        events: events
            .iter()
            .enumerate()
            .map(|(index, event)| CachedEvent {
                row: index + 1,
                id: event.id.clone(),
                title: event.title.clone(),
                start: event.start.clone(),
                end: event.end.clone(),
                calendar: event.calendar.clone(),
            })
            .collect(),
    };

    let json = serde_json::to_vec_pretty(&cache).context("failed to serialize event cache")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn write_reminders(reminders: &[ReminderReport]) -> Result<()> {
    let path = reminder_cache_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    }

    let cache = ReminderCache {
        version: CACHE_VERSION,
        reminders: reminders
            .iter()
            .enumerate()
            .map(|(index, reminder)| CachedReminder {
                row: index + 1,
                id: reminder.id.clone(),
                title: reminder.title.clone(),
                due: reminder.due.as_ref().and_then(|due| match due.kind {
                    ReminderDateKind::Date => due.date.clone(),
                    ReminderDateKind::Datetime => {
                        due.normalized.clone().or_else(|| due.local.clone())
                    }
                }),
                list: reminder.list.clone(),
            })
            .collect(),
    };

    let json = serde_json::to_vec_pretty(&cache).context("failed to serialize reminder cache")?;
    fs::write(&path, json).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn read_cache() -> Result<EventCache> {
    let path = cache_path()?;
    let json = fs::read(&path).with_context(|| {
        format!(
            "failed to read {}; run `icalctl today`, `icalctl list`, `icalctl upcoming`, or `icalctl search` first",
            path.display()
        )
    })?;
    let cache: EventCache = serde_json::from_slice(&json)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if cache.version != CACHE_VERSION {
        bail!("cached event list is from an unsupported cache version; run a list command again");
    }

    Ok(cache)
}

fn read_reminder_cache() -> Result<ReminderCache> {
    let path = reminder_cache_path()?;
    let json = fs::read(&path).with_context(|| {
        format!(
            "failed to read {}; run `icalctl reminders list` or `icalctl reminders search` first",
            path.display()
        )
    })?;
    let cache: ReminderCache = serde_json::from_slice(&json)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if cache.version != CACHE_VERSION {
        bail!(
            "cached reminder list is from an unsupported cache version; run a reminder list command again"
        );
    }
    Ok(cache)
}

fn cache_path() -> Result<PathBuf> {
    cache_file_path("last-events.json")
}

fn reminder_cache_path() -> Result<PathBuf> {
    cache_file_path("last-reminders.json")
}

fn cache_file_path(file_name: &str) -> Result<PathBuf> {
    if let Some(xdg_cache_home) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(xdg_cache_home)
            .join("icalctl")
            .join(file_name));
    }

    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Caches")
        .join("icalctl")
        .join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_numeric_reminder_reference_is_left_as_id() {
        assert_eq!(resolve_reminder_ref("REM-123").unwrap(), "REM-123");
    }

    #[test]
    fn explicit_show_reference_preserves_occurrence_start() {
        assert_eq!(
            resolve_event_show_ref("ABC-123", Some("2026-07-11T09:00:00+03:00".to_string()))
                .unwrap(),
            EventReference {
                id: "ABC-123".to_string(),
                occurrence_start: Some("2026-07-11T09:00:00+03:00".to_string()),
            }
        );
    }

    #[test]
    fn cached_show_reference_keeps_the_selected_occurrence_start() {
        let reference = cached_event_reference(&CachedEvent {
            row: 3,
            id: "SERIES-ID".to_string(),
            title: "Weekly sync".to_string(),
            start: "2026-07-20T09:00:00+03:00".to_string(),
            end: "2026-07-20T09:30:00+03:00".to_string(),
            calendar: Some("Work".to_string()),
        });

        assert_eq!(reference.id, "SERIES-ID");
        assert_eq!(
            reference.occurrence_start.as_deref(),
            Some("2026-07-20T09:00:00+03:00")
        );
    }

    #[test]
    fn reminder_and_event_caches_are_separate_files() {
        assert_eq!(
            cache_path().unwrap().file_name().unwrap(),
            "last-events.json"
        );
        assert_eq!(
            reminder_cache_path().unwrap().file_name().unwrap(),
            "last-reminders.json"
        );
    }
}
