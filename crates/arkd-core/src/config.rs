use std::{
    collections::BTreeSet,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub maa: MaaConfig,
    pub devices: Vec<DeviceConfig>,
    pub job_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub token_file: Option<PathBuf>,
    pub max_events: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MaaConfig {
    pub core_dir: PathBuf,
    pub resource_dir: PathBuf,
    pub user_dir: PathBuf,
    pub incremental: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    pub name: String,
    pub default: bool,
    #[serde(flatten)]
    pub kind: DeviceKind,
    pub connect_config: String,
    pub screenshot_size: (u32, u32),
    pub device_size: Option<(u32, u32)>,
    pub pause_button_screenshot: (u32, u32),
    pub client_type: String,
    pub hud_template: Option<PathBuf>,
    pub hud_roi: (u32, u32, u32, u32),
    pub hud_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceKind {
    Playtools {
        address: String,
    },
    Adb {
        adb_path: String,
        address: String,
        touch_mode: String,
    },
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            maa: MaaConfig::default(),
            devices: default_devices(),
            job_dir: None,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7717".parse().unwrap(),
            token_file: Some(PathBuf::from("~/.config/arkd/token")),
            max_events: 2000,
        }
    }
}

impl Default for MaaConfig {
    fn default() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self {
                core_dir: PathBuf::from("/Applications/MAA.app/Contents/Frameworks"),
                resource_dir: PathBuf::from("/Applications/MAA.app/Contents/Resources"),
                user_dir: PathBuf::from("~/.local/state/arkd"),
                incremental: Vec::new(),
            }
        }
        #[cfg(target_os = "windows")]
        {
            Self {
                core_dir: PathBuf::from("C:\\MAA"),
                resource_dir: PathBuf::from("C:\\MAA"),
                user_dir: PathBuf::from("%LOCALAPPDATA%\\arkd"),
                incremental: Vec::new(),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Self {
                core_dir: PathBuf::new(),
                resource_dir: PathBuf::new(),
                user_dir: PathBuf::new(),
                incremental: Vec::new(),
            }
        }
    }
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            default: false,
            kind: DeviceKind::Playtools {
                address: "127.0.0.1:1717".to_string(),
            },
            connect_config: "General".to_string(),
            screenshot_size: (1280, 720),
            device_size: None,
            pause_button_screenshot: (1210, 55),
            client_type: "Official".to_string(),
            hud_template: None,
            hud_roi: (1180, 30, 60, 50),
            hud_threshold: 0.85,
        }
    }
}

fn default_devices() -> Vec<DeviceConfig> {
    #[cfg(target_os = "macos")]
    {
        vec![DeviceConfig {
            name: "playcover".to_string(),
            default: true,
            device_size: Some((1920, 1080)),
            ..DeviceConfig::default()
        }]
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home).join(rest);
    }
    if s == "~"
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = s.strip_prefix("%LOCALAPPDATA%")
        && let Some(dir) = std::env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(dir).join(rest.trim_start_matches(['\\', '/']));
    }
    path.to_path_buf()
}

impl DeviceConfig {
    pub fn touch_mode(&self) -> &str {
        match &self.kind {
            DeviceKind::Playtools { .. } => "MacPlayTools",
            DeviceKind::Adb { touch_mode, .. } => touch_mode,
        }
    }

    pub fn address(&self) -> &str {
        match &self.kind {
            DeviceKind::Playtools { address } => address,
            DeviceKind::Adb { address, .. } => address,
        }
    }

    pub fn adb_path(&self) -> &str {
        match &self.kind {
            DeviceKind::Playtools { .. } => "",
            DeviceKind::Adb { adb_path, .. } => adb_path,
        }
    }
}

fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn resolve_job_path(raw: &str, job_dir: Option<&Path>) -> Result<String> {
    let Some(job_dir) = job_dir else {
        return Ok(raw.to_string());
    };
    let root = expand_tilde(job_dir)
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(&expand_tilde(job_dir)));
    let raw_path = Path::new(raw);
    let joined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        root.join(raw_path)
    };
    let candidate = joined
        .canonicalize()
        .unwrap_or_else(|_| normalize_path(&joined));
    if candidate != root && !candidate.starts_with(&root) {
        return Err(Error::Validation(format!(
            "Task file {raw:?} resolves outside the configured ARKD_JOB_DIR ({}). Move the file into that directory or unset ARKD_JOB_DIR to allow arbitrary paths.",
            root.display()
        )));
    }
    Ok(candidate.to_string_lossy().into_owned())
}

