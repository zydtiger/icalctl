use super::client::{ApiFlight, ApiPosition};
use super::{parse_timestamp, timestamp};
use crate::travel::{FlightFreshness, FlightPosition, FlightStatus, TravelLeg};
use chrono::{DateTime, Duration, Utc};

const PROVIDER: &str = "flightaware";

pub(super) fn normalize_status(flight: &ApiFlight, now: DateTime<Utc>) -> FlightStatus {
    let normalized = if flight.diverted {
        "diverted"
    } else if flight.actual_in.is_some() || flight.actual_on.is_some() {
        "arrived"
    } else if flight.cancelled {
        "tracking_ended"
    } else if flight.actual_out.is_some() || flight.actual_off.is_some() {
        "en_route"
    } else if flight.departure_delay.unwrap_or_default() > 15 * 60
        || flight.arrival_delay.unwrap_or_default() > 15 * 60
    {
        "delayed"
    } else {
        "scheduled"
    };
    let description = if flight.status.trim().is_empty() {
        normalized.replace('_', " ")
    } else {
        flight.status.clone()
    };
    FlightStatus {
        provider: PROVIDER.to_string(),
        provider_flight_id: flight.fa_flight_id.clone(),
        status: normalized.to_string(),
        description,
        scheduled_departure: normalized_time(
            flight.scheduled_out.as_deref(),
            flight.scheduled_off.as_deref(),
        ),
        estimated_departure: normalized_time(
            flight.estimated_out.as_deref(),
            flight.estimated_off.as_deref(),
        ),
        actual_departure: normalized_time(
            flight.actual_out.as_deref(),
            flight.actual_off.as_deref(),
        ),
        scheduled_arrival: normalized_time(
            flight.scheduled_in.as_deref(),
            flight.scheduled_on.as_deref(),
        ),
        estimated_arrival: normalized_time(
            flight.estimated_in.as_deref(),
            flight.estimated_on.as_deref(),
        ),
        actual_arrival: normalized_time(flight.actual_in.as_deref(), flight.actual_on.as_deref()),
        departure_delay_seconds: flight.departure_delay,
        arrival_delay_seconds: flight.arrival_delay,
        departure_terminal: flight.terminal_origin.clone(),
        departure_gate: flight.gate_origin.clone(),
        arrival_terminal: flight.terminal_destination.clone(),
        arrival_gate: flight.gate_destination.clone(),
        tracking_ended: flight.cancelled,
        diverted: flight.diverted,
        current_position: None,
        freshness: FlightFreshness {
            state: "fresh".to_string(),
            fetched_at: timestamp(now),
            expires_at: timestamp(now),
        },
    }
}

pub(super) fn normalize_position(position: ApiPosition) -> Option<FlightPosition> {
    if !(-90.0..=90.0).contains(&position.latitude)
        || !(-180.0..=180.0).contains(&position.longitude)
        || position
            .heading
            .is_some_and(|heading| !(0..=360).contains(&heading))
    {
        return None;
    }
    let received_at = parse_timestamp(&position.timestamp)?;
    Some(FlightPosition {
        latitude: position.latitude,
        longitude: position.longitude,
        timestamp: timestamp(received_at),
        altitude_feet: position.altitude.checked_mul(100),
        groundspeed_knots: Some(position.groundspeed),
        heading_degrees: position.heading,
    })
}

pub(super) fn provider_time(
    primary: Option<&str>,
    fallback: Option<&str>,
) -> Option<DateTime<Utc>> {
    primary
        .and_then(parse_timestamp)
        .or_else(|| fallback.and_then(parse_timestamp))
}

fn normalized_time(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    provider_time(primary, fallback).map(timestamp)
}

pub(super) fn flight_is_in_progress(flight: &ApiFlight) -> bool {
    !flight.cancelled
        && (flight.actual_out.is_some() || flight.actual_off.is_some())
        && flight.actual_in.is_none()
        && flight.actual_on.is_none()
}

pub(super) fn cache_ttl(status: &FlightStatus, leg: &TravelLeg, now: DateTime<Utc>) -> Duration {
    if matches!(status.status.as_str(), "arrived" | "tracking_ended") {
        return Duration::hours(6);
    }
    if status.status == "en_route" {
        return Duration::minutes(2);
    }
    let until_departure = parse_timestamp(&leg.departure.utc)
        .map(|departure| departure - now)
        .unwrap_or_else(Duration::zero);
    if until_departure <= Duration::hours(6) {
        Duration::minutes(5)
    } else if until_departure <= Duration::hours(24) {
        Duration::minutes(15)
    } else {
        Duration::hours(1)
    }
}
