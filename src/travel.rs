use crate::flightaware::FlightStatus;
use crate::models::EventReport;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, FixedOffset, Utc};
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const AIRPORTS_TSV: &str = include_str!("../assets/airports.tsv");

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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AirportMetadata {
    pub iata_code: String,
    pub icao_code: Option<String>,
    pub name: String,
    pub municipality: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TravelAirport {
    pub code: String,
    pub metadata: Option<AirportMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TravelMoment {
    pub scheduled: String,
    pub utc: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TravelEventSource {
    pub event_id: String,
    pub occurrence_date: Option<String>,
    pub title: String,
    pub calendar: Option<String>,
    pub calendar_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TravelLeg {
    pub flight_number: String,
    pub route: String,
    pub departure_airport: TravelAirport,
    pub arrival_airport: TravelAirport,
    pub departure: TravelMoment,
    pub arrival: TravelMoment,
    pub live_status: Option<FlightStatus>,
    pub source: TravelEventSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TravelWarning {
    pub kind: String,
    pub event_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct TravelCollection {
    pub legs: Vec<TravelLeg>,
    pub warnings: Vec<TravelWarning>,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedFlightNotes {
    flight_number: String,
    from_airport: String,
    to_airport: String,
    departure: DateTime<FixedOffset>,
    arrival: DateTime<FixedOffset>,
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
    parse_flight_notes(&notes).context("failed to validate formatted flight notes")?;

    Ok(FormattedFlight {
        title: format!("Flight {flight_number}: {route}"),
        location: route,
        start: departure_input.to_string(),
        end: arrival_input.to_string(),
        notes,
    })
}

fn parse_flight_notes(notes: &str) -> Result<ParsedFlightNotes> {
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

pub fn collect_travel_events(events: &[EventReport]) -> TravelCollection {
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

fn travel_leg_from_event(event: &EventReport) -> Result<(TravelLeg, Vec<TravelWarning>)> {
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

fn parse_event_datetime(value: &str, kind: &str) -> Result<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{kind} is not a valid RFC3339 timestamp: {value:?}"))
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

fn airport_for_code(code: &str) -> TravelAirport {
    TravelAirport {
        code: code.to_string(),
        metadata: airport_database().get(code).cloned(),
    }
}

fn airport_database() -> &'static HashMap<String, AirportMetadata> {
    static AIRPORTS: OnceLock<HashMap<String, AirportMetadata>> = OnceLock::new();
    AIRPORTS.get_or_init(|| {
        let mut airports = HashMap::new();
        for (line_index, line) in AIRPORTS_TSV.lines().enumerate() {
            let columns = line.split('\t').collect::<Vec<_>>();
            assert_eq!(
                columns.len(),
                6,
                "invalid bundled airport row {}",
                line_index + 1
            );
            let metadata = AirportMetadata {
                iata_code: columns[0].to_string(),
                icao_code: nonempty(columns[1]),
                name: columns[2].to_string(),
                municipality: nonempty(columns[3]),
                latitude: columns[4]
                    .parse()
                    .expect("bundled airport latitude must be numeric"),
                longitude: columns[5]
                    .parse()
                    .expect("bundled airport longitude must be numeric"),
            };
            airports
                .entry(metadata.iata_code.clone())
                .or_insert_with(|| metadata.clone());
            if let Some(icao_code) = &metadata.icao_code {
                airports
                    .entry(icao_code.clone())
                    .or_insert_with(|| metadata.clone());
            }
        }
        airports
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
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
    use crate::models::EventReport;

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

    fn event(
        id: &str,
        flight_number: &str,
        from_airport: &str,
        to_airport: &str,
        departure: &str,
        arrival: &str,
        occurrence_date: Option<&str>,
    ) -> EventReport {
        let formatted = format_flight(FlightInput {
            flight_number,
            from_airport,
            to_airport,
            departure,
            arrival,
            extra_notes: None,
        })
        .unwrap();
        let departure_datetime = DateTime::parse_from_rfc3339(departure).unwrap();
        let arrival_datetime = DateTime::parse_from_rfc3339(arrival).unwrap();

        EventReport {
            id: id.to_string(),
            title: formatted.title,
            start: departure.to_string(),
            end: arrival.to_string(),
            start_input: None,
            end_input: None,
            start_utc: departure_datetime.with_timezone(&Utc).to_rfc3339(),
            end_utc: arrival_datetime.with_timezone(&Utc).to_rfc3339(),
            start_local: departure.to_string(),
            end_local: arrival.to_string(),
            start_in_event_time_zone: None,
            end_in_event_time_zone: None,
            duration_seconds: (arrival_datetime - departure_datetime).num_seconds(),
            all_day: false,
            calendar: Some("Travel".to_string()),
            calendar_id: Some("calendar-id".to_string()),
            calendar_source: Some("iCloud".to_string()),
            calendar_source_id: Some("source-id".to_string()),
            calendar_type: Some("CalDav".to_string()),
            allows_calendar_modifications: Some(true),
            calendar_selection: None,
            write_action: None,
            write_scope: None,
            location: Some(formatted.location),
            notes: Some(formatted.notes),
            url: None,
            status: "Confirmed".to_string(),
            availability: "Busy".to_string(),
            has_notes: true,
            has_url: false,
            alarm_count: Some(0),
            recurrence_count: Some(0),
            recurrence_rules: None,
            is_detached: occurrence_date.is_some(),
            occurrence_date: occurrence_date.map(str::to_string),
            creation_date: None,
            last_modified_date: None,
            external_identifier: None,
            timezone: None,
            attachments_count: 0,
            attendees: Vec::new(),
            organizer: None,
            alarms: None,
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

    #[test]
    fn parses_formatted_notes_and_accepts_free_form_crlf_suffixes() {
        let formatted = format_flight(example(Some("Booking: ABC123\nSeat: 4A"))).unwrap();
        let parsed = parse_flight_notes(&formatted.notes).unwrap();
        let crlf = format_flight(example(Some("Booking: ABC123\r\nSeat: 4A"))).unwrap();

        assert_eq!(parsed.flight_number, "HO1607");
        assert_eq!(parsed.from_airport, "PVG");
        assert_eq!(parsed.to_airport, "HEL");
        assert_eq!(parsed.departure.to_rfc3339(), "2026-07-11T09:25:00+08:00");
        assert_eq!(parsed.arrival.to_rfc3339(), "2026-07-11T14:00:00+03:00");
        assert_eq!(
            parsed.departure.with_timezone(&Utc).to_rfc3339(),
            "2026-07-11T01:25:00+00:00"
        );
        assert!(parse_flight_notes(&crlf.notes).is_ok());
    }

    #[test]
    fn rejects_noncanonical_headers_and_single_newline_extra_text() {
        let formatted = format_flight(example(None)).unwrap();
        let wrong_header = formatted.notes.replacen("Route: ", "Route : ", 1);
        let single_newline = format!("{}\nSeat: 4A", formatted.notes);

        assert!(
            parse_flight_notes(&wrong_header)
                .unwrap_err()
                .to_string()
                .contains("Route line")
        );
        assert!(
            parse_flight_notes(&single_newline)
                .unwrap_err()
                .to_string()
                .contains("exactly four")
        );
    }

    #[test]
    fn rejects_route_mismatches_but_uses_calendar_event_times() {
        let formatted = format_flight(example(None)).unwrap();
        let wrong_airport = formatted
            .notes
            .replacen("Departure: PVG", "Departure: FRA", 1);
        assert!(
            parse_flight_notes(&wrong_airport)
                .unwrap_err()
                .to_string()
                .contains("must match")
        );

        let mut report = event(
            "event-1",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            None,
        );
        report.start_utc = "2026-07-11T01:30:00+00:00".to_string();
        let (leg, warnings) = travel_leg_from_event(&report).unwrap();

        assert_eq!(leg.departure.utc, "2026-07-11T01:30:00+00:00");
        assert_eq!(leg.departure.scheduled, "2026-07-11T09:30:00+08:00");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "calendar_time_mismatch");
    }

    #[test]
    fn tolerates_eventkit_subsecond_truncation_and_reports_it() {
        let mut report = event(
            "event-fractional",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00.750+08:00",
            "2026-07-11T14:00:00.500+03:00",
            None,
        );
        report.start_utc = "2026-07-11T01:25:00+00:00".to_string();
        report.end_utc = "2026-07-11T11:00:00+00:00".to_string();

        let (leg, warnings) = travel_leg_from_event(&report).unwrap();

        assert_eq!(leg.departure.utc, report.start_utc);
        assert_eq!(leg.arrival.utc, report.end_utc);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "calendar_time_mismatch");
    }

    #[test]
    fn resolves_iata_and_icao_codes_from_bundled_airport_metadata() {
        let expectations = [
            ("PVG", "PVG", 31.1434, 121.805),
            ("HEL", "HEL", 60.318363, 24.963341),
            ("FRA", "FRA", 50.026706, 8.55835),
            ("ZSPD", "PVG", 31.1434, 121.805),
        ];

        for (lookup, iata, latitude, longitude) in expectations {
            let airport = airport_for_code(lookup);
            let metadata = airport.metadata.unwrap();
            assert_eq!(metadata.iata_code, iata);
            assert!((metadata.latitude - latitude).abs() < 0.000001);
            assert!((metadata.longitude - longitude).abs() < 0.000001);
        }
    }

    #[test]
    fn retains_valid_legs_when_airport_metadata_is_unknown() {
        let report = event(
            "event-unknown",
            "HO1607",
            "ZZZZ",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            None,
        );

        let collection = collect_travel_events(&[report]);

        assert_eq!(collection.legs.len(), 1);
        assert_eq!(collection.legs[0].departure_airport.code, "ZZZZ");
        assert!(collection.legs[0].departure_airport.metadata.is_none());
        assert_eq!(collection.warnings.len(), 1);
        assert_eq!(collection.warnings[0].kind, "unknown_airport");
    }

    #[test]
    fn sorts_chronologically_and_deduplicates_only_exact_occurrences() {
        let early = event(
            "event-early",
            "AY141",
            "FRA",
            "HEL",
            "2026-07-10T09:00:00+02:00",
            "2026-07-10T12:20:00+03:00",
            None,
        );
        let occurrence_one = event(
            "recurring-event",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            Some("2026-07-11T01:25:00+00:00"),
        );
        let occurrence_one_duplicate = event(
            "recurring-event",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            Some("2026-07-11T01:25:00+00:00"),
        );
        let occurrence_two = event(
            "recurring-event",
            "HO1608",
            "HEL",
            "FRA",
            "2026-07-12T10:00:00+03:00",
            "2026-07-12T11:40:00+02:00",
            Some("2026-07-12T07:00:00+00:00"),
        );

        let collection = collect_travel_events(&[
            occurrence_two,
            occurrence_one_duplicate,
            early,
            occurrence_one,
        ]);

        assert_eq!(collection.legs.len(), 3);
        assert_eq!(
            collection
                .legs
                .iter()
                .map(|leg| leg.flight_number.as_str())
                .collect::<Vec<_>>(),
            ["AY141", "HO1607", "HO1608"]
        );
    }

    #[test]
    fn reports_malformed_candidates_but_ignores_ordinary_events() {
        let mut malformed = event(
            "event-malformed",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            None,
        );
        malformed.notes = Some("Flight: HO1607\nRoute: broken".to_string());
        let mut ordinary = event(
            "event-ordinary",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            None,
        );
        ordinary.title = "Flight training".to_string();
        ordinary.notes = None;

        let collection = collect_travel_events(&[ordinary, malformed]);

        assert!(collection.legs.is_empty());
        assert_eq!(collection.warnings.len(), 1);
        assert_eq!(collection.warnings[0].kind, "malformed_flight_event");
        assert_eq!(
            collection.warnings[0].event_id.as_deref(),
            Some("event-malformed")
        );
    }

    #[test]
    fn serialized_legs_do_not_expose_free_form_calendar_notes() {
        let mut report = event(
            "event-private-notes",
            "HO1607",
            "PVG",
            "HEL",
            "2026-07-11T09:25:00+08:00",
            "2026-07-11T14:00:00+03:00",
            None,
        );
        report
            .notes
            .as_mut()
            .unwrap()
            .push_str("\n\nBooking reference: PRIVATE123");

        let collection = collect_travel_events(&[report]);
        let serialized = serde_json::to_string(&collection).unwrap();

        assert!(!serialized.contains("PRIVATE123"));
        assert!(!serialized.contains("extra_notes"));
    }
}
