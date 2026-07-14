use super::parser::parse_flight_notes;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset};

pub(crate) struct FlightInput<'a> {
    pub flight_number: &'a str,
    pub from_airport: &'a str,
    pub to_airport: &'a str,
    pub departure: &'a str,
    pub arrival: &'a str,
    pub extra_notes: Option<&'a str>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct FormattedFlight {
    pub title: String,
    pub location: String,
    pub start: String,
    pub end: String,
    pub notes: String,
}

pub(crate) fn format_flight(input: FlightInput<'_>) -> Result<FormattedFlight> {
    let flight_number = normalize_flight_number(input.flight_number)?;
    let from_airport = normalize_airport(input.from_airport, "departure")?;
    let to_airport = normalize_airport(input.to_airport, "arrival")?;
    let departure_input = input.departure.trim();
    let arrival_input = input.arrival.trim();
    let departure = parse_offset_datetime(departure_input, "departure")?;
    let arrival = parse_offset_datetime(arrival_input, "arrival")?;
    if arrival <= departure {
        bail!("arrival instant must be after departure instant");
    }

    let route = format!("{from_airport} to {to_airport}");
    let mut notes = format!(
        "Flight: {flight_number}\nRoute: {route}\nDeparture: {from_airport} {departure_input}\nArrival: {to_airport} {arrival_input}"
    );
    if let Some(extra_notes) = input.extra_notes
        && !extra_notes.is_empty()
    {
        notes.push_str("\n\n");
        notes.push_str(extra_notes);
    }
    parse_flight_notes(&notes).context("failed to validate formatted flight notes")?;

    Ok(FormattedFlight {
        title: format!("Flight {flight_number}: {route}"),
        location: route,
        start: departure_input.to_string(),
        end: arrival_input.to_string(),
        notes,
    })
}

pub(super) fn normalize_flight_number(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("flight number must not be empty");
    }
    Ok(value.to_uppercase())
}

pub(super) fn normalize_airport(value: &str, kind: &str) -> Result<String> {
    let value = value.trim();
    if !(3..=4).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        bail!("{kind} airport code must contain 3 or 4 ASCII letters: {value:?}");
    }
    Ok(value.to_ascii_uppercase())
}

pub(super) fn parse_offset_datetime(value: &str, kind: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).with_context(|| {
        format!("{kind} must be an RFC3339 timestamp with an explicit UTC offset: {value:?}")
    })
}
