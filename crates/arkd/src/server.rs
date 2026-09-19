use std::sync::Arc;

use arkd_core::catalog;
use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult,
    ListResourcesResult, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, Role, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, tool_handler};
use serde_json::{Value, json};

use crate::app::AppState;

const INSTRUCTIONS: &str = "Drives MaaAssistantArknights (MAA), the Arknights automation core, \
on a connected Android device or PlayCover on macOS.

Typical flow:
1. `status` to see the core version, connection and queue. If no device is connected, \
`device_connect` (or `devices_list` first to pick one).
2. `maa_task_types` lists every task type; `maa_task_schema` gives a task type's JSON \
schema, notes and an example.
3. `maa_append` queues tasks, `maa_queue` shows them, `maa_start` runs the queue. \
`maa_stop` interrupts a run; `maa_back_home` returns the game to the home screen.
4. `maa_wait` blocks server-side until a condition (all_done, task_done, error, idle, \
battle_problem, battle_stalled, any_event) or a timeout -- prefer it over polling. \
`maa_events` pages through the callback history when you need the detail.
5. After a Copilot run, `battle_state` explains what happened in the battle. To drive \
a battle manually, stop the failing task first, then `battle_set_stage`, \
`battle_start` and `battle_action`.
6. `screen_capture` shows the screen; `screen_tap`/`screen_touch`/`screen_drag` act on \
it. Coordinates default to screenshot space -- the PNG you received -- so read \
positions off that image. `screen_capture` with scale=0.5 is recommended for chat \
clients to keep images small.
7. `battle_*` compound tools run their timing-sensitive loops server-side; use them \
rather than issuing raw touches yourself.
8. `maa_daily` queues a conventional daily run (StartUp, Fight, Recruit, Infrast, \
Mall, Award) in one call. Do not spend originite prime or sanity potions unless the \
user asks.";

#[derive(Clone)]
pub struct ArkdServer {
    pub state: Arc<AppState>,
    pub(crate) tool_router: ToolRouter<Self>,
}

fn text_resource(uri: &str, body: Value) -> ResourceContents {
    ResourceContents::text(
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string()),
        uri,
    )
    .with_mime_type("application/json")
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ArkdServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("arkd", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS.to_string())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new("arkd://catalog", "MAA task catalog")
                .with_title("MAA task catalog")
                .with_description("Every MAA task type with its summary and required parameters.")
                .with_mime_type("application/json"),
            Resource::new("arkd://catalog/{task_type}", "MAA task schema")
                .with_title("MAA task schema")
                .with_description("JSON Schema, notes and an example for one MAA task type.")
                .with_mime_type("application/json"),
            Resource::new("arkd://status", "arkd session status")
                .with_title("arkd session status")
                .with_description("Live core, connection and queue state.")
                .with_mime_type("application/json"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri.as_str();
        let body = if uri == "arkd://catalog" {
            json!({ "task_types": catalog::list() })
        } else if let Some(task_type) = uri.strip_prefix("arkd://catalog/") {
            let canonical = catalog::resolve(task_type)
                .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
            let spec = catalog::get(canonical).ok_or_else(|| {
                ErrorData::resource_not_found(format!("unknown task type {task_type:?}"), None)
            })?;
            json!({
                "task_type": spec.task_type,
                "summary": spec.summary,
                "notes": spec.notes,
                "schema": spec.schema,
                "example": spec.example,
            })
        } else if uri == "arkd://status" {
            let device = self
                .state
                .registry
                .get(None)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            let status = device.session.status();
            serde_json::to_value(&status)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
        } else {
            return Err(ErrorData::resource_not_found(
                format!("unknown resource {uri:?}"),
                None,
            ));
        };
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![text_resource(uri, body)],
        )))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(vec![
            Prompt::new(
                "daily_routine",
                Some("Queue and run a standard daily session, then report what happened."),
                Some(vec![PromptArgument::new("stage")]),
            ),
            Prompt::new(
                "diagnose",
                Some("Work out why the current run is failing or stalled."),
                None,
            ),
        ]))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let (description, text) = match request.name.as_str() {
            "daily_routine" => {
                let stage = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("stage"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let target = if stage.is_empty() {
                    "the last stage played".to_string()
                } else {
                    stage.clone()
                };
                let text = format!(
                    "Run today's Arknights dailies with MAA, farming {target}.\n\n\
                    1. Call status. If no device is connected, call device_connect.\n\
                    2. Call maa_daily{} then maa_start.\n\
                    3. Call maa_wait with until='all_done' and a generous timeout, \
                    then check maa_events for the chains that completed or errored.\n\
                    4. Report which task chains completed and which errored. For any \
                    error, call screen_capture and say what the screen shows.\n\n\
                    Do not spend originite prime or sanity potions unless I ask.",
                    if stage.is_empty() {
                        String::new()
                    } else {
                        format!(" with stage='{stage}'")
                    }
                );
                (
                    "Queue and run a standard daily session, then report what happened.",
                    text,
                )
            }
            "diagnose" => {
                let text = "The MAA run looks stuck. Diagnose it:\n\n\
                    1. status -- is the core loaded, is a device connected, is a run \
                    in progress, and what is last_error?\n\
                    2. maa_queue -- which task is 'running', and which reached 'error'?\n\
                    3. maa_events with significant_only=false and include_payload=true, \
                    scoped to the failing task's sequence range.\n\
                    4. screen_capture -- what is actually on screen?\n\
                    5. battle_state -- if a Copilot task ran, what does the battle \
                    report say?\n\n\
                    Then tell me the cause and the smallest fix. Do not stop or \
                    restart the run without asking."
                    .to_string();
                ("Work out why the current run is failing or stalled.", text)
            }
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown prompt {other:?}"),
                    None,
                ));
            }
        };
        let result = GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
            .with_description(description);
        Ok(GetPromptResponse::Complete(result))
    }
}
