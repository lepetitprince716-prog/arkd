use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    str::FromStr,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant, SystemTime},
};

use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    battle_state::{self, BattleReport, BattleState},
    catalog,
    core_api::{CallbackFn, CoreFactory, MaaCoreApi},
    error::{Error, Result},
    events::{EventLog, EventsPage},
    messages::{self, msg},
};

pub const STATE_QUEUED: &str = "queued";
pub const STATE_RUNNING: &str = "running";
pub const STATE_COMPLETED: &str = "completed";
pub const STATE_ERROR: &str = "error";
pub const STATE_STOPPED: &str = "stopped";

const TERMINAL_STATES: &[&str] = &[STATE_COMPLETED, STATE_ERROR, STATE_STOPPED];
const BATTLE_TASKCHAINS: &[&str] = &["Copilot", "SSSCopilot", "ParadoxCopilot", "SingleStep"];
const CONNECTION_FAILURES: &[&str] = &[
    "ConnectFailed",
    "Disconnect",
    "ScreencapFailed",
    "UnsupportedResolution",
    "ResolutionError",
    "TouchModeNotAvailable",
];

pub const DEFAULT_STALL: Duration = Duration::from_secs(90);

#[derive(Clone, Debug)]
pub struct QueuedTask {
    pub task_id: i32,
    pub task_type: String,
    pub params: Value,
    pub appended_at: SystemTime,
    pub state: String,
}

impl QueuedTask {
    pub fn describe(&self) -> Value {
        json!({
            "task_id": self.task_id,
            "task_type": self.task_type,
            "state": self.state,
            "params": self.params,
            "appended_at": crate::events::iso(self.appended_at),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionInfo {
    pub what: String,
    pub uuid: Option<String>,
    pub details: Value,
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub core_loaded: bool,
    pub core_version: Option<String>,
    pub connected: bool,
    pub running: bool,
    pub connection: Option<ConnectionInfo>,
    pub task_counts: BTreeMap<String, usize>,
    pub tasks: Vec<Value>,
    pub latest_event_seq: u64,
    pub last_error: Option<String>,
    pub frame_dims: Option<(u32, u32)>,
}

#[derive(Debug)]
pub struct StartResult {
    pub started: Vec<i32>,
    pub task_types: Vec<String>,
}

#[derive(Debug)]
pub struct StopResult {
    pub stopped: bool,
    pub cleared_tasks: usize,
    pub reason: Option<String>,
}

#[derive(Debug)]
pub struct WaitOutcome {
    pub triggered: bool,
    pub reason: String,
    pub waited: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitCondition {
    AnyEvent,
    TaskDone,
    AllDone,
    Error,
    Idle,
    BattleProblem,
    BattleStalled,
}

impl FromStr for WaitCondition {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "any_event" => Self::AnyEvent,
            "task_done" => Self::TaskDone,
            "all_done" => Self::AllDone,
            "error" => Self::Error,
            "idle" => Self::Idle,
            "battle_problem" => Self::BattleProblem,
            "battle_stalled" => Self::BattleStalled,
            other => {
                return Err(Error::Validation(format!(
                    "Unknown wait condition {other:?}; expected one of [\"any_event\", \"task_done\", \"all_done\", \"error\", \"idle\", \"battle_problem\", \"battle_stalled\"]"
                )));
            }
        })
    }
}

impl std::fmt::Display for WaitCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AnyEvent => "any_event",
            Self::TaskDone => "task_done",
            Self::AllDone => "all_done",
            Self::Error => "error",
            Self::Idle => "idle",
            Self::BattleProblem => "battle_problem",
            Self::BattleStalled => "battle_stalled",
        })
    }
}

#[derive(Debug)]
pub struct BgrFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

struct State {
    events: EventLog,
    tasks: BTreeMap<i32, QueuedTask>,
    connection: Option<ConnectionInfo>,
    last_error: Option<String>,
    battle: BattleState,
    frame_dims: Option<(u32, u32)>,
}

pub struct SessionInner {
    state: Mutex<State>,
    notify: tokio::sync::Notify,
    core: Box<dyn MaaCoreApi>,
    job_dir: Option<std::path::PathBuf>,
}

pub struct MaaSession {
    inner: Arc<SessionInner>,
}

impl Clone for MaaSession {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl MaaSession {
    pub fn open(factory: &dyn CoreFactory, max_events: usize) -> Result<MaaSession> {
        Self::open_with_job_dir(factory, max_events, None)
    }

