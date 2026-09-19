use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use arkd_core::config::Config;
use arkd_core::core_api::fake::FakeCoreFactory;
use arkd_core::core_api::{CoreFactory, RealCoreFactory, Runtime};
use clap::{Parser, Subcommand};

use crate::app::App;

#[derive(Parser)]
#[command(name = "arkd", version, about = "MCP daemon for MaaAssistantArknights")]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
}

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { config, bind, fake } => serve(config, bind, fake),
    }
}

fn serve(config_path: Option<PathBuf>, bind: Option<String>, fake: bool) -> anyhow::Result<()> {
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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(bind_addr)
            .await
            .with_context(|| format!("could not bind {bind_addr}"))?;
        tracing::info!("listening on http://{bind_addr}/mcp");
        App::serve(state, listener).await
    })
}
