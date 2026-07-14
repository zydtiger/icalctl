use super::api::*;
use super::assets::*;
use super::range::*;
use super::routing::*;
use super::security::*;
use super::*;
use crate::cli::ReadCalendarSelectorArgs;
use crate::config::{Config, MAX_TRAVEL_RANGE_DAYS};
use crate::models::EventReport;
use crate::travel::{FlightInput, format_flight};
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Timelike, Utc};
use std::cell::{Cell, RefCell};
use tiny_http::Method;

fn test_config() -> Config {
    let mut config = Config::default();
    config.flightaware.enabled = false;
    config
}

fn test_security() -> ServerSecurity {
    ServerSecurity {
        host: "127.0.0.1:8123".to_string(),
        origin: "http://127.0.0.1:8123".to_string(),
        token: "0123456789abcdef".to_string(),
    }
}

fn authorized_metadata(security: &ServerSecurity) -> RequestMetadata {
    RequestMetadata {
        hosts: vec![security.host.clone()],
        origins: Vec::new(),
        fetch_sites: vec!["same-origin".to_string()],
        cookies: vec![format!(
            "theme=dark; {CAPABILITY_COOKIE}={}",
            security.token
        )],
    }
}

fn no_events(
    _: DateTime<Local>,
    _: DateTime<Local>,
    _: &ReadCalendarSelectorArgs,
) -> Result<Vec<EventReport>> {
    Ok(Vec::new())
}

