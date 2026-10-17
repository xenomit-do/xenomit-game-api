//! ActiveX host adapter. STATE reads a host-side cache, never AS3 on demand.
//! Calls use the Flash-owned UI thread; FINVOKE completes after ENTER_FRAME.
use super::{BridgeTransport, Pipe, protocol, wire};
use crate::model::{
    ActionResult, ClientStatus, FunctionCall, GameClient, GameSnapshot, Inspection, KeyAction,
    MouseAction, StageInfo, WindowCommand,
};
use crate::{ApiError, GameCommand, Result};
use serde_json::json;
use std::sync::Arc;

pub(crate) const EVENT_MANAGER: &str =
    "net.bigpoint.darkorbit.managers.ScreenManager.getInstance().eventManager";
pub(crate) const SHIP_MANAGER: &str =
    "net.bigpoint.darkorbit.managers.ScreenManager.getInstance().map.getShipManager()";

pub struct FlashClient {
    pipe: Pipe,
}
impl FlashClient {
    pub fn connect(pid: u32) -> Result<Self> {
        if !cfg!(windows) {
            return Err(ApiError::UnsupportedPlatform(
                "Flash ActiveX requires Windows x64".into(),
            ));
        }
        Ok(Self::with_transport(Arc::new(FlashTransport { pid })))
    }
    /// Pepper host command socket. Same verbs as the Windows pipe:
    /// `STATE`, `INSPECT`, `INVOKE`, `FINVOKE`. Path is
    /// `/tmp/xenomit_flash_<pid>.sock`. Connecting waits until the first
    /// exchange, so a client can attach before Electron binds the socket.
    pub fn connect_linux_bridge(pid: u32) -> Result<Self> {
        #[cfg(unix)]
        {
            Ok(Self::with_transport(Arc::new(UnixFlashTransport { pid })))
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            Err(ApiError::UnsupportedPlatform(
                "the Pepper Flash command bridge is a Unix socket".into(),
            ))
        }
    }
    /// Diagnostic capture toggle. Disabled capture intentionally becomes stale.
    pub fn set_native_backend(&self, native: bool) -> Result<()> {
        let mode = if native { "native" } else { "cached" };
        let confirmed = self.pipe.command(&format!("BACKEND {mode}"))?;
        if confirmed.trim() != mode {
            return Err(ApiError::Protocol(
                "Flash did not confirm capture backend".into(),
            ));
        }
        Ok(())
    }
    /// Diagnostic capture toggle. Disabled capture intentionally becomes stale.
    pub fn set_capture_enabled(&self, enabled: bool) -> Result<()> {
        self.pipe
            .command(if enabled { "CAPTURE 1" } else { "CAPTURE 0" })
            .map(|_| ())
    }
    /// Toggle the native client window without pausing Flash.
    pub fn set_window_visible(&self, visible: bool) -> Result<()> {
        self.pipe
            .command(if visible { "WINDOW 1" } else { "WINDOW 0" })
            .map(|_| ())
    }
    /// Explicit slow AS reference for decoder validation, never native polling.
    pub fn reference_snapshot(&self) -> Result<GameSnapshot> {
        decode_snapshot(self.pipe.json_command("REFERENCE")?)
    }
    pub fn with_transport(transport: Arc<dyn BridgeTransport>) -> Self {
        Self {
            pipe: Pipe::new(transport),
        }
    }
    /// Host control verbs added with the in-process host. Older hosts answer
    /// `unknown command`, which is reported as Unavailable.
    fn host_verb(&self, command: &str) -> Result<()> {
        match self.pipe.command(command) {
            Ok(_) => Ok(()),
            Err(ApiError::Transport(message)) if message == "unknown command" => Err(
                ApiError::Unavailable(format!("this Flash host does not support {command}")),
            ),
            Err(error) => Err(error),
        }
    }
}

