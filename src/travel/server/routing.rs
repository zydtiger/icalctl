use super::api::build_payload;
use super::assets::{
    APP_CSS, APP_JAVASCRIPT, CSS_CONTENT_TYPE, HTML_CONTENT_TYPE, INDEX_HTML,
    JAVASCRIPT_CONTENT_TYPE, JSON_CONTENT_TYPE, MAPLIBRE_CSS, MAPLIBRE_JAVASCRIPT,
    MAPLIBRE_WORKER_JAVASCRIPT, static_asset,
};
use super::range::parse_range;
use super::security::{RequestMetadata, ServerSecurity, authorize_request};
use super::{HttpResponse, error_response, no_store_headers};
use crate::cli::ReadCalendarSelectorArgs;
use crate::config::Config;
use crate::models::EventReport;
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate, Utc};
use tiny_http::Method;

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_request<F>(
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

#[allow(clippy::too_many_arguments)]
pub(super) fn route_request<F>(
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
