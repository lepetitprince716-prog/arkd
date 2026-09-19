#![cfg(feature = "fake")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use arkd_core::battle::{self, DeployStep, StartPausedOptions};
use arkd_core::config::{Config, DeviceConfig};
use arkd_core::core_api::fake::{FakeCore, FakeCoreFactory};
use arkd_core::device::{Device, DeviceRegistry};

fn pair(seed: u8) -> (Vec<u8>, Vec<u8>) {
    let mut img = image::RgbImage::new(64, 36);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let v = ((x * 31 + y * 17 + seed as u32 * 97).wrapping_mul(2654435761) % 251) as u8;
        *p = image::Rgb([v, seed, 255u8.wrapping_sub(v)]);
    }
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(img.clone())
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut bgr = Vec::with_capacity(64 * 36 * 3);
    for px in img.pixels() {
        bgr.push(px[2]);
        bgr.push(px[1]);
        bgr.push(px[0]);
    }
    (png, bgr)
}

fn frames(seeds: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    seeds.iter().map(|&s| pair(s)).collect()
}

fn setup(hud_template: Option<&std::path::Path>) -> (Arc<Device>, Arc<FakeCore>) {
    let factory = FakeCoreFactory::new();
    let core = factory.core.clone();
    let config = Config {
        devices: vec![DeviceConfig {
            name: "playcover".to_string(),
            default: true,
            device_size: Some((1920, 1080)),
            hud_template: hud_template.map(|p| p.to_path_buf()),
            hud_roi: (40, 4, 16, 12),
            ..DeviceConfig::default()
        }],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, Arc::new(factory)).unwrap();
    (registry.get(None).unwrap(), core)
}

fn opts(stage: &str, timeout_ms: u64) -> StartPausedOptions {
    StartPausedOptions {
        stage: stage.to_string(),
        poll: Duration::from_millis(30),
        timeout: Duration::from_millis(timeout_ms),
        pause_gap: Duration::from_millis(30),
        settle: Duration::from_millis(30),
    }
}

