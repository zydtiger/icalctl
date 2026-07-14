use super::*;
use crate::travel::{AirportMetadata, TravelAirport, TravelEventSource, TravelMoment};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "icalctl-flightaware-test-{}-{counter}",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct FakeTransport {
    responses: RefCell<VecDeque<std::result::Result<ProviderResponse, ()>>>,
    requests: RefCell<Vec<ProviderRequest>>,
    api_keys: RefCell<Vec<String>>,
}

impl FakeTransport {
    fn new(responses: Vec<std::result::Result<ProviderResponse, ()>>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            requests: RefCell::new(Vec::new()),
            api_keys: RefCell::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.borrow().len()
    }
}

impl Transport for FakeTransport {
    fn get(
        &self,
        request: ProviderRequest,
        api_key: &str,
    ) -> std::result::Result<ProviderResponse, ()> {
        self.requests.borrow_mut().push(request);
        self.api_keys.borrow_mut().push(api_key.to_string());
        self.responses.borrow_mut().pop_front().unwrap_or(Err(()))
    }
}

fn now() -> DateTime<Utc> {
    parse_timestamp("2026-07-11T00:00:00Z").unwrap()
}

fn config(limit: u32, stale_if_error: bool) -> FlightAwareConfig {
    FlightAwareConfig {
        api_key: Some("TEST-API-KEY".to_string()),
        enabled: true,
        monthly_result_set_limit: limit,
        request_timeout_seconds: 10,
        stale_if_error,
    }
}

fn airport(code: &str, iata: &str, icao: &str) -> TravelAirport {
    TravelAirport {
        code: code.to_string(),
        metadata: Some(AirportMetadata {
            iata_code: iata.to_string(),
            icao_code: Some(icao.to_string()),
            name: format!("{iata} Airport"),
            municipality: None,
            latitude: 0.0,
            longitude: 0.0,
        }),
    }
}

fn leg(event_id: &str, flight_number: &str, departure_utc: &str, arrival_utc: &str) -> TravelLeg {
    TravelLeg {
        flight_number: flight_number.to_string(),
        route: "PVG to HEL".to_string(),
        departure_airport: airport("PVG", "PVG", "ZSPD"),
        arrival_airport: airport("HEL", "HEL", "EFHK"),
        departure: TravelMoment {
            scheduled: departure_utc.to_string(),
            utc: departure_utc.to_string(),
        },
        arrival: TravelMoment {
            scheduled: arrival_utc.to_string(),
            utc: arrival_utc.to_string(),
        },
        live_status: None,
        source: TravelEventSource {
            event_id: event_id.to_string(),
            occurrence_date: None,
            title: format!("Flight {flight_number}: PVG to HEL"),
            calendar: Some("Travel".to_string()),
            calendar_id: Some("calendar-id".to_string()),
        },
    }
}

fn collection(legs: Vec<TravelLeg>) -> TravelCollection {
    TravelCollection {
        legs,
        warnings: Vec::new(),
    }
}

fn api_airport(code: &str, iata: &str, icao: &str) -> ApiAirport {
    ApiAirport {
        code: Some(code.to_string()),
        code_icao: Some(icao.to_string()),
        code_iata: Some(iata.to_string()),
    }
}

fn api_flight(
    provider_flight_id: &str,
    ident: &str,
    codeshares: &[&str],
    origin: (&str, &str, &str),
    destination: (&str, &str, &str),
    scheduled_out: &str,
) -> ApiFlight {
    ApiFlight {
        ident: ident.to_string(),
        ident_icao: Some(ident.to_string()),
        ident_iata: None,
        fa_flight_id: provider_flight_id.to_string(),
        codeshares: Some(codeshares.iter().map(|value| value.to_string()).collect()),
        codeshares_iata: None,
        origin: Some(api_airport(origin.0, origin.1, origin.2)),
        destination: Some(api_airport(destination.0, destination.1, destination.2)),
        status: "Scheduled".to_string(),
        scheduled_out: Some(scheduled_out.to_string()),
        estimated_out: None,
        actual_out: None,
        scheduled_off: None,
        estimated_off: None,
        actual_off: None,
        scheduled_on: None,
        estimated_on: None,
        actual_on: None,
        scheduled_in: Some("2026-07-11T11:00:00Z".to_string()),
        estimated_in: None,
        actual_in: None,
        departure_delay: None,
        arrival_delay: None,
        terminal_origin: Some("2".to_string()),
        gate_origin: Some("D71".to_string()),
        terminal_destination: Some("2".to_string()),
        gate_destination: Some("32".to_string()),
        cancelled: false,
        diverted: false,
    }
}

