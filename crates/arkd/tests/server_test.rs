use std::sync::Arc;

use arkd::app::App;
use arkd_core::config::{Config, DeviceKind};
use arkd_core::core_api::fake::FakeCoreFactory;
use arkd_core::playtools::Frame;
use arkd_core::playtools::fake::FakePlayToolsServer;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Value, json};

fn args(v: Value) -> serde_json::Map<String, Value> {
    serde_json::from_value(v).unwrap()
}

fn structured(result: &rmcp::model::CallToolResult) -> Value {
    result
        .structured_content
        .clone()
        .or_else(|| {
            result
                .content
                .iter()
                .find_map(|c| c.as_text().and_then(|t| serde_json::from_str(&t.text).ok()))
        })
        .expect("tool result carries JSON")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_serves_mcp_and_healthz() {
    let playtools = FakePlayToolsServer::spawn(
        Frame {
            width: 16,
            height: 8,
            bgr: vec![7u8; 16 * 8 * 3],
        },
        1,
    )
    .await;

    let factory = FakeCoreFactory::new();
    let core = factory.core.clone();
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".parse().unwrap();
    config.devices = vec![arkd_core::config::DeviceConfig {
        name: "fake".to_string(),
        default: true,
        device_size: Some((1920, 1080)),
        ..arkd_core::config::DeviceConfig::default()
    }];
    config.devices[0].kind = DeviceKind::Playtools {
        address: playtools.addr.to_string(),
    };
    let state = App::build(config, Arc::new(factory), "fake".to_string()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(App::serve(state, listener));

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();
    let info = client.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().unwrap().name, "arkd");
    assert!(
        info.instructions
            .as_deref()
            .unwrap_or_default()
            .contains("maa_wait")
    );

    let tools = client.list_tools(None).await.unwrap().tools;
    let mut names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "battle_action",
            "battle_deploy_batch",
            "battle_is_paused",
            "battle_pause",
            "battle_resume_until",
            "battle_set_stage",
            "battle_start",
            "battle_start_paused",
            "battle_state",
            "device_connect",
            "devices_list",
            "doctor",
            "maa_append",
            "maa_back_home",
            "maa_daily",
            "maa_events",
            "maa_queue",
            "maa_start",
            "maa_stop",
            "maa_task_schema",
            "maa_task_types",
            "maa_update_params",
            "maa_wait",
            "screen_capture",
            "screen_drag",
            "screen_tap",
            "screen_touch",
            "status",
        ]
    );

    let status = structured(
        &client
            .call_tool(CallToolRequestParams::new("status").with_arguments(args(json!({}))))
            .await
            .unwrap(),
    );
    assert_eq!(status["connected"], json!(false));
    assert_eq!(status["geometry"]["source"], json!("config"));
    assert_eq!(status["geometry"]["device"], json!([1920, 1080]));

    let connect = structured(
        &client
            .call_tool(CallToolRequestParams::new("device_connect").with_arguments(args(json!({}))))
            .await
            .unwrap(),
    );
    assert_eq!(connect["what"], json!("Connected"));
    let status = structured(
        &client
            .call_tool(CallToolRequestParams::new("status").with_arguments(args(json!({}))))
            .await
            .unwrap(),
    );
    assert_eq!(status["connected"], json!(true));

    let err = client
        .call_tool(
            CallToolRequestParams::new("maa_task_schema")
                .with_arguments(args(json!({"task_type": "fite"}))),
        )
        .await
        .expect_err("fite should fail");
    assert!(err.to_string().contains("Did you mean Fight"), "{err}");

    let appended = structured(
        &client
            .call_tool(
                CallToolRequestParams::new("maa_append").with_arguments(args(
                    json!({"task_type": "Fight", "params": {"stage": "1-7"}}),
                )),
            )
            .await
            .unwrap(),
    );
    assert_eq!(appended["task_id"], json!(1));

    client
        .call_tool(CallToolRequestParams::new("maa_start").with_arguments(args(json!({}))))
        .await
        .unwrap();
    tokio::spawn({
        let core = core.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            core.finish_all();
        }
    });
    let waited = structured(
        &client
            .call_tool(
                CallToolRequestParams::new("maa_wait")
                    .with_arguments(args(json!({"until": "all_done", "timeout_seconds": 10}))),
            )
            .await
            .unwrap(),
    );
    assert_eq!(waited["triggered"], json!(true));
    assert!(waited["waited_seconds"].as_f64().unwrap() < 3.0, "{waited}");

    let png = Frame {
        width: 1280,
        height: 720,
        bgr: vec![128u8; 1280 * 720 * 3],
    }
    .to_png()
    .unwrap();
    core.set_image_png(Some(png));
    let caps = client
        .call_tool(CallToolRequestParams::new("screen_capture").with_arguments(args(json!({}))))
        .await
        .unwrap();
    let has_png = caps
        .content
        .iter()
        .any(|c| c.as_image().is_some_and(|i| i.mime_type == "image/png"));
    assert!(has_png, "{:?}", caps.content);
    let meta = structured(&caps);
    assert_eq!(meta["width"], json!(1280));

    let caps = client
        .call_tool(
            CallToolRequestParams::new("screen_capture")
                .with_arguments(args(json!({"scale": 0.5, "format": "jpeg"}))),
        )
        .await
        .unwrap();
    let meta = structured(&caps);
    assert_eq!(meta["width"], json!(640));
    assert_eq!(meta["height"], json!(360));

    let tap = structured(
        &client
            .call_tool(
                CallToolRequestParams::new("screen_tap")
                    .with_arguments(args(json!({"x": 1210, "y": 55}))),
            )
            .await
            .unwrap(),
    );
    assert_eq!(tap["device_x"], json!(1815));
    assert_eq!(tap["device_y"], json!(82));
    assert_eq!(tap["core_x"], json!(1210));
    assert_eq!(tap["core_y"], json!(55));
    assert_eq!(
        *core.clicks.lock().unwrap(),
        vec![(1210, 55)],
        "MaaCore clicks are in its normalised 1280x720 space, not device pixels"
    );

    let tap = structured(
        &client
            .call_tool(
                CallToolRequestParams::new("screen_tap")
                    .with_arguments(args(json!({"x": 1210, "y": 55, "via": "playtools"}))),
            )
            .await
            .unwrap(),
    );
    assert_eq!(tap["via"], json!("playtools"));
    let touches = playtools.touches.lock().unwrap().clone();
    assert_eq!(touches.len(), 2, "a tap is began+ended: {touches:?}");
    assert!(
        touches.iter().all(|t| (t.x, t.y) == (1815, 82)),
        "PlayTools taps keep device coordinates: {touches:?}"
    );

    let err = client
        .call_tool(
            CallToolRequestParams::new("maa_wait").with_arguments(args(json!({"until": "bogus"}))),
        )
        .await
        .expect_err("bogus until should fail");
    assert!(
        err.to_string().contains("Unknown wait condition")
            || err.to_string().contains("unknown wait condition"),
        "{err}"
    );

    let before = *core.screencap_calls.lock().unwrap();
    let caps = client
        .call_tool(
            CallToolRequestParams::new("screen_capture")
                .with_arguments(args(json!({"via": "playtools"}))),
        )
        .await
        .unwrap();
    let has_image = caps.content.iter().any(|c| c.as_image().is_some());
    assert!(has_image, "{:?}", caps.content);
    assert_eq!(
        *core.screencap_calls.lock().unwrap(),
        before,
        "playtools capture must not touch MaaCore screencap"
    );

    let health = reqwest::get(format!("http://{addr}/healthz"))
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let body: Value = health.json().await.unwrap();
    assert_eq!(body["ok"], json!(true));

    client.cancel().await.unwrap();
    serve.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_does_not_block_on_an_unreachable_playtools_device() {
    let factory = FakeCoreFactory::new();
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".parse().unwrap();
    config.devices = vec![arkd_core::config::DeviceConfig {
        name: "fake".to_string(),
        default: true,
        kind: DeviceKind::Playtools {
            address: "127.0.0.1:1".to_string(),
        },
        ..arkd_core::config::DeviceConfig::default()
    }];
    let state = App::build(config, Arc::new(factory), "fake".to_string()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(App::serve(state, listener));

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();

    let started = std::time::Instant::now();
    let status = structured(
        &client
            .call_tool(CallToolRequestParams::new("status").with_arguments(args(json!({}))))
            .await
            .unwrap(),
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "status took {:?}",
        started.elapsed()
    );
    assert_eq!(status["geometry"], json!(null));
    assert_eq!(status["geometry_source"], json!("unknown"));

    client.cancel().await.unwrap();
    serve.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn playtools_capture_is_normalised_to_screenshot_space() {
    // a Retina backing store can be much larger than screenshot_size
    let playtools = FakePlayToolsServer::spawn(
        Frame {
            width: 1920,
            height: 1080,
            bgr: vec![9u8; 1920 * 1080 * 3],
        },
        3,
    )
    .await;

    let factory = FakeCoreFactory::new();
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".parse().unwrap();
    config.devices = vec![arkd_core::config::DeviceConfig {
        name: "fake".to_string(),
        default: true,
        device_size: Some((1920, 1080)),
        ..arkd_core::config::DeviceConfig::default()
    }];
    config.devices[0].kind = DeviceKind::Playtools {
        address: playtools.addr.to_string(),
    };
    let state = App::build(config, Arc::new(factory), "fake".to_string()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(App::serve(state, listener));

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();

    let caps = client
        .call_tool(
            CallToolRequestParams::new("screen_capture")
                .with_arguments(args(json!({"via": "playtools"}))),
        )
        .await
        .unwrap();
    let meta = structured(&caps);
    assert_eq!(meta["width"], json!(1280), "{meta}");
    assert_eq!(meta["height"], json!(720), "{meta}");
    assert_eq!(meta["via"], json!("playtools"));
    assert_eq!(meta["coord_space"], json!("screenshot"));
    let image = caps
        .content
        .iter()
        .find_map(|c| c.as_image())
        .expect("an image block");
    let bytes = BASE64.decode(&image.data).unwrap();
    assert_eq!(
        arkd_core::screen::png_dimensions(&bytes).unwrap(),
        (1280, 720)
    );

    client.cancel().await.unwrap();
    serve.abort();
}
