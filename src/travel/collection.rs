use super::airports::airport_for_code;
use super::parser::{parse_event_datetime, parse_flight_notes};
use super::{TravelCollection, TravelEventSource, TravelLeg, TravelMoment, TravelWarning};
use crate::models::EventReport;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::cmp::Ordering;
use std::collections::HashSet;

pub(crate) fn collect_travel_events(events: &[EventReport]) -> TravelCollection {
    let mut legs = Vec::new();
    let mut warnings = Vec::new();

    for event in events {
        if !looks_like_flight_event(event) {
            continue;
        }

        match travel_leg_from_event(event) {
            Ok((leg, mut leg_warnings)) => {
                legs.push(leg);
                warnings.append(&mut leg_warnings);
            }
            Err(error) => warnings.push(TravelWarning {
                kind: "malformed_flight_event".to_string(),
                event_id: Some(event.id.clone()),
                message: format!("{}: {error:#}", event.title),
            }),
        }
    }

    legs.sort_by(compare_legs);
    let mut occurrence_ids = HashSet::new();
    legs.retain(|leg| occurrence_ids.insert(occurrence_identity(leg)));

    warnings.sort_by(|left, right| {
        (&left.event_id, &left.kind, &left.message).cmp(&(
            &right.event_id,
            &right.kind,
            &right.message,
        ))
    });
    warnings.dedup();

    TravelCollection { legs, warnings }
}

pub(super) fn travel_leg_from_event(
    event: &EventReport,
) -> Result<(TravelLeg, Vec<TravelWarning>)> {
    if event.all_day {
        bail!("canonical flight events must be timed events");
    }
    let notes = event
        .notes
        .as_deref()
        .context("canonical flight event is missing notes")?;
    let parsed = parse_flight_notes(notes).context("invalid canonical flight notes")?;
    let event_start = parse_event_datetime(&event.start_utc, "event start")?;
    let event_end = parse_event_datetime(&event.end_utc, "event end")?;
    if event_end <= event_start {
        bail!("Calendar event end must be after its start");
    }

    let mut warnings = Vec::new();
    let departure_differs = parsed.departure.with_timezone(&Utc) != event_start.with_timezone(&Utc);
    let arrival_differs = parsed.arrival.with_timezone(&Utc) != event_end.with_timezone(&Utc);
    if departure_differs || arrival_differs {
        let fields = match (departure_differs, arrival_differs) {
            (true, true) => "Departure and Arrival instants differ",
            (true, false) => "Departure instant differs",
            (false, true) => "Arrival instant differs",
            (false, false) => unreachable!(),
        };
        warnings.push(TravelWarning {
            kind: "calendar_time_mismatch".to_string(),
            event_id: Some(event.id.clone()),
            message: format!(
                "{}: canonical {fields} from the Calendar event; using Calendar times",
                event.title
            ),
        });
    }

    let departure_airport = airport_for_code(&parsed.from_airport);
    let arrival_airport = airport_for_code(&parsed.to_airport);
    for airport in [&departure_airport, &arrival_airport] {
        if airport.metadata.is_none() {
            warnings.push(TravelWarning {
                kind: "unknown_airport".to_string(),
                event_id: Some(event.id.clone()),
                message: format!(
                    "{} uses airport code {}, which is absent from the bundled airport metadata",
                    event.title, airport.code
                ),
            });
        }
    }

    Ok((
        TravelLeg {
            flight_number: parsed.flight_number,
            route: format!("{} to {}", parsed.from_airport, parsed.to_airport),
            departure_airport,
            arrival_airport,
            departure: TravelMoment {
                scheduled: event_start
                    .with_timezone(parsed.departure.offset())
                    .to_rfc3339(),
                utc: event_start.with_timezone(&Utc).to_rfc3339(),
            },
            arrival: TravelMoment {
                scheduled: event_end
                    .with_timezone(parsed.arrival.offset())
                    .to_rfc3339(),
                utc: event_end.with_timezone(&Utc).to_rfc3339(),
            },
            live_status: None,
            source: TravelEventSource {
                event_id: event.id.clone(),
                occurrence_date: event.occurrence_date.clone(),
                title: event.title.clone(),
                calendar: event.calendar.clone(),
                calendar_id: event.calendar_id.clone(),
            },
        },
        warnings,
    ))
}

fn looks_like_flight_event(event: &EventReport) -> bool {
    event
        .notes
        .as_deref()
        .is_some_and(|notes| notes.starts_with("Flight: "))
}

fn compare_legs(left: &TravelLeg, right: &TravelLeg) -> Ordering {
    (
        &left.departure.utc,
        &left.arrival.utc,
        &left.source.calendar_id,
        &left.source.event_id,
        &left.source.occurrence_date,
        &left.flight_number,
        &left.route,
    )
        .cmp(&(
            &right.departure.utc,
            &right.arrival.utc,
            &right.source.calendar_id,
            &right.source.event_id,
            &right.source.occurrence_date,
            &right.flight_number,
            &right.route,
        ))
}

fn occurrence_identity(leg: &TravelLeg) -> (Option<String>, String, String) {
    (
        leg.source.calendar_id.clone(),
        leg.source.event_id.clone(),
        leg.source
            .occurrence_date
            .clone()
            .unwrap_or_else(|| leg.departure.utc.clone()),
    )
}
