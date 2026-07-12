use crate::calendar::fetch_events;
use crate::cli::ReadCalendarSelectorArgs;
use crate::config::{Config, MAX_TRAVEL_RANGE_DAYS};
use crate::dates::{parse_end_datetime, parse_start_datetime};
use crate::models::EventReport;
use crate::travel::{TravelCollection, TravelLeg, TravelWarning, collect_travel_events};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Days, Local, NaiveDate, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::process::Command;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const API_SCHEMA_VERSION: u8 = 1;
const CAPABILITY_COOKIE: &str = "icalctl_travel_capability";
const DEFAULT_MAP_STYLE_URL: &str = "https://demotiles.maplibre.org/style.json";
const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
const JAVASCRIPT_CONTENT_TYPE: &str = "application/javascript; charset=utf-8";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const CSS_CONTENT_TYPE: &str = "text/css; charset=utf-8";
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; worker-src 'self'; child-src 'self'; connect-src 'self' data: blob: https: http:; img-src 'self' data: blob: https: http:; font-src 'self' data: https: http:";
const INDEX_HTML: &str = include_str!("../assets/web/index.html");
const APP_CSS: &str = include_str!("../assets/web/app.css");
const APP_JAVASCRIPT: &str = include_str!("../assets/web/app.js");
const MAPLIBRE_CSS: &str = include_str!("../assets/web/vendor/maplibre-gl/maplibre-gl.css");
const MAPLIBRE_JAVASCRIPT: &str =
    include_str!("../assets/web/vendor/maplibre-gl/maplibre-gl-csp.js");
const MAPLIBRE_WORKER_JAVASCRIPT: &str =
    include_str!("../assets/web/vendor/maplibre-gl/maplibre-gl-csp-worker.js");

#[derive(Debug, Serialize)]
struct TravelApiResponse {
    schema_version: u8,
    generated_at: String,
    range: TravelApiRange,
    calendar_ids: Vec<String>,
    map: TravelApiMap,
    legs: Vec<TravelLeg>,
    warnings: Vec<TravelWarning>,
}

#[derive(Debug, Serialize)]
struct TravelApiRange {
    start: String,
    end: String,
    end_inclusive: bool,
}

#[derive(Debug, Serialize)]
struct TravelApiMap {
    projection: String,
    style_url: String,
}

#[derive(Debug)]
struct DateRange {
    start: NaiveDate,
    end: NaiveDate,
    future_only_start: bool,
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: String,
    extra_headers: Vec<(String, String)>,
}

#[derive(Debug)]
struct ServerSecurity {
    host: String,
    origin: String,
    token: String,
}

#[derive(Debug, Default)]
struct RequestMetadata {
    hosts: Vec<String>,
    origins: Vec<String>,
    fetch_sites: Vec<String>,
    cookies: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    kind: &'static str,
    message: String,
}

