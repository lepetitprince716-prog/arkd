use std::sync::Arc;

use arkd::app::App;
use arkd_core::config::Config;
use arkd_core::core_api::fake::FakeCoreFactory;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig, ReadResourceRequestParams};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stdio_proxy_forwards_to_daemon() {
    let factory = FakeCoreFactory::new();
    let mut config = Config::default();
    config.server.bind = "127.0.0.1:0".parse().unwrap();
    let state = App::build(config, Arc::new(factory), "fake".to_string()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = tokio::spawn(App::serve(state, listener));

    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_arkd")).configure(|c| {
            c.args(["mcp", "--url", &format!("http://{addr}/mcp")]);
        }),
    )
    .unwrap();
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
            "battle_set_stage",
            "battle_start",
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

    let status = client
        .call_tool(
            CallToolRequestParams::new("status").with_arguments(
                serde_json::from_value::<serde_json::Map<String, serde_json::Value>>(json!({}))
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    let connected = status
        .structured_content
        .as_ref()
        .and_then(|v| v.get("connected"))
        .cloned();
    assert_eq!(connected, Some(json!(false)));

    let resource = client
        .read_resource(ReadResourceRequestParams::new("arkd://catalog"))
        .await
        .unwrap();
    assert!(!resource.contents.is_empty());

    let prompts = client.list_prompts(None).await.unwrap().prompts;
    assert!(prompts.iter().any(|p| p.name == "daily_routine"));

    client.cancel().await.unwrap();
    serve.abort();
}