    pub fn open_with_job_dir(
        factory: &dyn CoreFactory,
        max_events: usize,
        job_dir: Option<std::path::PathBuf>,
    ) -> Result<MaaSession> {
        let slot: Arc<Mutex<Option<Weak<SessionInner>>>> = Arc::new(Mutex::new(None));
        let slot2 = slot.clone();
        let callback: CallbackFn = Arc::new(move |id: i32, json: &str| {
            if let Some(weak) = slot2.lock().unwrap().as_ref()
                && let Some(inner) = weak.upgrade()
            {
                inner.on_callback(id, json);
            }
        });
        let core = factory.create(callback)?;
        let inner = Arc::new(SessionInner {
            state: Mutex::new(State {
                events: EventLog::new(max_events.max(1)),
                tasks: BTreeMap::new(),
                connection: None,
                last_error: None,
                battle: BattleState::default(),
                frame_dims: None,
            }),
            notify: tokio::sync::Notify::new(),
            core,
            job_dir,
        });
        *slot.lock().unwrap() = Some(Arc::downgrade(&inner));
        Ok(MaaSession { inner })
    }

    fn on_callback(inner: &SessionInner, message_id: i32, details_json: &str) {
        let payload = match serde_json::from_str::<Value>(details_json) {
            Ok(v) if v.is_object() => v,
            Ok(v) => json!({"raw": v}),
            Err(_) => json!({"raw": details_json}),
        };
        {
            let mut st = inner.state.lock().unwrap();
            st.events.push(message_id, payload);
            Self::apply_event(&mut st, message_id);
        }
        inner.notify.notify_waiters();
    }