pub fn serve(cli_calendar_ids: Vec<String>, json: bool) -> Result<()> {
    let config = crate::config::load().context("failed to load icalctl configuration")?;
    let calendar_ids = effective_calendar_ids(cli_calendar_ids, &config)?;
    let bind: IpAddr = config
        .travel
        .server
        .bind
        .parse()
        .context("travel.server.bind must be an IP address")?;
    if !bind.is_loopback() {
        bail!("travel.server.bind must be a loopback address");
    }

    let listener = TcpListener::bind(SocketAddr::new(bind, config.travel.server.port))
        .with_context(|| {
            format!(
                "failed to bind travel server to {}:{}",
                config.travel.server.bind, config.travel.server.port
            )
        })?;
    let address = listener
        .local_addr()
        .context("failed to read travel server address")?;
    let server = Server::from_listener(listener, None)
        .map_err(|error| anyhow!("failed to start travel server: {error}"))?;
    let security = ServerSecurity {
        host: address.to_string(),
        origin: format!("http://{address}"),
        token: create_capability_token()?,
    };
    let url = format!("{}/?token={}", security.origin, security.token);

    if json {
        println!(
            "{}",
            serde_json::json!({
                "type": "travel_server",
                "url": url,
                "bind": address.ip().to_string(),
                "port": address.port(),
                "calendar_ids": calendar_ids,
                "read_only": true,
            })
        );
    } else {
        println!("Serving read-only travel data at {url}");
        if calendar_ids.is_empty() {
            println!("Calendar filter: all calendars");
        } else {
            println!("Calendar ids: {}", calendar_ids.join(", "));
        }
        println!("Press Ctrl-C to stop.");
    }
    io::stdout()
        .flush()
        .context("failed to flush travel server address")?;

    if config.travel.server.open_browser {
        match Command::new("open").arg(&url).status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                eprintln!("warning: macOS open command exited with status {status}");
            }
            Err(error) => {
                eprintln!("warning: failed to open the travel visualization: {error}");
            }
        }
    }

    loop {
        let request = server
            .recv()
            .context("travel server failed while waiting for a request")?;
        if let Err(error) = respond(request, &config, &calendar_ids, &security) {
            eprintln!("warning: failed to send travel server response: {error:#}");
        }
    }
}

fn respond(
    request: Request,
    config: &Config,
    calendar_ids: &[String],
    security: &ServerSecurity,
) -> Result<()> {
    let metadata = request_metadata(&request);
    let response = dispatch_request(
        request.method(),
        request.url(),
        &metadata,
        security,
        config,
        calendar_ids,
        Local::now().date_naive(),
        Utc::now(),
        &fetch_events,
    );
    let is_head = request.method() == &Method::Head;
    let mut outgoing = if is_head {
        Response::empty(StatusCode(response.status)).boxed()
    } else {
        Response::from_string(response.body)
            .with_status_code(StatusCode(response.status))
            .boxed()
    };
    outgoing.add_header(header("Content-Type", response.content_type));
    outgoing.add_header(header("X-Content-Type-Options", "nosniff"));
    outgoing.add_header(header("Referrer-Policy", "no-referrer"));
    outgoing.add_header(header("Content-Security-Policy", CONTENT_SECURITY_POLICY));
    outgoing.add_header(header("Cross-Origin-Resource-Policy", "same-origin"));
    outgoing.add_header(header("X-Frame-Options", "DENY"));
    outgoing.add_header(header(
        "Permissions-Policy",
        "geolocation=(), camera=(), microphone=()",
    ));
    for (name, value) in response.extra_headers {
        outgoing.add_header(header(&name, &value));
    }
    request.respond(outgoing).context("response write failed")
}

#[allow(clippy::too_many_arguments)]
fn dispatch_request<F>(
    method: &Method,
    url: &str,
    metadata: &RequestMetadata,
    security: &ServerSecurity,
    config: &Config,
    calendar_ids: &[String],
    today: NaiveDate,
    now: DateTime<Utc>,
    fetch: &F,
) -> HttpResponse
where
    F: Fn(DateTime<Local>, DateTime<Local>, &ReadCalendarSelectorArgs) -> Result<Vec<EventReport>>,
{
    if let Some(response) = authorize_request(method, url, metadata, security) {
        return response;
    }
    route_request(method, url, config, calendar_ids, today, now, fetch)
}

