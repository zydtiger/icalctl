mod airports;
mod collection;
mod flight;
mod model;
mod parser;
pub mod server;
mod service;
#[cfg(test)]
mod tests;

pub(crate) use collection::collect_travel_events;
pub(crate) use flight::{FlightInput, format_flight};
pub(crate) use model::{
    AirportMetadata, FlightFreshness, FlightPosition, FlightStatus, TravelAirport,
    TravelCollection, TravelEventSource, TravelLeg, TravelMoment, TravelWarning,
};
