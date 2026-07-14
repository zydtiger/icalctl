mod api;
mod assets;
mod range;
mod routing;
mod security;
#[cfg(test)]
mod tests;

use self::assets::{CONTENT_SECURITY_POLICY, JSON_CONTENT_TYPE};
use self::routing::dispatch_request;
use self::security::{ServerSecurity, create_capability_token, request_metadata};
use crate::calendar::fetch_events;
use crate::config::Config;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, Utc};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::process::Command;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: String,
    extra_headers: Vec<(String, String)>,
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
