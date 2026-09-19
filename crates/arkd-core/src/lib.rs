pub mod battle;
pub mod battle_state;
pub mod catalog;
pub mod config;
pub mod core_api;
pub mod device;
pub mod error;
pub mod events;
pub mod hud;
pub mod messages;
pub mod playtools;
pub mod screen;
pub mod session;

pub use battle_state::{BattleReport, BattleState};
pub use config::{Config, DeviceConfig, DeviceKind, MaaConfig, ServerConfig};
pub use core_api::{CoreFactory, MaaCoreApi, RealCoreFactory, Runtime, RuntimeInfo};
pub use device::{Device, DeviceRegistry, DeviceSummary};
pub use error::{Error, Result};
pub use events::{Event, EventLog, EventsPage};
pub use playtools::{Frame, PlayToolsClient, TouchPhase};
pub use screen::{CoordSpace, EncodeOpts, Encoded, Geometry, ImageFormat};
pub use session::{
    BgrFrame, ConnectionInfo, DEFAULT_STALL, MaaSession, QueuedTask, StartResult, Status,
    StopResult, WaitCondition, WaitOutcome,
};

#[cfg(feature = "fake")]
pub use core_api::fake::{FakeCore, FakeCoreFactory};
#[cfg(feature = "fake")]
pub use playtools::fake::{FakePlayToolsServer, TouchEvent};
