use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

#[derive(Default)]
pub struct BattleState {
    pub taskchain: Option<String>,
    pub stage: Option<String>,
    pub formation_groups: Vec<String>,
    pub selected: Vec<(String, String)>,
    pub unavailable: Vec<(String, String)>,
    pub invalid_opers: Vec<String>,
    pub actions_run: u32,
    pub last_action: Option<Value>,
    pub last_action_at: Option<Instant>,
    pub started_at: Option<Instant>,
    pub parse_failed: bool,
    pub unsupported_level: Option<String>,
}

impl BattleState {
    pub fn new(taskchain: Option<String>) -> Self {
        Self {
            taskchain,
            started_at: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn note_event(&mut self, what: &str, details: &Value, now: Instant) {
        match what {
            "BattleFormation" => {
                self.formation_groups = details
                    .get("formation")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|v| v.as_str().unwrap_or("").to_string())
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "BattleFormationSelected" => {
                self.selected.push((
                    str_field(details, "selected"),
                    str_field(details, "group_name"),
                ));
            }
            "BattleFormationOperUnavailable" => {
                let entry = (
                    str_field(details, "oper_name"),
                    if details.get("requirement_type").is_some() {
                        str_field(details, "requirement_type")
                    } else {
                        "unknown".to_string()
                    },
                );
                if !self.unavailable.contains(&entry) {
                    self.unavailable.push(entry);
                }
            }
            "BattleFormationParseFailed" => self.parse_failed = true,
            "UnsupportedLevel" => {
                self.unsupported_level = Some(str_field(details, "level"));
            }
            "UserAdditionalOperInvalid" => {
                let name = str_field(details, "name");
                if !name.is_empty() && !self.invalid_opers.contains(&name) {
                    self.invalid_opers.push(name);
                }
            }
            "CopilotAction" => {
                self.actions_run += 1;
                self.last_action = Some(json!({
                    "action": details.get("action"),
                    "target": details.get("target"),
                    "doc": details.get("doc"),
                    "elapsed_time": details.get("elapsed_time"),
                }));
                self.last_action_at = Some(now);
            }
            _ => {}
        }
    }
}

fn str_field(details: &Value, key: &str) -> String {
    details
        .get(key)
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

#[derive(Serialize)]
pub struct BattleReport {
    pub status: String,
    pub diagnosis: String,
    pub problems: Vec<String>,
    pub running: bool,
    pub taskchain: Option<String>,
    pub stage: Option<String>,
    pub formation_groups: Vec<String>,
    pub selected: Vec<Value>,
    pub unavailable: Vec<Value>,
    pub invalid_opers: Vec<String>,
    pub actions_run: u32,
    pub last_action: Option<Value>,
    pub seconds_since_last_action: Option<f64>,
    pub seconds_in_battle: Option<f64>,
}

pub fn diagnose(
    battle: &BattleState,
    running: bool,
    since_action: Option<Duration>,
    in_battle: Option<Duration>,
    stall: Duration,
) -> (&'static str, String, Vec<String>) {
    if battle.started_at.is_none() {
        return (
            "no_battle",
            "No copilot battle has run on this connection.".to_string(),
            vec![],
        );
    }

    let mut problems: Vec<String> = Vec::new();

    if battle.parse_failed {
        problems.push(
            "MaaCore rejected the job file: its formation has duplicate group names. Nothing was deployed because no squad could be built."
                .to_string(),
        );
    }

    if let Some(level) = &battle.unsupported_level {
        problems.push(format!(
            "MaaCore has no tile data for stage '{level}', so it cannot map the job's deployment coordinates. The job is usually for a different stage or difficulty than the one that opened, or the stage is newer than the loaded resources."
        ));
    }

    if !battle.invalid_opers.is_empty() {
        let names = battle
            .invalid_opers
            .iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(", ");
        problems.push(format!(
            "Operator names MaaCore does not recognise: {names}. Check spelling against the client's language."
        ));
    }

    if !battle.unavailable.is_empty() {
        let listed = battle
            .unavailable
            .iter()
            .map(|(oper, req)| format!("{oper} (needs {req})"))
            .collect::<Vec<_>>()
            .join(", ");
        if battle.selected.is_empty() {
            problems.push(format!(
                "None of the job's operators could be fielded: {listed}. The squad was empty, so the battle ran with nothing deployed."
            ));
        } else {
            problems.push(format!(
                "Some operators could not be fielded: {listed}. The squad was only partly filled ({} placed).",
                battle.selected.len()
            ));
        }
    }

    if !problems.is_empty() {
        return ("problem", problems.join(" "), problems);
    }

    let stall_secs = stall.as_secs_f64();
    let stalled_before_acting = battle.actions_run == 0
        && in_battle
            .map(|d| d.as_secs_f64() > stall_secs)
            .unwrap_or(false);
    let stalled_mid_battle = since_action
        .map(|d| d.as_secs_f64() > stall_secs)
        .unwrap_or(false);

    if running && stalled_before_acting {
        let detail = format!(
            "The battle has been running {:.0}s and MaaCore has not executed a single copilot action. Formation may have succeeded but the job's first action is waiting on a condition that will not arrive.",
            in_battle.map(|d| d.as_secs_f64()).unwrap_or(0.0)
        );
        return ("stalled", detail.clone(), vec![detail]);
    }

    if running && stalled_mid_battle {
        let last = battle.last_action.clone().unwrap_or(json!({}));
        let action = last.get("action").map(repr).unwrap_or_default();
        let target = last.get("target").map(repr).unwrap_or_default();
        let detail = format!(
            "No copilot action for {:.0}s after {} action(s). The last was {action} on {target}. Actions do legitimately wait on cost and kill conditions, so raise stall_seconds if this job is simply slow.",
            since_action.map(|d| d.as_secs_f64()).unwrap_or(0.0),
            battle.actions_run
        );
        return ("stalled", detail.clone(), vec![detail]);
    }

    if battle.actions_run > 0 {
        return (
            "ok",
            format!(
                "{} copilot action(s) executed, {} operator(s) fielded.",
                battle.actions_run,
                battle.selected.len()
            ),
            vec![],
        );
    }

    (
        "ok",
        format!(
            "{} operator(s) fielded; no action executed yet.",
            battle.selected.len()
        ),
        vec![],
    )
}

fn repr(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{s}'"),
        other => other.to_string(),
    }
}

pub fn report(battle: &BattleState, running: bool, stall: Duration) -> BattleReport {
    let now = Instant::now();
    let since_action = battle.last_action_at.map(|t| now.duration_since(t));
    let in_battle = battle.started_at.map(|t| now.duration_since(t));
    let (status, diagnosis, problems) = diagnose(battle, running, since_action, in_battle, stall);
    BattleReport {
        status: status.to_string(),
        diagnosis,
        problems,
        running,
        taskchain: battle.taskchain.clone(),
        stage: battle.stage.clone(),
        formation_groups: battle.formation_groups.clone(),
        selected: battle
            .selected
            .iter()
            .map(|(oper, group)| json!({"oper": oper, "group": group}))
            .collect(),
        unavailable: battle
            .unavailable
            .iter()
            .map(|(oper, req)| json!({"oper": oper, "requirement": req}))
            .collect(),
        invalid_opers: battle.invalid_opers.clone(),
        actions_run: battle.actions_run,
        last_action: battle.last_action.clone(),
        seconds_since_last_action: since_action.map(|d| (d.as_secs_f64() * 10.0).round() / 10.0),
        seconds_in_battle: in_battle.map(|d| (d.as_secs_f64() * 10.0).round() / 10.0),
    }
}
