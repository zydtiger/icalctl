#[cfg(test)]
use serde::Serialize;
use serde::{Deserialize, Deserializer};
use std::time::Duration as StdDuration;

const BASE_URL: &str = "https://aeroapi.flightaware.com/aeroapi";
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProviderRequest {
    Flights {
        ident: String,
        start: String,
        end: String,
    },
    Position {
        provider_flight_id: String,
    },
}

pub(super) struct ProviderResponse {
    pub(super) status: u16,
    pub(super) retry_after: Option<u64>,
    pub(super) body: String,
}

pub(super) trait Transport {
    fn get(
        &self,
        request: ProviderRequest,
        api_key: &str,
    ) -> std::result::Result<ProviderResponse, ()>;
}

pub(super) struct UreqTransport {
    pub(super) agent: ureq::Agent,
}

impl UreqTransport {
    pub(super) fn new(timeout_seconds: u64) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(StdDuration::from_secs(timeout_seconds)))
            .https_only(true)
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(format!("icalctl/{}", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    fn response(
        &self,
        mut response: ureq::http::Response<ureq::Body>,
    ) -> std::result::Result<ProviderResponse, ()> {
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|_| ())?;
        Ok(ProviderResponse {
            status,
            retry_after,
            body,
        })
    }
}

impl Transport for UreqTransport {
    fn get(
        &self,
        request: ProviderRequest,
        api_key: &str,
    ) -> std::result::Result<ProviderResponse, ()> {
        let response = match request {
            ProviderRequest::Flights { ident, start, end } => self
                .agent
                .get(format!(
                    "{BASE_URL}/flights/{}",
                    encode_path_segment(&ident)
                ))
                .query("ident_type", "designator")
                .query("start", start)
                .query("end", end)
                .query("max_pages", "1")
                .header("accept", "application/json")
                .header("x-apikey", api_key)
                .call(),
            ProviderRequest::Position { provider_flight_id } => self
                .agent
                .get(format!(
                    "{BASE_URL}/flights/{}/position",
                    encode_path_segment(&provider_flight_id)
                ))
                .header("accept", "application/json")
                .header("x-apikey", api_key)
                .call(),
        }
        .map_err(|_| ())?;
        self.response(response)
    }
}

pub(super) fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
pub(super) struct FlightsResponse {
    pub(super) num_pages: u32,
    pub(super) flights: Vec<ApiFlight>,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
pub(super) struct ApiFlight {
    pub(super) ident: String,
    pub(super) ident_icao: Option<String>,
    pub(super) ident_iata: Option<String>,
    pub(super) fa_flight_id: String,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) codeshares: Option<Vec<String>>,
    pub(super) codeshares_iata: Option<Vec<String>>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) origin: Option<ApiAirport>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) destination: Option<ApiAirport>,
    pub(super) status: String,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) scheduled_out: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) estimated_out: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) actual_out: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) scheduled_off: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) estimated_off: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) actual_off: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) scheduled_on: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) estimated_on: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) actual_on: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) scheduled_in: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) estimated_in: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) actual_in: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) departure_delay: Option<i64>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) arrival_delay: Option<i64>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) terminal_origin: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) gate_origin: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) terminal_destination: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) gate_destination: Option<String>,
    pub(super) cancelled: bool,
    pub(super) diverted: bool,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
pub(super) struct ApiAirport {
    #[serde(deserialize_with = "required_nullable")]
    pub(super) code: Option<String>,
    pub(super) code_icao: Option<String>,
    pub(super) code_iata: Option<String>,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
pub(super) struct PositionResponse {
    #[serde(deserialize_with = "required_nullable")]
    pub(super) last_position: Option<ApiPosition>,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
pub(super) struct ApiPosition {
    pub(super) latitude: f64,
    pub(super) longitude: f64,
    pub(super) timestamp: String,
    pub(super) altitude: i64,
    pub(super) groundspeed: i64,
    #[serde(deserialize_with = "required_nullable")]
    pub(super) heading: Option<i64>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