fn flights_response(flights: Vec<ApiFlight>) -> ProviderResponse {
    ProviderResponse {
        status: 200,
        retry_after: None,
        body: serde_json::to_string(&FlightsResponse {
            num_pages: 1,
            flights,
        })
        .unwrap(),
    }
}

fn scheduled_flight() -> ApiFlight {
    api_flight(
        "FA-HO1607",
        "DKH1607",
        &["HO1607"],
        ("ZSPD", "PVG", "ZSPD"),
        ("EFHK", "HEL", "EFHK"),
        "2026-07-11T01:25:00Z",
    )
}

fn in_progress_flight() -> ApiFlight {
    let mut flight = scheduled_flight();
    flight.status = "En Route".to_string();
    flight.actual_off = Some("2026-07-11T01:40:00Z".to_string());
    flight
}

#[test]
fn matching_requires_ident_route_and_nearest_departure() {
    let travel_leg = leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    );
    let wrong_route = api_flight(
        "FA-WRONG-ROUTE",
        "HO1607",
        &[],
        ("ZSPD", "PVG", "ZSPD"),
        ("EDDF", "FRA", "EDDF"),
        "2026-07-11T01:25:00Z",
    );
    let wrong_time = api_flight(
        "FA-WRONG-TIME",
        "HO1607",
        &[],
        ("ZSPD", "PVG", "ZSPD"),
        ("EFHK", "HEL", "EFHK"),
        "2026-07-12T01:25:00Z",
    );
    let codeshare_match = scheduled_flight();

    let flights = [wrong_route, wrong_time, codeshare_match];
    let selection = select_match(&flights, &travel_leg);

    assert!(matches!(
        selection,
        MatchSelection::Matched(flight) if flight.fa_flight_id == "FA-HO1607"
    ));
}

#[test]
fn provider_queries_compact_spaced_flight_numbers() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
    let mut travel = collection(vec![leg(
        "event-1",
        "HO 1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(travel.legs[0].flight_number, "HO 1607");
    assert_eq!(transport.request_count(), 1);
    match &transport.requests.borrow()[0] {
        ProviderRequest::Flights { ident, .. } => assert_eq!(ident, "HO1607"),
        ProviderRequest::Position { .. } => panic!("expected a flight summary request"),
    }
    assert_eq!(
        travel.legs[0].live_status.as_ref().unwrap().status,
        "scheduled"
    );
}

#[test]
fn equally_scored_provider_results_are_ambiguous() {
    let travel_leg = leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    );
    let first = api_flight(
        "FA-1",
        "HO1607",
        &[],
        ("ZSPD", "PVG", "ZSPD"),
        ("EFHK", "HEL", "EFHK"),
        "2026-07-11T01:25:00Z",
    );
    let second = api_flight(
        "FA-2",
        "HO1607",
        &[],
        ("ZSPD", "PVG", "ZSPD"),
        ("EFHK", "HEL", "EFHK"),
        "2026-07-11T01:25:00Z",
    );

    assert!(matches!(
        select_match(&[second, first], &travel_leg),
        MatchSelection::Ambiguous
    ));
}

#[test]
fn sole_candidates_outside_four_hour_tolerance_are_rejected() {
    let travel_leg = leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    );
    let too_far = api_flight(
        "FA-TOO-FAR",
        "HO1607",
        &[],
        ("ZSPD", "PVG", "ZSPD"),
        ("EFHK", "HEL", "EFHK"),
        "2026-07-11T06:25:01Z",
    );
    assert!(matches!(
        select_match(&[too_far], &travel_leg),
        MatchSelection::NoMatch
    ));

    let within_tolerance = api_flight(
        "FA-WITHIN-TOLERANCE",
        "HO1607",
        &[],
        ("ZSPD", "PVG", "ZSPD"),
        ("EFHK", "HEL", "EFHK"),
        "2026-07-11T05:25:00Z",
    );
    assert!(matches!(
        select_match(&[within_tolerance], &travel_leg),
        MatchSelection::Matched(_)
    ));
}