fn canonical_flight_event() -> EventReport {
    let departure = "2026-07-11T09:25:00+08:00";
    let arrival = "2026-07-11T14:00:00+03:00";
    let formatted = format_flight(FlightInput {
        flight_number: "HO1607",
        from_airport: "PVG",
        to_airport: "HEL",
        departure,
        arrival,
        extra_notes: Some("private trip note"),
    })
    .unwrap();
    let departure_datetime = DateTime::parse_from_rfc3339(departure).unwrap();
    let arrival_datetime = DateTime::parse_from_rfc3339(arrival).unwrap();
    EventReport {
        id: "EVENT-1".to_string(),
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
        calendar_id: Some("CAL-1".to_string()),
        calendar_source: Some("iCloud".to_string()),
        calendar_source_id: Some("SOURCE-1".to_string()),
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
        is_detached: false,
        occurrence_date: None,
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
fn api_range_is_inclusive_and_eventkit_end_is_next_midnight() {
    let captured = RefCell::new(None);
    let fetch =
        |start: DateTime<Local>, end: DateTime<Local>, selector: &ReadCalendarSelectorArgs| {
            captured
                .borrow_mut()
                .replace((start, end, selector.calendar_ids.clone()));
            Ok(Vec::new())
        };
    let response = route_request(
        &Method::Get,
        "/api/travel?start=2026-07-10&end=2026-07-12",
        &test_config(),
        &["CAL-1".to_string()],
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        DateTime::parse_from_rfc3339("2026-07-01T12:00:00Z")
            .unwrap()
            .to_utc(),
        &fetch,
    );

    assert_eq!(response.status, 200);
    let (start, end, calendar_ids) = captured.borrow_mut().take().unwrap();
    assert_eq!(
        start.date_naive(),
        NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
    );
    assert_eq!(start.hour(), 0);
    assert_eq!(
        end.date_naive(),
        NaiveDate::from_ymd_opt(2026, 7, 13).unwrap()
    );
    assert_eq!(end.hour(), 0);
    assert_eq!(calendar_ids, ["CAL-1"]);
    let json: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(json["range"]["start"], "2026-07-10");
    assert_eq!(json["range"]["end"], "2026-07-12");
    assert_eq!(json["range"]["end_inclusive"], true);
    assert_eq!(json["schema_version"], API_SCHEMA_VERSION);
}

#[test]
fn default_range_excludes_events_that_ended_before_now() {
    let local_now = Local
        .with_ymd_and_hms(2026, 7, 12, 15, 30, 0)
        .single()
        .unwrap();
    let captured_start = RefCell::new(None);
    let fetch = |start: DateTime<Local>, _: DateTime<Local>, _: &ReadCalendarSelectorArgs| {
        captured_start.borrow_mut().replace(start);
        Ok(Vec::new())
    };
    let response = route_request(
        &Method::Get,
        "/api/travel",
        &test_config(),
        &[],
        local_now.date_naive(),
        local_now.with_timezone(&Utc),
        &fetch,
    );

    assert_eq!(response.status, 200);
    assert_eq!(captured_start.borrow_mut().take().unwrap(), local_now);
}

#[test]
fn empty_query_uses_the_configured_number_of_inclusive_days() {
    let range = parse_range(
        "/api/travel",
        NaiveDate::from_ymd_opt(2026, 7, 12).unwrap(),
        90,
    )
    .unwrap();
    assert_eq!(range.start, NaiveDate::from_ymd_opt(2026, 7, 12).unwrap());
    assert_eq!(range.end, NaiveDate::from_ymd_opt(2026, 10, 9).unwrap());
}

#[test]
fn api_serializes_only_the_canonical_travel_model() {
    let event = RefCell::new(Some(canonical_flight_event()));
    let fetch = |_: DateTime<Local>, _: DateTime<Local>, _: &ReadCalendarSelectorArgs| {
        Ok(vec![event.borrow_mut().take().unwrap()])
    };
    let response = route_request(
        &Method::Get,
        "/api/travel?start=2026-07-11&end=2026-07-11",
        &test_config(),
        &["CAL-1".to_string()],
        NaiveDate::from_ymd_opt(2026, 7, 11).unwrap(),
        Utc::now(),
        &fetch,
    );

    assert_eq!(response.status, 200);
    assert!(!response.body.contains("private trip note"));
    let json: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(json["legs"][0]["flight_number"], "HO1607");
    assert_eq!(json["legs"][0]["route"], "PVG to HEL");
    assert_eq!(json["legs"][0]["source"]["event_id"], "EVENT-1");
    assert_eq!(json["legs"][0]["live_status"], serde_json::Value::Null);
    assert_eq!(json["warnings"], serde_json::json!([]));
    assert_eq!(json["map"]["projection"], "globe");
    assert_eq!(
        json["map"]["style_url"],
        "https://tiles.openfreemap.org/styles/bright"
    );
}

#[test]
fn range_query_is_strict_and_requires_a_complete_pair() {
    let today = NaiveDate::from_ymd_opt(2026, 7, 12).unwrap();
    for url in [
        "/api/travel?start=2026-07-12",
        "/api/travel?start=2026-07-12&end=2026-07-11",
        "/api/travel?start=2026-7-12&end=2026-07-12",
        "/api/travel?start=2026-07-12&start=2026-07-13&end=2026-07-14",
        "/api/travel?from=2026-07-12&end=2026-07-14",
    ] {
        assert!(parse_range(url, today, 90).is_err(), "accepted {url}");
    }
}

#[test]
fn explicit_range_enforces_the_inclusive_maximum() {
    let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let maximum = parse_range("/api/travel?start=2026-01-01&end=2027-01-01", today, 90).unwrap();
    assert_eq!(
        maximum.end.signed_duration_since(maximum.start).num_days() + 1,
        i64::from(MAX_TRAVEL_RANGE_DAYS)
    );
    assert!(parse_range("/api/travel?start=2026-01-01&end=2027-01-02", today, 90).is_err());
    assert!(parse_range("/api/travel", today, MAX_TRAVEL_RANGE_DAYS + 1).is_err());
}

#[test]
fn calendar_ids_are_exact_deduplicated_and_cli_values_override_config() {
    let mut config = test_config();
    config.travel.calendar_ids = vec!["CONFIG-1".to_string(), "CONFIG-1".to_string()];
    assert_eq!(
        effective_calendar_ids(Vec::new(), &config).unwrap(),
        ["CONFIG-1"]
    );
    assert_eq!(
        effective_calendar_ids(
            vec![
                "CLI-1".to_string(),
                "CLI-1".to_string(),
                "CLI-2".to_string()
            ],
            &config
        )
        .unwrap(),
        ["CLI-1", "CLI-2"]
    );
    assert!(effective_calendar_ids(vec!["".to_string()], &config).is_err());
}

#[test]
fn head_and_rejected_methods_never_read_calendar_or_provider_data() {
    let calls = Cell::new(0);
    let fetch = |_: DateTime<Local>, _: DateTime<Local>, _: &ReadCalendarSelectorArgs| {
        calls.set(calls.get() + 1);
        Ok(Vec::new())
    };
    let today = NaiveDate::from_ymd_opt(2026, 7, 12).unwrap();
    let now = Utc::now();
    let head = route_request(
        &Method::Head,
        "/api/travel",
        &test_config(),
        &[],
        today,
        now,
        &fetch,
    );
    let post = route_request(
        &Method::Post,
        "/api/travel",
        &test_config(),
        &[],
        today,
        now,
        &fetch,
    );

    assert_eq!(head.status, 200);
    assert_eq!(post.status, 405);
    assert_eq!(calls.get(), 0);
    assert!(
        post.extra_headers
            .contains(&("Allow".to_string(), "GET, HEAD".to_string()))
    );

    let invalid_head = route_request(
        &Method::Head,
        "/api/travel?start=2026-07-12",
        &test_config(),
        &[],
        today,
        now,
        &fetch,
    );
    assert_eq!(invalid_head.status, 400);
    assert_eq!(calls.get(), 0);
}

#[test]
fn capability_and_request_headers_block_cross_site_reads() {
    let security = test_security();
    let calls = Cell::new(0);
    let fetch = |_: DateTime<Local>, _: DateTime<Local>, _: &ReadCalendarSelectorArgs| {
        calls.set(calls.get() + 1);
        Ok(Vec::new())
    };
    let today = NaiveDate::from_ymd_opt(2026, 7, 12).unwrap();
    let now = Utc::now();
    let hostile_requests = [
        RequestMetadata {
            hosts: vec!["attacker.example".to_string()],
            ..authorized_metadata(&security)
        },
        RequestMetadata {
            origins: vec!["https://attacker.example".to_string()],
            ..authorized_metadata(&security)
        },
        RequestMetadata {
            fetch_sites: vec!["cross-site".to_string()],
            ..authorized_metadata(&security)
        },
        RequestMetadata {
            cookies: Vec::new(),
            ..authorized_metadata(&security)
        },
    ];
    for metadata in hostile_requests {
        let response = dispatch_request(
            &Method::Get,
            "/api/travel",
            &metadata,
            &security,
            &test_config(),
            &[],
            today,
            now,
            &fetch,
        );
        assert_eq!(response.status, 403);
    }
    assert_eq!(calls.get(), 0);

    let launch = dispatch_request(
        &Method::Get,
        "/?token=0123456789abcdef",
        &RequestMetadata {
            hosts: vec![security.host.clone()],
            fetch_sites: vec!["none".to_string()],
            ..RequestMetadata::default()
        },
        &security,
        &test_config(),
        &[],
        today,
        now,
        &fetch,
    );
    assert_eq!(launch.status, 303);
    assert!(
        launch
            .extra_headers
            .iter()
            .any(|(name, value)| name == "Set-Cookie"
                && value.contains("HttpOnly")
                && value.contains("SameSite=Strict"))
    );
    assert_eq!(calls.get(), 0);

    let authorized = dispatch_request(
        &Method::Get,
        "/api/travel?start=2026-07-12&end=2026-07-12",
        &authorized_metadata(&security),
        &security,
        &test_config(),
        &[],
        today,
        now,
        &fetch,
    );
    assert_eq!(authorized.status, 200);
    assert_eq!(calls.get(), 1);
}

#[test]
fn capability_tokens_are_random_fixed_length_hex() {
    let first = create_capability_token().unwrap();
    let second = create_capability_token().unwrap();
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(first, second);
}

#[test]
fn api_errors_are_json_and_unknown_routes_are_not_fetched() {
    let response = route_request(
        &Method::Get,
        "/missing",
        &test_config(),
        &[],
        NaiveDate::from_ymd_opt(2026, 7, 12).unwrap(),
        Utc::now(),
        &no_events,
    );

    assert_eq!(response.status, 404);
    assert_eq!(response.content_type, JSON_CONTENT_TYPE);
    let json: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(json["error"]["kind"], "not_found");
}

#[test]
fn bundled_ui_and_csp_assets_are_self_hosted_and_safe_by_construction() {
    let config = test_config();
    let today = NaiveDate::from_ymd_opt(2026, 7, 12).unwrap();
    let now = Utc::now();
    for (path, content_type, needle) in [
        ("/", HTML_CONTENT_TYPE, "/assets/app.js"),
        ("/assets/app.css", CSS_CONTENT_TYPE, ".trip-card"),
        ("/assets/app.js", JAVASCRIPT_CONTENT_TYPE, "greatCircle"),
        (
            "/assets/vendor/maplibre-gl/maplibre-gl.css",
            CSS_CONTENT_TYPE,
            ".maplibregl-map",
        ),
        (
            "/assets/vendor/maplibre-gl/maplibre-gl-csp.js",
            JAVASCRIPT_CONTENT_TYPE,
            "maplibregl",
        ),
        (
            "/assets/vendor/maplibre-gl/maplibre-gl-csp-worker.js",
            JAVASCRIPT_CONTENT_TYPE,
            "worker",
        ),
    ] {
        let response = route_request(&Method::Get, path, &config, &[], today, now, &no_events);
        assert_eq!(response.status, 200, "failed to serve {path}");
        assert_eq!(response.content_type, content_type);
        assert!(response.body.contains(needle), "unexpected body for {path}");
    }
    for unsafe_api in ["innerHTML", "outerHTML", "insertAdjacentHTML", ".setHTML("] {
        assert!(
            !APP_JAVASCRIPT.contains(unsafe_api),
            "Calendar/provider text could reach unsafe DOM API {unsafe_api}"
        );
    }
    assert!(CONTENT_SECURITY_POLICY.contains("script-src 'self'"));
    assert!(CONTENT_SECURITY_POLICY.contains("worker-src 'self'"));
    assert!(!CONTENT_SECURITY_POLICY.contains("unsafe-eval"));
}
