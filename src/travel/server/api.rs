use super::range::DateRange;
use crate::cli::ReadCalendarSelectorArgs;
use crate::config::Config;
use crate::dates::{parse_end_datetime, parse_start_datetime};
use crate::models::EventReport;
use crate::travel::{TravelCollection, TravelLeg, TravelWarning, service};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::Serialize;

pub(super) const API_SCHEMA_VERSION: u8 = 1;
const DEFAULT_MAP_STYLE_URL: &str = "https://tiles.openfreemap.org/styles/bright";

#[derive(Debug, Serialize)]
pub(super) struct TravelApiResponse {
    pub(super) schema_version: u8,
    pub(super) generated_at: String,
    pub(super) range: TravelApiRange,
    pub(super) calendar_ids: Vec<String>,
    pub(super) map: TravelApiMap,
    pub(super) legs: Vec<TravelLeg>,
    pub(super) warnings: Vec<TravelWarning>,
}

#[derive(Debug, Serialize)]
pub(super) struct TravelApiRange {
    pub(super) start: String,
    pub(super) end: String,
    pub(super) end_inclusive: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct TravelApiMap {
    pub(super) projection: String,
    pub(super) style_url: String,
}

pub(super) fn build_payload<F>(
    config: &Config,
    calendar_ids: &[String],
    range: &DateRange,
    now: DateTime<Utc>,
    fetch: &F,
) -> Result<TravelApiResponse>
where
    F: Fn(DateTime<Local>, DateTime<Local>, &ReadCalendarSelectorArgs) -> Result<Vec<EventReport>>,
{
    let start_text = range.start.format("%Y-%m-%d").to_string();
    let end_text = range.end.format("%Y-%m-%d").to_string();
    let mut event_start = parse_start_datetime(&start_text).context("invalid range start")?;
    let event_end = parse_end_datetime(&end_text).context("invalid range end")?;
    if range.future_only_start {
        let current = now.with_timezone(&Local);
        if current > event_start {
            event_start = current;
        }
    }
    let collection = service::collect_for_range(
        &config.flightaware,
        calendar_ids,
        event_start,
        event_end,
        &|start, end, ids| {
            fetch(
                start,
                end,
                &ReadCalendarSelectorArgs {
                    calendars: Vec::new(),
                    calendar_ids: ids.to_vec(),
                    calendar_source: None,
                    source_id: None,
                },
            )
        },
    )?;
    let TravelCollection { legs, warnings } = collection;

    Ok(TravelApiResponse {
        schema_version: API_SCHEMA_VERSION,
        generated_at: now.to_rfc3339(),
        range: TravelApiRange {
            start: start_text,
            end: end_text,
            end_inclusive: true,
        },
        calendar_ids: calendar_ids.to_vec(),
        map: TravelApiMap {
            projection: config.travel.map.projection.clone(),
            style_url: config
                .travel
                .map
                .style_url
                .clone()
                .unwrap_or_else(|| DEFAULT_MAP_STYLE_URL.to_string()),
        },
        legs,
        warnings,
    })
}