#[test]
fn nullable_codeshares_are_accepted_from_the_official_schema() {
    let mut value = serde_json::to_value(FlightsResponse {
        num_pages: 1,
        flights: vec![scheduled_flight()],
    })
    .unwrap();
    value["flights"][0]["codeshares"] = serde_json::Value::Null;
    value["flights"][0]["codeshares_iata"] = serde_json::Value::Null;

    let parsed: FlightsResponse = serde_json::from_value(value).unwrap();

    assert!(parsed.flights[0].codeshares.is_none());
    assert!(parsed.flights[0].codeshares_iata.is_none());
}

#[test]
fn required_provider_response_fields_do_not_default_silently() {
    assert!(serde_json::from_str::<FlightsResponse>("{}").is_err());
    let valid = serde_json::to_value(FlightsResponse {
        num_pages: 1,
        flights: vec![scheduled_flight()],
    })
    .unwrap();
    for required in [
        "status",
        "cancelled",
        "diverted",
        "codeshares",
        "origin",
        "scheduled_out",
        "actual_in",
        "departure_delay",
        "gate_origin",
    ] {
        let mut missing = valid.clone();
        missing["flights"][0]
            .as_object_mut()
            .unwrap()
            .remove(required);
        assert!(
            serde_json::from_value::<FlightsResponse>(missing).is_err(),
            "missing required field {required} was accepted"
        );
    }
    for airport_field in ["origin", "destination"] {
        let mut nullable = valid.clone();
        nullable["flights"][0][airport_field]["code"] = serde_json::Value::Null;
        assert!(
            serde_json::from_value::<FlightsResponse>(nullable).is_ok(),
            "nullable {airport_field}.code was rejected"
        );

        let mut missing = valid.clone();
        missing["flights"][0][airport_field]
            .as_object_mut()
            .unwrap()
            .remove("code");
        assert!(
            serde_json::from_value::<FlightsResponse>(missing).is_err(),
            "missing required field {airport_field}.code was accepted"
        );
    }
    assert!(serde_json::from_str::<PositionResponse>("{}").is_err());
    assert!(
        serde_json::from_value::<PositionResponse>(serde_json::json!({
            "last_position": {
                "latitude": 55.5,
                "longitude": 42.25,
                "timestamp": "2026-07-11T02:15:00Z",
                "heading": null
            }
        }))
        .is_err()
    );

    let out_of_range = ApiPosition {
        latitude: 55.5,
        longitude: 42.25,
        timestamp: "2026-07-11T02:15:00Z".to_string(),
        altitude: 330,
        groundspeed: 455,
        heading: Some(361),
    };
    assert!(normalize_position(out_of_range).is_none());
}

#[test]
fn authenticated_transport_never_follows_redirects() {
    let transport = UreqTransport::new(10);

    assert_eq!(transport.agent.config().max_redirects(), 0);
}

#[test]
fn oversized_retry_after_is_capped_without_panicking() {
    let directory = TestDirectory::new();
    let mut usage = UsageState::new(now());

    record_failure(directory.path(), &mut usage, now(), 429, Some(u64::MAX)).unwrap();

    assert_eq!(
        parse_timestamp(usage.backoff_until.as_deref().unwrap()).unwrap() - now(),
        Duration::hours(24)
    );
}

#[test]
fn fresh_normalized_cache_prevents_repeat_provider_calls() {
    let directory = TestDirectory::new();
    let first_transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
    let mut first = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut first,
        &first_transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );
    assert_eq!(first_transport.request_count(), 1);
    assert_eq!(
        first.legs[0].live_status.as_ref().unwrap().status,
        "scheduled"
    );

    let cached_transport = FakeTransport::new(Vec::new());
    let mut cached = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);
    enrich_with(
        &config(10, true),
        &mut cached,
        &cached_transport,
        directory.path(),
        now() + Duration::minutes(1),
        "TEST-API-KEY",
    );

    assert_eq!(cached_transport.request_count(), 0);
    assert_eq!(
        cached.legs[0].live_status.as_ref().unwrap().freshness.state,
        "fresh"
    );
}

#[test]
fn monthly_limit_blocks_additional_result_sets() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
    let mut travel = collection(vec![
        leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        ),
        leg(
            "event-2",
            "HO1608",
            "2026-07-11T03:25:00Z",
            "2026-07-11T13:00:00Z",
        ),
    ]);

    enrich_with(
        &config(1, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 1);
    assert!(
        travel
            .warnings
            .iter()
            .any(|warning| warning.kind == "flightaware_monthly_limit")
    );
    assert_eq!(read_usage(directory.path(), now()).unwrap().result_sets, 1);
}

