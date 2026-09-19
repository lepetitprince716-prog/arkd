use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use arkd_core::catalog;
use arkd_core::device::Device;
use arkd_core::error::Error;
use arkd_core::playtools::TouchPhase;
use arkd_core::screen::{self, CoordSpace, EncodeOpts, ImageFormat};
use arkd_core::session::WaitCondition;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ProgressNotificationParam};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Json, RoleServer, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::ArkdServer;

const WAIT_SLICE: Duration = Duration::from_secs(5);

fn tool_error(e: Error) -> ErrorData {
    match e {
        Error::Validation(_) | Error::UnknownTaskType(_) | Error::Config(_) => {
            ErrorData::invalid_params(e.to_string(), None)
        }
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

async fn run<T, F>(f: F) -> Result<T, ErrorData>
where
    T: Send + 'static,
    F: FnOnce() -> arkd_core::error::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ErrorData::internal_error(format!("worker task failed: {e}"), None))?
        .map_err(tool_error)
}

fn json_result(v: Value) -> Result<Json<Value>, ErrorData> {
    Ok(Json(v))
}

#[derive(Deserialize, JsonSchema)]
pub struct DeviceParam {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ConnectParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default)]
    #[schemars(description = "Force a fresh AsstAsyncConnect even if already connected.")]
    reconnect: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskSchemaParams {
    #[schemars(description = "Task type name, e.g. 'Fight', 'Copilot'.")]
    task_type: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct AppendParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[schemars(description = "Task type name from maa_task_types.")]
    task_type: String,
    #[schemars(description = "Task parameters matching the task type's schema.")]
    params: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct UpdateParamsParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[schemars(description = "Task id returned by maa_append.")]
    task_id: i32,
    #[schemars(description = "New parameters for the queued task.")]
    params: Value,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default = "default_until")]
    #[schemars(
        description = "Wait condition: any_event, task_done, all_done, error, idle, battle_problem, battle_stalled."
    )]
    until: String,
    #[schemars(description = "Task id; required when until is 'task_done'.")]
    task_id: Option<i32>,
    #[serde(default)]
    #[schemars(description = "Only count events after this sequence number.")]
    after_seq: u64,
    #[serde(default = "default_timeout")]
    #[schemars(description = "Overall timeout in seconds (1..=300).")]
    timeout_seconds: u32,
    #[serde(default)]
    #[schemars(description = "Include raw callback payloads in returned events.")]
    include_payload: bool,
    #[serde(default = "default_stall")]
    #[schemars(
        description = "Seconds without a copilot action before a battle counts as stalled (5..=1800)."
    )]
    stall_seconds: f64,
}

fn default_until() -> String {
    "all_done".to_string()
}
fn default_timeout() -> u32 {
    60
}
fn default_stall() -> f64 {
    90.0
}
fn default_true() -> bool {
    true
}
fn default_limit() -> u32 {
    50
}
fn default_via() -> String {
    "auto".to_string()
}
fn default_scale() -> f64 {
    1.0
}
fn default_format() -> String {
    "png".to_string()
}
fn default_quality() -> u8 {
    80
}
fn default_space() -> String {
    "screenshot".to_string()
}
fn default_hold_ms() -> u64 {
    250
}
fn default_step_ms() -> u64 {
    20
}
fn default_action() -> String {
    "Deploy".to_string()
}