/// Pipe verb for a window command; shared with the in-process exchange path.
pub(crate) fn window_verb(command: &WindowCommand) -> String {
    match command {
        WindowCommand::Visible { visible } => format!("WINDOW {}", u8::from(*visible)),
        WindowCommand::Position { x, y } => format!("WINDOW pos {x} {y}"),
        WindowCommand::Size { width, height } => format!("WINDOW size {width} {height}"),
        WindowCommand::ClientSize { width, height } => format!("WINDOW client {width} {height}"),
        WindowCommand::MinClientSize { width, height } => {
            format!("WINDOW minclient {width} {height}")
        }
        WindowCommand::Minimized { minimized } => {
            format!("WINDOW minimize {}", u8::from(*minimized))
        }
        WindowCommand::Transparency { alpha } => format!("WINDOW alpha {alpha}"),
        WindowCommand::MaxFps { fps } => format!("FPS {fps}"),
        WindowCommand::Volume { percent } => format!("VOLUME {percent}"),
        WindowCommand::Quality { quality } => format!("QUALITY {quality}"),
        WindowCommand::UserInput { enabled } => format!("INPUT {}", u8::from(*enabled)),
    }
}

pub(crate) fn key_verb(action: KeyAction) -> &'static str {
    match action {
        KeyAction::Down => "down",
        KeyAction::Up => "up",
        KeyAction::Click => "click",
    }
}

pub(crate) fn mouse_verb(action: MouseAction) -> &'static str {
    match action {
        MouseAction::Move => "move",
        MouseAction::Down => "down",
        MouseAction::Up => "up",
        MouseAction::Click => "click",
    }
}
impl GameClient for FlashClient {
    fn feature_state(&self) -> Result<crate::ClientFeatureState> {
        crate::features::feature_state(self)
    }
    fn poll_events(&self, after_sequence: u64) -> Result<crate::ClientEventBatch> {
        crate::features::poll_events(self, after_sequence)
    }
    fn status(&self) -> Result<ClientStatus> {
        super::status(&self.pipe, "flash_activex", |_| {})
    }
    fn ping(&self) -> Result<()> {
        super::ping(&self.pipe)
    }
    fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
        let target = target.unwrap_or("stage");
        protocol::validate_token(target, "target")?;
        let value: serde_json::Value = match self.pipe.json_command(&format!("STATE\t{target}")) {
            Err(ApiError::Transport(message)) if message == "state unavailable or stale" => {
                return Err(ApiError::Unavailable(message));
            }
            result => result?,
        };
        decode_snapshot(value)
    }

    fn inspect(&self, target: &str) -> Result<Inspection> {
        super::inspect(&self.pipe, target)
    }
    fn call_function(&self, call: &FunctionCall) -> Result<ActionResult> {
        super::call_function(&self.pipe, call, "INVOKE", "FINVOKE")
    }
    fn game_command(&self, command: &GameCommand) -> Result<ActionResult> {
        let value =
            serde_json::to_value(command).map_err(|e| ApiError::InvalidCall(e.to_string()))?;
        self.call_function(
            &FunctionCall::new("xenomit", "gameCommand")
                .with_arguments(vec![value])
                .in_frame(),
        )
    }
    // Movement, deselect and collect-at use the same reflected calls the
    // runtime executor sends today, so they work with every Flash host.
    fn move_hero(&self, x: i32, y: i32) -> Result<ActionResult> {
        self.call_function(
            &FunctionCall::new(EVENT_MANAGER, "moveHeroToCordinates")
                .with_arguments(vec![json!(x), json!(y)])
                .in_frame(),
        )
    }
    fn stop_hero(&self) -> Result<ActionResult> {
        self.call_function(&FunctionCall::new(EVENT_MANAGER, "stopHero").in_frame())
    }
    fn select_target(&self, entity_id: i32, attack: bool) -> Result<ActionResult> {
        self.call_function(
            &FunctionCall::new("xenomit", "direct")
                .with_arguments(vec![json!("select"), json!(entity_id), json!(attack)])
                .in_frame(),
        )
    }
    fn deselect_target(&self) -> Result<ActionResult> {
        self.call_function(&FunctionCall::new(SHIP_MANAGER, "deselectSelectedShip"))
    }
    fn collect(&self, entity_id: i32, at: Option<(i32, i32)>) -> Result<ActionResult> {
        let call = match at {
            Some((x, y)) => FunctionCall::new(EVENT_MANAGER, "moveHeroToCordinates")
                .with_arguments(vec![
                    json!(x),
                    json!(y),
                    json!({"$xenomit_entity_id": entity_id}),
                ]),
            None => FunctionCall::new("xenomit", "direct")
                .with_arguments(vec![json!("collect"), json!(entity_id)]),
        };
        self.call_function(&call.in_frame())
    }
    fn send_key(&self, virtual_key: u32, action: KeyAction) -> Result<()> {
        self.host_verb(&format!("KEY {virtual_key} {}", key_verb(action)))
    }
    fn send_mouse(&self, action: MouseAction, x: i32, y: i32) -> Result<()> {
        self.host_verb(&format!("MOUSE {} {x} {y}", mouse_verb(action)))
    }
    fn set_window(&self, command: WindowCommand) -> Result<()> {
        self.host_verb(&window_verb(&command))
    }
    fn reload_client(&self) -> Result<()> {
        self.host_verb("RELOAD")
    }
}