#[test]
fn provider_backoff_prevents_follow_on_calls() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::new(vec![Ok(ProviderResponse {
        status: 429,
        retry_after: Some(600),
        body: String::new(),
    })]);
    let mut travel = collection(vec![
        leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        ),
        leg(
            "event-2",
            "HO1608",
            "2026-07-11T03:25:00Z",
            "2026-07-11T13:00:00Z",
        ),
    ]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 1);
    assert!(
        travel
            .warnings
            .iter()
            .any(|warning| warning.kind == "flightaware_backoff")
    );
    assert!(
        read_usage(directory.path(), now())
            .unwrap()
            .backoff_is_active(now())
    );
}

#[test]
fn backoff_persistence_errors_are_explicit_and_fail_closed_in_memory() {
    let directory = TestDirectory::new();
    fs::create_dir_all(directory.path().join("usage.json")).unwrap();
    let mut usage = UsageState::new(now());
    let mut warnings = Vec::new();

    record_failure_or_warn(
        directory.path(),
        &mut usage,
        now(),
        503,
        None,
        &mut warnings,
        Some("event-1".to_string()),
    );

    assert!(usage.backoff_is_active(now()));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].kind, "flightaware_usage_unavailable");
    assert!(read_usage(directory.path(), now()).is_err());
}

#[test]
fn expired_cache_is_used_only_when_stale_fallback_is_enabled() {
    let directory = TestDirectory::new();
    let travel_leg = leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    );
    let key = CacheKey::from_leg(&travel_leg).unwrap();
    let mut status = normalize_status(&scheduled_flight(), now() - Duration::hours(2));
    status.freshness.expires_at = timestamp(now() - Duration::hours(1));
    let entry = CacheEntry {
        version: CACHE_VERSION,
        key: key.clone(),
        fetched_at: timestamp(now() - Duration::hours(2)),
        expires_at: timestamp(now() - Duration::hours(1)),
        outcome: "matched".to_string(),
        status: Some(status),
    };
    write_cache_entry(&cache_path(directory.path(), &key), &entry).unwrap();

    let transport = FakeTransport::new(vec![Err(())]);
    let mut with_stale = collection(vec![travel_leg]);
    enrich_with(
        &config(10, true),
        &mut with_stale,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(
        with_stale.legs[0]
            .live_status
            .as_ref()
            .unwrap()
            .freshness
            .state,
        "stale"
    );
    assert!(
        with_stale
            .warnings
            .iter()
            .any(|warning| warning.kind == "flightaware_stale_cache")
    );

    let later = now() + Duration::hours(2);
    let mut without_stale = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);
    let transport = FakeTransport::new(vec![Err(())]);
    let separate_directory = TestDirectory::new();
    let key = CacheKey::from_leg(&without_stale.legs[0]).unwrap();
    write_cache_entry(
        &cache_path(separate_directory.path(), &key),
        &CacheEntry {
            version: CACHE_VERSION,
            key,
            fetched_at: timestamp(later - Duration::hours(2)),
            expires_at: timestamp(later - Duration::hours(1)),
            outcome: "matched".to_string(),
            status: Some(normalize_status(
                &scheduled_flight(),
                later - Duration::hours(2),
            )),
        },
    )
    .unwrap();
    enrich_with(
        &config(10, false),
        &mut without_stale,
        &transport,
        separate_directory.path(),
        later,
        "TEST-API-KEY",
    );
    assert!(without_stale.legs[0].live_status.is_none());
}

#[test]
fn in_progress_match_fetches_and_normalizes_current_position() {
    let directory = TestDirectory::new();
    let position = PositionResponse {
        last_position: Some(ApiPosition {
            latitude: 55.5,
            longitude: 42.25,
            timestamp: "2026-07-11T02:15:00Z".to_string(),
            altitude: 330,
            groundspeed: 455,
            heading: Some(310),
        }),
    };
    let transport = FakeTransport::new(vec![
        Ok(flights_response(vec![in_progress_flight()])),
        Ok(ProviderResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_string(&position).unwrap(),
        }),
    ]);
    let mut travel = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 2);
    let status = travel.legs[0].live_status.as_ref().unwrap();
    assert_eq!(status.status, "en_route");
    let position = status.current_position.as_ref().unwrap();
    assert_eq!(position.altitude_feet, Some(33_000));
    assert_eq!(position.groundspeed_knots, Some(455));
}

