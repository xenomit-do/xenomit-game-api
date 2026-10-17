use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Result;

/// Selectable client implementation for [`crate::connect`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientBackend {
    /// Shared-memory frames and native invoke commands (default).
    #[default]
    Native,
    /// The original AS3 reflection pipe commands; every request runs on the
    /// player thread and stalls the game.
    LegacyPipe,
}

impl ClientBackend {
    /// Human-facing name for selectors and labels.
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "Native (shared memory)",
            Self::LegacyPipe => "Legacy pipe",
        }
    }
}

/// Stable interface that behavior engines use regardless of the game client.
///
/// Methods are synchronous so the interface can be implemented without a
/// particular async runtime. Implementations may perform blocking I/O; async
/// callers should run these methods on a blocking worker.
pub trait GameClient: Send + Sync {
    /// Supplementary GUI/PET/gate/economy state. Read on a blocking worker;
    /// this can require a bounded player-thread observation on graphical clients.
    fn feature_state(&self) -> Result<crate::ClientFeatureState> {
        Err(crate::ApiError::Unavailable(
            "client feature observations are unsupported".into(),
        ))
    }
    /// Replay new log/game events using a consumer-owned cursor. Cursor zero
    /// reads retained history; a new client session needs a fresh cursor.
    fn poll_events(&self, _after_sequence: u64) -> Result<crate::ClientEventBatch> {
        Err(crate::ApiError::Unavailable(
            "client event stream is unsupported".into(),
        ))
    }
    /// Optional typed manager operations. Clients must explicitly implement
    /// these; unsupported backends cannot silently pretend they succeeded.
    fn game_command(&self, _command: &crate::GameCommand) -> Result<ActionResult> {
        Err(crate::ApiError::Unavailable(
            "this client does not support manager commands".into(),
        ))
    }
    /// Check whether the underlying client is running and ready to accept work.
    fn status(&self) -> Result<ClientStatus>;

    /// Verify that the client bridge responds to a round trip.
    fn ping(&self) -> Result<()>;

    /// Read a normalized snapshot. The target is client-specific; None uses
    /// the implementation's default game view.
    fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot>;

    /// Inspect a reflected target and return its available members.
    fn inspect(&self, target: &str) -> Result<Inspection>;

    /// Invoke a client function. The call's timing controls whether it runs
    /// between frames or in the next real frame when the client supports it.
    fn call_function(&self, call: &FunctionCall) -> Result<ActionResult>;

    /// Decode the newest live state frame into `frame`, reusing its
    /// allocations for 60 Hz polling. Backends without a live channel return
    /// false; the native backend returns false while the shared-memory
    /// mapping is missing or its newest frame is stale.
    fn read_live_frame(&self, _frame: &mut crate::live::LiveFrame) -> bool {
        false
    }

    // Direct interactions (InProcessPlayer moveShip/selectEntity/collectBox).
    // They run the game's own manager methods in the next frame and return
    // the real ActionScript result. Unsupported backends report Unavailable.

    /// Move the hero to world coordinates.
    fn move_hero(&self, _x: i32, _y: i32) -> Result<ActionResult> {
        Err(unsupported("direct movement"))
    }
    /// Stop the hero's current movement.
    fn stop_hero(&self) -> Result<ActionResult> {
        Err(unsupported("direct movement"))
    }
    /// Lock an entity by ID; `attack` also starts the laser attack.
    fn select_target(&self, _entity_id: i32, _attack: bool) -> Result<ActionResult> {
        Err(unsupported("direct target selection"))
    }
    /// Release the locked target.
    fn deselect_target(&self) -> Result<ActionResult> {
        Err(unsupported("direct target selection"))
    }
    /// Move to `at` (or the entity itself) and collect the entity there.
    fn collect(&self, _entity_id: i32, _at: Option<(i32, i32)>) -> Result<ActionResult> {
        Err(unsupported("direct collection"))
    }
    /// Keyboard input delivered to the game window (hotkeys).
    fn send_key(&self, _virtual_key: u32, _action: KeyAction) -> Result<()> {
        Err(unsupported("keyboard input"))
    }
    /// Mouse input in game-window client coordinates.
    fn send_mouse(&self, _action: MouseAction, _x: i32, _y: i32) -> Result<()> {
        Err(unsupported("mouse input"))
    }
    /// Game window and rendering controls.
    fn set_window(&self, _command: WindowCommand) -> Result<()> {
        Err(unsupported("window control"))
    }
    /// Reload the game movie without restarting the client.
    fn reload_client(&self) -> Result<()> {
        Err(unsupported("client reload"))
    }
}

fn unsupported(what: &str) -> crate::ApiError {
    crate::ApiError::Unavailable(format!("{what} is unsupported by this client"))
}

/// Keyboard action for [`GameClient::send_key`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Down,
    Up,
    Click,
}

/// Mouse action for [`GameClient::send_mouse`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Move,
    Down,
    Up,
    Click,
}

