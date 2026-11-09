//! The Cicada actor console: a single-page web UI over the HTTP API (`cicada-axum`), to list, inspect, create and
//! delete actors, send and cancel messages, follow message chains and failures. It polls the API every few seconds
//! while open.
//!
//! ```no_run
//! # fn api() -> axum::Router { axum::Router::new() }
//! let app: axum::Router = axum::Router::new()
//!     .nest("/api", api())                                  // the API (cicada_axum::actor_api), behind the host's auth
//!     .merge(cicada_ui::actor_ui("/console", "/api"));      // the console, at /console/
//! ```
//!
//! The console is static files plus `config.json` (which tells the page where the API is); it holds no data and calls
//! only the API, from the browser, on the same origin. Authentication stays the host's: the API decides who may do
//! what. A browser sends cookies and HTTP Basic credentials it already has; for a bearer-token API the console asks for
//! the token on the first `401` and keeps it in the tab's session storage. Wrap the console in the host's auth layer
//! too if the page itself should not be public.
//!
//! The page is a React + TypeScript app in the Sensus BPM Online design (ADR 0006),
//! built with Vite at compile time (see `build.rs`) and embedded in the binary.

use axum::Router;
use axum::body::Body;
use axum::extract::{OriginalUri, Path};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

/// The console's built files.
#[derive(RustEmbed)]
#[folder = "$OUT_DIR/web"]
struct Web;

/// The console loads only its own files and talks only to its own origin. Styles are set through the CSSOM, which
/// `style-src 'self'` allows; no inline script or style is used.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; \
     connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// An invalid mount or API path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPath {
    /// Which parameter.
    pub name: &'static str,
    /// The refused value.
    pub value: String,
}

impl std::fmt::Display for InvalidPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} must be an absolute path of unreserved characters, such as / or /api: '{}'",
            self.name, self.value
        )
    }
}

impl std::error::Error for InvalidPath {}

/// The console as a router with absolute routes under `mount_path` (`/console`, or `/` for the root), to be merged
/// into the host's router with [`Router::merge`].
///
/// `api_path` is where the API is mounted on this origin, as an absolute path such as `/` or `/api`. Only unreserved
/// URL characters and `/` are accepted in both. A `GET` on the mount path without a trailing slash redirects to it with
/// one, because the page's files are linked relative to it.
///
/// # Panics
///
/// When a path is not a plain absolute path; see [`try_actor_ui`].
pub fn actor_ui(mount_path: &str, api_path: &str) -> Router {
    try_actor_ui(mount_path, api_path).unwrap_or_else(|e| panic!("{e}"))
}

/// [`actor_ui`], failing instead of panicking on an invalid path.
pub fn try_actor_ui(mount_path: &str, api_path: &str) -> Result<Router, InvalidPath> {
    let valid = |p: &str| {
        p.starts_with('/')
            && !p.starts_with("//")
            && p.chars().all(|c| c.is_ascii_alphanumeric() || "-._~/".contains(c))
    };
    if !valid(api_path) {
        return Err(InvalidPath { name: "api_path", value: api_path.to_owned() });
    }
    if !valid(mount_path) || mount_path.contains("//") {
        return Err(InvalidPath { name: "mount_path", value: mount_path.to_owned() });
    }
    let base = mount_path.trim_end_matches('/');
    let index = Web::get("index.html").map(|f| f.data.into_owned()).unwrap_or_default();
    let config = format!(r#"{{"apiPath":"{api_path}"}}"#);
    let page_route = get(move |OriginalUri(uri): OriginalUri| {
        let index = index.clone();
        async move { page(uri.path(), index) }
    });
    let mut router = Router::new().route(&format!("{base}/"), page_route.clone());
    if !base.is_empty() {
        router = router.route(base, page_route);
    }
    Ok(
        router
            .route(
                &format!("{base}/config.json"),
                get(move || {
                    let config = config.clone();
                    async move {
                        secured(Response::builder().header(header::CONTENT_TYPE, "application/json"), config.into())
                    }
                }),
            )
            .route(&format!("{base}/{{*file}}"), get(|Path(file): Path<String>| async move { asset(&file) })),
    )
}

/// The page at the mount path; without the trailing slash a relative redirect to it, so it stays right behind a proxy
/// that strips a path prefix.
fn page(path: &str, index: Vec<u8>) -> Response {
    if !path.ends_with('/') {
        let last = path.rsplit('/').next().unwrap_or_default();
        return Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, format!("{last}/"))
            .body(Body::empty())
            .expect("a valid redirect");
    }
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY);
    secured(builder, index.into())
}

fn asset(file: &str) -> Response {
    match Web::get(file) {
        Some(content) if file != "index.html" => {
            let mime = mime_guess::from_path(file).first_or_octet_stream();
            secured(Response::builder().header(header::CONTENT_TYPE, mime.as_ref()), content.data.into_owned().into())
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// No sniffing, no caching without revalidation (files change with the library version), no referrer.
fn secured(builder: axum::http::response::Builder, body: Body) -> Response {
    builder
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"))
        .header(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"))
        .body(body)
        .expect("a valid response")
}

/// The text of an embedded console file, for tests and diagnostics.
pub fn file_text(path: &str) -> Option<String> {
    Web::get(path).map(|f| String::from_utf8_lossy(&f.data).into_owned())
}