#[test]
fn position_failures_accumulate_backoff_across_summary_successes() {
    let directory = TestDirectory::new();
    let mut first = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);
    let first_transport = FakeTransport::new(vec![
        Ok(flights_response(vec![in_progress_flight()])),
        Err(()),
    ]);
    enrich_with(
        &config(10, true),
        &mut first,
        &first_transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );
    assert_eq!(first_transport.request_count(), 2);
    assert_eq!(
        read_usage(directory.path(), now()).unwrap().failure_count,
        1
    );

    let second_now = now() + Duration::minutes(2) + Duration::seconds(1);
    let mut second = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);
    let second_transport = FakeTransport::new(vec![
        Ok(flights_response(vec![in_progress_flight()])),
        Err(()),
    ]);
    enrich_with(
        &config(10, true),
        &mut second,
        &second_transport,
        directory.path(),
        second_now,
        "TEST-API-KEY",
    );

    assert_eq!(second_transport.request_count(), 2);
    let usage = read_usage(directory.path(), second_now).unwrap();
    assert_eq!(usage.failure_count, 2);
    assert!(usage.backoff_is_active(second_now));
}

#[test]
fn invalid_position_payload_enters_provider_backoff() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::new(vec![
        Ok(flights_response(vec![in_progress_flight()])),
        Ok(ProviderResponse {
            status: 200,
            retry_after: None,
            body: "not-json".to_string(),
        }),
    ]);
    let mut travel = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert!(
        read_usage(directory.path(), now())
            .unwrap()
            .backoff_is_active(now())
    );
    assert!(
        travel
            .warnings
            .iter()
            .any(|warning| warning.kind == "flightaware_invalid_position")
    );
}

#[test]
fn provider_tracking_end_flag_is_not_presented_as_confirmed_cancellation() {
    let mut tracking_ended = scheduled_flight();
    tracking_ended.cancelled = true;
    tracking_ended.status = "No longer tracked".to_string();
    let normalized = normalize_status(&tracking_ended, now());
    assert_eq!(normalized.status, "tracking_ended");
    assert!(normalized.tracking_ended);

    let mut arrived = tracking_ended;
    arrived.actual_in = Some("2026-07-11T11:02:00Z".to_string());
    assert_eq!(normalize_status(&arrived, now()).status, "arrived");

    let mut diverted = arrived;
    diverted.diverted = true;
    assert_eq!(normalize_status(&diverted, now()).status, "diverted");
}

#[test]
fn provider_key_never_enters_output_warnings_or_cache_files() {
    let directory = TestDirectory::new();
    let secret = "SUPER-SECRET-FLIGHTAWARE-KEY";
    let transport = FakeTransport::new(vec![Ok(ProviderResponse {
        status: 401,
        retry_after: None,
        body: format!("provider echoed {secret}"),
    })]);
    let mut travel = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        secret,
    );

    assert_eq!(transport.api_keys.borrow().as_slice(), [secret]);
    assert!(!serde_json::to_string(&travel).unwrap().contains(secret));
    for entry in fs::read_dir(directory.path()).unwrap() {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(secret));
    }
}

#[test]
fn cache_files_are_owner_only_on_unix() {
    let directory = TestDirectory::new();
    let path = directory.path().join("entry.json");
    write_json_atomically(&path, &UsageState::new(now())).unwrap();

    #[cfg(unix)]
    assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
}