/// Window and rendering commands (InProcessPlayer's window/handler surface).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WindowCommand {
    Visible {
        visible: bool,
    },
    Position {
        x: i32,
        y: i32,
    },
    Size {
        width: i32,
        height: i32,
    },
    ClientSize {
        width: i32,
        height: i32,
    },
    MinClientSize {
        width: i32,
        height: i32,
    },
    Minimized {
        minimized: bool,
    },
    /// 0..=255; 255 is opaque.
    Transparency {
        alpha: u8,
    },
    /// 0 restores the game default (60); otherwise 1..=100.
    MaxFps {
        fps: u8,
    },
    /// 0..=100.
    Volume {
        percent: u8,
    },
    /// low, medium, high or best.
    Quality {
        quality: String,
    },
    /// False drops real keyboard/mouse input to the game; injected input stays.
    UserInput {
        enabled: bool,
    },
}

/// Uniform connection/bridge health, with implementation-specific details
/// left available for diagnostics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientStatus {
    pub backend: String,
    pub connected: bool,
    pub ready: bool,
    #[serde(default)]
    pub details: BTreeMap<String, Value>,
}

/// A state observation and its capture metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameSnapshot {
    #[serde(default)]
    pub navigation: crate::NavigationState,
    /// Unix time in milliseconds when this adapter received the snapshot.
    pub observed_at_unix_ms: u64,
    pub stage: StageInfo,
    /// Current map metadata, world bounds, player position, and known safe
    /// places when the client exposes its live map model.
    #[serde(default)]
    pub map: Option<GameMapInfo>,
    pub entities: Vec<GameEntity>,
    /// Number of display objects visited by the bridge, including objects
    /// filtered out of entities.
    pub visited: usize,
    /// True if the bridge stopped at its traversal limit.
    pub truncated: bool,
}

/// The active game map and the world-space data needed to render its overview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameMapInfo {
    pub id: Option<i32>,
    pub name: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub player_position: Option<Point>,
    #[serde(default)]
    pub safe_places: Vec<SafePlace>,
    pub in_safe_place: Option<bool>,
}

/// A safe location detected on the current map (portal, base, or battle station).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SafePlace {
    pub kind: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    /// Radius used by DarkBot's safety-place drawing and proximity checks.
    pub radius: f64,
}

/// Dimensions of the game display area, measured in stage pixels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StageInfo {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// A normalized object discovered in a client snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameEntity {
    /// Stable only within the current snapshot; it is a display-list path.
    pub path: String,
    pub class_name: String,
    pub name: String,
    pub category: EntityCategory,
    /// Stage-space coordinates in pixels, not world coordinates.
    pub position: Point,
    pub bounds: Bounds,
    /// Common fields normalized from aliases where possible.
    pub id: Option<Value>,
    pub health: Option<f64>,
    pub max_health: Option<f64>,
    pub shield: Option<f64>,
    pub max_shield: Option<f64>,
    /// Original primitive fields from the client, including fields this crate
    /// does not know yet. This keeps the shared API forward-compatible.
    #[serde(default)]
    pub fields: Map<String, Value>,
}

impl GameEntity {
    /// Look up a field from the raw client attributes.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }
}

/// Best-effort category derived from class/name hints. Unknown is expected for
/// many game objects; consumers should use class_name and fields when they
/// need client-specific distinctions.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityCategory {
    Player,
    Ship,
    Npc,
    Portal,
    Projectile,
    #[default]
    Unknown,
}

/// A point in a 2D display coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Axis-aligned display bounds in stage pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Reflected type information returned by an inspection request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    pub name: String,
    pub is_class: bool,
    pub qualified: String,
    #[serde(default)]
    pub methods: Vec<MemberMethod>,
    #[serde(default)]
    pub accessors: Vec<MemberProperty>,
    #[serde(default)]
    pub variables: Vec<MemberVariable>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberMethod {
    pub name: String,
    pub params: usize,
    pub owner: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberProperty {
    pub name: String,
    pub access: String,
    pub owner: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemberVariable {
    pub name: String,
    pub owner: String,
}

/// Frame scheduling requested for a function call.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallTiming {
    /// Invoke outside a frame callback. Suitable for ordinary reflected calls.
    #[default]
    Immediate,
    /// Run inside the next real frame callback. Some movement methods require
    /// this because they depend on frame context.
    InFrame,
}

/// A client function invocation in the shared API format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub target: String,
    pub function: String,
    #[serde(default)]
    pub arguments: Vec<Value>,
    #[serde(default)]
    pub timing: CallTiming,
}

impl FunctionCall {
    pub fn new(target: impl Into<String>, function: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            function: function.into(),
            arguments: Vec::new(),
            timing: CallTiming::Immediate,
        }
    }

    pub fn with_arguments(mut self, arguments: Vec<Value>) -> Self {
        self.arguments = arguments;
        self
    }

    pub fn in_frame(mut self) -> Self {
        self.timing = CallTiming::InFrame;
        self
    }
}

/// Successful result from a function invocation. A missing data represents
/// an ActionScript undefined/null result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub data: Option<Value>,
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_backend_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(ClientBackend::Native).unwrap(),
            serde_json::json!("native")
        );
        assert_eq!(
            serde_json::to_value(ClientBackend::LegacyPipe).unwrap(),
            serde_json::json!("legacy_pipe")
        );
        assert_eq!(
            serde_json::from_value::<ClientBackend>(serde_json::json!("legacy_pipe")).unwrap(),
            ClientBackend::LegacyPipe
        );
        assert_eq!(ClientBackend::default(), ClientBackend::Native);
        assert_eq!(ClientBackend::Native.label(), "Native (shared memory)");
        assert_eq!(ClientBackend::LegacyPipe.label(), "Legacy pipe");
    }
}
