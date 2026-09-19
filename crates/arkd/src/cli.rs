use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use arkd_core::config::{Config, DeviceKind, expand_tilde};
use arkd_core::core_api::fake::FakeCoreFactory;
use arkd_core::core_api::{CoreFactory, RealCoreFactory, Runtime};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};

use crate::app::App;
use crate::client::DaemonClient;

#[derive(Parser)]
#[command(name = "arkd", version, about = "MCP daemon for MaaAssistantArknights")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Clone)]
struct ClientOpts {
    #[arg(
        long,
        env = "ARKD_URL",
        default_value = "http://127.0.0.1:7717/mcp",
        help = "Daemon MCP endpoint"
    )]
    url: String,
    #[arg(long, help = "File holding the bearer token (ARKD_TOKEN env wins)")]
    token_file: Option<PathBuf>,
    #[arg(long, help = "Device name; defaults to the daemon's default device")]
    device: Option<String>,
    #[arg(long, help = "Print compact JSON instead of pretty-printed")]
    json: bool,
    #[arg(
        long,
        env = "ARKD_HEADERS",
        value_delimiter = ';',
        value_name = "Name: value",
        help = "Extra HTTP header for the daemon request; repeatable (e.g. Cloudflare Access service tokens). ARKD_HEADERS takes a ';'-separated list."
    )]
    header: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Serve the MCP endpoint over streamable HTTP")]
    Serve {
        #[arg(long, help = "Config file path")]
        config: Option<PathBuf>,
        #[arg(long, help = "Bind address, e.g. 127.0.0.1:8765")]
        bind: Option<String>,
        #[arg(
            long,
            help = "Use the fake MaaCore backend instead of loading libMaaCore"
        )]
        fake: bool,
    },
    #[command(about = "Proxy the daemon's MCP endpoint over stdio")]
    Mcp {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Show daemon session status")]
    Status {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "List configured devices")]
    Devices {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Connect a device to MaaCore")]
    Connect {
        #[arg(long, help = "Force a fresh connect even if already connected")]
        reconnect: bool,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Capture the game screen to a file")]
    Screenshot {
        #[arg(short, long, help = "Output image path")]
        output: PathBuf,
        #[arg(long, default_value = "auto")]
        via: String,
        #[arg(long, default_value_t = 1.0)]
        scale: f64,
        #[arg(long, default_value = "png")]
        format: String,
        #[arg(long, default_value_t = 80)]
        quality: u8,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Tap a screen coordinate")]
    Tap {
        x: f64,
        y: f64,
        #[arg(long, default_value = "screenshot")]
        space: String,
        #[arg(long, default_value = "auto")]
        via: String,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Task queue operations")]
    Task {
        #[command(subcommand)]
        task: TaskCommand,
    },
    #[command(about = "Manual battle control")]
    Battle {
        #[command(subcommand)]
        battle: BattleCommand,
    },
    #[command(about = "Check the local environment, config and reachability")]
    Doctor {
        #[arg(long, help = "Config file path")]
        config: Option<PathBuf>,
        #[arg(long, help = "Skip loading MaaCore")]
        fake: bool,
    },
    #[command(about = "Manage the daemon auth token")]
    Token {
        #[command(subcommand)]
        token: TokenCommand,
    },
    #[command(about = "launchd integration")]
    Launchd {
        #[command(subcommand)]
        launchd: LaunchdCommand,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    #[command(about = "List MAA task types")]
    Types {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Show a task type's schema")]
    Schema {
        task_type: String,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Queue a task")]
    Append {
        task_type: String,
        params: Option<String>,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Replace a queued task's params")]
    Update {
        task_id: i32,
        params: String,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Show the queued tasks")]
    Queue {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Start the queue")]
    Start {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Stop the run")]
    Stop {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Wait for a condition")]
    Wait {
        #[arg(long, default_value = "all_done")]
        until: String,
        #[arg(long)]
        task_id: Option<i32>,
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
        #[arg(long, default_value_t = 60)]
        timeout: u32,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Page through event history")]
    Events {
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long, help = "Include routine events, not just significant ones")]
        all: bool,
        #[arg(long, help = "Include raw callback payloads")]
        payload: bool,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Queue a conventional daily run")]
    Daily {
        #[arg(long, default_value = "")]
        stage: String,
        #[arg(long, default_value_t = 0)]
        medicine: u32,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Navigate the game back to the home screen")]
    Home {
        #[command(flatten)]
        opts: ClientOpts,
    },
}

#[derive(Subcommand)]
enum BattleCommand {
    #[command(about = "Battle diagnosis")]
    State {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Set the stage for manual control")]
    Stage {
        name: String,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Begin the battle")]
    Start {
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Perform one battle action; JSON is the battle_action argument object")]
    Action {
        json: String,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Enter a stage and pause as soon as the field renders")]
    StartPaused {
        stage: String,
        #[arg(long, default_value_t = 60, help = "Timeout in seconds (5..=300)")]
        timeout: u32,
        #[arg(short, long, help = "Save the paused frame to a PNG")]
        output: Option<PathBuf>,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Deploy a batch of operators from a JSON plan file while paused")]
    Deploy {
        plan: PathBuf,
        #[arg(long, default_value_t = 1000, help = "Settle time between steps (ms)")]
        settle_ms: u64,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Resume a paused battle until the screen settles or times out")]
    Resume {
        #[arg(
            long,
            default_value_t = 30,
            help = "Seconds to let the battle run (1..=600)"
        )]
        seconds: u32,
        #[arg(short, long, help = "Save the final frame to a PNG")]
        output: Option<PathBuf>,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Pause the battle and return the paused frame")]
    Pause {
        #[arg(short, long, help = "Save the paused frame to a PNG")]
        output: Option<PathBuf>,
        #[command(flatten)]
        opts: ClientOpts,
    },
    #[command(about = "Report whether the battle screen is paused")]
    IsPaused {
        #[command(flatten)]
        opts: ClientOpts,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    #[command(about = "Generate and store a bearer token")]
    Init {
        #[arg(long, help = "Config file path")]
        config: Option<PathBuf>,
        #[arg(long, help = "Overwrite an existing token file")]
        force: bool,
    },
}

#[derive(Subcommand)]
enum LaunchdCommand {
    #[command(about = "Print a LaunchAgent plist to stdout")]
    Print {
        #[arg(long, help = "Config file path")]
        config: Option<PathBuf>,
    },
}

fn resolve_headers(opts: &ClientOpts) -> anyhow::Result<Vec<(String, String)>> {
    opts.header
        .iter()
        .map(|h| {
            let (name, value) = h
                .split_once(':')
                .with_context(|| format!("header {h:?} is not in 'Name: value' form"))?;
            Ok((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn resolve_token(opts: &ClientOpts) -> Option<String> {
    if let Ok(t) = std::env::var("ARKD_TOKEN")
        && !t.is_empty()
    {
        return Some(t);
    }
    let path = opts
        .token_file
        .clone()
        .unwrap_or_else(|| expand_tilde(Path::new("~/.config/arkd/token")));
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn connect_client(opts: &ClientOpts) -> anyhow::Result<DaemonClient> {
    DaemonClient::connect_with_headers(
        &opts.url,
        resolve_token(opts).as_deref(),
        &resolve_headers(opts)?,
    )
    .await
}

async fn run_tool(opts: &ClientOpts, name: &str, args: Value) -> anyhow::Result<()> {
    let client = connect_client(opts).await?;
    let result = client.call(name, with_device(opts, args)).await?;
    print_result(&result, opts.json)
}

async fn run_tool_frame(
    opts: &ClientOpts,
    name: &str,
    args: Value,
    output: Option<&Path>,
) -> anyhow::Result<()> {
    let client = connect_client(opts).await?;
    let result = client.call(name, with_device(opts, args)).await?;
    if result.is_error.unwrap_or(false) {
        return print_result(&result, opts.json);
    }
    if let Some(image) = result.content.iter().find_map(|c| c.as_image()) {
        match output {
            Some(path) => {
                let bytes = BASE64.decode(&image.data).context("bad base64 image")?;
                std::fs::write(path, &bytes)
                    .with_context(|| format!("could not write {}", path.display()))?;
            }
            None => {
                eprintln!("note: the tool returned a frame image; pass -o FRAME.png to save it");
            }
        }
    }
    print_result(&result, opts.json)
}

fn with_device(opts: &ClientOpts, mut args: Value) -> Value {
    if let Some(device) = &opts.device {
        args["device"] = json!(device);
    }
    args
}

fn print_result(result: &rmcp::model::CallToolResult, raw: bool) -> anyhow::Result<()> {
    if result.is_error.unwrap_or(false) {
        for c in &result.content {
            if let Some(t) = c.as_text() {
                eprintln!("{}", t.text);
            }
        }
        anyhow::bail!("tool call failed");
    }
    if let Some(v) = &result.structured_content {
        if raw {
            println!("{}", serde_json::to_string(v)?);
        } else {
            println!("{}", serde_json::to_string_pretty(v)?);
        }
    } else {
        for c in &result.content {
            if let Some(t) = c.as_text() {
                if let Ok(v) = serde_json::from_str::<Value>(&t.text) {
                    if raw {
                        println!("{}", serde_json::to_string(&v)?);
                    } else {
                        println!("{}", serde_json::to_string_pretty(&v)?);
                    }
                } else {
                    println!("{}", t.text);
                }
            }
        }
    }
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_async(cli))
}

async fn run_async(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Serve { config, bind, fake } => serve(config, bind, fake).await,
        Command::Mcp { opts } => mcp_proxy(&opts).await,
        Command::Status { opts } => run_tool(&opts, "status", json!({})).await,
        Command::Devices { opts } => run_tool(&opts, "devices_list", json!({})).await,
        Command::Connect { reconnect, opts } => {
            run_tool(&opts, "device_connect", json!({"reconnect": reconnect})).await
        }
        Command::Screenshot {
            output,
            via,
            scale,
            format,
            quality,
            opts,
        } => screenshot(&opts, &output, &via, scale, &format, quality).await,
        Command::Tap {
            x,
            y,
            space,
            via,
            opts,
        } => {
            run_tool(
                &opts,
                "screen_tap",
                json!({"x": x, "y": y, "coord_space": space, "via": via}),
            )
            .await
        }
        Command::Task { task } => run_task(task).await,
        Command::Battle { battle } => run_battle(battle).await,
        Command::Doctor { config, fake } => doctor(config, fake).await,
        Command::Token { token } => match token {
            TokenCommand::Init { config, force } => token_init(config, force),
        },
        Command::Launchd { launchd } => match launchd {
            LaunchdCommand::Print { config } => launchd_print(config),
        },
    }
}

async fn run_task(cmd: TaskCommand) -> anyhow::Result<()> {
    match cmd {
        TaskCommand::Types { opts } => run_tool(&opts, "maa_task_types", json!({})).await,
        TaskCommand::Schema { task_type, opts } => {
            run_tool(&opts, "maa_task_schema", json!({"task_type": task_type})).await
        }
        TaskCommand::Append {
            task_type,
            params,
            opts,
        } => {
            let params = match params {
                Some(raw) => {
                    serde_json::from_str::<Value>(&raw).context("PARAMS_JSON is not valid JSON")?
                }
                None => json!({}),
            };
            if !params.is_object() {
                anyhow::bail!("PARAMS_JSON must be a JSON object");
            }
            run_tool(
                &opts,
                "maa_append",
                json!({"task_type": task_type, "params": params}),
            )
            .await
        }
        TaskCommand::Update {
            task_id,
            params,
            opts,
        } => {
            let params =
                serde_json::from_str::<Value>(&params).context("PARAMS_JSON is not valid JSON")?;
            if !params.is_object() {
                anyhow::bail!("PARAMS_JSON must be a JSON object");
            }
            run_tool(
                &opts,
                "maa_update_params",
                json!({"task_id": task_id, "params": params}),
            )
            .await
        }
        TaskCommand::Queue { opts } => run_tool(&opts, "maa_queue", json!({})).await,
        TaskCommand::Start { opts } => run_tool(&opts, "maa_start", json!({})).await,
        TaskCommand::Stop { opts } => run_tool(&opts, "maa_stop", json!({})).await,
        TaskCommand::Wait {
            until,
            task_id,
            after_seq,
            timeout,
            opts,
        } => {
            run_tool(
                &opts,
                "maa_wait",
                json!({
                    "until": until,
                    "task_id": task_id,
                    "after_seq": after_seq,
                    "timeout_seconds": timeout,
                }),
            )
            .await
        }
        TaskCommand::Events {
            after_seq,
            limit,
            all,
            payload,
            opts,
        } => {
            run_tool(
                &opts,
                "maa_events",
                json!({
                    "after_seq": after_seq,
                    "limit": limit,
                    "significant_only": !all,
                    "include_payload": payload,
                }),
            )
            .await
        }
        TaskCommand::Daily {
            stage,
            medicine,
            opts,
        } => {
            run_tool(
                &opts,
                "maa_daily",
                json!({"stage": stage, "medicine": medicine}),
            )
            .await
        }
        TaskCommand::Home { opts } => run_tool(&opts, "maa_back_home", json!({})).await,
    }
}

async fn run_battle(cmd: BattleCommand) -> anyhow::Result<()> {
    match cmd {
        BattleCommand::State { opts } => run_tool(&opts, "battle_state", json!({})).await,
        BattleCommand::Stage { name, opts } => {
            run_tool(&opts, "battle_set_stage", json!({"stage": name})).await
        }
        BattleCommand::Start { opts } => run_tool(&opts, "battle_start", json!({})).await,
        BattleCommand::Action { json: raw, opts } => {
            let args = serde_json::from_str::<Value>(&raw).context("JSON is not valid JSON")?;
            if !args.is_object() {
                anyhow::bail!("JSON must be a JSON object");
            }
            run_tool(&opts, "battle_action", args).await
        }
        BattleCommand::StartPaused {
            stage,
            timeout,
            output,
            opts,
        } => {
            run_tool_frame(
                &opts,
                "battle_start_paused",
                json!({"stage": stage, "timeout_seconds": timeout}),
                output.as_deref(),
            )
            .await
        }
        BattleCommand::Deploy {
            plan,
            settle_ms,
            opts,
        } => {
            let text = std::fs::read_to_string(&plan)
                .with_context(|| format!("could not read {}", plan.display()))?;
            let plan_json =
                serde_json::from_str::<Value>(&text).context("PLAN.json is not valid JSON")?;
            if !plan_json.is_array() {
                anyhow::bail!("PLAN.json must be a JSON array of deployment steps");
            }
            run_tool(
                &opts,
                "battle_deploy_batch",
                json!({"plan": plan_json, "settle_ms": settle_ms}),
            )
            .await
        }
        BattleCommand::Resume {
            seconds,
            output,
            opts,
        } => {
            run_tool_frame(
                &opts,
                "battle_resume_until",
                json!({"seconds": seconds}),
                output.as_deref(),
            )
            .await
        }
        BattleCommand::Pause { output, opts } => {
            run_tool_frame(&opts, "battle_pause", json!({}), output.as_deref()).await
        }
        BattleCommand::IsPaused { opts } => run_tool(&opts, "battle_is_paused", json!({})).await,
    }
}

async fn screenshot(
    opts: &ClientOpts,
    output: &Path,
    via: &str,
    scale: f64,
    format: &str,
    quality: u8,
) -> anyhow::Result<()> {
    let client = connect_client(opts).await?;
    let result = client
        .call(
            "screen_capture",
            with_device(
                opts,
                json!({
                    "via": via,
                    "fresh": true,
                    "scale": scale,
                    "format": format,
                    "quality": quality,
                }),
            ),
        )
        .await?;
    if result.is_error.unwrap_or(false) {
        return print_result(&result, opts.json);
    }
    let image = result
        .content
        .iter()
        .find_map(|c| c.as_image())
        .context("screen_capture returned no image content")?;
    let bytes = BASE64.decode(&image.data).context("bad base64 image")?;
    std::fs::write(output, &bytes)
        .with_context(|| format!("could not write {}", output.display()))?;
    let meta = result
        .content
        .iter()
        .find_map(|c| {
            c.as_text()
                .and_then(|t| serde_json::from_str::<Value>(&t.text).ok())
        })
        .unwrap_or_else(|| json!({}));
    let mut meta = meta;
    meta["path"] = json!(output.display().to_string());
    if opts.json {
        println!("{}", serde_json::to_string(&meta)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&meta)?);
    }
    Ok(())
}

async fn mcp_proxy(opts: &ClientOpts) -> anyhow::Result<()> {
    let client = connect_client(opts).await?;
    let handler = crate::proxy::ProxyHandler::new(client);
    let service = rmcp::serve_server(handler, rmcp::transport::io::stdio())
        .await
        .context("could not start the stdio proxy")?;
    service.waiting().await?;
    Ok(())
}

async fn serve(
    config_path: Option<PathBuf>,
    bind: Option<String>,
    fake: bool,
) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "arkd=info,tower_http=info".into()),
        )
        .init();

    let mut config = Config::load(config_path.as_deref()).map_err(anyhow::Error::from)?;
    if let Some(bind) = bind {
        config.server.bind = bind.parse().context("invalid --bind address")?;
    }

    let (factory, core_version): (Arc<dyn CoreFactory>, String) = if fake {
        let factory = FakeCoreFactory::new();
        (Arc::new(factory), "fake".to_string())
    } else {
        let info = Runtime::init(
            &config.maa.core_dir,
            &config.maa.resource_dir,
            &config.maa.user_dir,
            &config.maa.incremental,
        )
        .map_err(anyhow::Error::from)?;
        (Arc::new(RealCoreFactory), info.version.clone())
    };

    let state = App::build(config, factory, core_version).map_err(anyhow::Error::from)?;
    let bind_addr = state.config.server.bind;
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("could not bind {bind_addr}"))?;
    tracing::info!("listening on http://{bind_addr}/mcp");
    App::serve(state, listener).await
}

async fn doctor(config_path: Option<PathBuf>, fake: bool) -> anyhow::Result<()> {
    let path = config_path
        .or_else(|| std::env::var_os("ARKD_CONFIG").map(PathBuf::from))
        .unwrap_or_else(arkd_core::config::default_config_path);
    println!("config: {}", path.display());
    let config = Config::load(Some(&path)).map_err(anyhow::Error::from)?;

    let mut failures = 0usize;
    let mut report = |check: &str, ok: bool, detail: String| {
        println!(
            "{:<28} {:<4} {}",
            check,
            if ok { "OK" } else { "FAIL" },
            detail
        );
        if !ok {
            failures += 1;
        }
    };

    let lib_name = if cfg!(target_os = "macos") {
        "libMaaCore.dylib"
    } else if cfg!(target_os = "windows") {
        "MaaCore.dll"
    } else {
        "libMaaCore.so"
    };
    let lib = config.maa.core_dir.join(lib_name);
    report("core library", lib.is_file(), lib.display().to_string());

    if fake {
        report("MaaCore load", true, "skipped (--fake)".to_string());
    } else {
        match Runtime::init(
            &config.maa.core_dir,
            &config.maa.resource_dir,
            &config.maa.user_dir,
            &config.maa.incremental,
        ) {
            Ok(info) => report("MaaCore load", true, info.version.clone()),
            Err(e) => report("MaaCore load", false, e.to_string()),
        }
    }

    let resource = config.maa.resource_dir.join("resource");
    report(
        "resource_dir/resource",
        resource.is_dir(),
        resource.display().to_string(),
    );

    for device in &config.devices {
        let name = &device.name;
        let reachable = device
            .address()
            .parse::<std::net::SocketAddr>()
            .ok()
            .and_then(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(1)).ok())
            .is_some();
        report(
            &format!("device:{name}:reachable"),
            reachable,
            device.address().to_string(),
        );
        if matches!(device.kind, DeviceKind::Playtools { .. }) {
            match arkd_core::playtools::PlayToolsClient::connect(
                device.address(),
                Duration::from_secs(3),
            )
            .await
            {
                Ok(client) => {
                    let version = client.version();
                    let (w, h) = client.size();
                    report(
                        &format!("device:{name}:playtools"),
                        true,
                        format!("VERN={version} SIZE={w}x{h}"),
                    );
                }
                Err(e) => report(&format!("device:{name}:playtools"), false, e.to_string()),
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let maa_running = std::process::Command::new("pgrep")
            .args(["-x", "MAA"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        report(
            "MAA.app not running",
            !maa_running,
            if maa_running {
                "MAA.app is running; it holds its own MaaCore connection and may conflict with arkd"
                    .to_string()
            } else {
                "MAA.app is not running".to_string()
            },
        );
    }

    let health_url = format!("http://{}/healthz", config.server.bind);
    let daemon_ok = reqwest::get(&health_url)
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    println!(
        "{:<28} {:<4} {}",
        "daemon /healthz",
        if daemon_ok { "OK" } else { "FAIL" },
        if daemon_ok {
            health_url
        } else {
            format!("{health_url} not reachable (informational)")
        }
    );

    if failures > 0 {
        anyhow::bail!("{failures} check(s) failed");
    }
    Ok(())
}

fn token_init(config_path: Option<PathBuf>, force: bool) -> anyhow::Result<()> {
    let config = Config::load(config_path.as_deref()).map_err(anyhow::Error::from)?;
    let path = config
        .server
        .token_file
        .clone()
        .context("no server.token_file configured")?;
    if path.exists() && !force {
        anyhow::bail!(
            "token file {} already exists; pass --force to overwrite",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes: [u8; 32] = rand::random();
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    std::fs::write(&path, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    println!("{}", path.display());
    Ok(())
}

fn launchd_print(config_path: Option<PathBuf>) -> anyhow::Result<()> {
    let config = Config::load(config_path.as_deref()).map_err(anyhow::Error::from)?;
    let exe = std::env::current_exe()?;
    let user_dir = expand_tilde(&config.maa.user_dir);
    let out_log = user_dir.join("arkd.out.log");
    let err_log = user_dir.join("arkd.err.log");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.arkd.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>serve</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{out}</string>
    <key>StandardErrorPath</key>
    <string>{err}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin</string>
    </dict>
</dict>
</plist>
"#,
        exe = exe.display(),
        out = out_log.display(),
        err = err_log.display(),
    );
    print!("{plist}");
    eprintln!();
    eprintln!("install with:");
    eprintln!("  cp <this-file> ~/Library/LaunchAgents/ai.arkd.daemon.plist");
    eprintln!("  launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/ai.arkd.daemon.plist");
    Ok(())
}
