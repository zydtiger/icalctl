use super::flight::{normalize_airport, normalize_flight_number, parse_offset_datetime};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ParsedFlightNotes {
    pub(super) flight_number: String,
    pub(super) from_airport: String,
    pub(super) to_airport: String,
    pub(super) departure: DateTime<FixedOffset>,
    pub(super) arrival: DateTime<FixedOffset>,
}

pub(super) fn parse_flight_notes(notes: &str) -> Result<ParsedFlightNotes> {
    let header = match notes.split_once("\n\n") {
        Some((_header, "")) => bail!("extra flight notes must not be empty"),
        Some((header, _extra_notes)) => header,
        None => notes,
    };
    if header.contains('\r') {
        bail!("canonical flight header must use LF line endings");
    }
    let lines = header.split('\n').collect::<Vec<_>>();
    if lines.len() != 4 {
        bail!("flight notes must contain exactly four canonical header lines");
    }

    let flight_number_value = canonical_value(lines[0], "Flight: ", "Flight")?;
    let flight_number = normalize_flight_number(flight_number_value)?;
    if flight_number != flight_number_value {
        bail!("flight number must use its canonical uppercase form");
    }

    let route = canonical_value(lines[1], "Route: ", "Route")?;
    let (from_value, to_value) = route
        .split_once(" to ")
        .context("Route must use the canonical '<FROM> to <TO>' form")?;
    let from_airport = canonical_airport(from_value, "departure")?;
    let to_airport = canonical_airport(to_value, "arrival")?;

    let departure_value = canonical_value(lines[2], "Departure: ", "Departure")?;
    let (departure_airport, departure_scheduled) = departure_value
        .split_once(' ')
        .context("Departure must contain an airport and RFC3339 timestamp")?;
    let departure_airport = canonical_airport(departure_airport, "departure")?;
    if departure_airport != from_airport {
        bail!("Departure airport must match the Route departure airport");
    }
    let departure = parse_offset_datetime(departure_scheduled, "departure")?;

    let arrival_value = canonical_value(lines[3], "Arrival: ", "Arrival")?;
    let (arrival_airport, arrival_scheduled) = arrival_value
        .split_once(' ')
        .context("Arrival must contain an airport and RFC3339 timestamp")?;
    let arrival_airport = canonical_airport(arrival_airport, "arrival")?;
    if arrival_airport != to_airport {
        bail!("Arrival airport must match the Route arrival airport");
    }
    let arrival = parse_offset_datetime(arrival_scheduled, "arrival")?;
    if arrival <= departure {
        bail!("arrival instant must be after departure instant");
    }

    Ok(ParsedFlightNotes {
        flight_number,
        from_airport,
        to_airport,
        departure,
        arrival,
    })
}

fn canonical_value<'a>(line: &'a str, prefix: &str, field: &str) -> Result<&'a str> {
    let value = line
        .strip_prefix(prefix)
        .with_context(|| format!("{field} line must start with {prefix:?}"))?;
    if value.is_empty() || value.trim() != value {
        bail!("{field} value must be non-empty and have no surrounding whitespace");
    }
    Ok(value)
}

fn canonical_airport(value: &str, kind: &str) -> Result<String> {
    let airport = normalize_airport(value, kind)?;
    if airport != value {
        bail!("{kind} airport code must use its canonical uppercase form");
    }
    Ok(airport)
}

pub(super) fn parse_event_datetime(value: &str, kind: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{kind} is not a valid RFC3339 timestamp: {value:?}"))
}
