use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::{Mutex, MutexGuard};

use crate::{
    config::{Config, DeviceConfig, DeviceKind},
    core_api::CoreFactory,
    error::{Error, Result},
    hud::HudTemplate,
    playtools::PlayToolsClient,
    screen::Geometry,
    session::{ConnectionInfo, MaaSession},
};

pub struct Device {
    pub name: String,
    pub config: DeviceConfig,
    pub session: MaaSession,
    playtools: Mutex<Option<PlayToolsClient>>,
    hud: OnceLock<Result<Option<HudTemplate>>>,
    hud_paused: OnceLock<Result<Option<HudTemplate>>>,
    action_lock: Mutex<()>,
    connect_lock: Mutex<()>,
    busy: AtomicBool,
}

impl Device {
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn is_connected(&self) -> bool {
        self.session.connected()
    }

    pub async fn ensure_connected(&self) -> Result<ConnectionInfo> {
        let _guard = self.connect_lock.lock().await;
        if self.session.connected() {
            return self
                .session
                .connection()
                .ok_or_else(|| Error::NotConnected("no connection record".to_string()));
        }
        let session = self.session.clone();
        let adb_path = self.config.adb_path().to_string();
        let address = self.config.address().to_string();
        let connect_config = self.config.connect_config.clone();
        let touch_mode = self.config.touch_mode().to_string();
        tokio::task::spawn_blocking(move || {
            session.connect(&adb_path, &address, &connect_config, &touch_mode)
        })
        .await
        .map_err(|e| Error::DeviceConnection(format!("connect task failed: {e}")))?
    }

    pub async fn playtools(&self) -> Result<MutexGuard<'_, Option<PlayToolsClient>>> {
        if !matches!(self.config.kind, DeviceKind::Playtools { .. }) {
            return Err(Error::PlayTools(format!(
                "device {} is not a PlayTools device",
                self.name
            )));
        }
        let mut guard = self.playtools.lock().await;
        if guard.is_none() {
            let client =
                PlayToolsClient::connect(self.config.address(), std::time::Duration::from_secs(5))
                    .await?;
            *guard = Some(client);
        }
        Ok(guard)
    }

    pub async fn geometry_info(&self) -> Result<GeometryInfo> {
        let screenshot = self.config.screenshot_size;
        if let Some(device) = self.config.device_size {
            return Ok(GeometryInfo {
                geometry: Geometry { device, screenshot },
                source: "config",
            });
        }
        if let Some(device) = self.session.resolution() {
            return Ok(GeometryInfo {
                geometry: Geometry { device, screenshot },
                source: "maacore",
            });
        }
        if matches!(self.config.kind, DeviceKind::Playtools { .. }) {
            let guard = self.playtools().await?;
            if let Some(client) = guard.as_ref() {
                let (w, h) = client.size();
                return Ok(GeometryInfo {
                    geometry: Geometry {
                        device: (w as u32, h as u32),
                        screenshot,
                    },
                    source: "playtools",
                });
            }
        }
        Err(Error::Validation(format!(
            "device size unknown for {}: connect the device first or set devices[].device_size",
            self.name
        )))
    }

    pub async fn geometry(&self) -> Result<Geometry> {
        Ok(self.geometry_info().await?.geometry)
    }

    pub async fn geometry_hint(&self) -> Option<GeometryInfo> {
        let screenshot = self.config.screenshot_size;
        if let Some(device) = self.config.device_size {
            return Some(GeometryInfo {
                geometry: Geometry { device, screenshot },
                source: "config",
            });
        }
        if let Some(device) = self.session.resolution() {
            return Some(GeometryInfo {
                geometry: Geometry { device, screenshot },
                source: "maacore",
            });
        }
        if let Ok(guard) = self.playtools.try_lock()
            && let Some(client) = guard.as_ref()
        {
            let (w, h) = client.size();
            return Some(GeometryInfo {
                geometry: Geometry {
                    device: (w as u32, h as u32),
                    screenshot,
                },
                source: "playtools",
            });
        }
        None
    }

    fn load_hud(
        &self,
        path: Option<&PathBuf>,
        embedded: &'static [u8],
    ) -> Result<Option<HudTemplate>> {
        match path {
            Some(path) if path.as_os_str() == "none" => Ok(None),
            Some(path) => HudTemplate::load(
                path,
                self.config.hud_roi,
                self.config.screenshot_size,
                self.config.hud_threshold,
            )
            .map(Some),
            None if matches!(self.config.kind, DeviceKind::Playtools { .. })
                && self.config.hud_roi == DeviceConfig::default().hud_roi =>
            {
                HudTemplate::from_png(
                    embedded,
                    EMBEDDED_HUD_ROI,
                    EMBEDDED_HUD_SPACE,
                    self.config.hud_threshold,
                )
                .map(Some)
            }
            None => Ok(None),
        }
    }

    pub fn hud_template(&self) -> Result<Option<&HudTemplate>> {
        let loaded = self
            .hud
            .get_or_init(|| self.load_hud(self.config.hud_template.as_ref(), EMBEDDED_HUD));
        match loaded {
            Ok(v) => Ok(v.as_ref()),
            Err(e) => Err(Error::Image(format!("HUD template unavailable: {e}"))),
        }
    }

    pub fn paused_template(&self) -> Result<Option<&HudTemplate>> {
        let loaded = self.hud_paused.get_or_init(|| {
            self.load_hud(
                self.config.hud_paused_template.as_ref(),
                EMBEDDED_PAUSED_HUD,
            )
        });
        match loaded {
            Ok(v) => Ok(v.as_ref()),
            Err(e) => Err(Error::Image(format!("HUD template unavailable: {e}"))),
        }
    }

    pub async fn with_action<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let _guard = self.action_lock.lock().await;
        self.busy.store(true, Ordering::SeqCst);
        struct Reset<'a>(&'a AtomicBool);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset(&self.busy);
        f().await
    }

    pub async fn with_blocking_action<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.with_action(|| async {
            tokio::task::spawn_blocking(f)
                .await
                .map_err(|e| Error::CoreLoad(format!("worker task failed: {e}")))?
        })
        .await
    }
}