fn authorize_request(
    method: &Method,
    url: &str,
    metadata: &RequestMetadata,
    security: &ServerSecurity,
) -> Option<HttpResponse> {
    if metadata.hosts.len() != 1 || metadata.hosts[0] != security.host {
        return Some(error_response(
            403,
            "forbidden_host",
            "request Host is not authorized for this local server",
            no_store_headers(),
        ));
    }
    if metadata.origins.len() > 1
        || metadata
            .origins
            .first()
            .is_some_and(|origin| origin != &security.origin)
    {
        return Some(error_response(
            403,
            "forbidden_origin",
            "cross-origin requests are not authorized",
            no_store_headers(),
        ));
    }
    if metadata.fetch_sites.len() > 1
        || metadata.fetch_sites.first().is_some_and(|site| {
            !site.eq_ignore_ascii_case("same-origin") && !site.eq_ignore_ascii_case("none")
        })
    {
        return Some(error_response(
            403,
            "forbidden_fetch_site",
            "cross-site requests are not authorized",
            no_store_headers(),
        ));
    }

    if method == &Method::Get && valid_launch_token(url, &security.token) {
        return Some(HttpResponse {
            status: 303,
            content_type: HTML_CONTENT_TYPE,
            body: String::new(),
            extra_headers: vec![
                ("Location".to_string(), "/".to_string()),
                (
                    "Set-Cookie".to_string(),
                    format!(
                        "{CAPABILITY_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
                        security.token
                    ),
                ),
                ("Cache-Control".to_string(), "no-store".to_string()),
            ],
        });
    }

    if !has_capability_cookie(&metadata.cookies, &security.token) {
        return Some(error_response(
            403,
            "capability_required",
            "a valid per-launch capability is required",
            no_store_headers(),
        ));
    }
    None
}

fn request_metadata(request: &Request) -> RequestMetadata {
    let mut metadata = RequestMetadata::default();
    for request_header in request.headers() {
        let value = request_header.value.as_str().to_string();
        if request_header.field.equiv("Host") {
            metadata.hosts.push(value);
        } else if request_header.field.equiv("Origin") {
            metadata.origins.push(value);
        } else if request_header.field.equiv("Sec-Fetch-Site") {
            metadata.fetch_sites.push(value);
        } else if request_header.field.equiv("Cookie") {
            metadata.cookies.push(value);
        }
    }
    metadata
}

fn valid_launch_token(url: &str, expected: &str) -> bool {
    let Some((path, query)) = url.split_once('?') else {
        return false;
    };
    if path != "/" {
        return false;
    }
    let parameters = form_urlencoded::parse(query.as_bytes()).collect::<Vec<_>>();
    parameters.len() == 1 && parameters[0].0 == "token" && parameters[0].1.as_ref() == expected
}

fn has_capability_cookie(cookies: &[String], expected: &str) -> bool {
    let expected = format!("{CAPABILITY_COOKIE}={expected}");
    cookies
        .iter()
        .flat_map(|header| header.split(';'))
        .any(|cookie| cookie.trim() == expected)
}

fn create_capability_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|error| {
        anyhow!("failed to generate the travel server capability token: {error}")
    })?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push(HEX[usize::from(byte >> 4)] as char);
        token.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    Ok(token)
}