#[cfg(unix)]
#[test]
fn a_busy_usage_ledger_fails_closed_without_provider_calls() {
    let directory = TestDirectory::new();
    let _lock = acquire_usage_lock(directory.path()).unwrap();
    let transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
    let mut travel = collection(vec![leg(
        "event-1",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 0);
    assert!(
        travel
            .warnings
            .iter()
            .any(|warning| warning.kind == "flightaware_usage_unavailable")
    );
}

#[test]
fn far_future_flights_do_not_consume_provider_quota() {
    let travel_leg = leg(
        "event-future",
        "HO1607",
        "2026-07-20T01:25:00Z",
        "2026-07-20T11:00:00Z",
    );

    assert!(query_window(&travel_leg, now()).is_none());
}

#[test]
fn far_future_flights_stay_calendar_only_without_warnings() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::new(Vec::new());
    let mut travel = collection(vec![leg(
        "event-future",
        "FI342",
        "2026-07-20T01:25:00Z",
        "2026-07-20T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 0);
    assert!(travel.legs[0].live_status.is_none());
    assert!(travel.warnings.is_empty());
}

#[test]
fn far_future_only_collection_skips_unconfigured_provider_warning() {
    let departure = Utc::now() + Duration::days(30);
    let arrival = departure + Duration::hours(9);
    let mut travel = collection(vec![leg(
        "event-future",
        "FI342",
        &timestamp(departure),
        &timestamp(arrival),
    )]);
    let mut provider_config = config(10, true);
    provider_config.api_key = None;

    enrich_collection(&provider_config, &mut travel);

    assert!(travel.legs[0].live_status.is_none());
    assert!(travel.warnings.is_empty());
}

#[cfg(unix)]
#[test]
fn far_future_only_collection_ignores_busy_usage_ledger() {
    let directory = TestDirectory::new();
    let _lock = acquire_usage_lock(directory.path()).unwrap();
    let transport = FakeTransport::new(Vec::new());
    let mut travel = collection(vec![leg(
        "event-future",
        "FI342",
        "2026-07-20T01:25:00Z",
        "2026-07-20T11:00:00Z",
    )]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 0);
    assert!(travel.legs[0].live_status.is_none());
    assert!(travel.warnings.is_empty());
}

#[test]
fn mixed_collection_skips_future_positive_negative_and_stale_cache_entries() {
    let directory = TestDirectory::new();
    let positive_leg = leg(
        "event-positive",
        "FI342",
        "2026-07-20T01:25:00Z",
        "2026-07-20T11:00:00Z",
    );
    let positive_key = CacheKey::from_leg(&positive_leg).unwrap();
    write_cache_entry(
        &cache_path(directory.path(), &positive_key),
        &CacheEntry {
            version: CACHE_VERSION,
            key: positive_key,
            fetched_at: timestamp(now()),
            expires_at: timestamp(now() + Duration::hours(1)),
            outcome: "matched".to_string(),
            status: Some(normalize_status(&scheduled_flight(), now())),
        },
    )
    .unwrap();

    let negative_leg = leg(
        "event-negative",
        "HO1608",
        "2026-07-20T03:25:00Z",
        "2026-07-20T13:00:00Z",
    );
    let negative_key = CacheKey::from_leg(&negative_leg).unwrap();
    write_cache_entry(
        &cache_path(directory.path(), &negative_key),
        &CacheEntry::negative(negative_key, now(), "no_match"),
    )
    .unwrap();

    let stale_leg = leg(
        "event-stale",
        "DY627",
        "2026-07-20T05:25:00Z",
        "2026-07-20T07:00:00Z",
    );
    let stale_key = CacheKey::from_leg(&stale_leg).unwrap();
    write_cache_entry(
        &cache_path(directory.path(), &stale_key),
        &CacheEntry {
            version: CACHE_VERSION,
            key: stale_key,
            fetched_at: timestamp(now() - Duration::hours(2)),
            expires_at: timestamp(now() - Duration::hours(1)),
            outcome: "matched".to_string(),
            status: Some(normalize_status(
                &scheduled_flight(),
                now() - Duration::hours(2),
            )),
        },
    )
    .unwrap();

    let transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
    let current_leg = leg(
        "event-current",
        "HO1607",
        "2026-07-11T01:25:00Z",
        "2026-07-11T11:00:00Z",
    );
    let mut travel = collection(vec![current_leg, positive_leg, negative_leg, stale_leg]);
    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 1);
    assert!(travel.legs[0].live_status.is_some());
    assert!(travel.legs[1..].iter().all(|leg| leg.live_status.is_none()));
    assert!(travel.warnings.is_empty());
}

#[test]
fn far_future_flights_stay_silent_during_provider_backoff() {
    let directory = TestDirectory::new();
    let transport = FakeTransport::new(vec![Ok(ProviderResponse {
        status: 429,
        retry_after: Some(600),
        body: String::new(),
    })]);
    let mut travel = collection(vec![
        leg(
            "event-current",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        ),
        leg(
            "event-future",
            "FI342",
            "2026-07-20T01:25:00Z",
            "2026-07-20T11:00:00Z",
        ),
    ]);

    enrich_with(
        &config(10, true),
        &mut travel,
        &transport,
        directory.path(),
        now(),
        "TEST-API-KEY",
    );

    assert_eq!(transport.request_count(), 1);
    assert!(travel.legs[1].live_status.is_none());
    assert_eq!(travel.warnings.len(), 1);
    assert_eq!(travel.warnings[0].kind, "flightaware_request_failed");
}

#[test]
fn path_segments_are_percent_encoded() {
    assert_eq!(encode_path_segment("HO 16/07"), "HO%2016%2F07");
}
