use super::HttpResponse;

pub(super) const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
pub(super) const JAVASCRIPT_CONTENT_TYPE: &str = "application/javascript; charset=utf-8";
pub(super) const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
pub(super) const CSS_CONTENT_TYPE: &str = "text/css; charset=utf-8";
pub(super) const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; worker-src 'self'; child-src 'self'; connect-src 'self' data: blob: https: http:; img-src 'self' data: blob: https: http:; font-src 'self' data: https: http:";
pub(super) const INDEX_HTML: &str = include_str!("../../../assets/web/index.html");
pub(super) const APP_CSS: &str = include_str!("../../../assets/web/app.css");
pub(super) const APP_JAVASCRIPT: &str = include_str!("../../../assets/web/app.js");
pub(super) const MAPLIBRE_CSS: &str =
    include_str!("../../../assets/web/vendor/maplibre-gl/maplibre-gl.css");
pub(super) const MAPLIBRE_JAVASCRIPT: &str =
    include_str!("../../../assets/web/vendor/maplibre-gl/maplibre-gl-csp.js");
pub(super) const MAPLIBRE_WORKER_JAVASCRIPT: &str =
    include_str!("../../../assets/web/vendor/maplibre-gl/maplibre-gl-csp-worker.js");

pub(super) fn static_asset(content_type: &'static str, body: &'static str) -> HttpResponse {
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
