use super::assets::HTML_CONTENT_TYPE;
use super::{HttpResponse, error_response, no_store_headers};
use anyhow::{Result, anyhow};
use tiny_http::{Method, Request};

pub(super) const CAPABILITY_COOKIE: &str = "icalctl_travel_capability";

#[derive(Debug)]
pub(super) struct ServerSecurity {
    pub(super) host: String,
    pub(super) origin: String,
    pub(super) token: String,
}

#[derive(Debug, Default)]
pub(super) struct RequestMetadata {
    pub(super) hosts: Vec<String>,
    pub(super) origins: Vec<String>,
    pub(super) fetch_sites: Vec<String>,
    pub(super) cookies: Vec<String>,
}

pub(super) fn authorize_request(
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

pub(super) fn request_metadata(request: &Request) -> RequestMetadata {
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

pub(super) fn create_capability_token() -> Result<String> {
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
