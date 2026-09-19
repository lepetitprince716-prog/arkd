use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use arkd::app::AppState;
use arkd::auth::require_bearer;
use arkd_core::config::Config;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tower::ServiceExt;

fn state_with_token(token: Option<&str>) -> Arc<AppState> {
    Arc::new(AppState {
        config: Config::default(),
        registry: Arc::new(
            arkd_core::device::DeviceRegistry::from_config(
                &Config::default(),
                Arc::new(arkd_core::core_api::fake::FakeCoreFactory::new()),
            )
            .unwrap(),
        ),
        core_version: "test".to_string(),
        started_at: Instant::now(),
        token: token.map(str::to_string),
    })
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/mcp", get(|| async { Json(json!({"ok": true})) }))
        .route_layer(middleware::from_fn_with_state(state, require_bearer))
}

fn req(ip: Ipv4Addr, auth: Option<&str>) -> Request<axum::body::Body> {
    let mut b = Request::builder()
        .uri("/mcp")
        .extension(ConnectInfo(SocketAddr::new(IpAddr::V4(ip), 40000)));
    if let Some(token) = auth {
        b = b.header("authorization", format!("Bearer {token}"));
    }
    b.body(axum::body::Body::empty()).unwrap()
}

async fn status_of(router: &Router, ip: Ipv4Addr, auth: Option<&str>) -> StatusCode {
    router
        .clone()
        .oneshot(req(ip, auth))
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn loopback_passes_without_a_token() {
    let r = router(state_with_token(Some("secret")));
    assert_eq!(
        status_of(&r, Ipv4Addr::new(127, 0, 0, 1), None).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn non_loopback_without_header_is_unauthorized() {
    let r = router(state_with_token(Some("secret")));
    assert_eq!(
        status_of(&r, Ipv4Addr::new(10, 0, 0, 1), None).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn non_loopback_with_correct_token_passes() {
    let r = router(state_with_token(Some("secret")));
    assert_eq!(
        status_of(&r, Ipv4Addr::new(10, 0, 0, 1), Some("secret")).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn wrong_token_is_unauthorized() {
    let r = router(state_with_token(Some("secret")));
    let resp = router_response(&r, Ipv4Addr::new(10, 0, 0, 1), Some("nope")).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers().get("www-authenticate").unwrap(), "Bearer");
}

async fn router_response(r: &Router, ip: Ipv4Addr, auth: Option<&str>) -> axum::response::Response {
    r.clone().oneshot(req(ip, auth)).await.unwrap()
}

fn app_state() -> Arc<AppState> {
    let config = Config::default();
    arkd::app::App::build(
        config,
        Arc::new(arkd_core::core_api::fake::FakeCoreFactory::new()),
        "fake".to_string(),
    )
    .unwrap()
}

fn mcp_req(ip: Ipv4Addr, auth: Option<&str>) -> Request<axum::body::Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(axum::body::Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
        ))
        .unwrap();
    if let Some(token) = auth {
        req.headers_mut().insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
    }
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::new(IpAddr::V4(ip), 5000)));
    req
}

fn with_token(mut state: Arc<AppState>) -> Router {
    Arc::get_mut(&mut state).unwrap().token = Some("secret".to_string());
    arkd::app::App::router(state)
}

#[tokio::test]
async fn route_mcp_rejects_non_loopback_without_token() {
    let app = with_token(app_state());
    let resp = app
        .oneshot(mcp_req(Ipv4Addr::new(10, 0, 0, 1), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers().get("www-authenticate").unwrap(), "Bearer");
}

#[tokio::test]
async fn route_mcp_passes_non_loopback_with_token() {
    let app = with_token(app_state());
    let resp = app
        .oneshot(mcp_req(Ipv4Addr::new(10, 0, 0, 1), Some("secret")))
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn route_healthz_is_open_to_non_loopback() {
    let app = with_token(app_state());
    let mut req = Request::builder()
        .uri("/healthz")
        .body(axum::body::Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        5000,
    )));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn route_mcp_passes_loopback_without_token() {
    let app = with_token(app_state());
    let resp = app
        .oneshot(mcp_req(Ipv4Addr::new(127, 0, 0, 1), None))
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
}
