use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::{Mutex, MutexGuard};

use crate::{
    config::{Config, DeviceConfig, DeviceKind},
    core_api::CoreFactory,
    error::{Error, Result},
    playtools::PlayToolsClient,
    screen::Geometry,
    session::{ConnectionInfo, MaaSession},
};

pub struct Device {
    pub name: String,
    pub config: DeviceConfig,
    pub session: MaaSession,
    playtools: Mutex<Option<PlayToolsClient>>,
    action_lock: Mutex<()>,
    busy: AtomicBool,
}

impl Device {
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn is_connected(&self) -> bool {
        self.session.status().connected
    }

    pub async fn ensure_connected(&self) -> Result<ConnectionInfo> {
        if self.session.status().connected {
            return self
                .session
                .status()
                .connection
                .ok_or_else(|| Error::NotConnected("no connection record".to_string()));
        }
        self.session.connect(
            self.config.adb_path(),
            self.config.address(),
            &self.config.connect_config,
            self.config.touch_mode(),
        )
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

    pub async fn geometry(&self) -> Result<Geometry> {
        let screenshot = self.config.screenshot_size;
        if matches!(self.config.kind, DeviceKind::Playtools { .. })
            && let Ok(mut guard) = self.playtools().await
            && let Some(client) = guard.as_mut()
            && let Ok((w, h)) = client.size().await
        {
            return Ok(Geometry {
                device: (w as u32, h as u32),
                screenshot,
            });
        }
        Ok(Geometry {
            device: screenshot,
            screenshot,
        })
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
}

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
                action_lock: Mutex::new(()),
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
