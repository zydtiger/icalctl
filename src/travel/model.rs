use serde::{Deserialize, Serialize};

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlightStatus {
    pub provider: String,
    pub provider_flight_id: String,
    pub status: String,
    pub description: String,
    pub scheduled_departure: Option<String>,
    pub estimated_departure: Option<String>,
    pub actual_departure: Option<String>,
    pub scheduled_arrival: Option<String>,
    pub estimated_arrival: Option<String>,
    pub actual_arrival: Option<String>,
    pub departure_delay_seconds: Option<i64>,
    pub arrival_delay_seconds: Option<i64>,
    pub departure_terminal: Option<String>,
    pub departure_gate: Option<String>,
    pub arrival_terminal: Option<String>,
    pub arrival_gate: Option<String>,
    pub tracking_ended: bool,
    pub diverted: bool,
    pub current_position: Option<FlightPosition>,
    pub freshness: FlightFreshness,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlightFreshness {
    pub state: String,
    pub fetched_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlightPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: String,
    pub altitude_feet: Option<i64>,
    pub groundspeed_knots: Option<i64>,
    pub heading_degrees: Option<i64>,
}
