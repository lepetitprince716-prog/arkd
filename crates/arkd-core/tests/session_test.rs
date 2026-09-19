#![cfg(feature = "fake")]

mod helpers;

use std::sync::Arc;
use std::time::{Duration, Instant};

use arkd_core::MaaCoreApi;
use arkd_core::error::Error;
use arkd_core::session::{MaaSession, WaitCondition};
use helpers::*;

#[test]
fn connect_applies_touch_mode_before_connecting() {
    let (session, core) = session();
    session
        .connect("adb", "127.0.0.1:5555", "General", "maatouch")
        .unwrap();
    let opts = core.instance_options.lock().unwrap();
    assert!(opts.iter().any(|(k, v)| *k == 2 && v == "maatouch"));
    assert_eq!(
        core.connect_calls.lock().unwrap()[0],
        (
            "adb".to_string(),
            "127.0.0.1:5555".to_string(),
            "General".to_string()
        )
    );
}

#[test]
fn failed_connect_explains_what_to_check() {
    let (session, core) = session();
    core.set_connect_ok(false);
    let err = session
        .connect("adb", "127.0.0.1:9999", "General", "maatouch")
        .unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, Error::DeviceConnection(_)));
    assert!(msg.contains("127.0.0.1:9999"), "{msg}");
    assert!(msg.contains("device offline"), "{msg}");
    assert!(msg.contains("adb devices"), "{msg}");
}

#[test]
fn operations_require_a_connection() {
    let (session, _core) = session();
    let err = session
        .append_task("Depot", serde_json::json!({}))
        .unwrap_err();
    assert!(matches!(err, Error::NotConnected(_)));
    assert!(err.to_string().contains("device_connect"));
}

#[test]
fn append_task_validates_before_reaching_the_core() {
    let (session, core) = connected();
    assert!(
        session
            .append_task("Fight", serde_json::json!({"stagee": "1-7"}))
            .is_err()
    );
    assert!(core.tasks.lock().unwrap().is_empty());
}

#[test]
fn append_task_records_the_queue() {
    let (session, core) = connected();
    let task = session
        .append_task("Fight", serde_json::json!({"stage": "1-7", "times": 3}))
        .unwrap();
    assert_eq!(task.state, "queued");
    let tasks = core.tasks.lock().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].0, 1);
    assert_eq!(tasks[0].1, "Fight");
    assert_eq!(session.tasks()[0]["task_type"], "Fight");
}

#[test]
fn append_task_reports_a_core_refusal() {
    let (session, core) = session();
    core.refuse_append(true);
    connect(&session);
    let err = session
        .append_task("Fight", serde_json::json!({"stage": "XX-9"}))
        .unwrap_err();
    assert!(
        err.to_string().contains("AsstAppendTask returned 0"),
        "{err}"
    );
}

#[test]
fn set_task_params_rejects_an_unknown_id() {
    let (session, _core) = connected();
    session.append_task("Depot", serde_json::json!({})).unwrap();
    let err = session
        .set_task_params(99, serde_json::json!({}))
        .unwrap_err();
    assert!(err.to_string().contains("Queued ids: 1"), "{err}");
}

#[test]
fn set_task_params_validates_against_the_original_type() {
    let (session, _core) = connected();
    let task = session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    assert!(
        session
            .set_task_params(task.task_id, serde_json::json!({"facility": ["Mfg"]}))
            .is_err()
    );
}

#[test]
fn set_task_params_updates_the_core() {
    let (session, core) = connected();
    let task = session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session
        .set_task_params(
            task.task_id,
            serde_json::json!({"stage": "1-7", "times": 9}),
        )
        .unwrap();
    let tasks = core.tasks.lock().unwrap();
    assert_eq!(tasks[0].2, serde_json::json!({"stage": "1-7", "times": 9}));
}

#[test]
fn start_requires_something_queued() {
    let (session, _core) = connected();
    let err = session.start().unwrap_err();
    assert!(err.to_string().contains("maa_append"), "{err}");
}

#[test]
fn start_moves_tasks_to_running() {
    let (session, _core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.append_task("Award", serde_json::json!({})).unwrap();
    let result = session.start().unwrap();
    assert_eq!(result.started, vec![1, 2]);
    assert_eq!(result.task_types, vec!["Fight", "Award"]);
    assert!(session.tasks().iter().all(|t| t["state"] == "running"));
}

#[test]
fn run_completion_is_folded_into_task_state() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    core.finish_all();
    assert!(session.tasks().iter().all(|t| t["state"] == "completed"));
    let status = session.status();
    assert_eq!(status.task_counts.get("completed"), Some(&1));
}

