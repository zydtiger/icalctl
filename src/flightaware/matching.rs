use super::client::{ApiAirport, ApiFlight};
use super::normalization::provider_time;
use super::parse_timestamp;
use crate::travel::{TravelAirport, TravelLeg};
use std::cmp::Ordering;

const MATCH_TOLERANCE_HOURS: i64 = 4;

pub(super) enum MatchSelection<'a> {
    Matched(&'a ApiFlight),
    NoMatch,
    Ambiguous,
}

pub(super) fn select_match<'a>(flights: &'a [ApiFlight], leg: &TravelLeg) -> MatchSelection<'a> {
    let mut candidates = flights
        .iter()
        .filter_map(|flight| {
            let ident_rank = ident_rank(flight, &leg.flight_number)?;
            if !airport_matches(&leg.departure_airport, flight.origin.as_ref())
                || !airport_matches(&leg.arrival_airport, flight.destination.as_ref())
            {
                return None;
            }
            let provider_departure = provider_time(
                flight.scheduled_out.as_deref(),
                flight.scheduled_off.as_deref(),
            )?;
            let calendar_departure = parse_timestamp(&leg.departure.utc)?;
            let delta = (provider_departure - calendar_departure)
                .num_seconds()
                .unsigned_abs();
            (delta <= (MATCH_TOLERANCE_HOURS * 60 * 60) as u64)
                .then_some((flight, (delta, ident_rank)))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(compare_candidates);
    let Some((best, score)) = candidates.first() else {
        return MatchSelection::NoMatch;
    };
    if candidates
        .get(1)
        .is_some_and(|(_, next_score)| next_score == score)
    {
        return MatchSelection::Ambiguous;
    }
    MatchSelection::Matched(best)
}

fn compare_candidates(left: &(&ApiFlight, (u64, u8)), right: &(&ApiFlight, (u64, u8))) -> Ordering {
    left.1
        .cmp(&right.1)
        .then_with(|| left.0.fa_flight_id.cmp(&right.0.fa_flight_id))
}

fn ident_rank(flight: &ApiFlight, expected: &str) -> Option<u8> {
    let expected = normalize_ident(expected);
    [
        Some(flight.ident.as_str()),
        flight.ident_icao.as_deref(),
        flight.ident_iata.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|ident| normalize_ident(ident) == expected)
    .then_some(0)
    .or_else(|| {
        flight
            .codeshares
            .iter()
            .flatten()
            .chain(flight.codeshares_iata.iter().flatten())
            .any(|ident| normalize_ident(ident) == expected)
            .then_some(1)
    })
}

pub(super) fn normalize_ident(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

fn airport_matches(calendar: &TravelAirport, provider: Option<&ApiAirport>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    let mut calendar_codes = vec![calendar.code.as_str()];
    if let Some(metadata) = &calendar.metadata {
        calendar_codes.push(&metadata.iata_code);
        if let Some(icao_code) = metadata.icao_code.as_deref() {
            calendar_codes.push(icao_code);
        }
    }
    let provider_codes = [
        provider.code.as_deref(),
        provider.code_icao.as_deref(),
        provider.code_iata.as_deref(),
    ];
    calendar_codes.iter().any(|calendar_code| {
        provider_codes
            .iter()
            .flatten()
            .any(|provider_code| calendar_code.eq_ignore_ascii_case(provider_code))
    })
}