// Expansion runs in the controller, avoiding repeated long field names and
// XML escaping in Flash. Each payload carries its own bounded field dictionary.
pub(crate) fn decode_snapshot(mut value: serde_json::Value) -> Result<GameSnapshot> {
    expand_packed_entities(&mut value)?;
    let envelope: wire::SnapshotEnvelope =
        serde_json::from_value(value).map_err(|e| ApiError::Protocol(e.to_string()))?;
    let data = envelope.into_data()?;
    let observed = data
        .observed_at_unix_ms
        .ok_or_else(|| ApiError::Protocol("Flash snapshot missing capture timestamp".into()))?;
    // Timestamp describes capture, not polling. Do not rejuvenate stale data.
    let now = wire::now_unix_ms();
    if now.saturating_sub(observed) > 1500 || observed > now.saturating_add(1000) {
        return Err(ApiError::Unavailable(
            "Flash snapshot is stale or has an invalid timestamp".into(),
        ));
    }
    Ok(GameSnapshot {
        observed_at_unix_ms: observed,
        navigation: data.navigation,
        stage: StageInfo {
            width: data.stage_width,
            height: data.stage_height,
        },
        map: data.map.map(Into::into),
        entities: data.entities.into_iter().map(Into::into).collect(),
        visited: data.visited,
        truncated: data.truncated,
    })
}

fn expand_packed_entities(envelope: &mut serde_json::Value) -> Result<()> {
    use serde_json::{Map, Value};
    let Some(data) = envelope.get_mut("data").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    let Some(encoding) = data.get("entitiesEncoding") else {
        return Ok(());
    };
    if encoding.as_str() != Some("packed_v1") {
        return Err(ApiError::Protocol(
            "Unsupported Flash entity encoding".into(),
        ));
    }
    let schema = data
        .get("entityFields")
        .and_then(Value::as_array)
        .filter(|a| a.len() <= 64)
        .ok_or_else(|| ApiError::Protocol("Missing/oversize packed field dictionary".into()))?;
    let mut names = Vec::new();
    for name in schema {
        let name = name
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128 && !["id", "worldX", "worldY"].contains(s))
            .ok_or_else(|| ApiError::Protocol("Invalid packed field name".into()))?;
        if names.iter().any(|s| s == name) {
            return Err(ApiError::Protocol("Duplicate packed field name".into()));
        }
        names.push(name.to_owned());
    }
    let rows = data
        .get_mut("entities")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ApiError::Protocol("Packed Flash entities missing".into()))?;
    if rows.len() > 2000 {
        return Err(ApiError::Protocol("Too many packed Flash entities".into()));
    }
    for row in rows {
        let values = row
            .as_array()
            .filter(|r| r.len() == 8)
            .ok_or_else(|| ApiError::Protocol("Invalid packed Flash entity".into()))?;
        let id = values[0]
            .as_i64()
            .ok_or_else(|| ApiError::Protocol("Invalid packed entity ID".into()))?;
        if !values[1].is_string() || !values[2].is_string() {
            return Err(ApiError::Protocol(
                "Invalid packed entity name/class".into(),
            ));
        }
        let packed = values[7]
            .as_object()
            .ok_or_else(|| ApiError::Protocol("Invalid packed entity fields".into()))?;
        let mut fields = Map::from_iter([
            ("id".into(), values[0].clone()),
            ("worldX".into(), values[5].clone()),
            ("worldY".into(), values[6].clone()),
        ]);
        for (key, value) in packed {
            let index = key
                .parse::<usize>()
                .ok()
                .and_then(|i| names.get(i))
                .ok_or_else(|| ApiError::Protocol("Unknown packed entity field".into()))?;
            fields.insert(index.clone(), value.clone());
        }
        *row = serde_json::json!({"path":format!("map/{id}"),"className":values[1],"name":values[2],"x":values[3],"y":values[4],"fields":fields});
    }
    Ok(())
}