pub struct GeometryInfo {
    pub geometry: Geometry,
    pub source: &'static str,
}

const EMBEDDED_HUD: &[u8] = include_bytes!("../assets/hud/playcover_running.png");
const EMBEDDED_PAUSED_HUD: &[u8] = include_bytes!("../assets/hud/playcover_paused.png");
const EMBEDDED_HUD_ROI: (u32, u32, u32, u32) = (1180, 30, 60, 50);
const EMBEDDED_HUD_SPACE: (u32, u32) = (1280, 720);

pub struct DeviceSummary {
    pub name: String,
    pub kind: String,
    pub address: String,
    pub default: bool,
    pub connected: bool,
    pub busy: bool,
}

pub struct DeviceRegistry {
    devices: Vec<Arc<Device>>,
}

impl DeviceRegistry {
    pub fn from_config(config: &Config, factory: Arc<dyn CoreFactory>) -> Result<Self> {
        let mut devices = Vec::new();
        for d in &config.devices {
            let session = MaaSession::open_with_job_dir(
                factory.as_ref(),
                config.server.max_events,
                config.job_dir.clone(),
            )?;
            devices.push(Arc::new(Device {
                name: d.name.clone(),
                config: d.clone(),
                session,
                playtools: Mutex::new(None),
                hud: OnceLock::new(),
                hud_paused: OnceLock::new(),
                action_lock: Mutex::new(()),
                connect_lock: Mutex::new(()),
                busy: AtomicBool::new(false),
            }));
        }
        Ok(Self { devices })
    }

    pub fn get(&self, name: Option<&str>) -> Result<Arc<Device>> {
        match name {
            Some(name) => self
                .devices
                .iter()
                .find(|d| d.name == name)
                .cloned()
                .ok_or_else(|| {
                    let known = self
                        .devices
                        .iter()
                        .map(|d| d.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    Error::Config(format!("unknown device {name:?}; known devices: {known}"))
                }),
            None => self
                .devices
                .iter()
                .find(|d| d.config.default)
                .or_else(|| self.devices.first())
                .cloned()
                .ok_or_else(|| Error::Config("no devices configured".to_string())),
        }
    }

    pub fn list(&self) -> Vec<DeviceSummary> {
        self.devices
            .iter()
            .map(|d| DeviceSummary {
                name: d.name.clone(),
                kind: match &d.config.kind {
                    DeviceKind::Playtools { .. } => "playtools".to_string(),
                    DeviceKind::Adb { .. } => "adb".to_string(),
                },
                address: d.config.address().to_string(),
                default: d.config.default,
                connected: d.is_connected(),
                busy: d.is_busy(),
            })
            .collect()
    }
}
