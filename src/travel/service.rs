use super::{TravelCollection, collect_travel_events};
use crate::config::FlightAwareConfig;
use crate::models::EventReport;
use anyhow::{Context, Result};
use chrono::{DateTime, Local};

pub(super) fn collect_for_range<F>(
    flightaware: &FlightAwareConfig,
    calendar_ids: &[String],
    start: DateTime<Local>,
    end: DateTime<Local>,
    fetch: &F,
) -> Result<TravelCollection>
where
    F: Fn(DateTime<Local>, DateTime<Local>, &[String]) -> Result<Vec<EventReport>>,
{
    let events = fetch(start, end, calendar_ids)
        .context("failed to read local Apple Calendar through EventKit")?;
    let mut collection = collect_travel_events(&events);
    crate::flightaware::enrich_collection(flightaware, &mut collection);
    Ok(collection)
}
