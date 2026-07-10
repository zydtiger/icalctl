use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset};

pub struct FlightInput<'a> {
    pub flight_number: &'a str,
    pub from_airport: &'a str,
    pub to_airport: &'a str,
    pub departure: &'a str,
    pub arrival: &'a str,
    pub extra_notes: Option<&'a str>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct FormattedFlight {
    pub title: String,
    pub location: String,
    pub start: String,
    pub end: String,
    pub notes: String,
}

pub fn format_flight(input: FlightInput<'_>) -> Result<FormattedFlight> {
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

    Ok(FormattedFlight {
        title: format!("Flight {flight_number}: {route}"),
        location: route,
        start: departure_input.to_string(),
        end: arrival_input.to_string(),
        notes,
    })
}

fn normalize_flight_number(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("flight number must not be empty");
    }
    Ok(value.to_uppercase())
}

fn normalize_airport(value: &str, kind: &str) -> Result<String> {
    let value = value.trim();
    if !(3..=4).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        bail!("{kind} airport code must contain 3 or 4 ASCII letters: {value:?}");
    }
    Ok(value.to_ascii_uppercase())
}

fn parse_offset_datetime(value: &str, kind: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).with_context(|| {
        format!("{kind} must be an RFC3339 timestamp with an explicit UTC offset: {value:?}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(extra_notes: Option<&str>) -> FlightInput<'_> {
        FlightInput {
            flight_number: "HO1607",
            from_airport: "PVG",
            to_airport: "HEL",
            departure: "2026-07-11T09:25:00+08:00",
            arrival: "2026-07-11T14:00:00+03:00",
            extra_notes,
        }
    }

    #[test]
    fn formats_the_canonical_flight_event_exactly() {
        let flight = format_flight(example(None)).unwrap();

        assert_eq!(flight.title, "Flight HO1607: PVG to HEL");
        assert_eq!(flight.location, "PVG to HEL");
        assert_eq!(flight.start, "2026-07-11T09:25:00+08:00");
        assert_eq!(flight.end, "2026-07-11T14:00:00+03:00");
        assert_eq!(
            flight.notes,
            "Flight: HO1607\nRoute: PVG to HEL\nDeparture: PVG 2026-07-11T09:25:00+08:00\nArrival: HEL 2026-07-11T14:00:00+03:00"
        );
    }

    #[test]
    fn normalizes_identifiers_and_appends_extra_notes_after_one_blank_line() {
        let flight = format_flight(FlightInput {
            flight_number: "  ho1607 ",
            from_airport: " pvg ",
            to_airport: " hel ",
            extra_notes: Some("Booking: ABC123\nSeat: 4A\n"),
            ..example(None)
        })
        .unwrap();

        assert_eq!(flight.title, "Flight HO1607: PVG to HEL");
        assert!(flight.notes.ends_with("\n\nBooking: ABC123\nSeat: 4A\n"));
    }

    #[test]
    fn rejects_invalid_airport_codes() {
        let short = format_flight(FlightInput {
            from_airport: "PV",
            ..example(None)
        });
        let non_alpha = format_flight(FlightInput {
            to_airport: "H3L",
            ..example(None)
        });
        let long = format_flight(FlightInput {
            to_airport: "HELLO",
            ..example(None)
        });

        assert!(short.is_err());
        assert!(non_alpha.is_err());
        assert!(long.is_err());
    }

    #[test]
    fn rejects_timezone_less_timestamps() {
        let error = format_flight(FlightInput {
            departure: "2026-07-11T09:25:00",
            ..example(None)
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("explicit UTC offset"));
    }

    #[test]
    fn compares_absolute_instants_not_displayed_local_clocks() {
        let error = format_flight(FlightInput {
            departure: "2026-07-11T09:25:00+08:00",
            arrival: "2026-07-11T03:00:00+03:00",
            ..example(None)
        })
        .unwrap_err()
        .to_string();

        assert_eq!(error, "arrival instant must be after departure instant");
    }

    #[test]
    fn rejects_empty_flight_numbers() {
        let error = format_flight(FlightInput {
            flight_number: "  ",
            ..example(None)
        })
        .unwrap_err()
        .to_string();

        assert_eq!(error, "flight number must not be empty");
    }
}