#[test]
fn stop_clears_the_queue_and_marks_tasks_stopped() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    let result = session.stop().unwrap();
    assert!(result.stopped);
    assert_eq!(result.cleared_tasks, 1);
    assert!(session.tasks().is_empty());
    assert!(core.tasks.lock().unwrap().is_empty());
}

#[test]
fn task_chain_error_is_recorded_as_last_error() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    core.emit(
        10000,
        serde_json::json!({"taskchain": "Fight", "taskid": 1, "why": "stage not found"}),
    );
    let status = session.status();
    assert_eq!(status.last_error.as_deref(), Some("stage not found"));
    assert_eq!(status.tasks[0]["state"], "error");
}

#[test]
fn connection_loss_is_reflected_in_status() {
    let (session, core) = connected();
    core.emit(
        2,
        serde_json::json!({"what": "Disconnect", "why": "emulator closed"}),
    );
    assert!(session.status().connection.is_none());
}

#[test]
fn events_are_returned_newest_after_the_cursor() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    let first = session.events(0, 10, true, false);
    assert!(first.count >= 1);
    assert_eq!(first.next_seq, first.latest_seq);

    core.finish_all();
    let second = session.events(first.next_seq, 10, true, false);
    assert!(second.count >= 1);
    assert!(
        second
            .events
            .iter()
            .all(|e| e["seq"].as_u64().unwrap() > first.next_seq)
    );
}

#[test]
fn events_limit_sets_has_more() {
    let (session, core) = connected();
    for _ in 0..5 {
        core.emit(
            10002,
            serde_json::json!({"taskchain": "Fight", "taskid": 1}),
        );
    }
    let page = session.events(0, 2, true, false);
    assert_eq!(page.count, 2);
    assert!(page.has_more);
}

#[test]
fn significant_filter_drops_subtask_chatter() {
    let (session, core) = connected();
    core.emit(
        20003,
        serde_json::json!({"what": "StageDrops", "details": {}}),
    );
    core.emit(
        10002,
        serde_json::json!({"taskchain": "Fight", "taskid": 1}),
    );
    let filtered = session.events(0, 50, true, false);
    assert!(
        filtered
            .events
            .iter()
            .all(|e| e["message"] != "SUB_TASK_EXTRA_INFO")
    );
    let unfiltered = session.events(0, 50, false, false);
    assert!(
        unfiltered
            .events
            .iter()
            .any(|e| e["message"] == "SUB_TASK_EXTRA_INFO")
    );
}

#[test]
fn payload_is_omitted_unless_requested() {
    let (session, core) = connected();
    core.emit(
        10000,
        serde_json::json!({"taskchain": "Fight", "taskid": 1, "why": "boom"}),
    );
    let lean = session.events(0, 10, true, false);
    let last = lean.events.last().unwrap();
    assert!(last.get("payload").is_none());
    assert_eq!(last["why"], "boom");
    let full = session.events(0, 10, true, true);
    assert_eq!(full.events.last().unwrap()["payload"]["taskchain"], "Fight");
}

#[test]
fn ring_buffer_reports_dropped_events() {
    let (session, core) = session_with_events(5);
    connect(&session);
    for index in 0..20 {
        core.emit(
            10002,
            serde_json::json!({"taskchain": "Fight", "taskid": index}),
        );
    }
    let page = session.events(1, 50, false, false);
    assert!(page.dropped > 0);
}

#[test]
fn malformed_callback_json_is_kept_not_dropped() {
    let (session, core) = connected();
    core.emit_raw(10003, "not json at all");
    let events = session.events(0, 50, false, true);
    assert_eq!(
        events.events.last().unwrap()["payload"]["raw"],
        "not json at all"
    );
}

#[test]
fn screenshot_captures_a_fresh_frame_by_default() {
    let (session, core) = session();
    core.set_image_png(Some(b"stale".to_vec()));
    core.set_fresh_image_png(Some(b"\x89PNGfresh".to_vec()));
    connect(&session);
    assert_eq!(session.screenshot_png(true).unwrap(), b"\x89PNGfresh");
    assert_eq!(*core.screencap_calls.lock().unwrap(), 1);
}