#[allow(clippy::too_many_arguments)]
fn route_request<F>(
    method: &Method,
    url: &str,
    config: &Config,
    calendar_ids: &[String],
    today: NaiveDate,
    now: DateTime<Utc>,
    fetch: &F,
) -> HttpResponse
where
    F: Fn(DateTime<Local>, DateTime<Local>, &ReadCalendarSelectorArgs) -> Result<Vec<EventReport>>,
{
    let (path, _) = url.split_once('?').unwrap_or((url, ""));
    if !matches!(method, &Method::Get | &Method::Head) {
        return error_response(
            405,
            "method_not_allowed",
            "only GET and HEAD are supported",
            vec![("Allow".to_string(), "GET, HEAD".to_string())],
        );
    }

    match path {
        "/" => HttpResponse {
            status: 200,
            content_type: HTML_CONTENT_TYPE,
            body: INDEX_HTML.to_string(),
            extra_headers: no_store_headers(),
        },
        "/assets/app.css" => static_asset(CSS_CONTENT_TYPE, APP_CSS),
        "/assets/app.js" => static_asset(JAVASCRIPT_CONTENT_TYPE, APP_JAVASCRIPT),
        "/assets/vendor/maplibre-gl/maplibre-gl.css" => {
            static_asset(CSS_CONTENT_TYPE, MAPLIBRE_CSS)
        }
        "/assets/vendor/maplibre-gl/maplibre-gl-csp.js" => {
            static_asset(JAVASCRIPT_CONTENT_TYPE, MAPLIBRE_JAVASCRIPT)
        }
        "/assets/vendor/maplibre-gl/maplibre-gl-csp-worker.js" => {
            static_asset(JAVASCRIPT_CONTENT_TYPE, MAPLIBRE_WORKER_JAVASCRIPT)
        }
        "/api/travel" if method == &Method::Head => {
            match parse_range(url, today, config.travel.default_range_days) {
                Ok(_) => HttpResponse {
                    status: 200,
                    content_type: JSON_CONTENT_TYPE,
                    body: String::new(),
                    extra_headers: no_store_headers(),
                },
                Err(error) => error_response(
                    400,
                    "invalid_date_range",
                    error.to_string(),
                    no_store_headers(),
                ),
            }
        }
        "/api/travel" => {
            let range = match parse_range(url, today, config.travel.default_range_days) {
                Ok(range) => range,
                Err(error) => {
                    return error_response(
                        400,
                        "invalid_date_range",
                        error.to_string(),
                        no_store_headers(),
                    );
                }
            };
            match build_payload(config, calendar_ids, &range, now, fetch) {
                Ok(payload) => match serde_json::to_string(&payload) {
                    Ok(body) => HttpResponse {
                        status: 200,
                        content_type: JSON_CONTENT_TYPE,
                        body,
                        extra_headers: no_store_headers(),
                    },
                    Err(error) => error_response(
                        500,
                        "serialization_failed",
                        format!("failed to serialize travel data: {error}"),
                        no_store_headers(),
                    ),
                },
                Err(error) => error_response(
                    500,
                    "travel_data_unavailable",
                    format!("failed to load travel data: {error:#}"),
                    no_store_headers(),
                ),
            }
        }
        _ => error_response(404, "not_found", "route not found", Vec::new()),
    }
}

fn build_payload<F>(
    config: &Config,
    calendar_ids: &[String],
    range: &DateRange,
    now: DateTime<Utc>,
    fetch: &F,
) -> Result<TravelApiResponse>
where
    F: Fn(DateTime<Local>, DateTime<Local>, &ReadCalendarSelectorArgs) -> Result<Vec<EventReport>>,
{
    let start_text = range.start.format("%Y-%m-%d").to_string();
    let end_text = range.end.format("%Y-%m-%d").to_string();
    let mut event_start = parse_start_datetime(&start_text).context("invalid range start")?;
    let event_end = parse_end_datetime(&end_text).context("invalid range end")?;
    if range.future_only_start {
        let current = now.with_timezone(&Local);
        if current > event_start {
            event_start = current;
        }
    }
    let selector = ReadCalendarSelectorArgs {
        calendars: Vec::new(),
        calendar_ids: calendar_ids.to_vec(),
        calendar_source: None,
        source_id: None,
    };
    let events = fetch(event_start, event_end, &selector)
        .context("failed to read local Apple Calendar through EventKit")?;
    let mut collection = collect_travel_events(&events);
    crate::flightaware::enrich_collection(&config.flightaware, &mut collection);
    let TravelCollection { legs, warnings } = collection;

    Ok(TravelApiResponse {
        schema_version: API_SCHEMA_VERSION,
        generated_at: now.to_rfc3339(),
        range: TravelApiRange {
            start: start_text,
            end: end_text,
            end_inclusive: true,
        },
        calendar_ids: calendar_ids.to_vec(),
        map: TravelApiMap {
            projection: config.travel.map.projection.clone(),
            style_url: config
                .travel
                .map
                .style_url
                .clone()
                .unwrap_or_else(|| DEFAULT_MAP_STYLE_URL.to_string()),
        },
        legs,
        warnings,
    })
}