    fn apply_event(st: &mut State, message_id: i32) {
        let event = st.events.iter().last().unwrap();
        let payload = event.payload.clone();

        let chain_state = match message_id {
            msg::TASK_CHAIN_START => Some(STATE_RUNNING),
            msg::TASK_CHAIN_COMPLETED => Some(STATE_COMPLETED),
            msg::TASK_CHAIN_ERROR => Some(STATE_ERROR),
            msg::TASK_CHAIN_STOPPED => Some(STATE_STOPPED),
            _ => None,
        };
        if let Some(state) = chain_state
            && let Some(task_id) = payload.get("taskid").and_then(|v| v.as_i64())
            && let Some(task) = st.tasks.get_mut(&(task_id as i32))
        {
            task.state = state.to_string();
        }

        let taskchain = payload.get("taskchain").and_then(|v| v.as_str());
        if message_id == msg::TASK_CHAIN_START
            && taskchain
                .map(|t| BATTLE_TASKCHAINS.contains(&t))
                .unwrap_or(false)
        {
            st.battle = BattleState::new(taskchain.map(String::from));
            if let Some(task_id) = payload.get("taskid").and_then(|v| v.as_i64())
                && let Some(task) = st.tasks.get(&(task_id as i32))
            {
                st.battle.stage = task
                    .params
                    .get("stage")
                    .or_else(|| task.params.get("filename"))
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
        }

        if let Some(what) = payload.get("what").and_then(|v| v.as_str())
            && messages::BATTLE_WHAT.contains(&what)
        {
            let details = payload.get("details").cloned().unwrap_or(json!({}));
            st.battle.note_event(what, &details, Instant::now());
            if messages::BATTLE_FAILURE_WHAT.contains(&what) {
                st.last_error = Some(format!("{what}: {details}"));
            }
        }

        if message_id == msg::ALL_TASKS_COMPLETED {
            if let Some(finished) = payload.get("finished_tasks").and_then(|v| v.as_array()) {
                for id in finished {
                    if let Some(task_id) = id.as_i64()
                        && let Some(task) = st.tasks.get_mut(&(task_id as i32))
                        && (task.state == STATE_QUEUED || task.state == STATE_RUNNING)
                    {
                        task.state = STATE_COMPLETED.to_string();
                    }
                }
            }
        } else if message_id == msg::CONNECTION_INFO {
            let what = payload.get("what").and_then(|v| v.as_str()).unwrap_or("");
            match what {
                "Connected" | "UuidGot" | "Reconnected" => {
                    let prev_uuid = st.connection.as_ref().and_then(|c| c.uuid.clone());
                    st.connection = Some(ConnectionInfo {
                        what: what.to_string(),
                        uuid: payload
                            .get("uuid")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .or(prev_uuid),
                        details: payload.get("details").cloned().unwrap_or(json!({})),
                    });
                }
                "ConnectFailed" | "Disconnect" => {
                    st.connection = None;
                }
                _ => {}
            }
            if CONNECTION_FAILURES.contains(&what) {
                st.last_error = Some(
                    payload
                        .get("why")
                        .and_then(|v| v.as_str())
                        .unwrap_or(what)
                        .to_string(),
                );
            }
        }

        if messages::is_error(message_id) {
            st.last_error = Some(
                payload
                    .get("why")
                    .or_else(|| payload.get("what"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| messages::message_name(message_id)),
            );
        }
    }

    pub fn connect(
        &self,
        adb_path: &str,
        address: &str,
        config: &str,
        touch_mode: &str,
    ) -> Result<ConnectionInfo> {
        let inner = &self.inner;
        inner
            .core
            .set_instance_option(maa_types::InstanceOptionKey::TouchMode, touch_mode)?;
        match inner.core.connect(adb_path, address, config) {
            Err(e) => {
                let reason = inner
                    .state
                    .lock()
                    .unwrap()
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "no reason reported by MaaCore".to_string());
                Err(Error::DeviceConnection(format!(
                    "Could not connect to {address} using adb at {adb_path:?} (profile {config:?}): {reason}. Check that the emulator is running and that 'adb devices' lists this address; 'maa_events' has the connection callbacks in full. ({e})"
                )))
            }
            Ok(()) => {
                let uuid = inner.core.uuid();
                let conn = ConnectionInfo {
                    what: "Connected".to_string(),
                    uuid,
                    details: json!({
                        "adb": adb_path,
                        "address": address,
                        "config": config,
                        "touch_mode": touch_mode,
                    }),
                };
                inner.state.lock().unwrap().connection = Some(conn.clone());
                Ok(conn)
            }
        }
    }

    pub fn require_connection(&self) -> Result<()> {
        if self.inner.core.connected() {
            Ok(())
        } else {
            Err(Error::NotConnected(
                "No device is connected. Call device_connect first.".to_string(),
            ))
        }
    }

    pub fn append_task(&self, task_type: &str, params: Value) -> Result<QueuedTask> {
        let (canonical, checked) = catalog::validate(task_type, params)?;
        let checked = self.resolve_paths(checked)?;
        self.require_connection()?;
        let json = checked.to_string();
        let task_id = self.inner.core.append_task(&canonical, &json)?;
        if task_id <= 0 {
            return Err(Error::Refused(format!(
                "MaaCore refused the {canonical} task (AsstAppendTask returned {task_id}). The parameters are structurally valid, so this is usually a value the loaded resources do not know -- an unknown stage code, or a theme not present in this resource version."
            )));
        }
        let task = QueuedTask {
            task_id,
            task_type: canonical,
            params: checked,
            appended_at: SystemTime::now(),
            state: STATE_QUEUED.to_string(),
        };
        self.inner
            .state
            .lock()
            .unwrap()
            .tasks
            .insert(task_id, task.clone());
        Ok(task)
    }

    fn resolve_paths(&self, params: Value) -> Result<Value> {
        let Some(job_dir) = self.inner.job_dir.as_deref() else {
            return Ok(params);
        };
        let mut resolved = params;
        if let Some(obj) = resolved.as_object_mut() {
            for field in catalog::FILE_PATH_FIELDS {
                if let Some(raw) = obj.get(*field).and_then(|v| v.as_str()).map(String::from) {
                    let path = crate::config::resolve_job_path(&raw, Some(job_dir))?;
                    obj.insert((*field).to_string(), Value::String(path));
                }
            }
        }
        Ok(resolved)
    }

    pub fn set_task_params(&self, task_id: i32, params: Value) -> Result<Value> {
        let mut st = self.inner.state.lock().unwrap();
        let task = st.tasks.get(&task_id).ok_or_else(|| {
            let known = st
                .tasks
                .keys()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Error::Validation(format!(
                "No task with id {task_id} in this session. Queued ids: {}.",
                if known.is_empty() {
                    "none".into()
                } else {
                    known
                }
            ))
        })?;
        let (_, checked) = catalog::validate(&task.task_type, params)?;
        self.inner.core.set_task_params(task_id, &checked.to_string()).map_err(|e| {
            Error::Refused(format!(
                "MaaCore refused the parameter update for task {task_id} ({}). Several fields cannot be changed once the task is running -- 'stage' on Fight and 'filename' on the copilot tasks among them. ({e})",
                task.task_type
            ))
        })?;
        let task = st.tasks.get_mut(&task_id).unwrap();
        task.params = checked;
        Ok(task.describe())
    }

    pub fn start(&self) -> Result<StartResult> {
        self.require_connection()?;
        let st = self.inner.state.lock().unwrap();
        let pending: Vec<&QueuedTask> = st
            .tasks
            .values()
            .filter(|t| t.state == STATE_QUEUED)
            .collect();
        if pending.is_empty() {
            return Err(Error::CoreLoad(
                "Nothing to start: the queue holds no tasks in the 'queued' state. Append tasks with 'maa_append' first.".to_string(),
            ));
        }
        let started = pending.iter().map(|t| t.task_id).collect();
        let task_types = pending.iter().map(|t| t.task_type.clone()).collect();
        drop(st);
        self.inner.core.start().map_err(|_| {
            Error::CoreLoad(
                "AsstStart returned false. MaaCore is connected but declined to run -- most often because a run is already in progress; check 'status'.".to_string(),
            )
        })?;
        Ok(StartResult {
            started,
            task_types,
        })
    }

    pub fn stop(&self) -> Result<StopResult> {
        let mut st = self.inner.state.lock().unwrap();
        let stopped = self.inner.core.stop().is_ok();
        for task in st.tasks.values_mut() {
            if task.state == STATE_QUEUED || task.state == STATE_RUNNING {
                task.state = STATE_STOPPED.to_string();
            }
        }
        let cleared = st.tasks.len();
        st.tasks.clear();
        Ok(StopResult {
            stopped,
            cleared_tasks: cleared,
            reason: None,
        })
    }

    pub fn single_step(&self, subtype: &str, details: Option<Value>) -> Result<QueuedTask> {
        self.require_connection()?;
        if self.inner.core.running() {
            return Err(Error::Validation(
                "MaaCore is already running a task, and AsstStart refuses to start another. SingleStep drives a battle yourself instead of alongside a Copilot task -- call maa_stop first.".to_string(),
            ));
        }
        let mut params = json!({"type": "copilot", "subtype": subtype});
        if let Some(d) = details {
            params["details"] = d;
        }
        let task = self.append_task("SingleStep", params)?;
        self.start()?;
        Ok(task)
    }

    pub fn back_to_home(&self) -> Result<()> {
        self.require_connection()?;
        self.inner.core.back_to_home()
    }

    pub fn click(&self, x: i32, y: i32) -> Result<()> {
        self.require_connection()?;
        self.inner.core.click(x, y)
    }

    pub fn screenshot_png(&self, fresh: bool) -> Result<Vec<u8>> {
        self.require_connection()?;
        if fresh {
            self.inner.core.screencap()?;
        }
        let image = self.inner.core.image_png()?;
        let Some(png) = image else {
            return Err(Error::NotConnected(
                "MaaCore has no frame to return. It caches the last screenshot taken while running a task, so there is nothing to show until a task has run at least once on this connection.".to_string(),
            ));
        };
        if let Ok(reader) =
            image::ImageReader::new(std::io::Cursor::new(&png)).with_guessed_format()
            && let Ok((w, h)) = reader.into_dimensions()
        {
            self.inner.state.lock().unwrap().frame_dims = Some((w, h));
        }
        Ok(png)
    }

    pub fn screenshot_bgr(&self, fresh: bool) -> Result<BgrFrame> {
        self.require_connection()?;
        let dims = self.inner.state.lock().unwrap().frame_dims;
        let (w, h) = match dims {
            Some(d) => d,
            None => {
                self.screenshot_png(true)?;
                self.inner.state.lock().unwrap().frame_dims.ok_or_else(|| {
                    Error::Image("could not determine frame dimensions".to_string())
                })?
            }
        };
        if fresh {
            self.inner.core.screencap()?;
        }
        let Some(data) = self.inner.core.image_bgr()? else {
            return Err(Error::NotConnected(
                "MaaCore has no frame to return. It caches the last screenshot taken while running a task, so there is nothing to show until a task has run at least once on this connection.".to_string(),
            ));
        };
        let expected = (w as usize) * (h as usize) * 3;
        if data.len() != expected {
            return Err(Error::Image(format!(
                "BGR frame is {} bytes, expected {expected} for {w}x{h}",
                data.len()
            )));
        }
        Ok(BgrFrame {
            width: w,
            height: h,
            data,
        })
    }

    pub fn status(&self) -> Status {
        let st = self.inner.state.lock().unwrap();
        let tasks: Vec<Value> = st.tasks.values().map(|t| t.describe()).collect();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for t in st.tasks.values() {
            *counts.entry(t.state.clone()).or_insert(0) += 1;
        }
        Status {
            core_loaded: true,
            core_version: Some(self.inner.core.version()),
            connected: self.inner.core.connected(),
            running: self.inner.core.running(),
            connection: st.connection.clone(),
            task_counts: counts,
            tasks,
            latest_event_seq: st.events.latest_seq(),
            last_error: st.last_error.clone(),
            frame_dims: st.frame_dims,
        }
    }

    pub fn tasks(&self) -> Vec<Value> {
        self.inner
            .state
            .lock()
            .unwrap()
            .tasks
            .values()
            .map(|t| t.describe())
            .collect()
    }

    pub fn events(
        &self,
        after_seq: u64,
        limit: usize,
        significant_only: bool,
        include_payload: bool,
    ) -> EventsPage {
        self.inner.state.lock().unwrap().events.page(
            after_seq,
            limit,
            significant_only,
            include_payload,
        )
    }

    pub fn battle_state(&self, stall: Duration) -> BattleReport {
        let running = self.inner.core.running();
        let st = self.inner.state.lock().unwrap();
        battle_state::report(&st.battle, running, stall)
    }

    fn condition_met(
        &self,
        running: bool,
        st: &State,
        until: WaitCondition,
        task_id: Option<i32>,
        after_seq: u64,
        stall: Duration,
    ) -> Option<String> {
        match until {
            WaitCondition::AnyEvent => {
                let hit = st
                    .events
                    .iter()
                    .filter(|e| e.seq > after_seq)
                    .any(|e| messages::is_significant(e.message_id, &e.payload));
                hit.then(|| "event".to_string())
            }
            WaitCondition::TaskDone => match task_id.and_then(|id| st.tasks.get(&id)) {
                None => Some("task_gone".to_string()),
                Some(t) if TERMINAL_STATES.contains(&t.state.as_str()) => {
                    Some(format!("task_{}", t.state))
                }
                Some(_) => None,
            },
            WaitCondition::AllDone => {
                if st
                    .tasks
                    .values()
                    .any(|t| t.state == STATE_QUEUED || t.state == STATE_RUNNING)
                {
                    None
                } else {
                    Some("all_done".to_string())
                }
            }
            WaitCondition::Error => {
                if st.events.iter().any(|e| e.seq > after_seq && e.is_error()) {
                    return Some("error_event".to_string());
                }
                if st.tasks.values().any(|t| t.state == STATE_ERROR) {
                    return Some("task_error".to_string());
                }
                None
            }
            WaitCondition::Idle => {
                if running {
                    None
                } else {
                    Some("idle".to_string())
                }
            }
            WaitCondition::BattleProblem => {
                let now = Instant::now();
                let since = st.battle.last_action_at.map(|t| now.duration_since(t));
                let in_battle = st.battle.started_at.map(|t| now.duration_since(t));
                let (status, _, _) =
                    battle_state::diagnose(&st.battle, running, since, in_battle, stall);
                (status == "problem").then(|| "battle_problem".to_string())
            }
            WaitCondition::BattleStalled => {
                let now = Instant::now();
                let since = st.battle.last_action_at.map(|t| now.duration_since(t));
                let in_battle = st.battle.started_at.map(|t| now.duration_since(t));
                let (status, _, _) =
                    battle_state::diagnose(&st.battle, running, since, in_battle, stall);
                (status == "stalled").then(|| "battle_stalled".to_string())
            }
        }
    }

    pub async fn wait(
        &self,
        until: WaitCondition,
        task_id: Option<i32>,
        after_seq: u64,
        timeout: Duration,
        stall: Duration,
    ) -> Result<WaitOutcome> {
        if until == WaitCondition::TaskDone && task_id.is_none() {
            return Err(Error::Validation(
                "Waiting for 'task_done' needs a task_id. Pass the id returned by maa_append, or wait for 'all_done' instead.".to_string(),
            ));
        }
        let started = Instant::now();
        let deadline = started + timeout;
        loop {
            let notified = self.inner.notify.notified();
            {
                let running = self.inner.core.running();
                let st = self.inner.state.lock().unwrap();
                if let Some(reason) =
                    self.condition_met(running, &st, until, task_id, after_seq, stall)
                {
                    return Ok(WaitOutcome {
                        triggered: true,
                        reason,
                        waited: started.elapsed(),
                    });
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(WaitOutcome {
                    triggered: false,
                    reason: String::new(),
                    waited: started.elapsed(),
                });
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(remaining.min(Duration::from_secs(1))) => {}
            }
        }
    }
}

impl SessionInner {
    fn on_callback(self: &Arc<Self>, id: i32, json: &str) {
        MaaSession::on_callback(self, id, json);
    }
}

pub fn frame_digest(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}
