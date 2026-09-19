use std::process::Command;
use std::sync::Arc;

use arkd::app::App;
use arkd_core::config::Config;
use arkd_core::core_api::fake::FakeCoreFactory;
use serde_json::Value;

fn arkd(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_arkd"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_init_writes_a_private_token_file() {
    let dir = tempfile::tempdir().unwrap();
    let token_path = dir.path().join("nested").join("token");
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        format!("[server]\ntoken_file = \"{}\"\n", token_path.display()),
    )
    .unwrap();
    let config = config_path.to_string_lossy().to_string();

    let out = arkd(&["token", "init", "--config", &config]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(token_path.is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&token_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let printed = stdout(&out);
    assert!(printed.contains(token_path.to_string_lossy().as_ref()));
    let contents = std::fs::read_to_string(&token_path).unwrap();
    assert!(
        !printed.contains(contents.trim()),
        "the token must never be printed"
    );

    let out = arkd(&["token", "init", "--config", &config]);
    assert!(
        !out.status.success(),
        "second init without --force must fail"
    );

    let out = arkd(&["token", "init", "--config", &config, "--force"]);
    assert!(out.status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn launchd_print_emits_a_plist() {
    let out = arkd(&["launchd", "print"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = stdout(&out);
    assert!(stdout.contains("ai.arkd.daemon"));
    assert!(stdout.contains("<string>serve</string>"));
    let exe = std::env::current_exe().unwrap();
    let _ = exe;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("launchctl bootstrap"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cli_talks_to_a_fake_daemon() {
    let factory = FakeCoreFactory::new();
    let core = factory.core.clone();
    core.set_image_png(Some(
        arkd_core::playtools::Frame {
            width: 1280,
            height: 720,
            bgr: vec![64u8; 1280 * 720 * 3],
        }
        .to_png()
        .unwrap(),
    ));
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".parse().unwrap();
    let state = App::build(config, Arc::new(factory), "fake".to_string()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(App::serve(state, listener));
    let url = format!("http://{addr}/mcp");

    let out = arkd(&["status", "--url", &url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(v["connected"], serde_json::json!(false));

    let out = arkd(&["connect", "--url", &url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("shot.png");
    let out = arkd(&["screenshot", "-o", png.to_str().unwrap(), "--url", &url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data = std::fs::read(&png).unwrap();
    assert_eq!(&data[..4], b"\x89PNG");

    let out = arkd(&["tap", "1210", "55", "--url", &url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(*core.clicks.lock().unwrap(), vec![(1815, 82)]);

    let out = arkd(&["task", "types", "--url", &url]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert!(v["task_types"].as_array().unwrap().len() > 10);

    serve.abort();
}
