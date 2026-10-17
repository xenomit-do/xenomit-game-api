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
    BridgeTransport, FlashClient, FlashNativeClient, LegacyPipeClient, NamedPipeTransport,
    NativeClient,
};
pub use error::{ApiError, Result};
pub use live::{LiveEntity, LiveFrame, LiveHero, LiveKind, LiveMap, LiveStateReader};
pub use model::{
    ActionResult, Bounds, CallTiming, ClientBackend, ClientStatus, EntityCategory, FunctionCall,
    GameClient, GameEntity, GameMapInfo, GameSnapshot, Inspection, Point, SafePlace, StageInfo,
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
