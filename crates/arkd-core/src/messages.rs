pub mod msg {
    pub const INTERNAL_ERROR: i32 = 0;
    pub const INIT_FAILED: i32 = 1;
    pub const CONNECTION_INFO: i32 = 2;
    pub const ALL_TASKS_COMPLETED: i32 = 3;
    pub const ASYNC_CALL_INFO: i32 = 4;
    pub const DESTROYED: i32 = 5;

    pub const TASK_CHAIN_ERROR: i32 = 10000;
    pub const TASK_CHAIN_START: i32 = 10001;
    pub const TASK_CHAIN_COMPLETED: i32 = 10002;
    pub const TASK_CHAIN_EXTRA_INFO: i32 = 10003;
    pub const TASK_CHAIN_STOPPED: i32 = 10004;

    pub const SUB_TASK_ERROR: i32 = 20000;
    pub const SUB_TASK_START: i32 = 20001;
    pub const SUB_TASK_COMPLETED: i32 = 20002;
    pub const SUB_TASK_EXTRA_INFO: i32 = 20003;
    pub const SUB_TASK_STOPPED: i32 = 20004;

    pub const REPORT_REQUEST: i32 = 30000;
}

const NAMES: &[(i32, &str)] = &[
    (msg::INTERNAL_ERROR, "INTERNAL_ERROR"),
    (msg::INIT_FAILED, "INIT_FAILED"),
    (msg::CONNECTION_INFO, "CONNECTION_INFO"),
    (msg::ALL_TASKS_COMPLETED, "ALL_TASKS_COMPLETED"),
    (msg::ASYNC_CALL_INFO, "ASYNC_CALL_INFO"),
    (msg::DESTROYED, "DESTROYED"),
    (msg::TASK_CHAIN_ERROR, "TASK_CHAIN_ERROR"),
    (msg::TASK_CHAIN_START, "TASK_CHAIN_START"),
    (msg::TASK_CHAIN_COMPLETED, "TASK_CHAIN_COMPLETED"),
    (msg::TASK_CHAIN_EXTRA_INFO, "TASK_CHAIN_EXTRA_INFO"),
    (msg::TASK_CHAIN_STOPPED, "TASK_CHAIN_STOPPED"),
    (msg::SUB_TASK_ERROR, "SUB_TASK_ERROR"),
    (msg::SUB_TASK_START, "SUB_TASK_START"),
    (msg::SUB_TASK_COMPLETED, "SUB_TASK_COMPLETED"),
    (msg::SUB_TASK_EXTRA_INFO, "SUB_TASK_EXTRA_INFO"),
    (msg::SUB_TASK_STOPPED, "SUB_TASK_STOPPED"),
    (msg::REPORT_REQUEST, "REPORT_REQUEST"),
];

const ERROR_IDS: &[i32] = &[
    msg::INTERNAL_ERROR,
    msg::INIT_FAILED,
    msg::TASK_CHAIN_ERROR,
    msg::SUB_TASK_ERROR,
];

const SIGNIFICANT_IDS: &[i32] = &[
    msg::INTERNAL_ERROR,
    msg::INIT_FAILED,
    msg::CONNECTION_INFO,
    msg::ALL_TASKS_COMPLETED,
    msg::TASK_CHAIN_ERROR,
    msg::TASK_CHAIN_START,
    msg::TASK_CHAIN_COMPLETED,
    msg::TASK_CHAIN_STOPPED,
    msg::SUB_TASK_ERROR,
    msg::SUB_TASK_STOPPED,
];

pub const BATTLE_WHAT: &[&str] = &[
    "BattleFormation",
    "BattleFormationSelected",
    "BattleFormationOperUnavailable",
    "BattleFormationParseFailed",
    "UnsupportedLevel",
    "CopilotAction",
    "UserAdditionalOperInvalid",
];

pub const BATTLE_FAILURE_WHAT: &[&str] = &[
    "BattleFormationOperUnavailable",
    "BattleFormationParseFailed",
    "UnsupportedLevel",
    "UserAdditionalOperInvalid",
];

pub fn message_name(id: i32) -> String {
    NAMES
        .iter()
        .find(|(mid, _)| *mid == id)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| format!("Unknown({id})"))
}

pub fn is_error(id: i32) -> bool {
    ERROR_IDS.contains(&id)
}

pub fn is_significant(id: i32, payload: &serde_json::Value) -> bool {
    if SIGNIFICANT_IDS.contains(&id) {
        return true;
    }
    if id == msg::SUB_TASK_EXTRA_INFO {
        let what = payload.get("what").and_then(|v| v.as_str()).unwrap_or("");
        return BATTLE_WHAT.contains(&what);
    }
    false
}
