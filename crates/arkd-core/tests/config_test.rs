#![cfg(feature = "fake")]

use std::path::PathBuf;

use arkd_core::config::{Config, DeviceKind};
use arkd_core::error::Error;

#[test]
fn defaults_on_macos() {
    let path = PathBuf::from("/nonexistent/arkd-config-test.toml");
    let config = Config::load(Some(&path)).unwrap();
    #[cfg(target_os = "macos")]
    {
        assert_eq!(
            config.maa.core_dir,
            PathBuf::from("/Applications/MAA.app/Contents/Frameworks")
        );
        assert_eq!(
            config.maa.resource_dir,
            PathBuf::from("/Applications/MAA.app/Contents/Resources")
        );
        assert_eq!(config.devices.len(), 1);
        assert_eq!(config.devices[0].name, "playcover");
        assert!(matches!(
            config.devices[0].kind,
            DeviceKind::Playtools { .. }
        ));
        assert!(config.devices[0].default);
        assert_eq!(config.devices[0].touch_mode(), "MacPlayTools");
        assert_eq!(config.devices[0].connect_config, "General");
    }
    assert_eq!(config.server.max_events, 2000);
}

#[test]
fn bind_default_and_env_override() {
    let path = PathBuf::from("/nonexistent/arkd-config-bind.toml");
    let config = Config::load(Some(&path)).unwrap();
    assert_eq!(config.server.bind.to_string(), "127.0.0.1:7717");
    unsafe {
        std::env::set_var("ARKD_BIND", "0.0.0.0:9999");
    }
    let config = Config::load(Some(&path)).unwrap();
    unsafe {
        std::env::remove_var("ARKD_BIND");
    }
    assert_eq!(config.server.bind.to_string(), "0.0.0.0:9999");
}

fn write_temp(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn toml_with_two_devices_and_one_default() {
    let path = write_temp(
        "arkd-config-two-devices.toml",
        r#"
[[devices]]
name = "playcover"
kind = "playtools"
address = "127.0.0.1:1717"
default = true

[[devices]]
name = "mumu"
kind = "adb"
adb_path = "adb"
address = "127.0.0.1:16384"
touch_mode = "maatouch"
"#,
    );
    let config = Config::load(Some(&path)).unwrap();
    assert_eq!(config.devices.len(), 2);
    assert_eq!(config.devices[1].name, "mumu");
    assert!(matches!(config.devices[1].kind, DeviceKind::Adb { .. }));
    assert_eq!(config.devices[1].touch_mode(), "maatouch");
}

#[test]
fn zero_defaults_with_two_devices_is_an_error() {
    let path = write_temp(
        "arkd-config-zero-default.toml",
        r#"
[[devices]]
name = "a"
kind = "playtools"
address = "127.0.0.1:1717"

[[devices]]
name = "b"
kind = "playtools"
address = "127.0.0.1:1718"
"#,
    );
    assert!(matches!(
        Config::load(Some(&path)).unwrap_err(),
        Error::Config(_)
    ));
}

#[test]
fn two_defaults_is_an_error() {
    let path = write_temp(
        "arkd-config-two-default.toml",
        r#"
[[devices]]
name = "a"
kind = "playtools"
address = "127.0.0.1:1717"
default = true

[[devices]]
name = "b"
kind = "playtools"
address = "127.0.0.1:1718"
default = true
"#,
    );
    assert!(matches!(
        Config::load(Some(&path)).unwrap_err(),
        Error::Config(_)
    ));
}

#[test]
fn single_device_becomes_default_implicitly() {
    let path = write_temp(
        "arkd-config-single.toml",
        r#"
[[devices]]
name = "only"
kind = "playtools"
address = "127.0.0.1:1717"
"#,
    );
    let config = Config::load(Some(&path)).unwrap();
    assert!(config.devices[0].default);
}

#[test]
fn duplicate_names_is_an_error() {
    let dup = write_temp(
        "arkd-config-dup.toml",
        r#"
[[devices]]
name = "a"
kind = "playtools"
address = "127.0.0.1:1717"
default = true

[[devices]]
name = "a"
kind = "playtools"
address = "127.0.0.1:1718"
"#,
    );
    assert!(matches!(
        Config::load(Some(&dup)).unwrap_err(),
        Error::Config(_)
    ));
}

#[test]
fn describe_is_json_friendly() {
    let config = Config::load(Some(&PathBuf::from("/nonexistent/x.toml"))).unwrap();
    let json = serde_json::to_string(&config.describe()).unwrap();
    assert!(json.contains("core_dir"));
    assert!(json.contains("bind"));
}
