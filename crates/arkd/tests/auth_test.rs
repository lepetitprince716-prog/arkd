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
