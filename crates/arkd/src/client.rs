use std::sync::Arc;

use anyhow::Context;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig, ServerPeerInfo};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::Value;

pub struct DaemonClient {
    pub inner: RunningService<RoleClient, ClientConfig>,
}

impl DaemonClient {
    pub async fn connect(url: &str, token: Option<&str>) -> anyhow::Result<Self> {
        Self::connect_with_headers(url, token, &[]).await
    }

    pub async fn connect_with_headers(
        url: &str,
        token: Option<&str>,
        headers: &[(String, String)],
    ) -> anyhow::Result<Self> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        if let Some(token) = token {
            config.auth_header = Some(format!("Bearer {token}"));
        }
        for (name, value) in headers {
            let name: reqwest::header::HeaderName = name
                .parse()
                .with_context(|| format!("invalid header name {name:?}"))?;
            let value: reqwest::header::HeaderValue = value
                .parse()
                .with_context(|| format!("invalid header value for {name}"))?;
            config.custom_headers.insert(name, value);
        }
        let transport = StreamableHttpClientTransport::from_config(config);
        let inner = ClientConfig::default()
            .serve(transport)
            .await
            .with_context(|| {
                format!(
                    "arkd daemon not reachable at {url}. Start it with 'arkd serve', or install the launchd agent ('arkd launchd print')."
                )
            })?;
        Ok(Self { inner })
    }

    pub async fn call(&self, name: &str, args: Value) -> anyhow::Result<CallToolResult> {
        let arguments = args.as_object().cloned().unwrap_or_default();
        self.inner
            .call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(arguments))
            .await
            .with_context(|| format!("{name} failed"))
    }

    pub fn server_info(&self) -> Option<Arc<ServerPeerInfo>> {
        self.inner.peer_info()
    }
}
