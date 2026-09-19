#![cfg(feature = "fake")]

use std::sync::Arc;

use arkd_core::config::{Config, DeviceConfig, DeviceKind};
use arkd_core::core_api::fake::FakeCoreFactory;
use arkd_core::device::DeviceRegistry;
use arkd_core::error::Error;

fn adb_device() -> DeviceConfig {
    DeviceConfig {
        name: "mumu".to_string(),
        default: true,
        kind: DeviceKind::Adb {
            adb_path: "/opt/platform-tools/adb".to_string(),
            address: "127.0.0.1:16384".to_string(),
            touch_mode: "maatouch".to_string(),
        },
        connect_config: "MuMuEmulator12".to_string(),
        ..DeviceConfig::default()
    }
}

#[tokio::test]
async fn ensure_connected_uses_configured_values() {
    let factory = Arc::new(FakeCoreFactory::new());
    let core = factory.core.clone();
    let config = Config {
        devices: vec![adb_device()],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    let device = registry.get(None).unwrap();
    let info = device.ensure_connected().await.unwrap();
    assert_eq!(info.what, "Connected");
    assert_eq!(
        core.connect_calls.lock().unwrap().as_slice(),
        &[(
            "/opt/platform-tools/adb".to_string(),
            "127.0.0.1:16384".to_string(),
            "MuMuEmulator12".to_string()
        )]
    );
    let opts = core.instance_options.lock().unwrap();
    assert!(opts.iter().any(|(_, v)| v == "maatouch"));
}

#[tokio::test]
async fn ensure_connected_is_lazy_and_connects_once() {
    let factory = Arc::new(FakeCoreFactory::new());
    let core = factory.core.clone();
    let config = Config {
        devices: vec![adb_device()],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    let device = registry.get(None).unwrap();
    assert!(!device.is_connected());
    device.ensure_connected().await.unwrap();
    device.ensure_connected().await.unwrap();
    assert_eq!(core.connect_calls.lock().unwrap().len(), 1);
    assert!(device.is_connected());
}

#[tokio::test]
async fn adb_devices_have_no_playtools() {
    let factory = Arc::new(FakeCoreFactory::new());
    let config = Config {
        devices: vec![adb_device()],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    let device = registry.get(None).unwrap();
    let err = device.playtools().await.err().unwrap();
    assert!(matches!(err, Error::PlayTools(_)));
    assert!(err.to_string().contains("not a PlayTools device"), "{err}");
}

#[test]
fn get_picks_the_default_and_rejects_unknown_names() {
    let factory = Arc::new(FakeCoreFactory::new());
    let config = Config {
        devices: vec![
            DeviceConfig {
                name: "first".to_string(),
                default: false,
                ..DeviceConfig::default()
            },
            DeviceConfig {
                name: "second".to_string(),
                default: true,
                ..DeviceConfig::default()
            },
        ],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    assert_eq!(registry.get(None).unwrap().name, "second");
    assert_eq!(registry.get(Some("first")).unwrap().name, "first");
    let err = registry.get(Some("nope")).err().unwrap();
    assert!(matches!(err, Error::Config(_)));
    assert!(err.to_string().contains("first"), "{err}");
    assert!(err.to_string().contains("second"), "{err}");
    let list = registry.list();
    assert_eq!(list.len(), 2);
    assert!(list[1].default);
    assert!(!list[1].connected);
    assert!(!list[1].busy);
}

#[tokio::test]
async fn geometry_prefers_configured_device_size() {
    let factory = Arc::new(FakeCoreFactory::new());
    let config = Config {
        devices: vec![DeviceConfig {
            name: "playcover".to_string(),
            default: true,
            device_size: Some((1920, 1080)),
            ..DeviceConfig::default()
        }],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    let device = registry.get(None).unwrap();
    let geo = device.geometry().await.unwrap();
    assert_eq!(geo.device, (1920, 1080));
    assert_eq!(geo.screenshot, (1280, 720));
}

#[tokio::test]
async fn geometry_uses_an_already_connected_playtools_client() {
    use arkd_core::playtools::Frame;
    use arkd_core::playtools::fake::FakePlayToolsServer;

    let server = FakePlayToolsServer::spawn(
        Frame {
            width: 4,
            height: 2,
            bgr: vec![0u8; 4 * 2 * 3],
        },
        3,
    )
    .await;
    let factory = Arc::new(FakeCoreFactory::new());
    let config = Config {
        devices: vec![DeviceConfig {
            name: "playcover".to_string(),
            default: true,
            kind: DeviceKind::Playtools {
                address: server.addr.to_string(),
            },
            ..DeviceConfig::default()
        }],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    let device = registry.get(None).unwrap();
    drop(device.playtools().await.unwrap());
    let geo = device.geometry().await.unwrap();
    assert_eq!(geo.device, (4, 2));
}

#[tokio::test]
async fn geometry_falls_back_to_screenshot_size_without_a_socket() {
    let factory = Arc::new(FakeCoreFactory::new());
    let config = Config {
        devices: vec![DeviceConfig {
            name: "playcover".to_string(),
            default: true,
            kind: DeviceKind::Playtools {
                address: "127.0.0.1:1".to_string(),
            },
            ..DeviceConfig::default()
        }],
        ..Config::default()
    };
    let registry = DeviceRegistry::from_config(&config, factory).unwrap();
    let device = registry.get(None).unwrap();
    let geo = device.geometry().await.unwrap();
    assert_eq!(geo.device, (1280, 720));
}