#[test]
fn screenshot_can_reuse_the_cached_frame() {
    let (session, core) = session();
    core.set_image_png(Some(b"stale".to_vec()));
    core.set_fresh_image_png(Some(b"\x89PNGfresh".to_vec()));
    connect(&session);
    assert_eq!(session.screenshot_png(false).unwrap(), b"stale");
    assert_eq!(*core.screencap_calls.lock().unwrap(), 0);
}

#[test]
fn screenshot_without_a_frame_explains_why() {
    let (session, core) = session();
    core.set_image_png(None);
    connect(&session);
    let err = session.screenshot_png(true).unwrap_err();
    assert!(matches!(err, Error::NotConnected(_)));
    assert!(err.to_string().contains("no frame to return"), "{err}");
}

#[test]
fn back_to_home_goes_through_to_the_core() {
    let (session, core) = connected();
    session.back_to_home().unwrap();
    assert_eq!(*core.back_to_home_calls.lock().unwrap(), 1);
}

#[test]
fn status_reports_core_and_connection() {
    let (session, _core) = connected();
    let status = session.status();
    assert!(status.core_loaded);
    assert_eq!(status.core_version.as_deref(), Some("v5.99.0-fake"));
    assert!(status.connected);
}

#[test]
fn click_reaches_the_core() {
    let (session, core) = connected();
    session.click(640, 360).unwrap();
    assert_eq!(core.clicks.lock().unwrap()[0], (640, 360));
}

#[test]
fn click_requires_a_connection() {
    let (session, _core) = session();
    assert!(matches!(
        session.click(1, 1).unwrap_err(),
        Error::NotConnected(_)
    ));
}

// -- waiting -----------------------------------------------------------------

#[tokio::test]
async fn wait_returns_at_once_when_the_condition_already_holds() {
    let (session, _core) = connected();
    let started = Instant::now();
    let outcome = session
        .wait(
            WaitCondition::AllDone,
            None,
            0,
            Duration::from_secs(5),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(outcome.triggered);
    assert_eq!(outcome.reason, "all_done");
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn wait_wakes_on_a_callback_instead_of_timing_out() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    let short = session
        .wait(
            WaitCondition::AllDone,
            None,
            0,
            Duration::from_millis(200),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!short.triggered);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        core.finish_all();
    });
    let started = Instant::now();
    let outcome = session
        .wait(
            WaitCondition::AllDone,
            None,
            0,
            Duration::from_secs(10),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert!(outcome.triggered);
    assert_eq!(outcome.reason, "all_done");
    assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");
}

