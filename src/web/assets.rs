//! Embedded browser assets and uniform HTTP security policy.

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

pub(super) const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const INDEX_HTML: &[u8] = include_bytes!("../../web/index.html");
const APP_CSS: &[u8] = include_bytes!("../../web/app.css");
const APP_JS: &[u8] = include_bytes!("../../web/app.js");
const XTERM_CSS: &[u8] = include_bytes!("../../web/vendor/xterm-6.0.0.css");
const XTERM_JS: &[u8] = include_bytes!("../../web/vendor/xterm-6.0.0.mjs");
const ADDON_FIT_JS: &[u8] = include_bytes!("../../web/vendor/addon-fit-0.11.0.mjs");

pub(super) async fn index() -> Response {
    asset(INDEX_HTML, "text/html; charset=utf-8", "no-store")
}

pub(super) async fn app_css() -> Response {
    asset(APP_CSS, "text/css; charset=utf-8", "no-store")
}

pub(super) async fn app_js() -> Response {
    asset(APP_JS, "text/javascript; charset=utf-8", "no-store")
}

pub(super) async fn xterm_css() -> Response {
    asset(
        XTERM_CSS,
        "text/css; charset=utf-8",
        "public, max-age=31536000, immutable",
    )
}

pub(super) async fn xterm_js() -> Response {
    asset(
        XTERM_JS,
        "text/javascript; charset=utf-8",
        "public, max-age=31536000, immutable",
    )
}

pub(super) async fn addon_fit_js() -> Response {
    asset(
        ADDON_FIT_JS,
        "text/javascript; charset=utf-8",
        "public, max-age=31536000, immutable",
    )
}

pub(super) async fn not_found() -> Response {
    secured_response(StatusCode::NOT_FOUND, Body::from("Not found"))
}

fn asset(
    bytes: &'static [u8],
    content_type: &'static str,
    cache_control: &'static str,
) -> Response {
    let mut response = secured_response(StatusCode::OK, Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}

pub(super) fn secured_response(status: StatusCode, body: Body) -> Response {
    let mut response = (status, body).into_response();
    apply_security_headers(&mut response);
    response
}

pub(super) async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    apply_security_headers(&mut response);
    response
}

fn apply_security_headers(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}
