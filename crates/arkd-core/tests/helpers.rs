#![cfg(feature = "fake")]
#![allow(dead_code)]

use std::sync::Arc;

use arkd_core::core_api::fake::{FakeCore, FakeCoreFactory};
use arkd_core::session::MaaSession;

pub fn session() -> (MaaSession, Arc<FakeCore>) {
    session_with_events(2000)
}

pub fn session_with_events(max_events: usize) -> (MaaSession, Arc<FakeCore>) {
    let factory = FakeCoreFactory::new();
    let core = factory.core.clone();
    let session = MaaSession::open(&factory, max_events).unwrap();
    (session, core)
}

pub fn connect(session: &MaaSession) {
    session
        .connect("adb", "127.0.0.1:5555", "General", "maatouch")
        .unwrap();
}

pub fn connected() -> (MaaSession, Arc<FakeCore>) {
    let (session, core) = session();
    connect(&session);
    (session, core)
}

pub fn start_copilot(session: &MaaSession, core: &Arc<FakeCore>, task_id: i32, stage: &str) {
    session
        .append_task(
            "Copilot",
            serde_json::json!({"filename": format!("{stage}.json")}),
        )
        .unwrap();
    session.start().unwrap();
    core.emit(
        10001,
        serde_json::json!({"taskchain": "Copilot", "taskid": task_id}),
    );
}

pub fn formation_events(
    core: &Arc<FakeCore>,
    unavailable: &[(&str, &str)],
    selected: &[(&str, &str)],
    groups: &[&str],
) {
    core.emit(
        20003,
        serde_json::json!({"what": "BattleFormation", "details": {"formation": groups}}),
    );
    for (oper, req) in unavailable {
        core.emit(
            20003,
            serde_json::json!({
                "what": "BattleFormationOperUnavailable",
                "details": {"oper_name": oper, "requirement_type": req},
            }),
        );
    }
    for (oper, group) in selected {
        core.emit(
            20003,
            serde_json::json!({
                "what": "BattleFormationSelected",
                "details": {"selected": oper, "group_name": group},
            }),
        );
    }
}