#[tokio::test]
async fn wait_latency_variant_300ms() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    core.set_auto_finish(Duration::from_millis(300));
    session.start().unwrap();
    let started = Instant::now();
    let outcome = session
        .wait(
            WaitCondition::AllDone,
            None,
            0,
            Duration::from_secs(30),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert!(outcome.triggered);
    assert_eq!(outcome.reason, "all_done");
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

#[tokio::test]
async fn wait_times_out_cleanly() {
    let (session, _core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    let started = Instant::now();
    let outcome = session
        .wait(
            WaitCondition::AllDone,
            None,
            0,
            Duration::from_millis(300),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!outcome.triggered);
    assert!(started.elapsed() >= Duration::from_millis(300));
}

#[tokio::test]
async fn wait_for_a_specific_task() {
    let (session, core) = connected();
    let task = session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    let other = session.append_task("Award", serde_json::json!({})).unwrap();
    session.start().unwrap();
    core.emit(
        10002,
        serde_json::json!({"taskchain": "Fight", "taskid": task.task_id}),
    );
    let outcome = session
        .wait(
            WaitCondition::TaskDone,
            Some(task.task_id),
            0,
            Duration::from_secs(1),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert_eq!(outcome.reason, "task_completed");
    let pending = session
        .wait(
            WaitCondition::TaskDone,
            Some(other.task_id),
            0,
            Duration::from_millis(200),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!pending.triggered);
}

#[tokio::test]
async fn wait_for_task_done_needs_a_task_id() {
    let (session, _core) = connected();
    let err = session
        .wait(
            WaitCondition::TaskDone,
            None,
            0,
            Duration::from_secs(1),
            Duration::from_secs(90),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
    assert!(err.to_string().contains("needs a task_id"), "{err}");
}

#[tokio::test]
async fn wait_for_error_catches_a_failing_chain() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    let early = session
        .wait(
            WaitCondition::Error,
            None,
            0,
            Duration::from_millis(200),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!early.triggered);
    core.emit(
        10000,
        serde_json::json!({"taskchain": "Fight", "taskid": 1, "why": "stage not found"}),
    );
    let outcome = session
        .wait(
            WaitCondition::Error,
            None,
            0,
            Duration::from_secs(1),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome.reason.as_str(),
        "error_event" | "task_error"
    ));
}

#[tokio::test]
async fn wait_for_any_event_ignores_subtask_chatter() {
    let (session, core) = connected();
    let seq = session.status().latest_event_seq;
    core.emit(20003, serde_json::json!({"what": "StageDrops"}));
    let quiet = session
        .wait(
            WaitCondition::AnyEvent,
            None,
            seq,
            Duration::from_millis(200),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!quiet.triggered);
    core.emit(
        10002,
        serde_json::json!({"taskchain": "Fight", "taskid": 1}),
    );
    let outcome = session
        .wait(
            WaitCondition::AnyEvent,
            None,
            seq,
            Duration::from_secs(1),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert_eq!(outcome.reason, "event");
}

#[tokio::test]
async fn wait_only_counts_events_after_the_cursor() {
    let (session, core) = connected();
    core.emit(
        10002,
        serde_json::json!({"taskchain": "Fight", "taskid": 1}),
    );
    let seq = session.status().latest_event_seq;
    let hit = session
        .wait(
            WaitCondition::AnyEvent,
            None,
            0,
            Duration::from_secs(1),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert_eq!(hit.reason, "event");
    let quiet = session
        .wait(
            WaitCondition::AnyEvent,
            None,
            seq,
            Duration::from_millis(200),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!quiet.triggered);
}

#[test]
fn wait_rejects_an_unknown_condition() {
    assert!("whenever".parse::<WaitCondition>().is_err());
}

#[tokio::test]
async fn a_wait_does_not_block_other_calls() {
    let (session, core) = connected();
    session
        .append_task("Fight", serde_json::json!({"stage": "1-7"}))
        .unwrap();
    session.start().unwrap();
    let waiter = {
        let session = session.clone();
        tokio::spawn(async move {
            session
                .wait(
                    WaitCondition::AllDone,
                    None,
                    0,
                    Duration::from_secs(5),
                    Duration::from_secs(90),
                )
                .await
        })
    };
    let started = Instant::now();
    assert!(session.status().running);
    assert!(started.elapsed() < Duration::from_secs(1));
    core.finish_all();
    let outcome = waiter.await.unwrap().unwrap();
    assert_eq!(outcome.reason, "all_done");
}

#[tokio::test]
async fn no_lost_wakeup_when_event_lands_during_the_check() {
    let (session, core) = connected();
    core.set_running(true);
    let core2 = core.clone();
    core.set_running_hook(Arc::new(move || {
        core2.set_running(false);
        core2.emit(
            3,
            serde_json::json!({"taskchain": "", "uuid": "fake-uuid", "finished_tasks": []}),
        );
    }));
    let outcome = session
        .wait(
            WaitCondition::Idle,
            None,
            0,
            Duration::from_secs(5),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(outcome.triggered);
    assert_eq!(outcome.reason, "idle");
    assert!(
        session
            .events(0, 50, false, false)
            .events
            .iter()
            .any(|e| e["message"] == "ALL_TASKS_COMPLETED")
    );
}

// -- battle diagnosis ---------------------------------------------------------

#[test]
fn no_battle_before_one_runs() {
    let (session, _core) = connected();
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "no_battle");
    assert!(state.problems.is_empty());
}

#[test]
fn a_squad_nobody_could_fill_is_diagnosed() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(
        &core,
        &[("Thorns", "elite"), ("Mlynar", "level")],
        &[],
        &["Group A"],
    );
    core.emit(
        10002,
        serde_json::json!({"taskchain": "Copilot", "taskid": 1}),
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "problem");
    assert!(
        state
            .diagnosis
            .contains("None of the job's operators could be fielded")
    );
    assert!(state.diagnosis.contains("Thorns (needs elite)"));
    assert!(state.diagnosis.contains("Mlynar (needs level)"));
    assert!(state.selected.is_empty());
    assert_eq!(state.actions_run, 0);
    assert_eq!(session.tasks()[0]["state"], "completed");
}

#[test]
fn a_partly_filled_squad_reads_differently() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(
        &core,
        &[("Thorns", "module")],
        &[("Ptilopsis", "Group A")],
        &["Group A"],
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "problem");
    assert!(state.diagnosis.contains("only partly filled"));
    assert!(state.diagnosis.contains("1 placed"));
}

#[test]
fn an_unsupported_stage_is_diagnosed() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    core.emit(
        20003,
        serde_json::json!({"what": "UnsupportedLevel", "details": {"level": "SV-9"}}),
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "problem");
    assert!(state.diagnosis.contains("'SV-9'"));
    assert!(state.diagnosis.contains("no tile data"));
}

#[test]
fn an_unparseable_job_is_diagnosed() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    core.emit(
        20003,
        serde_json::json!({"what": "BattleFormationParseFailed", "details": {}}),
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "problem");
    assert!(state.diagnosis.contains("duplicate group names"));
}