/// Separate endpoint avoids accidentally treating Ruffle native snapshots as
/// Flash data. Each operation has a 12-second deadline, including I/O.
struct FlashTransport {
    pid: u32,
}
impl BridgeTransport for FlashTransport {
    fn exchange(&self, command: &str) -> Result<String> {
        if command.len() > 65535 {
            return Err(ApiError::InvalidCall(
                "Flash command exceeds 65535 bytes".into(),
            ));
        }
        #[cfg(windows)]
        {
            bounded_exchange(&format!(r"\\.\pipe\xenomit_flash_{}", self.pid), command)
        }
        #[cfg(not(windows))]
        {
            let _ = (self.pid, command);
            Err(ApiError::UnsupportedPlatform(
                "Flash ActiveX requires Windows".into(),
            ))
        }
    }
}

#[cfg(windows)]
fn bounded_exchange(name: &str, command: &str) -> Result<String> {
    use std::{
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::*,
        Storage::FileSystem::*,
        System::{IO::*, Threading::*},
    };
    struct Event(HANDLE);
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let deadline = Instant::now() + Duration::from_secs(12);
    let file = loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            .open(name)
        {
            Ok(file) => break file,
            Err(e) if matches!(e.raw_os_error(), Some(2 | 231)) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(ApiError::Transport(e.to_string())),
        }
    };
    let pipe = file.as_raw_handle() as HANDLE;
    let event = Event(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) });
    if event.0.is_null() {
        return Err(ApiError::Transport(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    // Pending I/O is cancelled AND drained before stack OVERLAPPED/buffers drop.
    let io = |buffer: *mut u8, length: u32, write: bool| -> Result<u32> {
        let mut overlap: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlap.hEvent = event.0;
        let mut bytes = 0;
        unsafe {
            ResetEvent(event.0);
            let started = if write {
                WriteFile(pipe, buffer, length, &mut bytes, &mut overlap)
            } else {
                ReadFile(pipe, buffer, length, &mut bytes, &mut overlap)
            };
            if started == 0 {
                if GetLastError() != ERROR_IO_PENDING {
                    return Err(ApiError::Transport(
                        std::io::Error::last_os_error().to_string(),
                    ));
                }
                let remaining = deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(u128::from(u32::MAX)) as u32;
                if WaitForSingleObject(event.0, remaining) != WAIT_OBJECT_0 {
                    CancelIoEx(pipe, &overlap);
                    GetOverlappedResult(pipe, &overlap, &mut bytes, 1);
                    return Err(ApiError::Transport(
                        "Flash pipe timed out; action outcome may be unknown; no automatic retry"
                            .into(),
                    ));
                }
                if GetOverlappedResult(pipe, &overlap, &mut bytes, 0) == 0 {
                    return Err(ApiError::Transport(
                        std::io::Error::last_os_error().to_string(),
                    ));
                }
            }
        }
        Ok(bytes)
    };
    if io(command.as_ptr() as *mut u8, command.len() as u32, true)? as usize != command.len() {
        return Err(ApiError::Transport("Incomplete Flash command write".into()));
    }
    let mut reply = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = io(buffer.as_mut_ptr(), buffer.len() as u32, false)? as usize;
        if read == 0 {
            return Err(ApiError::Transport("Flash pipe closed before reply".into()));
        }
        if reply.len() + read > 1024 * 1024 {
            return Err(ApiError::ResponseTooLarge { limit: 1024 * 1024 });
        }
        reply.extend_from_slice(&buffer[..read]);
        if reply.ends_with(b"\n") {
            break;
        }
    }
    // The server retains the pipe until this receipt, avoiding the unbounded
    // FlushFileBuffers used by the MVP and preventing DisconnectNamedPipe from
    // discarding a buffered reply before the reader consumes it.
    let mut ack = b'A';
    io(&mut ack, 1, true)?;
    String::from_utf8(reply).map_err(|e| ApiError::Protocol(e.to_string()))
}

/// Filesystem socket the Linux host binds. Electron's pid is the bot's child
/// pid, because the launcher `exec`s Electron in place.
#[cfg(unix)]
pub(crate) fn linux_bridge_path(pid: u32) -> String {
    format!("/tmp/xenomit_flash_{pid}.sock")
}

#[cfg(unix)]
struct UnixFlashTransport {
    pid: u32,
}

#[cfg(unix)]
impl BridgeTransport for UnixFlashTransport {
    fn exchange(&self, command: &str) -> Result<String> {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;
        use std::time::{Duration, Instant};
        if command.len() > 65535 {
            return Err(ApiError::InvalidCall(
                "Flash command exceeds 65535 bytes".into(),
            ));
        }
        if command.contains(['\n', '\0']) {
            return Err(ApiError::InvalidCall(
                "Flash command cannot contain a newline or NUL".into(),
            ));
        }
        if !std::path::Path::new(&format!("/proc/{}", self.pid)).is_dir() {
            return Err(ApiError::Unavailable("flash host process is gone".into()));
        }
        let path = linux_bridge_path(self.pid);
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut stream = loop {
            match UnixStream::connect(&path) {
                Ok(stream) => break stream,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(ApiError::Transport(error.to_string())),
            }
        };
        let remaining = || deadline.saturating_duration_since(Instant::now());
        stream
            .set_write_timeout(Some(remaining().max(Duration::from_millis(1))))
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        stream
            .set_read_timeout(Some(remaining().max(Duration::from_millis(1))))
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        let line = command.to_owned() + "\n";
        stream
            .write_all(line.as_bytes())
            .map_err(|error| ApiError::Transport(error.to_string()))?;
        let mut reply = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let read = match stream.read(&mut buffer) {
                Ok(0) => {
                    return Err(ApiError::Transport(
                        "Flash bridge closed before reply".into(),
                    ));
                }
                Ok(n) => n,
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                    return Err(ApiError::Transport(
                        "Flash bridge timed out; action outcome may be unknown; no automatic retry"
                            .into(),
                    ));
                }
                Err(error) => return Err(ApiError::Transport(error.to_string())),
            };
            if reply.len() + read > 1024 * 1024 {
                return Err(ApiError::ResponseTooLarge { limit: 1024 * 1024 });
            }
            reply.extend_from_slice(&buffer[..read]);
            if reply.contains(&b'\n') {
                break;
            }
        }
        if let Some(end) = reply.iter().position(|byte| *byte == b'\n') {
            reply.truncate(end + 1);
        }
        // The host keeps the socket open until this receipt so it does not
        // tear down the stream while the reply is still unread.
        let _ = stream.write_all(b"A");
        let _ = stream.flush();
        String::from_utf8(reply).map_err(|error| ApiError::Protocol(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    struct Fake {
        commands: Mutex<Vec<String>>,
        reply: String,
    }
    impl BridgeTransport for Fake {
        fn exchange(&self, c: &str) -> Result<String> {
            self.commands.lock().unwrap().push(c.into());
            Ok(self.reply.clone())
        }
    }
    fn fake(reply: String) -> Arc<Fake> {
        Arc::new(Fake {
            commands: Mutex::new(vec![]),
            reply,
        })
    }
    #[test]
    fn backend_switch_requires_confirmation() {
        for (native, mode) in [(true, "native"), (false, "cached")] {
            let t = fake(format!("OK {mode}\n"));
            FlashClient::with_transport(t.clone())
                .set_native_backend(native)
                .unwrap();
            assert_eq!(*t.commands.lock().unwrap(), vec![format!("BACKEND {mode}")]);
        }
        for reply in ["OK cached\n", "ERR unsupported runtime\n"] {
            assert!(
                FlashClient::with_transport(fake(reply.into()))
                    .set_native_backend(true)
                    .is_err()
            );
        }
    }
    #[test]
    fn frame_commands_use_flash_frame_queue() {
        let t = fake("OK {\"status\":\"ok\",\"data\":null}\n".into());
        let c = FlashClient::with_transport(t.clone());
        c.call_function(&FunctionCall::new("map/123", "click").in_frame())
            .unwrap();
        c.game_command(&GameCommand::JumpPortal { portal_id: 7 })
            .unwrap();
        let sent = t.commands.lock().unwrap();
        assert!(sent[0].starts_with("FINVOKE\tmap/123\tclick\t"));
        assert!(sent[1].starts_with("FINVOKE\txenomit\tgameCommand\t"));
    }
    #[test]
    fn cached_snapshot_preserves_navigation_and_capture_time() {
        let now = wire::now_unix_ms();
        let t = fake(format!(
            "OK {{\"status\":\"ok\",\"data\":{{\"observedAtUnixMs\":{now},\"navigation\":{{\"map_id\":1,\"geometry_ready\":true}}}}}}\n"
        ));
        let c = FlashClient::with_transport(t.clone());
        let s = c.snapshot(None).unwrap();
        assert_eq!(s.observed_at_unix_ms, now);
        assert!(s.navigation.geometry_ready);
        assert_eq!(s.navigation.map_id, Some(1));
        assert_eq!(t.commands.lock().unwrap()[0], "STATE\tstage");
    }
    #[test]
    fn compact_item_flags_default_to_false() {
        let now = wire::now_unix_ms();
        let t = fake(format!(
            "OK {{\"status\":\"ok\",\"data\":{{\"observedAtUnixMs\":{now},\"navigation\":{{\"hero\":{{\"items\":{{\"laser\":{{\"quantity\":5,\"available\":true}},\"empty\":{{}}}}}}}}}}}}\n"
        ));
        let snapshot = FlashClient::with_transport(t).snapshot(None).unwrap();
        let laser = &snapshot.navigation.hero.items["laser"];
        assert_eq!(laser.quantity, Some(5.0));
        assert!(laser.available);
        assert!(!laser.blocked);
        assert_eq!(
            snapshot.navigation.hero.items["empty"],
            crate::navigation::ItemState::default()
        );
    }
    #[test]
    fn packed_entities_preserve_world_health_and_entity_targets() {
        let now = wire::now_unix_ms();
        let reply = serde_json::json!({"status":"ok","data":{"observedAtUnixMs":now,"entitiesEncoding":"packed_v1","entityFields":["isNpc","health","maxHealth","typeID"],"entities":[[77,"Ship","npc",5,6,700,800,{"0":true,"1":900,"2":1000,"3":42}]]}});
        let c = FlashClient::with_transport(fake(format!("OK {reply}\n")));
        let snapshot = c.snapshot(None).unwrap();
        let e = &snapshot.entities[0];
        assert_eq!(e.path, "map/77");
        assert_eq!(e.fields["worldX"], 700);
        assert_eq!(e.fields["health"], 900);
        assert_eq!(e.fields["isNpc"], true);
        assert_eq!(e.fields["typeID"], 42);
    }
    #[test]
    fn malformed_packed_entities_fail_closed() {
        for data in [
            serde_json::json!({"entitiesEncoding":"packed_v2","entities":[]}),
            serde_json::json!({"entitiesEncoding":"packed_v1","entities":[[1]]}),
            serde_json::json!({"entitiesEncoding":"packed_v1","entityFields":["health"],"entities":[[1,"Ship","name",0,0,0,0,{"999":1}]]}),
        ] {
            let mut envelope = serde_json::json!({"status":"ok","data":data});
            assert!(expand_packed_entities(&mut envelope).is_err());
        }
    }
    #[test]
    fn host_stale_state_is_unavailability_not_transport_failure() {
        let c = FlashClient::with_transport(fake("ERR state unavailable or stale\n".into()));
        assert!(matches!(c.snapshot(None), Err(ApiError::Unavailable(_))));
    }
    #[test]
    fn stale_cache_is_rejected() {
        let c = FlashClient::with_transport(fake(
            "OK {\"status\":\"ok\",\"data\":{\"observedAtUnixMs\":1}}\n".into(),
        ));
        assert!(matches!(c.snapshot(None), Err(ApiError::Unavailable(_))));
    }
    #[test]
    fn as_error_is_not_success() {
        let c = FlashClient::with_transport(fake(
            "OK {\"status\":\"error\",\"message\":\"missing method\"}\n".into(),
        ));
        assert!(
            c.call_function(&FunctionCall::new("root", "absent"))
                .is_err()
        );
    }
}
