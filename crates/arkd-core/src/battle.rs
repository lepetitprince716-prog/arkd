use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::device::Device;
use crate::error::{Error, Result};
use crate::playtools::Frame;
use crate::screen::{CoordSpace, EncodeOpts, ImageFormat, encode_frame};
use crate::session::{BgrFrame, DEFAULT_STALL, WaitCondition, frame_digest};

pub const PAUSE_GAP: Duration = Duration::from_millis(1200);

const VALID_DIRECTIONS: &[&str] = &[
    "Left", "Right", "Up", "Down", "None", "左", "右", "上", "下", "无",
];

async fn spawn<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::CoreLoad(format!("worker task failed: {e}")))?
}

async fn capture_bgr(device: &Device) -> Result<BgrFrame> {
    let session = device.session.clone();
    spawn(move || session.screenshot_bgr(true)).await
}

fn digest(frame: &BgrFrame) -> u64 {
    frame_digest(&frame.data)
}

async fn pause_button_device(device: &Device) -> Result<(i32, i32)> {
    let (x, y) = device.config.pause_button_screenshot;
    let g = device.geometry().await?;
    Ok(g.to_device(x as f64, y as f64, CoordSpace::Screenshot))
}

async fn click(device: &Device, x: i32, y: i32) -> Result<()> {
    let session = device.session.clone();
    spawn(move || session.click(x, y)).await
}

async fn click_pause(device: &Device) -> Result<()> {
    let (x, y) = pause_button_device(device).await?;
    click(device, x, y).await
}

fn task_state(device: &Device, task_id: i32) -> Option<String> {
    device
        .session
        .tasks()
        .iter()
        .find(|t| t.get("task_id").and_then(Value::as_i64) == Some(task_id as i64))
        .and_then(|t| t.get("state").and_then(Value::as_str).map(String::from))
}

fn is_terminal(state: Option<&str>) -> bool {
    matches!(state, Some("completed") | Some("error") | Some("stopped"))
}

#[derive(Debug)]
enum StepOutcome {
    Done(String),
    Rejected(String),
    Timeout,
}

impl StepOutcome {
    fn describe(&self) -> String {
        match self {
            Self::Done(s) => format!("done ({s})"),
            Self::Rejected(e) => format!("rejected: {e}"),
            Self::Timeout => "timeout".to_string(),
        }
    }
}

async fn run_step(
    device: &Device,
    subtype: &str,
    details: Option<Value>,
    timeout: Duration,
) -> StepOutcome {
    let session = device.session.clone();
    let subtype = subtype.to_string();
    let task = match spawn(move || session.single_step(&subtype, details)).await {
        Ok(t) => t,
        Err(e) => return StepOutcome::Rejected(e.to_string()),
    };
    let outcome = device
        .session
        .wait(
            WaitCondition::TaskDone,
            Some(task.task_id),
            0,
            timeout,
            DEFAULT_STALL,
        )
        .await;
    match outcome {
        Ok(o) if o.triggered && o.reason == "task_completed" => StepOutcome::Done(o.reason),
        Ok(o) if o.triggered => StepOutcome::Rejected(format!(
            "step ended in state {}",
            o.reason.trim_start_matches("task_")
        )),
        Ok(_) => StepOutcome::Timeout,
        Err(e) => StepOutcome::Rejected(e.to_string()),
    }
}

pub async fn is_paused(device: &Device, gap: Duration) -> Result<bool> {
    let first = capture_bgr(device).await?;
    tokio::time::sleep(gap).await;
    let second = capture_bgr(device).await?;
    Ok(digest(&first) == digest(&second))
}

fn png_of(frame: &BgrFrame) -> Result<Vec<u8>> {
    let f = Frame {
        width: frame.width,
        height: frame.height,
        bgr: frame.data.clone(),
    };
    let encoded = encode_frame(
        &f,
        EncodeOpts {
            scale: 1.0,
            format: ImageFormat::Png,
            quality: 80,
        },
    )?;
    Ok(encoded.bytes)
}

#[derive(Debug, Clone)]
pub struct StartPausedOptions {
    pub stage: String,
    pub poll: Duration,
    pub timeout: Duration,
    pub pause_gap: Duration,
    pub settle: Duration,
}

