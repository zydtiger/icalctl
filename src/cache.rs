use crate::models::{EventReport, JsonOutput};
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

pub fn update_from_output(output: &JsonOutput) -> Result<()> {
    let JsonOutput::Events { events } = output else {
        return Ok(());
    };

    write_events(events)
}

pub fn resolve_event_ref(reference: &str) -> Result<String> {
    let Ok(row) = reference.parse::<usize>() else {
        return Ok(reference.to_string());
    };

    if row == 0 {
        bail!("row numbers start at 1");
    }

    let cache = read_cache()?;
    let event = cache
        .events
        .iter()
        .find(|event| event.row == row)
        .ok_or_else(|| anyhow!("no cached event at row {row}; run a list command first"))?;

    Ok(event.id.clone())
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

fn cache_path() -> Result<PathBuf> {
    if let Some(xdg_cache_home) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(xdg_cache_home)
            .join("icalctl")
            .join("last-events.json"));
    }

    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Caches")
        .join("icalctl")
        .join("last-events.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_numeric_reference_is_left_as_id() {
        assert_eq!(resolve_event_ref("ABC-123").unwrap(), "ABC-123");
    }
}
