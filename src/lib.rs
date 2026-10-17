//! A client-agnostic API for reading game state and issuing client actions.
//!
//! Consumers depend on `GameClient` and the shared data types in `model`.
//! Two backends adapt xenomit-client through `connect`:
//!
//! - `ClientBackend::Native` (default): per-frame state from the
//!   `Local\xenomit_state_<pid>` shared-memory mapping plus `NINVOKE`/
//!   `NFINVOKE` calls resolved natively in the player. State polling never
//!   stalls the game.
//! - `ClientBackend::LegacyPipe`: the original AS3 reflection commands
//!   (`STATE`, `INSPECT`, `INVOKE`, `FINVOKE`) over the named pipe. Each
//!   request runs on the player thread and stalls the game — kept for
//!   compatibility with old clients and for diagnostics.
//!
//! The authentic Flash host connects through `connect_flash` /
//! `connect_flash_native` (out-of-process exe, pipe) or runs inside this
//! process through `start_flash_inproc` (`xenomit_flash.dll`, direct calls).

use std::sync::Arc;

pub mod behavior;
pub mod bridge;
mod error;
pub mod features;
pub use features::*;
pub mod live;
pub mod model;
pub mod navigation;
pub mod star_map;
pub use navigation::{
    GalaxyGateInfo, GameCommand, GroupInvite, GroupMemberInfo, GroupState, HeroState, ItemState,
    MapDescriptor, MapLink, NavigationState, Obstacle, ObstacleKind, ObstacleShape, PortalInfo,
    StarSystem,
};

pub use behavior::{
    BehaviorRuntime, ModuleConfig, ModuleDescriptor, PluginDescriptor, SettingChoice,
    SettingDescriptor, SettingKind, set_setting_value, setting_value,
};
pub use bridge::{
    BridgeTransport, FlashClient, FlashHostStatus, FlashInProcClient, FlashInProcConfig,
    FlashLogSink, FlashNativeClient, LegacyPipeClient, NamedPipeTransport, NativeClient,
    set_flash_log_sink,
};
pub use error::{ApiError, Result};
pub use live::{LiveEntity, LiveFrame, LiveHero, LiveKind, LiveMap, LiveStateReader};
pub use model::{
    ActionResult, Bounds, CallTiming, ClientBackend, ClientStatus, EntityCategory, FunctionCall,
    GameClient, GameEntity, GameMapInfo, GameSnapshot, Inspection, KeyAction, MouseAction, Point,
    SafePlace, StageInfo, WindowCommand,
};

/// Connect a [`GameClient`] for a running xenomit-client process.
pub fn connect(pid: u32, backend: ClientBackend) -> Result<Arc<dyn GameClient>> {
    match backend {
        ClientBackend::Native => {
            NativeClient::connect(pid).map(|client| Arc::new(client) as Arc<dyn GameClient>)
        }
        ClientBackend::LegacyPipe => {
            LegacyPipeClient::connect(pid).map(|client| Arc::new(client) as Arc<dyn GameClient>)
        }
    }
}

/// Connect to the authentic Flash ActiveX host (`xenomit-flash-client`).
/// Its snapshots are cached in the host and use a separate bounded pipe;
/// the Ruffle backend preference does not apply to this client. Linux uses
/// [`connect_flash_native`]; this entry point stays Windows-only.
pub fn connect_flash(pid: u32) -> Result<Arc<dyn GameClient>> {
    FlashClient::connect(pid).map(|client| Arc::new(client) as Arc<dyn GameClient>)
}

/// Memory-offset Flash capture. Snapshots come from the version 2 frame.
/// Commands, reflected targets, inspect, features, and events use the Flash
/// protocol: the Windows pipe, or `/tmp/xenomit_flash_<pid>.sock` on Linux.
pub fn connect_flash_native(pid: u32) -> Result<Arc<dyn GameClient>> {
    FlashNativeClient::connect(pid).map(|client| Arc::new(client) as Arc<dyn GameClient>)
}

/// Host Flash inside this process through `xenomit_flash.dll`, the way
/// DarkBot hosts InProcessPlayer: direct calls, no pipe, and the native LiveFrame
/// read from the host's own memory. Windows x64 only; one host per process.
/// The DLL is found through `XENOMIT_FLASH_DLL`, beside the executable, or in
/// a sibling `xenomit-flash-client/build` directory.
pub fn start_flash_inproc(config: &FlashInProcConfig) -> Result<Arc<FlashInProcClient>> {
    FlashInProcClient::start(config).map(Arc::new)
}