pub fn default_config_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        expand_tilde(Path::new("~/.config/arkd/config.toml"))
    }
    #[cfg(target_os = "windows")]
    {
        expand_tilde(Path::new("%LOCALAPPDATA%\\arkd\\config.toml"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        PathBuf::from("config.toml")
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Config> {
        let path = path
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("ARKD_CONFIG").map(PathBuf::from))
            .unwrap_or_else(default_config_path);
        let mut config = if path.is_file() {
            let text = std::fs::read_to_string(&path)?;
            toml::from_str::<Config>(&text)
                .map_err(|e| Error::Config(format!("could not parse {}: {e}", path.display())))?
        } else {
            Config::default()
        };
        config.apply_env();
        config.expand_paths();
        config.validate()?;
        Ok(config)
    }

    fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("ARKD_BIND") {
            match v.parse() {
                Ok(addr) => self.server.bind = addr,
                Err(e) => {
                    tracing::warn!("ignoring ARKD_BIND {v:?}: {e}");
                }
            }
        }
        if let Ok(v) = std::env::var("ARKD_TOKEN_FILE") {
            self.server.token_file = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("ARKD_MAA_CORE_DIR") {
            self.maa.core_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("ARKD_MAA_RESOURCE_DIR") {
            self.maa.resource_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("ARKD_MAA_USER_DIR") {
            self.maa.user_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("ARKD_JOB_DIR") {
            self.job_dir = Some(PathBuf::from(v));
        }
    }

    fn expand_paths(&mut self) {
        self.maa.core_dir = expand_tilde(&self.maa.core_dir);
        self.maa.resource_dir = expand_tilde(&self.maa.resource_dir);
        self.maa.user_dir = expand_tilde(&self.maa.user_dir);
        self.maa.incremental = self
            .maa
            .incremental
            .iter()
            .map(|p| expand_tilde(p))
            .collect();
        self.server.token_file = self.server.token_file.as_ref().map(|p| expand_tilde(p));
        self.job_dir = self.job_dir.as_ref().map(|p| expand_tilde(p));
        for d in &mut self.devices {
            d.hud_template = d.hud_template.as_ref().map(|p| expand_tilde(p));
        }
    }

    fn validate(&mut self) -> Result<()> {
        if self.server.max_events < 1 {
            return Err(Error::Config(format!(
                "server.max_events must be >= 1, got {}",
                self.server.max_events
            )));
        }
        let defaults = self.devices.iter().filter(|d| d.default).count();
        match (self.devices.len(), defaults) {
            (1, 0) => self.devices[0].default = true,
            (_, 1) => {}
            (0, _) => {}
            (_, n) => {
                return Err(Error::Config(format!(
                    "config must mark exactly one device as default, got {n}"
                )));
            }
        }
        if self.devices.len() > 1 && defaults == 0 {
            return Err(Error::Config(
                "config has multiple devices but none marked default".to_string(),
            ));
        }
        let mut names = BTreeSet::new();
        for d in &self.devices {
            if d.name.is_empty() {
                return Err(Error::Config("device name must not be empty".to_string()));
            }
            if !names.insert(d.name.clone()) {
                return Err(Error::Config(format!("duplicate device name {:?}", d.name)));
            }
            if d.address().is_empty() {
                return Err(Error::Config(format!(
                    "device {:?} has an empty address",
                    d.name
                )));
            }
        }
        Ok(())
    }

    pub fn describe(&self) -> serde_json::Value {
        serde_json::json!({
            "server": {
                "bind": self.server.bind.to_string(),
                "token_file": self.server.token_file.as_ref().map(|p| p.display().to_string()),
                "max_events": self.server.max_events,
            },
            "maa": {
                "core_dir": self.maa.core_dir.display().to_string(),
                "resource_dir": self.maa.resource_dir.display().to_string(),
                "user_dir": self.maa.user_dir.display().to_string(),
                "incremental": self.maa.incremental.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            },
            "job_dir": self.job_dir.as_ref().map(|p| p.display().to_string()),
            "devices": self.devices.iter().map(|d| {
                serde_json::json!({
                    "name": d.name,
                    "default": d.default,
                    "address": d.address(),
                    "connect_config": d.connect_config,
                    "touch_mode": d.touch_mode(),
                    "client_type": d.client_type,
                    "screenshot_size": [d.screenshot_size.0, d.screenshot_size.1],
                    "device_size": d.device_size.map(|(w, h)| [w, h]),
                    "pause_button_screenshot": [d.pause_button_screenshot.0, d.pause_button_screenshot.1],
                })
            }).collect::<Vec<_>>(),
        })
    }
}
