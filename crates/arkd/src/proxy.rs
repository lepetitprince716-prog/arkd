use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, GetPromptRequestParams, GetPromptResponse,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ServerConfig,
};
use rmcp::service::{RequestContext, ServiceError};
use rmcp::{ErrorData, RoleServer};

use crate::client::DaemonClient;

pub struct ProxyHandler {
    upstream: DaemonClient,
    info: ServerConfig,
}

impl ProxyHandler {
    pub fn new(upstream: DaemonClient) -> Self {
        let info = upstream
            .server_info()
            .map(|i| {
                ServerConfig::new(i.capabilities.clone())
                    .with_server_info(
                        i.server_info
                            .clone()
                            .unwrap_or_else(rmcp::model::Implementation::from_build_env),
                    )
                    .with_instructions(i.instructions.clone().unwrap_or_default())
            })
            .unwrap_or_default();
        Self { upstream, info }
    }
}

fn forward_error(e: ServiceError) -> ErrorData {
    match e {
        ServiceError::McpError(data) => data,
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.upstream
            .inner
            .list_tools(request)
            .await
            .map_err(forward_error)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.upstream
            .inner
            .call_tool(request)
            .await
            .map(CallToolResponse::Complete)
            .map_err(forward_error)
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.upstream
            .inner
            .list_resources(request)
            .await
            .map_err(forward_error)
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.upstream
            .inner
            .list_resource_templates(request)
            .await
            .map_err(forward_error)
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        self.upstream
            .inner
            .read_resource(request)
            .await
            .map(ReadResourceResponse::Complete)
            .map_err(forward_error)
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        self.upstream
            .inner
            .list_prompts(request)
            .await
            .map_err(forward_error)
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        self.upstream
            .inner
            .get_prompt(request)
            .await
            .map(GetPromptResponse::Complete)
            .map_err(forward_error)
    }
}
