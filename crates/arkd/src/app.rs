use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use arkd_core::config::Config;
use arkd_core::core_api::CoreFactory;
use arkd_core::device::DeviceRegistry;
use arkd_core::error::{Error, Result};
use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::{Value, json};
use tower_http::trace::TraceLayer;

use crate::auth::require_bearer;
use crate::server::ArkdServer;

pub struct AppState {
    pub config: Config,
    pub registry: Arc<DeviceRegistry>,
    pub core_version: String,
    pub started_at: Instant,
    pub token: Option<String>,
}

pub struct App;

impl App {
    pub fn build(
        config: Config,
        factory: Arc<dyn CoreFactory>,
        core_version: String,
    ) -> Result<Arc<AppState>> {
        let registry = Arc::new(DeviceRegistry::from_config(&config, factory)?);
        let token = std::env::var("ARKD_TOKEN").ok().or_else(|| {
            config.server.token_file.as_ref().and_then(|p| {
                std::fs::read_to_string(p)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
        });
        if !config.server.bind.ip().is_loopback() && token.is_none() {
            return Err(Error::Config(
                "refusing to bind to a non-loopback address without a token; run 'arkd token init' or set ARKD_TOKEN"
                    .to_string(),
            ));
        }
        Ok(Arc::new(AppState {
            config,
            registry,
            core_version,
            started_at: Instant::now(),
            token,
        }))
    }

    pub fn router(state: Arc<AppState>) -> Router {
        let mcp_state = state.clone();
        let service: StreamableHttpService<ArkdServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(ArkdServer::new(mcp_state.clone())),
                Arc::new(LocalSessionManager::default()),
                StreamableHttpServerConfig::default(),
            );
        let mcp = Router::new().nest_service("/mcp", service).route_layer(
            middleware::from_fn_with_state(state.clone(), require_bearer),
        );
        Router::new()
            .merge(mcp)
            .route("/healthz", get(healthz))
            .layer(TraceLayer::new_for_http())
            .with_state(state)
    }

    pub async fn serve(
        state: Arc<AppState>,
        listener: tokio::net::TcpListener,
    ) -> anyhow::Result<()> {
        let app = Self::router(state);
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
        Ok(())
    }
}

async fn healthz() -> Json<Value> {
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
