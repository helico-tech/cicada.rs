//! The embedded console served by `actor_ui()`.

use std::collections::{BTreeSet, VecDeque};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use cicada_ui::{actor_ui, file_text, try_actor_ui};
use http_body_util::BodyExt;
use regex::Regex;
use tower::ServiceExt;

async fn get(app: &Router, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

fn console(api_path: &str) -> Router {
    Router::new().merge(actor_ui("/console", api_path))
}

#[tokio::test]
async fn the_page_is_served_at_the_mount_path_with_a_trailing_slash_with_a_strict_content_security_policy() {
    let (status, headers, page) = get(&console("/api"), "/console/").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    assert!(
        Regex::new(r#"<script type="module" crossorigin src="\./assets/[^"]+\.js"></script>"#).unwrap().is_match(&page),
        "{page}"
    );
    // Nothing inline: the policy allows only the console's own files.
    assert!(
        !Regex::new(r"<script(\s[^>]*)?>").unwrap().find_iter(&page).any(|m| !m.as_str().contains(" src=")),
        "an inline script"
    );
    assert!(!page.contains("<style") && !page.contains(" style="), "an inline style");
    let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"), "{csp}");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
}

#[tokio::test]
async fn the_mount_path_without_a_slash_redirects_relatively_so_the_pages_relative_links_resolve() {
    let (status, headers, _) = get(&console("/api"), "/console").await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers[header::LOCATION], "console/");
}

#[tokio::test]
async fn config_json_tells_the_page_where_the_api_is() {
    let (status, _, body) = get(&console("/api"), "/console/config.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"apiPath":"/api"}"#);
}

#[tokio::test]
async fn scripts_and_styles_are_served_as_such_and_unknown_files_are_404() {
    let index = file_text("index.html").expect("index.html is embedded");
    let js = Regex::new(r#"src="\./(assets/[^"]+\.js)""#).unwrap().captures(&index).unwrap()[1].to_owned();
    let css = Regex::new(r#"href="\./(assets/[^"]+\.css)""#).unwrap().captures(&index).unwrap()[1].to_owned();
    let app = console("/");
    let (status, headers, _) = get(&app, &format!("/console/{js}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CONTENT_TYPE].to_str().unwrap().contains("javascript"));
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let (status, headers, _) = get(&app, &format!("/console/{css}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/css"));
    assert_eq!(get(&app, "/console/assets/nope.js").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mounted_at_the_root_the_page_is_the_root() {
    let app = actor_ui("/", "/");
    assert_eq!(get(&app, "/").await.0, StatusCode::OK);
    assert_eq!(get(&app, "/config.json").await.2, r#"{"apiPath":"/"}"#);
}

#[test]
fn api_path_must_be_a_plain_absolute_path() {
    for bad in ["api", "", "/a b", "/a\"b", "//evil.example/x?"] {
        assert!(try_actor_ui("/console", bad).is_err(), "{bad}");
        assert!(try_actor_ui(bad, "/api").is_err(), "{bad}");
    }
    assert!(try_actor_ui("/console", "/api/v1").is_ok());
}

#[test]
fn every_file_the_page_links_and_every_chunk_a_script_loads_exists() {
    let index = file_text("index.html").expect("index.html is embedded");
    let linked: Vec<String> =
        Regex::new(r#"(?:src|href)="\./([^"]+)""#).unwrap().captures_iter(&index).map(|c| c[1].to_owned()).collect();
    assert!(linked.len() >= 3, "favicon, script and style: {linked:?}");
    for path in &linked {
        assert!(file_text(path).is_some(), "index.html links {path}");
    }
    // Vite names lazily loaded chunks relative to the script that loads them (assets/).
    let chunk = Regex::new(r#""\./([\w.-]+\.(?:js|css))""#).unwrap();
    let mut pending: VecDeque<String> = linked.iter().filter(|p| p.ends_with(".js")).cloned().collect();
    let mut seen = BTreeSet::new();
    while let Some(script) = pending.pop_front() {
        if !seen.insert(script.clone()) {
            continue;
        }
        let source = file_text(&script).unwrap_or_else(|| panic!("missing chunk {script}"));
        for m in chunk.captures_iter(&source) {
            let path = format!("assets/{}", &m[1]);
            if path.ends_with(".js") {
                pending.push_back(path);
            } else {
                assert!(file_text(&path).is_some(), "missing {path}");
            }
        }
    }
    assert!(seen.len() >= 2, "the chain view is a chunk of its own: {seen:?}");
}