#[derive(Deserialize, JsonSchema)]
pub struct EventsParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default)]
    #[schemars(description = "Only return events after this sequence number.")]
    after_seq: u64,
    #[serde(default = "default_limit")]
    #[schemars(description = "Maximum events to return (1..=500).")]
    limit: u32,
    #[serde(default = "default_true")]
    #[schemars(description = "Only include events a human would care about.")]
    significant_only: bool,
    #[serde(default)]
    #[schemars(description = "Include raw callback payloads.")]
    include_payload: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct DailyParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Stage to farm, e.g. '1-7' or 'CE-6'. Empty repeats the last stage played."
    )]
    stage: String,
    #[schemars(
        description = "Game client type for StartUp/CloseDown; defaults to the device's configured client_type."
    )]
    client_type: Option<String>,
    #[serde(default)]
    #[schemars(description = "Sanity potions to drink while farming.")]
    medicine: u32,
    #[serde(default = "default_true")]
    #[schemars(description = "Launch the client if it is not running.")]
    start_game: bool,
    #[serde(default)]
    #[schemars(description = "Close the client after the last task.")]
    close_when_done: bool,
    #[schemars(
        description = "Steps to queue, in order. Defaults to ['StartUp', 'Fight', 'Recruit', 'Infrast', 'Mall', 'Award']."
    )]
    include: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CaptureParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default = "default_via")]
    #[schemars(description = "Capture path: 'auto', 'maacore' or 'playtools'.")]
    via: String,
    #[serde(default = "default_true")]
    #[schemars(description = "Capture a fresh frame rather than MaaCore's cached one.")]
    fresh: bool,
    #[serde(default = "default_scale")]
    #[schemars(description = "Downscale factor 0.1..=1.0 applied to the returned image.")]
    scale: f64,
    #[serde(default = "default_format")]
    #[schemars(description = "Image format: 'png' or 'jpeg'.")]
    format: String,
    #[serde(default = "default_quality")]
    #[schemars(description = "JPEG quality 1..=100; ignored for PNG.")]
    quality: u8,
    #[schemars(description = "Write the image to this path instead of returning it inline.")]
    save_to: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TapParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[schemars(description = "Horizontal coordinate.")]
    x: f64,
    #[schemars(description = "Vertical coordinate.")]
    y: f64,
    #[serde(default = "default_space")]
    #[schemars(description = "Coordinate space: 'screenshot' (the captured PNG) or 'device'.")]
    coord_space: String,
    #[serde(default = "default_via")]
    #[schemars(description = "Input path: 'auto', 'maacore' or 'playtools'.")]
    via: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct TouchParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[schemars(description = "Touch phase: 'began', 'moved' or 'ended'.")]
    phase: String,
    #[schemars(description = "Horizontal coordinate.")]
    x: f64,
    #[schemars(description = "Vertical coordinate.")]
    y: f64,
    #[serde(default = "default_space")]
    #[schemars(description = "Coordinate space: 'screenshot' or 'device'.")]
    coord_space: String,
    #[serde(default)]
    #[schemars(description = "Contact index for multi-touch.")]
    contact: u8,
}

#[derive(Deserialize, JsonSchema)]
pub struct DragParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[schemars(description = "Polyline points as [x, y] pairs.")]
    points: Vec<[f64; 2]>,
    #[serde(default = "default_space")]
    #[schemars(description = "Coordinate space: 'screenshot' or 'device'.")]
    coord_space: String,
    #[serde(default = "default_hold_ms")]
    #[schemars(description = "Milliseconds to hold before moving.")]
    hold_ms: u64,
    #[serde(default = "default_step_ms")]
    #[schemars(description = "Milliseconds between move events.")]
    step_ms: u64,
}

#[derive(Deserialize, JsonSchema)]
pub struct BattleStateParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default = "default_stall")]
    #[schemars(
        description = "Seconds without a copilot action before the battle reads as stalled (5..=1800)."
    )]
    stall_seconds: f64,
}

#[derive(Deserialize, JsonSchema)]
pub struct BattleStageParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[schemars(description = "Stage code, e.g. '1-7'. Must be one MAA has tile data for.")]
    stage: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct BattleActionParams {
    #[schemars(description = "Device name; defaults to the configured default device.")]
    device: Option<String>,
    #[serde(default = "default_action")]
    #[schemars(
        description = "One of 'Deploy', 'Skill', 'Retreat', 'SpeedUp', 'BulletTime', 'SkillUsage', 'SkillDaemon', 'Output', 'MoveCamera'."
    )]
    action: String,
    #[schemars(
        description = "Operator name in the client's language. Required for 'Deploy'; for 'Skill' and 'Retreat' either this or location."
    )]
    name: Option<String>,
    #[schemars(
        description = "Deployment tile as [x, y] in the stage's own grid, origin top-left. Required for 'Deploy'."
    )]
    location: Option<[i32; 2]>,
    #[schemars(description = "Facing for 'Deploy': 'Left', 'Right', 'Up', 'Down' or 'None'.")]
    direction: Option<String>,
    #[schemars(description = "Skill usage mode. Required for action='SkillUsage'.")]
    skill_usage: Option<i64>,
}