fn parse_range(url: &str, today: NaiveDate, default_range_days: u32) -> Result<DateRange> {
    let query = url.split_once('?').map(|(_, query)| query).unwrap_or("");
    let mut start = None;
    let mut end = None;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        let slot = match key.as_ref() {
            "start" => &mut start,
            "end" => &mut end,
            _ => bail!("unknown query parameter `{key}`"),
        };
        if slot.replace(value.into_owned()).is_some() {
            bail!("query parameter `{key}` may only be specified once");
        }
    }

    match (start, end) {
        (None, None) => {
            let days = default_range_days
                .checked_sub(1)
                .ok_or_else(|| anyhow!("travel.default_range_days must be greater than zero"))?;
            if default_range_days > MAX_TRAVEL_RANGE_DAYS {
                bail!(
                    "travel.default_range_days must not exceed {MAX_TRAVEL_RANGE_DAYS} inclusive days"
                );
            }
            let end = today
                .checked_add_days(Days::new(u64::from(days)))
                .ok_or_else(|| anyhow!("default travel date range is out of bounds"))?;
            Ok(DateRange {
                start: today,
                end,
                future_only_start: true,
            })
        }
        (Some(start), Some(end)) => {
            let start = parse_query_date(&start, "start")?;
            let end = parse_query_date(&end, "end")?;
            if start > end {
                bail!("start must be on or before end");
            }
            let inclusive_days = end.signed_duration_since(start).num_days() + 1;
            if inclusive_days > i64::from(MAX_TRAVEL_RANGE_DAYS) {
                bail!("date range must not exceed {MAX_TRAVEL_RANGE_DAYS} inclusive days");
            }
            Ok(DateRange {
                start,
                end,
                future_only_start: false,
            })
        }
        _ => bail!("start and end must be provided together"),
    }
}

fn parse_query_date(value: &str, name: &str) -> Result<NaiveDate> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        bail!("{name} must use YYYY-MM-DD");
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .with_context(|| format!("{name} must use YYYY-MM-DD"))
}

fn effective_calendar_ids(cli_calendar_ids: Vec<String>, config: &Config) -> Result<Vec<String>> {
    let values = if cli_calendar_ids.is_empty() {
        config.travel.calendar_ids.clone()
    } else {
        cli_calendar_ids
    };
    let mut seen = HashSet::new();
    let mut calendar_ids = Vec::new();
    for id in values {
        if id.trim().is_empty() {
            bail!("calendar ids must not be empty");
        }
        if seen.insert(id.clone()) {
            calendar_ids.push(id);
        }
    }
    Ok(calendar_ids)
}

fn static_asset(content_type: &'static str, body: &'static str) -> HttpResponse {
    HttpResponse {
        status: 200,
        content_type,
        body: body.to_string(),
        extra_headers: vec![(
            "Cache-Control".to_string(),
            "private, max-age=3600".to_string(),
        )],
    }
}

fn error_response(
    status: u16,
    kind: &'static str,
    message: impl Into<String>,
    extra_headers: Vec<(String, String)>,
) -> HttpResponse {
    let envelope = ErrorEnvelope {
        error: ErrorBody {
            kind,
            message: message.into(),
        },
    };
    HttpResponse {
        status,
        content_type: JSON_CONTENT_TYPE,
        body: serde_json::to_string(&envelope)
            .unwrap_or_else(|_| "{\"error\":{\"kind\":\"serialization_failed\",\"message\":\"failed to serialize error\"}}".to_string()),
        extra_headers,
    }
}

fn no_store_headers() -> Vec<(String, String)> {
    vec![("Cache-Control".to_string(), "no-store".to_string())]
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static HTTP header must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::travel::{FlightInput, format_flight};
    use chrono::{TimeZone, Timelike};
    use std::cell::{Cell, RefCell};

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
        assert_eq!(json["map"]["style_url"], DEFAULT_MAP_STYLE_URL);
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
        let maximum =
            parse_range("/api/travel?start=2026-01-01&end=2027-01-01", today, 90).unwrap();
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
}