const PAUSE_CLICK: (i32, i32) = (1815, 82);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_paused_happy_path_without_template() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.set_auto_finish(Duration::from_millis(40));
        // leading frame is consumed by the first dimensions probe; then the
        // formation screen, a battlefield frame, and identical paused frames.
        let mut seeds = vec![200, 1, 1, 2, 3];
        seeds.resize(20, 9);
        core.set_frames(frames(&seeds));

        let report = battle::start_paused(&device, opts("LS-1", 5000))
            .await
            .unwrap();
        assert!(report.ok, "report: {report:?}");
        assert!(report.paused);
        assert_eq!(report.hud_detected, None);
        assert_eq!(report.start_task_state.as_deref(), Some("completed"));
        assert!(
            report
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("start-step"),
            "detail: {:?}",
            report.detail
        );
        assert_eq!(core.clicks.lock().unwrap().as_slice(), &[PAUSE_CLICK]);
        let img = image::load_from_memory(&report.frame_png.unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (64, 36));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_paused_with_template_fires_before_task_completes() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let template_path =
            std::env::temp_dir().join(format!("arkd-hud-test-{}-tpl.png", std::process::id()));
        let (tpl_png, _) = pair(77);
        std::fs::write(&template_path, &tpl_png).unwrap();

        let (device, core) = setup(Some(&template_path));
        device.ensure_connected().await.unwrap();
        // the start task stays running well past the moment the HUD frame appears
        core.set_auto_finish(Duration::from_millis(800));
        let mut seeds = vec![200, 1, 1, 77];
        seeds.resize(20, 9);
        core.set_frames(frames(&seeds));

        let report = battle::start_paused(&device, opts("LS-1", 8000))
            .await
            .unwrap();
        std::fs::remove_file(&template_path).ok();
        assert!(report.ok, "report: {report:?}");
        assert_eq!(report.hud_detected, Some(true));
        assert!(
            report.detail.as_deref().unwrap_or_default().contains("HUD"),
            "detail: {:?}",
            report.detail
        );
        assert_eq!(core.clicks.lock().unwrap().as_slice(), &[PAUSE_CLICK]);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_paused_reports_stage_rejection() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.refuse_append(true);

        let report = battle::start_paused(&device, opts("LS-1", 2000))
            .await
            .unwrap();
        assert!(!report.ok);
        assert_eq!(report.failed_at.as_deref(), Some("stage"));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_paused_times_out_when_battlefield_never_renders() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.set_auto_finish(Duration::from_millis(30));
        core.set_frames(frames(&[200, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5]));

        let report = battle::start_paused(&device, opts("LS-1", 600))
            .await
            .unwrap();
        assert!(!report.ok);
        assert_eq!(report.failed_at.as_deref(), Some("wait-for-battle"));
        assert_eq!(core.clicks.lock().unwrap().len(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_paused_retries_the_pause_click_once() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.set_auto_finish(Duration::from_millis(40));
        let mut seeds = vec![200, 1, 1, 2, 3];
        seeds.resize(30, 4);
        core.set_frames(frames(&seeds));

        let count = Arc::new(AtomicUsize::new(0));
        let core2 = core.clone();
        let count2 = count.clone();
        core.set_click_hook(Arc::new(move |_, _| {
            let n = count2.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                let seeds: Vec<u8> = (0..40u8).map(|i| 10 + i).collect();
                core2.set_frames(frames(&seeds));
            } else {
                core2.set_frames(frames(&[9, 9, 9, 9, 9, 9, 9, 9]));
            }
        }));

        let report = battle::start_paused(&device, opts("LS-1", 5000))
            .await
            .unwrap();
        assert!(report.ok, "report: {report:?}");
        assert!(report.paused);
        assert_eq!(core.clicks.lock().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_batch_reports_frame_change_and_continues_after_rejection() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.set_auto_finish(Duration::from_millis(20));
        // the first append (step 1's Deploy) is refused; the batch continues
        core.fail_next_appends(1);
        // before/after pairs: [A,B] changed, [C,C] unchanged, [D,E] changed
        let mut seeds = vec![200, 1, 2, 3, 3, 4, 5];
        seeds.resize(20, 5);
        core.set_frames(frames(&seeds));

        let plan = vec![
            DeployStep {
                name: "Amiya".into(),
                location: [4, 5],
                direction: "Right".into(),
                skill_usage: None,
            },
            DeployStep {
                name: "Exusiai".into(),
                location: [6, 3],
                direction: "Left".into(),
                skill_usage: None,
            },
            DeployStep {
                name: "Texas".into(),
                location: [2, 2],
                direction: "上".into(),
                skill_usage: Some(2),
            },
        ];
        let results = battle::deploy_batch(
            &device,
            plan,
            Duration::from_millis(10),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].error.is_some());
        assert!(results[0].frame_changed);
        assert!(results[1].error.is_none());
        assert!(!results[1].frame_changed);
        assert!(results[2].error.is_none());
        assert!(results[2].frame_changed);
        assert!(results[2].skill_error.is_none());
        let tasks = core.tasks.lock().unwrap();
        assert!(
            tasks
                .iter()
                .any(|(_, t, p)| t == "SingleStep" && p["details"]["type"] == "SkillUsage")
        );
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_paused_falls_back_to_task_completion_when_template_never_matches() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let template_path =
            std::env::temp_dir().join(format!("arkd-hud-test-{}-fb.png", std::process::id()));
        let (tpl_png, _) = pair(77);
        std::fs::write(&template_path, &tpl_png).unwrap();

        let (device, core) = setup(Some(&template_path));
        device.ensure_connected().await.unwrap();
        core.set_auto_finish(Duration::from_millis(40));
        // no frame ever carries the template; the start task still completes
        let mut seeds = vec![200, 1, 1, 2, 3];
        seeds.resize(20, 9);
        core.set_frames(frames(&seeds));

        let report = battle::start_paused(&device, opts("LS-1", 5000))
            .await
            .unwrap();
        std::fs::remove_file(&template_path).ok();
        assert!(report.ok, "report: {report:?}");
        assert_eq!(report.hud_detected, Some(false));
        assert!(
            report
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("never matched"),
            "detail: {:?}",
            report.detail
        );
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_batch_marks_a_step_whose_task_errored() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.set_auto_finish(Duration::from_millis(400));
        // before/after pairs per step: [A,B] changed, [C,D] changed
        let mut seeds = vec![200, 1, 2, 3, 4];
        seeds.resize(20, 4);
        core.set_frames(frames(&seeds));

        // the first Deploy's task chain errors instead of completing
        let core2 = core.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            core2.emit(
                10000,
                serde_json::json!({"taskchain": "SingleStep", "taskid": 1, "uuid": "fake-uuid"}),
            );
            core2.set_running(false);
        });

        let plan = vec![
            DeployStep {
                name: "Amiya".into(),
                location: [4, 5],
                direction: "Right".into(),
                skill_usage: None,
            },
            DeployStep {
                name: "Texas".into(),
                location: [2, 2],
                direction: "Up".into(),
                skill_usage: None,
            },
        ];
        let results = battle::deploy_batch(
            &device,
            plan,
            Duration::from_millis(10),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 2);
        let err = results[0].error.as_deref().unwrap_or_default();
        assert!(err.contains("error"), "error: {err}");
        assert!(results[0].frame_changed);
        assert!(results[1].error.is_none());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deploy_batch_rejects_bad_direction_before_touching_the_core() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();

        let plan = vec![DeployStep {
            name: "Amiya".into(),
            location: [4, 5],
            direction: "North".into(),
            skill_usage: None,
        }];
        let err = battle::deploy_batch(
            &device,
            plan,
            Duration::from_millis(10),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("direction"));
        assert!(core.tasks.lock().unwrap().is_empty());
        assert_eq!(*core.screencap_calls.lock().unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_until_settles_on_stable_frames() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        let mut seeds = vec![200, 1, 2, 3, 4, 5, 6];
        seeds.resize(40, 6);
        core.set_frames(frames(&seeds));

        let report = battle::resume_until(
            &device,
            Duration::from_secs(10),
            Duration::from_millis(15),
            3,
        )
        .await
        .unwrap();
        assert_eq!(report.reason, "settled");
        assert_eq!(core.clicks.lock().unwrap().len(), 1);
        assert!(!report.frame_png.is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_until_pauses_on_timeout() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        let mut seeds = vec![200u8];
        seeds.extend((0..60u8).map(|i| 30 + i));
        core.set_frames(frames(&seeds));

        let report = battle::resume_until(
            &device,
            Duration::from_millis(400),
            Duration::from_millis(15),
            3,
        )
        .await
        .unwrap();
        assert_eq!(report.reason, "timeout_paused");
        assert_eq!(core.clicks.lock().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn is_paused_true_on_identical_frames_false_on_changing() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (device, core) = setup(None);
        device.ensure_connected().await.unwrap();
        core.set_frames(frames(&[200, 7, 7, 7, 7]));
        assert!(
            battle::is_paused(&device, Duration::from_millis(10))
                .await
                .unwrap()
        );
        core.set_frames(frames(&[200, 1, 2, 3, 4, 5]));
        assert!(
            !battle::is_paused(&device, Duration::from_millis(10))
                .await
                .unwrap()
        );
    })
    .await
    .unwrap();
}