impl ArkdServer {
    pub fn new(state: Arc<crate::app::AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    fn device(&self, name: Option<&String>) -> Result<Arc<Device>, ErrorData> {
        self.state
            .registry
            .get(name.map(String::as_str))
            .map_err(tool_error)
    }

    fn via_is_playtools(&self, device: &Device, via: &str) -> Result<bool, ErrorData> {
        match via {
            "maacore" => Ok(false),
            "playtools" => Ok(true),
            "auto" => Ok(matches!(
                device.config.kind,
                arkd_core::config::DeviceKind::Playtools { .. }
            ) && device.session.running()),
            other => Err(ErrorData::invalid_params(
                format!("unknown via {other:?}; expected 'auto', 'maacore' or 'playtools'"),
                None,
            )),
        }
    }
}

#[tool_router(vis = "pub(crate)")]
impl ArkdServer {
    #[tool(
        description = "List every configured device with its kind, address and live state. Use this to pick a device before calling device_connect, or to see which devices are busy running a task."
    )]
    async fn devices_list(
        &self,
        Parameters(_p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let mut devices = Vec::new();
        for s in self.state.registry.list() {
            let d = self.device(Some(&s.name))?;
            devices.push(json!({
                "name": s.name,
                "kind": s.kind,
                "address": s.address,
                "default": s.default,
                "connected": s.connected,
                "busy": s.busy,
                "device_size": d.config.device_size,
                "screenshot_size": d.config.screenshot_size,
            }));
        }
        json_result(json!({ "devices": devices }))
    }

    #[tool(
        description = "Connect a device to MaaCore. PlayTools devices connect over their TCP channel; ADB devices use the configured adb path, address and touch mode. If already connected this is a no-op unless reconnect=true."
    )]
    async fn device_connect(
        &self,
        Parameters(p): Parameters<ConnectParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let info = device
            .with_action(|| async {
                if !p.reconnect {
                    return device.ensure_connected().await;
                }
                let session = device.session.clone();
                let adb_path = device.config.adb_path().to_string();
                let address = device.config.address().to_string();
                let connect_config = device.config.connect_config.clone();
                let touch_mode = device.config.touch_mode().to_string();
                tokio::task::spawn_blocking(move || {
                    session.connect(&adb_path, &address, &connect_config, &touch_mode)
                })
                .await
                .map_err(|e| Error::DeviceConnection(format!("connect task failed: {e}")))?
            })
            .await
            .map_err(tool_error)?;
        json_result(serde_json::to_value(info).unwrap_or(json!(null)))
    }

    #[tool(
        description = "Report the daemon's live state: MaaCore version, connection, running flag, queued tasks, recent error and uptime. Call this first to see whether a device needs connecting."
    )]
    async fn status(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let status = run(move || Ok(session.status())).await?;
        let mut v = serde_json::to_value(status)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        v["device"] = json!(device.name);
        v["busy"] = json!(device.is_busy());
        v["uptime_seconds"] = json!(self.state.started_at.elapsed().as_secs_f64());
        v["config"] = self.state.config.describe();
        json_result(v)
    }

    #[tool(
        description = "Check the daemon's environment: MaaCore library and resource paths, per-device TCP reachability, a conflicting MAA.app on macOS, and the configured bind address. Read-only; safe to run any time."
    )]
    async fn doctor(
        &self,
        Parameters(_p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let config = self.state.config.clone();
        let core_version = self.state.core_version.clone();
        let registry = self.state.registry.clone();
        run(move || {
            let mut checks = Vec::new();
            let lib_name = if cfg!(target_os = "macos") {
                "libMaaCore.dylib"
            } else if cfg!(target_os = "windows") {
                "MaaCore.dll"
            } else {
                "libMaaCore.so"
            };
            let lib = config.maa.core_dir.join(lib_name);
            checks.push(json!({
                "check": "core library",
                "ok": lib.is_file(),
                "detail": lib.display().to_string(),
            }));
            checks.push(json!({
                "check": "core version",
                "ok": !core_version.is_empty(),
                "detail": core_version,
            }));
            let resource = config.maa.resource_dir.join("resource");
            checks.push(json!({
                "check": "resource_dir/resource",
                "ok": resource.is_dir(),
                "detail": resource.display().to_string(),
            }));
            for s in registry.list() {
                let reachable = s
                    .address
                    .parse::<std::net::SocketAddr>()
                    .ok()
                    .and_then(|a| {
                        std::net::TcpStream::connect_timeout(&a, Duration::from_secs(1)).ok()
                    })
                    .is_some();
                checks.push(json!({
                    "check": format!("device:{}:reachable", s.name),
                    "ok": reachable,
                    "detail": s.address,
                }));
            }
            #[cfg(target_os = "macos")]
            {
                let maa_running = std::process::Command::new("pgrep")
                    .args(["-x", "MAA"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                checks.push(json!({
                    "check": "MAA.app running",
                    "ok": !maa_running,
                    "detail": if maa_running {
                        "MAA.app is running; it holds its own MaaCore connection and may conflict with arkd"
                    } else {
                        "MAA.app is not running"
                    },
                }));
            }
            checks.push(json!({
                "check": "server bind",
                "ok": true,
                "detail": config.server.bind.to_string(),
            }));
            let ok = checks.iter().all(|c| c["ok"].as_bool().unwrap_or(false));
            Ok(json!({ "ok": ok, "checks": checks }))
        })
        .await
        .and_then(json_result)
    }

    #[tool(
        description = "List the MAA task types this server can queue, each with a summary and its required parameters. Call maa_task_schema for one type's full JSON schema, notes and example."
    )]
    async fn maa_task_types(
        &self,
        Parameters(_p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let types = catalog::list();
        let task_types: Vec<Value> = types
            .iter()
            .map(|t| {
                json!({
                    "task_type": t.task_type,
                    "summary": t.summary,
                    "required": t.required,
                })
            })
            .collect();
        json_result(json!({
            "count": task_types.len(),
            "task_types": task_types,
        }))
    }

    #[tool(
        description = "Return one task type's JSON schema, usage notes and an example params object. The schema is what maa_append validates params against."
    )]
    async fn maa_task_schema(
        &self,
        Parameters(p): Parameters<TaskSchemaParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let canonical = catalog::resolve(&p.task_type).map_err(tool_error)?;
        let spec = catalog::get(canonical).ok_or_else(|| {
            ErrorData::invalid_params(format!("unknown task type {:?}", p.task_type), None)
        })?;
        json_result(json!({
            "task_type": spec.task_type,
            "summary": spec.summary,
            "notes": spec.notes,
            "schema": spec.schema,
            "example": spec.example,
        }))
    }

    #[tool(
        description = "Queue a task on the device's session. Params are validated against the task type's schema before anything is queued. Nothing runs until maa_start."
    )]
    async fn maa_append(
        &self,
        Parameters(p): Parameters<AppendParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let params = p.params.unwrap_or_else(|| json!({}));
        let task_type = p.task_type.clone();
        let task = device
            .with_blocking_action(move || session.append_task(&task_type, params))
            .await
            .map_err(tool_error)?;
        json_result(task.describe())
    }

    #[tool(
        description = "Replace the parameters of a queued task, identified by the task_id maa_append returned. The new params are validated against the task type's schema."
    )]
    async fn maa_update_params(
        &self,
        Parameters(p): Parameters<UpdateParamsParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let task_id = p.task_id;
        let params = p.params;
        let out = device
            .with_blocking_action(move || session.set_task_params(task_id, params))
            .await
            .map_err(tool_error)?;
        json_result(out)
    }

    #[tool(
        description = "Show the queued tasks for a device: ids, types, params and per-task state."
    )]
    async fn maa_queue(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let tasks = run(move || Ok(session.tasks())).await?;
        json_result(json!({
            "count": tasks.len(),
            "tasks": tasks,
        }))
    }

    #[tool(
        description = "Start running the queued tasks. Returns immediately after the run begins; use maa_wait to block until the queue drains."
    )]
    async fn maa_start(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let (started, task_types, next_seq) = device
            .with_blocking_action(move || {
                let r = session.start()?;
                let seq = session.status().latest_event_seq;
                Ok((r.started, r.task_types, seq))
            })
            .await
            .map_err(tool_error)?;
        json_result(json!({
            "started": started,
            "task_types": task_types,
            "next_seq": next_seq,
        }))
    }

    #[tool(
        description = "Stop the current run and clear the queue. Interrupts whatever the game is doing part-way. Queued tasks are discarded, not paused -- re-append anything that should still run."
    )]
    async fn maa_stop(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let r = device
            .with_blocking_action(move || session.stop())
            .await
            .map_err(tool_error)?;
        json_result(json!({
            "stopped": r.stopped,
            "cleared_tasks": r.cleared_tasks,
            "reason": r.reason,
        }))
    }

    #[tool(
        description = "Tell MaaCore to navigate the game back to the home screen. Useful before queuing a routine whose tasks assume they start from home."
    )]
    async fn maa_back_home(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        device
            .with_blocking_action(move || session.back_to_home())
            .await
            .map_err(tool_error)?;
        json_result(json!({ "ok": true }))
    }

    #[tool(
        description = "Wait server-side for a condition: all_done (queue drained), task_done (a task id reached a terminal state), error, idle, battle_problem, battle_stalled or any_event. Blocks until the condition fires or timeout_seconds elapses; prefer this over polling maa_events in a loop."
    )]
    async fn maa_wait(
        &self,
        Parameters(p): Parameters<WaitParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<Value>, ErrorData> {
        if !(1..=300).contains(&p.timeout_seconds) {
            return Err(ErrorData::invalid_params(
                "timeout_seconds must be within 1..=300",
                None,
            ));
        }
        if !(5.0..=1800.0).contains(&p.stall_seconds) {
            return Err(ErrorData::invalid_params(
                "stall_seconds must be within 5..=1800",
                None,
            ));
        }
        let until = WaitCondition::from_str(&p.until).map_err(tool_error)?;
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let stall = Duration::from_secs_f64(p.stall_seconds);
        let total = Duration::from_secs(p.timeout_seconds as u64);
        let started = std::time::Instant::now();
        let progress_token = context.meta.get_progress_token();
        let mut outcome;
        loop {
            let remaining = total.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                outcome = arkd_core::session::WaitOutcome {
                    triggered: false,
                    reason: "timeout".to_string(),
                    waited: started.elapsed(),
                };
                break;
            }
            let slice = remaining.min(WAIT_SLICE);
            outcome = session
                .wait(until, p.task_id, p.after_seq, slice, stall)
                .await
                .map_err(tool_error)?;
            if outcome.triggered {
                break;
            }
            if started.elapsed() >= total {
                outcome.waited = started.elapsed();
                outcome.reason = "timeout".to_string();
                break;
            }
            if let Some(token) = progress_token.clone() {
                let _ = context
                    .peer
                    .notify_progress(
                        ProgressNotificationParam::new(token, started.elapsed().as_secs_f64())
                            .with_total(p.timeout_seconds as f64)
                            .with_message(format!("waiting for {}", p.until)),
                    )
                    .await;
            }
        }
        let session2 = device.session.clone();
        let after_seq = p.after_seq;
        let include_payload = p.include_payload;
        let (page, status) = run(move || {
            let page = session2.events(after_seq, 100, true, include_payload);
            let status = session2.status();
            Ok((page, status))
        })
        .await?;
        json_result(json!({
            "triggered": outcome.triggered,
            "reason": outcome.reason,
            "waited_seconds": (outcome.waited.as_secs_f64() * 100.0).round() / 100.0,
            "until": p.until,
            "events": page.events,
            "next_seq": page.next_seq.max(after_seq),
            "running": status.running,
            "task_counts": status.task_counts,
        }))
    }

    #[tool(
        description = "Page through the event history recorded from MaaCore callbacks. Pass the previous next_seq back as after_seq to resume. significant_only filters down to the events a human cares about; include_payload attaches the raw callback JSON."
    )]
    async fn maa_events(
        &self,
        Parameters(p): Parameters<EventsParams>,
    ) -> Result<Json<Value>, ErrorData> {
        if !(1..=500).contains(&p.limit) {
            return Err(ErrorData::invalid_params(
                "limit must be within 1..=500",
                None,
            ));
        }
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let (after, limit, sig, payload) = (
            p.after_seq,
            p.limit as usize,
            p.significant_only,
            p.include_payload,
        );
        let page = run(move || Ok(session.events(after, limit, sig, payload))).await?;
        json_result(json!({
            "events": page.events,
            "count": page.count,
            "next_seq": page.next_seq,
            "latest_seq": page.latest_seq,
            "has_more": page.has_more,
            "dropped": page.dropped,
        }))
    }

    #[tool(
        description = "Queue a conventional Arknights daily run in one call: StartUp, Fight, Recruit, Infrast, Mall, Award, with the presets the MAA GUI ships. Nothing is started; call maa_start next. For non-standard steps queue tasks individually with maa_append."
    )]
    async fn maa_daily(
        &self,
        Parameters(p): Parameters<DailyParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let client = p
            .client_type
            .clone()
            .unwrap_or_else(|| device.config.client_type.clone());
        let mut steps = p.include.clone().unwrap_or_else(|| {
            vec![
                "StartUp".to_string(),
                "Fight".to_string(),
                "Recruit".to_string(),
                "Infrast".to_string(),
                "Mall".to_string(),
                "Award".to_string(),
            ]
        });
        if p.close_when_done && !steps.iter().any(|s| s == "CloseDown") {
            steps.push("CloseDown".to_string());
        }
        let presets: Vec<(String, Option<Value>)> = vec![
            (
                "StartUp".to_string(),
                Some(json!({"client_type": client, "start_game_enabled": p.start_game})),
            ),
            (
                "Fight".to_string(),
                Some(json!({"stage": p.stage, "medicine": p.medicine})),
            ),
            (
                "Recruit".to_string(),
                Some(json!({"select": [4], "confirm": [3, 4], "times": 4, "skip_robot": true})),
            ),
            (
                "Infrast".to_string(),
                Some(json!({
                    "facility": ["Mfg", "Trade", "Power", "Control", "Reception", "Office", "Dorm"],
                    "drones": "Money",
                    "threshold": 0.3,
                })),
            ),
            (
                "Mall".to_string(),
                Some(json!({"shopping": true, "visit_friends": true})),
            ),
            (
                "Award".to_string(),
                Some(json!({"award": true, "mail": true})),
            ),
            (
                "CloseDown".to_string(),
                Some(json!({"client_type": client})),
            ),
        ];
        let session = device.session.clone();
        let (queued, skipped) = device
            .with_blocking_action(move || {
                let mut queued = Vec::new();
                let mut skipped = Vec::new();
                for step in &steps {
                    let canonical = catalog::resolve(step)?;
                    let params = presets
                        .iter()
                        .find(|(name, _)| name == canonical)
                        .and_then(|(_, v)| v.clone());
                    match params {
                        None => skipped.push(json!({
                            "task_type": canonical,
                            "reason": "no preset for this task type in the daily routine; queue it with maa_append",
                        })),
                        Some(params) => {
                            let task = session.append_task(canonical, params)?;
                            queued.push(task.describe());
                        }
                    }
                }
                Ok((queued, skipped))
            })
            .await
            .map_err(tool_error)?;
        json_result(json!({
            "queued": queued,
            "count": queued.len(),
            "skipped": skipped,
            "next": "call maa_start",
        }))
    }

    #[tool(
        description = "Capture what is on the game's screen right now. Returns an image plus a text block describing its dimensions; coordinates in screen_tap refer to this screenshot space. Use scale=0.5 to keep images small for chat clients. via='playtools' captures straight from the emulator while a task runs."
    )]
    async fn screen_capture(
        &self,
        Parameters(p): Parameters<CaptureParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !(0.1..=1.0).contains(&p.scale) {
            return Err(ErrorData::invalid_params(
                "scale must be within 0.1..=1.0",
                None,
            ));
        }
        let format = match p.format.as_str() {
            "png" => ImageFormat::Png,
            "jpeg" => ImageFormat::Jpeg,
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown format {other:?}; expected 'png' or 'jpeg'"),
                    None,
                ));
            }
        };
        let device = self.device(p.device.as_ref())?;
        let use_playtools = self.via_is_playtools(&device, &p.via)?;
        let opts = EncodeOpts {
            scale: p.scale,
            format,
            quality: p.quality,
        };
        let via_name = if use_playtools {
            "playtools"
        } else {
            "maacore"
        };
        let encoded = if use_playtools {
            let mut guard = device.playtools().await.map_err(tool_error)?;
            let client = guard
                .as_mut()
                .ok_or_else(|| ErrorData::internal_error("PlayTools client unavailable", None))?;
            let frame = client.capture().await.map_err(tool_error)?;
            tokio::task::spawn_blocking(move || screen::encode_frame(&frame, opts))
                .await
                .map_err(|e| ErrorData::internal_error(format!("encode failed: {e}"), None))?
                .map_err(tool_error)?
        } else {
            let session = device.session.clone();
            let fresh = p.fresh;
            let png = run(move || session.screenshot_png(fresh)).await?;
            tokio::task::spawn_blocking(move || screen::encode_png(&png, opts))
                .await
                .map_err(|e| ErrorData::internal_error(format!("encode failed: {e}"), None))?
                .map_err(tool_error)?
        };
        let meta = json!({
            "width": encoded.width,
            "height": encoded.height,
            "via": via_name,
            "coord_space": "screenshot",
        });
        if let Some(path) = p.save_to {
            let path =
                arkd_core::config::resolve_job_path(&path, self.state.config.job_dir.as_deref())
                    .map_err(tool_error)?;
            let bytes = encoded.bytes.len();
            tokio::fs::write(&path, &encoded.bytes).await.map_err(|e| {
                ErrorData::internal_error(format!("could not write {path}: {e}"), None)
            })?;
            let mut m = meta;
            m["path"] = json!(path);
            m["bytes"] = json!(bytes);
            return Ok(CallToolResult::success(vec![ContentBlock::text(
                m.to_string(),
            )]));
        }
        Ok(CallToolResult::success(vec![
            ContentBlock::image(BASE64.encode(&encoded.bytes), encoded.mime),
            ContentBlock::text(meta.to_string()),
        ]))
    }

    #[tool(
        description = "Tap a screen coordinate directly, bypassing the task queue. Coordinates default to screenshot space -- read them off a screen_capture image. This does not coordinate with a running task; prefer stopping the run first."
    )]
    async fn screen_tap(
        &self,
        Parameters(p): Parameters<TapParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let space = parse_coord_space(&p.coord_space)?;
        let use_playtools = self.via_is_playtools(&device, &p.via)?;
        let geometry = device.geometry().await.map_err(tool_error)?;
        let (dx, dy) = geometry.to_device(p.x, p.y, space);
        let dx_u16 = to_u16(dx, "x")?;
        let dy_u16 = to_u16(dy, "y")?;
        let via_name = if use_playtools {
            "playtools"
        } else {
            "maacore"
        };
        device
            .with_action(|| async {
                if use_playtools {
                    let mut guard = device.playtools().await?;
                    let client = guard
                        .as_mut()
                        .ok_or_else(|| Error::PlayTools("PlayTools client unavailable".into()))?;
                    client
                        .tap(dx_u16, dy_u16, Duration::from_millis(60))
                        .await?;
                } else {
                    let session = device.session.clone();
                    tokio::task::spawn_blocking(move || session.click(dx, dy))
                        .await
                        .map_err(|e| Error::CoreLoad(format!("click task failed: {e}")))??;
                }
                Ok(())
            })
            .await
            .map_err(tool_error)?;
        json_result(json!({
            "ok": true,
            "device_x": dx,
            "device_y": dy,
            "via": via_name,
        }))
    }

    #[tool(
        description = "Send a single touch phase to a PlayTools device: began, moved or ended. PlayTools only; use screen_tap for a simple tap. Coordinates default to screenshot space."
    )]
    async fn screen_touch(
        &self,
        Parameters(p): Parameters<TouchParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let phase = match p.phase.as_str() {
            "began" => TouchPhase::Began,
            "moved" => TouchPhase::Moved,
            "ended" => TouchPhase::Ended,
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown phase {other:?}; expected 'began', 'moved' or 'ended'"),
                    None,
                ));
            }
        };
        let device = self.device(p.device.as_ref())?;
        let space = parse_coord_space(&p.coord_space)?;
        let geometry = device.geometry().await.map_err(tool_error)?;
        let (dx, dy) = geometry.to_device(p.x, p.y, space);
        let dx_u16 = to_u16(dx, "x")?;
        let dy_u16 = to_u16(dy, "y")?;
        device
            .with_action(|| async {
                let mut guard = device.playtools().await?;
                let client = guard
                    .as_mut()
                    .ok_or_else(|| Error::PlayTools("PlayTools client unavailable".into()))?;
                client.touch(phase, dx_u16, dy_u16, p.contact).await
            })
            .await
            .map_err(tool_error)?;
        json_result(json!({
            "ok": true,
            "device_x": dx,
            "device_y": dy,
            "via": "playtools",
        }))
    }

    #[tool(
        description = "Drag along a polyline on a PlayTools device. PlayTools only. Note: dragging an operator onto a tile has not been shown to deploy the operator -- use battle_action or battle_deploy_batch for deployments."
    )]
    async fn screen_drag(
        &self,
        Parameters(p): Parameters<DragParams>,
    ) -> Result<Json<Value>, ErrorData> {
        if p.points.is_empty() {
            return Err(ErrorData::invalid_params("points must not be empty", None));
        }
        let device = self.device(p.device.as_ref())?;
        let space = parse_coord_space(&p.coord_space)?;
        let geometry = device.geometry().await.map_err(tool_error)?;
        let points: Vec<(u16, u16)> = p
            .points
            .iter()
            .map(|&[x, y]| {
                let (dx, dy) = geometry.to_device(x, y, space);
                Ok((to_u16(dx, "x")?, to_u16(dy, "y")?))
            })
            .collect::<Result<_, ErrorData>>()?;
        let count = points.len();
        device
            .with_action(|| async {
                let mut guard = device.playtools().await?;
                let client = guard
                    .as_mut()
                    .ok_or_else(|| Error::PlayTools("PlayTools client unavailable".into()))?;
                client
                    .drag(
                        &points,
                        Duration::from_millis(p.hold_ms),
                        Duration::from_millis(p.step_ms),
                    )
                    .await
            })
            .await
            .map_err(tool_error)?;
        json_result(json!({
            "ok": true,
            "points": count,
            "via": "playtools",
        }))
    }

    #[tool(
        description = "Report what happened in the copilot battle and what it means. Copilot failures are quiet by default; this collects MaaCore's copilot callbacks and names the cause -- empty squads, unavailable operators, stalls. Call after a Copilot task, when a run 'succeeded' but the stage was lost, or mid-battle."
    )]
    async fn battle_state(
        &self,
        Parameters(p): Parameters<BattleStateParams>,
    ) -> Result<Json<Value>, ErrorData> {
        if !(5.0..=1800.0).contains(&p.stall_seconds) {
            return Err(ErrorData::invalid_params(
                "stall_seconds must be within 5..=1800",
                None,
            ));
        }
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let stall = p.stall_seconds;
        let report = run(move || Ok(session.battle_state(Duration::from_secs_f64(stall)))).await?;
        json_result(
            serde_json::to_value(report)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
        )
    }

    #[tool(
        description = "Tell MaaCore which stage's tile map to use for manual battle control. The first step of driving a battle yourself; requires nothing else running -- stop a failing Copilot task first."
    )]
    async fn battle_set_stage(
        &self,
        Parameters(p): Parameters<BattleStageParams>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let stage = p.stage.clone();
        let task = device
            .with_blocking_action(move || {
                session.single_step("stage", Some(json!({"stage_name": stage})))
            })
            .await
            .map_err(tool_error)?;
        json_result(task.describe())
    }

    #[tool(
        description = "Begin the battle on the stage set by battle_set_stage. Returns the queued step; follow with battle_action calls or a battle_* compound tool."
    )]
    async fn battle_start(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<Json<Value>, ErrorData> {
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let task = device
            .with_blocking_action(move || session.single_step("start", None))
            .await
            .map_err(tool_error)?;
        json_result(task.describe())
    }

    #[tool(
        description = "Perform one battle action: deploy an operator, use a skill, or retreat. One action per call for rescuing a failing copilot job; the battle context lives in this session, so a fresh connect cannot Deploy until it has run battle_set_stage and battle_start. A Deploy waits for enough DP and a Skill for readiness, so an action can take a while."
    )]
    async fn battle_action(
        &self,
        Parameters(p): Parameters<BattleActionParams>,
    ) -> Result<Json<Value>, ErrorData> {
        if p.action == "Deploy"
            && (p.name.is_none() || p.location.is_none() || p.direction.is_none())
        {
            return Err(ErrorData::invalid_params(
                "A 'Deploy' needs name, location and direction. Read the tile coordinates off a screen_capture image rather than guessing.",
                None,
            ));
        }
        if matches!(p.action.as_str(), "Skill" | "Retreat")
            && p.name.is_none()
            && p.location.is_none()
        {
            return Err(ErrorData::invalid_params(
                format!(
                    "A {:?} needs either name or location to say which operator.",
                    p.action
                ),
                None,
            ));
        }
        if p.action == "SkillUsage" && p.skill_usage.is_none() {
            return Err(ErrorData::invalid_params(
                "A 'SkillUsage' action needs skill_usage.",
                None,
            ));
        }
        let mut details = json!({"type": p.action});
        if let Some(name) = &p.name {
            details["name"] = json!(name);
        }
        if let Some(location) = p.location {
            details["location"] = json!(location);
        }
        if let Some(direction) = &p.direction {
            details["direction"] = json!(direction);
        }
        if let Some(skill_usage) = p.skill_usage {
            details["skill_usage"] = json!(skill_usage);
        }
        let device = self.device(p.device.as_ref())?;
        let session = device.session.clone();
        let task = device
            .with_blocking_action(move || session.single_step("action", Some(details)))
            .await
            .map_err(tool_error)?;
        json_result(task.describe())
    }
}

fn to_u16(v: i32, axis: &str) -> Result<u16, ErrorData> {
    if v < 0 {
        return Err(ErrorData::invalid_params(
            format!("{axis} resolved to {v}, below the device coordinate range"),
            None,
        ));
    }
    Ok(v.min(u16::MAX as i32) as u16)
}

fn parse_coord_space(s: &str) -> Result<CoordSpace, ErrorData> {
    match s {
        "screenshot" => Ok(CoordSpace::Screenshot),
        "device" => Ok(CoordSpace::Device),
        other => Err(ErrorData::invalid_params(
            format!("unknown coord_space {other:?}; expected 'screenshot' or 'device'"),
            None,
        )),
    }
}