#[test]
fn unknown_operator_names_are_diagnosed() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    core.emit(
        20000,
        serde_json::json!({"what": "UserAdditionalOperInvalid", "details": {"name": "Nonexistent"}}),
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "problem");
    assert!(state.diagnosis.contains("'Nonexistent'"));
}

#[test]
fn a_healthy_battle_reads_ok() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(
        &core,
        &[],
        &[("Ptilopsis", "Group A"), ("Thorns", "Group B")],
        &["Group A"],
    );
    for index in 0..3 {
        core.emit(
            20003,
            serde_json::json!({
                "what": "CopilotAction",
                "details": {"action": "Deploy", "target": format!("Oper{index}"), "doc": "", "elapsed_time": index * 1000},
            }),
        );
    }
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.status, "ok");
    assert_eq!(state.actions_run, 3);
    assert_eq!(state.last_action.unwrap()["target"], "Oper2");
    assert!(state.seconds_since_last_action.is_some());
}

#[test]
fn battle_state_resets_between_battles() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(&core, &[("Thorns", "elite")], &[], &["Group A"]);
    assert_eq!(
        session.battle_state(Duration::from_secs(90)).status,
        "problem"
    );
    core.emit(
        10001,
        serde_json::json!({"taskchain": "Copilot", "taskid": 2}),
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert!(state.unavailable.is_empty());
    assert_eq!(state.status, "ok");
}

#[test]
fn battle_events_survive_the_significance_filter() {
    let (session, core) = connected();
    let seq = session.status().latest_event_seq;
    core.emit(
        20003,
        serde_json::json!({"what": "StageDrops", "details": {}}),
    );
    core.emit(
        20003,
        serde_json::json!({
            "what": "BattleFormationOperUnavailable",
            "details": {"oper_name": "Thorns", "requirement_type": "elite"},
        }),
    );
    let page = session.events(seq, 50, true, false);
    let whats: Vec<&str> = page
        .events
        .iter()
        .filter_map(|e| e.get("what").and_then(|v| v.as_str()))
        .collect();
    assert!(whats.contains(&"BattleFormationOperUnavailable"));
    assert!(!whats.contains(&"StageDrops"));
    let entry = page
        .events
        .iter()
        .find(|e| e["what"] == "BattleFormationOperUnavailable")
        .unwrap();
    assert_eq!(entry["details"]["oper_name"], "Thorns");
}

#[test]
fn a_formation_failure_shows_up_as_last_error() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(&core, &[("Thorns", "elite")], &[], &["Group A"]);
    assert!(
        session
            .status()
            .last_error
            .unwrap()
            .contains("BattleFormationOperUnavailable")
    );
}

#[tokio::test]
async fn waiting_for_a_battle_problem() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    let quiet = session
        .wait(
            WaitCondition::BattleProblem,
            None,
            0,
            Duration::from_millis(200),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert!(!quiet.triggered);
    let core2 = core.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        formation_events(&core2, &[("Thorns", "elite")], &[], &["Group A"]);
    });
    let started = Instant::now();
    let outcome = session
        .wait(
            WaitCondition::BattleProblem,
            None,
            0,
            Duration::from_secs(10),
            Duration::from_secs(90),
        )
        .await
        .unwrap();
    assert_eq!(outcome.reason, "battle_problem");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_stall_is_detected_after_the_threshold() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(&core, &[], &[("Ptilopsis", "Group A")], &["Group A"]);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(session.battle_state(Duration::from_secs(60)).status, "ok");
    let state = session.battle_state(Duration::from_millis(20));
    assert_eq!(state.status, "stalled");
    assert!(
        state
            .diagnosis
            .contains("not executed a single copilot action")
    );
}