impl StartPausedOptions {
    pub fn new(stage: impl Into<String>, timeout: Duration) -> Self {
        Self {
            stage: stage.into(),
            poll: Duration::from_millis(200),
            timeout,
            pause_gap: PAUSE_GAP,
            settle: Duration::from_millis(1000),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StartPausedReport {
    pub ok: bool,
    pub stage: String,
    pub failed_at: Option<String>,
    pub detail: Option<String>,
    pub fired_on: Option<String>,
    pub pause_attempts: u32,
    pub paused: bool,
    pub hud_detected: Option<bool>,
    pub start_task_state: Option<String>,
    pub waited_seconds: f64,
    #[serde(skip_serializing)]
    pub frame_png: Option<Vec<u8>>,
}

pub async fn start_paused(device: &Device, opts: StartPausedOptions) -> Result<StartPausedReport> {
    let started = Instant::now();
    let mut report = StartPausedReport {
        ok: false,
        stage: opts.stage.clone(),
        failed_at: None,
        detail: None,
        fired_on: None,
        pause_attempts: 0,
        paused: false,
        hud_detected: None,
        start_task_state: None,
        waited_seconds: 0.0,
        frame_png: None,
    };

    let stage_outcome = run_step(
        device,
        "stage",
        Some(json!({"stage_name": opts.stage})),
        Duration::from_secs(30),
    )
    .await;
    if !matches!(stage_outcome, StepOutcome::Done(_)) {
        report.failed_at = Some("stage".to_string());
        report.detail = Some(stage_outcome.describe());
        report.waited_seconds = started.elapsed().as_secs_f64();
        return Ok(report);
    }

    let before = digest(&capture_bgr(device).await?);

    let session = device.session.clone();
    let start_task = match spawn(move || session.single_step("start", None)).await {
        Ok(t) => t,
        Err(e) => {
            report.failed_at = Some("start".to_string());
            report.detail = Some(e.to_string());
            report.waited_seconds = started.elapsed().as_secs_f64();
            return Ok(report);
        }
    };
    let start_id = start_task.task_id;

    let has_template = device.hud_template()?.is_some();
    let deadline = started + opts.timeout;
    let mut fired = false;
    let mut last_frame: Option<BgrFrame> = None;
    while Instant::now() < deadline {
        let frame = capture_bgr(device).await?;
        let hud = match device.hud_template()? {
            Some(t) => Some(t.matches(&frame)?),
            None => None,
        };
        report.hud_detected = hud;
        let state = task_state(device, start_id);
        report.start_task_state = state.clone();
        if hud == Some(true) {
            report.fired_on = Some("hud".to_string());
            fired = true;
            last_frame = Some(frame);
            break;
        }
        if is_terminal(state.as_deref()) && digest(&frame) != before {
            report.fired_on = Some("start_step".to_string());
            fired = true;
            last_frame = Some(frame);
            break;
        }
        last_frame = Some(frame);
        tokio::time::sleep(opts.poll).await;
    }

    if !fired {
        report.failed_at = Some("wait-for-battle".to_string());
        report.detail = Some("battlefield never rendered".to_string());
        report.waited_seconds = started.elapsed().as_secs_f64();
        if let Some(f) = last_frame {
            report.frame_png = Some(png_of(&f)?);
        }
        return Ok(report);
    }

    click_pause(device).await?;
    report.pause_attempts = 1;

    let remaining = deadline.saturating_duration_since(Instant::now());
    if !is_terminal(task_state(device, start_id).as_deref()) && !remaining.is_zero() {
        let outcome = device
            .session
            .wait(
                WaitCondition::TaskDone,
                Some(start_id),
                0,
                remaining,
                DEFAULT_STALL,
            )
            .await;
        report.start_task_state = task_state(device, start_id);
        if matches!(outcome, Ok(o) if !o.triggered) {
            report.failed_at = Some("start".to_string());
            report.detail = Some("start step did not finish before the timeout".to_string());
            report.waited_seconds = started.elapsed().as_secs_f64();
            let frame = capture_bgr(device).await?;
            report.frame_png = Some(png_of(&frame)?);
            return Ok(report);
        }
    }
    report.start_task_state = task_state(device, start_id);

    tokio::time::sleep(opts.settle).await;
    let mut paused = is_paused(device, opts.pause_gap).await?;
    if !paused {
        click_pause(device).await?;
        report.pause_attempts = 2;
        tokio::time::sleep(opts.settle).await;
        paused = is_paused(device, opts.pause_gap).await?;
    }

    let frame = capture_bgr(device).await?;
    report.frame_png = Some(png_of(&frame)?);
    report.paused = paused;
    report.ok = paused;
    if paused {
        let mut detail = match report.fired_on.as_deref() {
            Some("hud") => "paused on HUD template match".to_string(),
            _ if has_template => {
                "paused on start-step completion; HUD template never matched (check hud_template/hud_roi)".to_string()
            }
            _ => "paused on start-step completion with a changed frame".to_string(),
        };
        if report.pause_attempts == 2 {
            detail.push_str("; second click needed");
        }
        report.detail = Some(detail);
    } else {
        report.failed_at = Some("pause".to_string());
        report.detail = Some(
            "screen kept changing after two pause clicks; the battle is still running".to_string(),
        );
    }
    report.waited_seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployStep {
    pub name: String,
    pub location: [i32; 2],
    #[serde(default = "default_direction")]
    pub direction: String,
    #[serde(default)]
    pub skill_usage: Option<i32>,
}

fn default_direction() -> String {
    "Right".to_string()
}

#[derive(Debug, Serialize)]
pub struct DeployResult {
    pub name: String,
    pub location: [i32; 2],
    pub direction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_error: Option<String>,
    pub frame_changed: bool,
    pub elapsed_seconds: f64,
}

pub async fn deploy_batch(
    device: &Device,
    plan: Vec<DeployStep>,
    settle: Duration,
    step_timeout: Duration,
) -> Result<Vec<DeployResult>> {
    for (i, step) in plan.iter().enumerate() {
        if !VALID_DIRECTIONS.contains(&step.direction.as_str()) {
            return Err(Error::Validation(format!(
                "deploy plan step {i} ({:?}) has invalid direction {:?}; expected one of {VALID_DIRECTIONS:?}",
                step.name, step.direction
            )));
        }
    }
    let mut results = Vec::with_capacity(plan.len());
    for step in plan {
        let t0 = Instant::now();
        let before = digest(&capture_bgr(device).await?);
        let details = json!({
            "type": "Deploy",
            "name": step.name,
            "location": step.location,
            "direction": step.direction,
        });
        let outcome = run_step(device, "action", Some(details), step_timeout).await;
        let error = match &outcome {
            StepOutcome::Done(_) => None,
            other => Some(other.describe()),
        };
        let skill_error = if let Some(usage) = step.skill_usage {
            let skill = json!({
                "type": "SkillUsage",
                "name": step.name,
                "skill_usage": usage,
            });
            let skill_outcome = run_step(device, "action", Some(skill), step_timeout).await;
            match &skill_outcome {
                StepOutcome::Done(_) => None,
                other => Some(other.describe()),
            }
        } else {
            None
        };
        tokio::time::sleep(settle).await;
        let after = capture_bgr(device).await?;
        results.push(DeployResult {
            name: step.name,
            location: step.location,
            direction: step.direction,
            error,
            skill_error,
            frame_changed: digest(&after) != before,
            elapsed_seconds: t0.elapsed().as_secs_f64(),
        });
    }
    Ok(results)
}

#[derive(Debug, Serialize)]
pub struct ResumeReport {
    pub reason: String,
    pub elapsed_seconds: f64,
    pub frames_observed: u32,
    #[serde(skip_serializing)]
    pub frame_png: Vec<u8>,
}

pub async fn resume_until(
    device: &Device,
    seconds: Duration,
    poll: Duration,
    stable_frames: u32,
) -> Result<ResumeReport> {
    let started = Instant::now();
    let deadline = started + seconds;
    click_pause(device).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut seen = 0u64;
    let mut same = 0u32;
    let mut frames = 0u32;
    loop {
        let frame = capture_bgr(device).await?;
        frames += 1;
        let d = digest(&frame);
        same = if d == seen { same + 1 } else { 0 };
        seen = d;
        if same >= stable_frames {
            return Ok(ResumeReport {
                reason: "settled".to_string(),
                elapsed_seconds: started.elapsed().as_secs_f64(),
                frames_observed: frames,
                frame_png: png_of(&frame)?,
            });
        }
        if Instant::now() + poll >= deadline {
            break;
        }
        tokio::time::sleep(poll).await;
    }
    click_pause(device).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let frame = capture_bgr(device).await?;
    Ok(ResumeReport {
        reason: "timeout_paused".to_string(),
        elapsed_seconds: started.elapsed().as_secs_f64(),
        frames_observed: frames,
        frame_png: png_of(&frame)?,
    })
}

#[derive(Debug, Serialize)]
pub struct PauseReport {
    pub paused: bool,
    #[serde(skip_serializing)]
    pub frame_png: Vec<u8>,
}

pub async fn pause(device: &Device) -> Result<PauseReport> {
    click_pause(device).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let paused = is_paused(device, PAUSE_GAP).await?;
    let frame = capture_bgr(device).await?;
    Ok(PauseReport {
        paused,
        frame_png: png_of(&frame)?,
    })
}