#[test]
fn a_mid_battle_stall_names_the_last_action() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(&core, &[], &[("Ptilopsis", "Group A")], &["Group A"]);
    core.emit(
        20003,
        serde_json::json!({"what": "CopilotAction", "details": {"action": "Deploy", "target": "Ptilopsis"}}),
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(session.battle_state(Duration::from_secs(60)).status, "ok");
    let state = session.battle_state(Duration::from_millis(20));
    assert_eq!(state.status, "stalled");
    assert!(state.diagnosis.contains("'Deploy'"));
    assert!(state.diagnosis.contains("'Ptilopsis'"));
}

#[test]
fn a_stall_only_counts_while_running() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "1-7");
    formation_events(&core, &[], &[("Ptilopsis", "Group A")], &["Group A"]);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        session.battle_state(Duration::from_millis(20)).status,
        "stalled"
    );
    core.stop().unwrap();
    assert_eq!(session.battle_state(Duration::from_millis(20)).status, "ok");
}

// -- manual battle control -----------------------------------------------------

#[test]
fn single_step_queues_and_starts() {
    let (session, core) = connected();
    core.stop().unwrap();
    session
        .single_step("stage", Some(serde_json::json!({"stage_name": "1-7"})))
        .unwrap();
    assert_eq!(session.tasks()[0]["task_type"], "SingleStep");
    assert_eq!(
        session.tasks()[0]["params"],
        serde_json::json!({"type": "copilot", "subtype": "stage", "details": {"stage_name": "1-7"}})
    );
}

#[test]
fn single_step_refuses_while_a_run_is_in_progress() {
    let (session, core) = connected();
    session
        .append_task("Copilot", serde_json::json!({"filename": "job.json"}))
        .unwrap();
    session.start().unwrap();
    assert!(core.running());
    let err = session.single_step("start", None).unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
    assert!(err.to_string().contains("maa_stop first"), "{err}");
}

#[test]
fn job_dir_rejects_paths_that_escape_it() {
    let dir = std::env::temp_dir().join(format!("arkd-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let factory = arkd_core::core_api::fake::FakeCoreFactory::new();
    let session = MaaSession::open_with_job_dir(&factory, 2000, Some(dir.clone())).unwrap();
    connect(&session);
    let err = session
        .append_task(
            "Copilot",
            serde_json::json!({"filename": "../../etc/passwd"}),
        )
        .unwrap_err();
    assert!(matches!(err, Error::Validation(_)));
    assert!(err.to_string().contains("ARKD_JOB_DIR"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn job_dir_resolves_relative_paths() {
    let dir = std::env::temp_dir().join(format!("arkd-jobs-rel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.json"), "{}").unwrap();
    let factory = arkd_core::core_api::fake::FakeCoreFactory::new();
    let core = factory.core.clone();
    let session = MaaSession::open_with_job_dir(&factory, 2000, Some(dir.clone())).unwrap();
    connect(&session);
    session
        .append_task("Copilot", serde_json::json!({"filename": "a.json"}))
        .unwrap();
    let tasks = core.tasks.lock().unwrap();
    let expected = dir.join("a.json").canonicalize().unwrap();
    assert_eq!(
        tasks[0].2["filename"],
        serde_json::Value::String(expected.to_string_lossy().into_owned())
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn without_job_dir_paths_pass_through() {
    let (session, core) = connected();
    session
        .append_task(
            "Copilot",
            serde_json::json!({"filename": "anywhere/a.json"}),
        )
        .unwrap();
    let tasks = core.tasks.lock().unwrap();
    assert_eq!(tasks[0].2["filename"], "anywhere/a.json");
}

#[test]
fn zero_elapsed_is_reported_as_zero_not_missing() {
    let (session, core) = connected();
    start_copilot(&session, &core, 1, "LS-1");
    core.emit(
        20003,
        serde_json::json!({"what": "CopilotAction", "details": {"action": "Deploy", "target": "X"}}),
    );
    let state = session.battle_state(Duration::from_secs(90));
    assert_eq!(state.actions_run, 1);
    let since_action = state
        .seconds_since_last_action
        .expect("a just-run action must report 0-ish elapsed, not missing");
    assert!(since_action < 1.0, "{since_action}");
    let in_battle = state
        .seconds_in_battle
        .expect("a just-started battle must report 0-ish elapsed, not missing");
    assert!(in_battle < 1.0, "{in_battle}");
}
