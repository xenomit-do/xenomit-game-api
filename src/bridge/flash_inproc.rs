//! In-process Flash host: `xenomit_flash.dll` runs Flash on its own STA thread
//! inside this process, like DarkBot's InProcessPlayer. Calls are direct function
//! calls into the DLL (no pipe, no per-call connection, no global request
//! lock), the native LiveFrame is read straight from the host's memory, and
//! status is a plain struct.
//!
//! Only one host can run per process. Dropping the client stops it; a new
//! [`FlashInProcClient::start`] may follow (a relogin).

use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::flash::{decode_snapshot, window_verb};
use super::flash_ffi::{self as ffi, Api, Buffer, XfConfig, XfStatus};
use super::wire;
use crate::live::{LiveFrame, LiveStateReader};
use crate::model::{
    ActionResult, CallTiming, ClientStatus, FunctionCall, GameClient, GameSnapshot, Inspection,
    KeyAction, MouseAction, WindowCommand,
};
use crate::{ApiError, GameCommand, Result};

pub use super::flash_ffi::XfStatus as FlashHostStatus;
pub use super::flash_ffi::{FlashLogSink, set_log_sink as set_flash_log_sink};

/// Authenticated bootstrap plus launch options. The same six records the
/// out-of-process host reads from stdin; nothing here is written to argv.
#[derive(Clone, Debug, Default)]
pub struct FlashInProcConfig {
    pub movie: String,
    pub base_url: String,
    pub sid: String,
    pub flash_vars: String,
    pub width: i32,
    pub height: i32,
    /// "native" (LiveFrame) or "cached".
    pub backend: String,
    pub native_discovery: Option<String>,
    pub quality: Option<String>,
    pub wmode: Option<String>,
    /// Defaults to Flash.ocx beside xenomit_flash.dll.
    pub ocx_path: Option<String>,
    pub title: Option<String>,
    pub hidden: bool,
    pub hidden_draw_bootstrap: bool,
    /// 0 keeps the game default of 60.
    pub max_fps: u8,
    pub local_proxy_port: u16,
    pub clear_cache: Option<String>,
    pub game_ports: Option<String>,
    /// Also serve `\\.\pipe\xenomit_flash_<pid>` for external diagnostics.
    pub pipe_server: bool,
}

pub struct FlashInProcClient {
    api: &'static Api,
    native: AtomicBool,
    stopped: AtomicBool,
}

/// Default bound for frame calls; matches the host's FINVOKE bound.
const DEFAULT_TIMEOUT_MS: u32 = 0;

impl FlashInProcClient {
    /// Load the DLL (once per process) and start the Flash host. Returns
    /// after Flash is loaded and the bridged movie is playing.
    pub fn start(config: &FlashInProcConfig) -> Result<Self> {
        let api = ffi::library()?;
        let text = |value: &str, what: &str| ffi::c_text(value, what);
        let some = |value: &Option<String>, what: &str| -> Result<Option<CString>> {
            value.as_deref().map(|v| text(v, what)).transpose()
        };
        let movie = text(&config.movie, "movie")?;
        let base_url = text(&config.base_url, "base_url")?;
        let sid = text(&config.sid, "sid")?;
        let flash_vars = text(&config.flash_vars, "flash_vars")?;
        let backend = text(&config.backend, "backend")?;
        let discovery = some(&config.native_discovery, "native_discovery")?;
        let quality = some(&config.quality, "quality")?;
        let wmode = some(&config.wmode, "wmode")?;
        let ocx = some(&config.ocx_path, "ocx_path")?;
        let title = some(&config.title, "title")?;
        let clear_cache = some(&config.clear_cache, "clear_cache")?;
        let game_ports = some(&config.game_ports, "game_ports")?;
        let raw = XfConfig {
            size: std::mem::size_of::<XfConfig>() as u32,
            movie: movie.as_ptr(),
            base_url: base_url.as_ptr(),
            sid: sid.as_ptr(),
            flash_vars: flash_vars.as_ptr(),
            width: config.width,
            height: config.height,
            backend: backend.as_ptr(),
            native_discovery: ffi::optional(&discovery),
            quality: ffi::optional(&quality),
            wmode: ffi::optional(&wmode),
            ocx_path: ffi::optional(&ocx),
            user_agent: std::ptr::null(),
            title: ffi::optional(&title),
            hidden: config.hidden.into(),
            hidden_draw_bootstrap: config.hidden_draw_bootstrap.into(),
            max_fps: i32::from(config.max_fps),
            local_proxy_port: i32::from(config.local_proxy_port),
            clear_cache: ffi::optional(&clear_cache),
            game_ports: ffi::optional(&game_ports),
            publish_named_frame: 0,
            pipe_server: config.pipe_server.into(),
            log: Some(ffi::log_to_stderr),
            log_context: std::ptr::null_mut(),
        };
        let mut error = Buffer::new(api);
        // SAFETY: every pointer in `raw` outlives the call; the host copies them.
        let code = unsafe { (api.xf_start)(&raw, error.as_mut_ptr()) };
        ffi::check(code, error.text())?;
        Ok(Self {
            api,
            native: AtomicBool::new(config.backend == "native"),
            stopped: AtomicBool::new(false),
        })
    }

    /// Raw host status (frames, capture cost, network counters, CPU, memory).
    pub fn host_status(&self) -> Result<XfStatus> {
        let mut status = XfStatus {
            size: std::mem::size_of::<XfStatus>() as u32,
            ..XfStatus::default()
        };
        // SAFETY: `status` is a writable XfStatus with its size set.
        let code = unsafe { (self.api.xf_status)(&mut status) };
        ffi::check(code, String::new())?;
        Ok(status)
    }

    fn reader(&self) -> Option<LiveStateReader> {
        let mut base = std::ptr::null();
        let mut len = 0usize;
        // SAFETY: out-pointers are valid.
        if unsafe { (self.api.xf_live_frame)(&mut base, &mut len) } != ffi::XF_OK || base.is_null()
        {
            return None;
        }
        // SAFETY: the host keeps the frame mapped until the process exits.
        Some(unsafe { LiveStateReader::from_raw(base, len) })
    }

    fn envelope(&self, code: i32, buffer: &Buffer<'_>) -> Result<ActionResult> {
        let payload = ffi::check(code, buffer.text())?;
        let envelope: wire::ActionEnvelope = serde_json::from_str(&payload)
            .map_err(|error| ApiError::Protocol(format!("{error}; response: {payload}")))?;
        envelope.into_data()
    }

    /// Run a pipe-grammar command in-process (diagnostics and verbs without
    /// a dedicated export, e.g. `BRIDGE`, `CAPTURE 0`, `REFERENCE`).
    pub fn exchange(&self, command: &str) -> Result<String> {
        let command = ffi::c_text(command, "command")?;
        let mut reply = Buffer::new(self.api);
        // SAFETY: NUL-terminated command and a writable buffer.
        let code = unsafe { (self.api.xf_exchange)(command.as_ptr(), reply.as_mut_ptr()) };
        ffi::check(code, String::new())?;
        super::protocol::unwrap_reply(&reply.text())
    }

    /// Which direct actions resolved against the running game.
    pub fn capabilities(&self) -> Result<BTreeMap<String, bool>> {
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer.
        let code = unsafe { (self.api.xf_caps)(result.as_mut_ptr()) };
        let data = self.envelope(code, &result)?.data.unwrap_or(Value::Null);
        serde_json::from_value(data).map_err(|error| ApiError::Protocol(error.to_string()))
    }

    pub fn set_backend(&self, native: bool) -> Result<()> {
        let mode = ffi::c_text(if native { "native" } else { "cached" }, "backend")?;
        let mut error = Buffer::new(self.api);
        // SAFETY: NUL-terminated mode and a writable buffer.
        let code = unsafe { (self.api.xf_set_backend)(mode.as_ptr(), error.as_mut_ptr()) };
        ffi::check(code, error.text())?;
        self.native.store(native, Ordering::Relaxed);
        Ok(())
    }

    /// Stop the host now instead of when the last reference drops; other
    /// holders then see Unavailable. A later [`Self::start`] may follow.
    pub fn shutdown(&self) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            // SAFETY: plain value. The host ends its thread; the DLL stays loaded.
            unsafe { (self.api.xf_stop)(10_000) };
        }
    }

    fn native(&self) -> bool {
        self.native.load(Ordering::Relaxed)
    }

    pub fn set_blocking_patterns(&self, patterns: &[&str]) -> Result<()> {
        let owned = patterns
            .iter()
            .map(|pattern| ffi::c_text(pattern, "pattern"))
            .collect::<Result<Vec<_>>>()?;
        let pointers: Vec<_> = owned.iter().map(|value| value.as_ptr()).collect();
        // SAFETY: `pointers` holds `len` valid strings for the call.
        let code =
            unsafe { (self.api.xf_set_blocking_patterns)(pointers.as_ptr(), pointers.len()) };
        ffi::check(code, String::new()).map(|_| ())
    }

    /// Delete WinINet URL-cache entries containing `substring`.
    pub fn clear_cache(&self, substring: &str) -> Result<u32> {
        let substring = ffi::c_text(substring, "substring")?;
        let mut deleted = 0u32;
        // SAFETY: valid string and out-pointer.
        let code = unsafe { (self.api.xf_clear_cache)(substring.as_ptr(), &mut deleted) };
        ffi::check(code, String::new()).map(|_| deleted)
    }

    /// Trim the process working set (InProcessPlayer emptyWorkingSet).
    pub fn trim_memory(&self) -> Result<()> {
        // SAFETY: no arguments.
        ffi::check(unsafe { (self.api.xf_trim)() }, String::new()).map(|_| ())
    }

    pub fn send_text(&self, text: &str) -> Result<()> {
        let text = ffi::c_text(text, "text")?;
        // SAFETY: NUL-terminated text.
        ffi::check(unsafe { (self.api.xf_text)(text.as_ptr()) }, String::new()).map(|_| ())
    }

    pub fn paste_text(&self, text: &str) -> Result<()> {
        let text = ffi::c_text(text, "text")?;
        // SAFETY: NUL-terminated text.
        ffi::check(unsafe { (self.api.xf_paste)(text.as_ptr()) }, String::new()).map(|_| ())
    }

    /// Guarded read of this process' memory. Not synchronized with the
    /// Flash thread; hot state belongs to the LiveFrame.
    pub fn read_memory(&self, address: u64, out: &mut [u8]) -> Result<()> {
        // SAFETY: `out` is writable for its length; the host catches faults.
        let code = unsafe { (self.api.xf_read)(address, out.as_mut_ptr().cast(), out.len()) };
        ffi::check(code, "address is not readable".into()).map(|_| ())
    }

    /// First addresses of `pattern` (with optional `mask`, 0xff compares) in
    /// committed private read/write regions up to 32 MiB.
    pub fn query_bytes(&self, pattern: &[u8], mask: Option<&[u8]>, max: usize) -> Result<Vec<u64>> {
        if mask.is_some_and(|mask| mask.len() != pattern.len()) {
            return Err(ApiError::InvalidCall(
                "mask and pattern lengths differ".into(),
            ));
        }
        let mut out = vec![0u64; max.max(1)];
        let mut count = 0usize;
        // SAFETY: slices are valid for the lengths passed.
        let code = unsafe {
            (self.api.xf_query_bytes)(
                pattern.as_ptr(),
                mask.map_or(std::ptr::null(), <[u8]>::as_ptr),
                pattern.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut count,
            )
        };
        ffi::check(code, String::new())?;
        out.truncate(count);
        Ok(out)
    }

    /// Queue a move and return at once (InProcessPlayer's ring-queue path).
    /// The outcome is collected with [`Self::take_result`] or simply observed
    /// in the LiveFrame destination.
    pub fn move_hero_async(&self, x: i32, y: i32) -> Result<u64> {
        let mut ticket = 0u64;
        let mut error = Buffer::new(self.api);
        // SAFETY: writable buffer and ticket.
        let code =
            unsafe { (self.api.xf_move)(x, y, ffi::XF_ASYNC, error.as_mut_ptr(), &mut ticket) };
        ffi::check(code, error.text()).map(|_| ticket)
    }

    /// `Ok(None)` while an asynchronous request is still running. Tickets
    /// expire 10 s after completion.
    pub fn take_result(&self, ticket: u64) -> Result<Option<ActionResult>> {
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer.
        let code = unsafe { (self.api.xf_take_result)(ticket, result.as_mut_ptr()) };
        if code == ffi::XF_PENDING {
            return Ok(None);
        }
        self.envelope(code, &result).map(Some)
    }

    fn code(&self, code: i32) -> Result<()> {
        ffi::check(code, String::new()).map(|_| ())
    }
}

/// Status details use the same keys as the pipe host's STATUS line.
pub(crate) fn status_details(status: &XfStatus, live: bool) -> BTreeMap<String, Value> {
    let age = |value: u64| {
        if value == u64::MAX {
            Value::from(-1)
        } else {
            Value::from(value)
        }
    };
    let mut details = BTreeMap::new();
    let mut put = |key: &str, value: Value| {
        details.insert(key.to_owned(), value);
    };
    put("player", status.player.into());
    put("bridge", status.bridge.into());
    put("ready", status.ready.into());
    put("native", status.native.into());
    put("live", live.into());
    put("native_root", ffi::fixed_text(&status.native_root).into());
    put("parity_checks", status.parity_checks.into());
    put("parity_mismatches", status.parity_mismatches.into());
    put("bootstrap_draws", status.bootstrap_draws.into());
    put("bootstrap_failures", status.bootstrap_failures.into());
    put("frames", status.frames.into());
    put("fps_milli", status.fps_milli.into());
    put("captures", status.captures.into());
    put("capture_us_avg", status.capture_us_avg.into());
    put("capture_us_max", status.capture_us_max.into());
    put("item_updates", status.item_updates.into());
    put("item_refreshes", status.item_refreshes.into());
    put("state_age_ms", status.state_age_ms.into());
    put("net_reads", status.net_reads.into());
    put("net_read_age_ms", age(status.net_read_age_ms));
    put("game_socket", status.game_socket.into());
    put("game_recv", status.game_recv.into());
    put("game_recv_bytes", status.game_recv_bytes.into());
    put("game_recv_age_ms", age(status.game_recv_age_ms));
    put("vfs_hits", status.vfs_hits.into());
    put("blocked_requests", status.blocked_requests.into());
    put("cpu_pct", status.cpu_percent.into());
    put("mem_ws_bytes", status.working_set_bytes.into());
    details
}

fn flags(timing: CallTiming) -> u32 {
    match timing {
        CallTiming::Immediate => 0,
        CallTiming::InFrame => ffi::XF_FRAME,
    }
}

impl GameClient for FlashInProcClient {
    fn feature_state(&self) -> Result<crate::ClientFeatureState> {
        crate::features::feature_state(self)
    }
    fn poll_events(&self, after_sequence: u64) -> Result<crate::ClientEventBatch> {
        crate::features::poll_events(self, after_sequence)
    }
    fn status(&self) -> Result<ClientStatus> {
        let status = self.host_status()?;
        let live = self.native() && self.reader().is_some();
        Ok(ClientStatus {
            backend: if self.native() {
                "flash_inproc_native"
            } else {
                "flash_inproc"
            }
            .into(),
            connected: status.player == 1,
            ready: status.ready == 1,
            details: status_details(&status, live),
        })
    }
    fn ping(&self) -> Result<()> {
        let reply = self.exchange("BRIDGE")?;
        if reply == "dobot-bridge" {
            Ok(())
        } else {
            Err(ApiError::Protocol(format!(
                "unexpected ping response: {reply}"
            )))
        }
    }
    fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
        if target.is_none() && self.native() {
            let mut frame = LiveFrame::default();
            if !self.read_live_frame(&mut frame) {
                return Err(ApiError::Unavailable(
                    "native Flash frame unavailable or stale".into(),
                ));
            }
            return Ok(frame.to_game_snapshot());
        }
        let target = ffi::c_text(target.unwrap_or("stage"), "target")?;
        let mut result = Buffer::new(self.api);
        // SAFETY: NUL-terminated target and a writable buffer.
        let code = unsafe { (self.api.xf_state)(target.as_ptr(), result.as_mut_ptr()) };
        let payload = ffi::check(code, result.text())?;
        let value: Value = serde_json::from_str(&payload)
            .map_err(|error| ApiError::Protocol(format!("{error}; response: {payload}")))?;
        decode_snapshot(value)
    }
    fn read_live_frame(&self, frame: &mut LiveFrame) -> bool {
        let Some(reader) = self.reader() else {
            return false;
        };
        // A 60 Hz publisher can briefly own the seqlock; yield between
        // bounded attempts. Stale successful reads still fail immediately.
        for attempt in 0..8 {
            if reader.read_into(frame) {
                return frame.is_fresh(wire::now_unix_ms());
            }
            if attempt < 7 {
                std::thread::yield_now();
            }
        }
        false
    }
    fn inspect(&self, target: &str) -> Result<Inspection> {
        let target = ffi::c_text(target, "target")?;
        let mut result = Buffer::new(self.api);
        // SAFETY: NUL-terminated target and a writable buffer.
        let code = unsafe { (self.api.xf_inspect)(target.as_ptr(), result.as_mut_ptr()) };
        let payload = ffi::check(code, result.text())?;
        let envelope: wire::InspectionEnvelope = serde_json::from_str(&payload)
            .map_err(|error| ApiError::Protocol(format!("{error}; response: {payload}")))?;
        envelope.into_data()
    }
    fn call_function(&self, call: &FunctionCall) -> Result<ActionResult> {
        let target = ffi::c_text(&call.target, "target")?;
        let function = ffi::c_text(&call.function, "function")?;
        let args = serde_json::to_string(&call.arguments)
            .map_err(|error| ApiError::InvalidCall(error.to_string()))?;
        let args = ffi::c_text(&args, "arguments")?;
        let mut result = Buffer::new(self.api);
        // SAFETY: NUL-terminated strings, writable buffer, no ticket (sync).
        let code = unsafe {
            (self.api.xf_invoke)(
                target.as_ptr(),
                function.as_ptr(),
                args.as_ptr(),
                flags(call.timing),
                DEFAULT_TIMEOUT_MS,
                result.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        self.envelope(code, &result)
    }
    fn game_command(&self, command: &GameCommand) -> Result<ActionResult> {
        let json =
            serde_json::to_string(command).map_err(|e| ApiError::InvalidCall(e.to_string()))?;
        let json = ffi::c_text(&json, "command")?;
        let mut result = Buffer::new(self.api);
        // SAFETY: NUL-terminated JSON, writable buffer, sync call.
        let code = unsafe {
            (self.api.xf_game_command)(
                json.as_ptr(),
                0,
                DEFAULT_TIMEOUT_MS,
                result.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        self.envelope(code, &result)
    }
    fn move_hero(&self, x: i32, y: i32) -> Result<ActionResult> {
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer, sync call.
        let code =
            unsafe { (self.api.xf_move)(x, y, 0, result.as_mut_ptr(), std::ptr::null_mut()) };
        self.envelope(code, &result)
    }
    fn stop_hero(&self) -> Result<ActionResult> {
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer, sync call.
        let code = unsafe { (self.api.xf_stop_hero)(0, result.as_mut_ptr(), std::ptr::null_mut()) };
        self.envelope(code, &result)
    }
    fn select_target(&self, entity_id: i32, attack: bool) -> Result<ActionResult> {
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer, sync call.
        let code = unsafe {
            (self.api.xf_select)(
                entity_id,
                attack.into(),
                0,
                result.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        self.envelope(code, &result)
    }
    fn deselect_target(&self) -> Result<ActionResult> {
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer, sync call.
        let code = unsafe { (self.api.xf_deselect)(0, result.as_mut_ptr(), std::ptr::null_mut()) };
        self.envelope(code, &result)
    }
    fn collect(&self, entity_id: i32, at: Option<(i32, i32)>) -> Result<ActionResult> {
        let (x, y) = at.unwrap_or((ffi::XF_AT_ENTITY, ffi::XF_AT_ENTITY));
        let mut result = Buffer::new(self.api);
        // SAFETY: writable buffer, sync call.
        let code = unsafe {
            (self.api.xf_collect)(
                entity_id,
                x,
                y,
                0,
                result.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        self.envelope(code, &result)
    }
    fn send_key(&self, virtual_key: u32, action: KeyAction) -> Result<()> {
        let kind = match action {
            KeyAction::Down => 0,
            KeyAction::Up => 1,
            KeyAction::Click => 2,
        };
        // SAFETY: plain values.
        self.code(unsafe { (self.api.xf_key)(virtual_key, kind) })
    }
    fn send_mouse(&self, action: MouseAction, x: i32, y: i32) -> Result<()> {
        let kind = match action {
            MouseAction::Move => 0,
            MouseAction::Down => 1,
            MouseAction::Up => 2,
            MouseAction::Click => 3,
        };
        // SAFETY: plain values.
        self.code(unsafe { (self.api.xf_mouse)(kind, x, y) })
    }
    fn set_window(&self, command: WindowCommand) -> Result<()> {
        // SAFETY (all arms): plain values or a NUL-terminated string.
        let code = unsafe {
            match &command {
                WindowCommand::Visible { visible } => (self.api.xf_set_visible)((*visible).into()),
                WindowCommand::Position { x, y } => (self.api.xf_set_position)(*x, *y),
                WindowCommand::Size { width, height } => (self.api.xf_set_size)(*width, *height),
                WindowCommand::ClientSize { width, height } => {
                    (self.api.xf_set_client_size)(*width, *height)
                }
                WindowCommand::MinClientSize { width, height } => {
                    (self.api.xf_set_min_client_size)(*width, *height)
                }
                WindowCommand::Minimized { minimized } => {
                    (self.api.xf_set_minimized)((*minimized).into())
                }
                WindowCommand::Transparency { alpha } => {
                    (self.api.xf_set_transparency)(i32::from(*alpha))
                }
                WindowCommand::MaxFps { fps } => (self.api.xf_set_max_fps)(i32::from(*fps)),
                WindowCommand::Volume { percent } => (self.api.xf_set_volume)(i32::from(*percent)),
                WindowCommand::Quality { quality } => {
                    let quality = ffi::c_text(quality, "quality")?;
                    (self.api.xf_set_quality)(quality.as_ptr())
                }
                WindowCommand::UserInput { enabled } => {
                    (self.api.xf_set_user_input)((*enabled).into())
                }
            }
        };
        self.code(code).map_err(|error| match error {
            ApiError::InvalidCall(_) => {
                ApiError::InvalidCall(format!("rejected {}", window_verb(&command)))
            }
            other => other,
        })
    }
    fn reload_client(&self) -> Result<()> {
        let mut error = Buffer::new(self.api);
        // SAFETY: writable buffer.
        let code = unsafe { (self.api.xf_reload)(error.as_mut_ptr()) };
        ffi::check(code, error.text()).map(|_| ())
    }
}

impl Drop for FlashInProcClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_details_use_pipe_status_keys() {
        let mut status = XfStatus {
            player: 1,
            ready: 1,
            native: 1,
            fps_milli: 60_000,
            capture_us_avg: 250,
            net_read_age_ms: u64::MAX,
            game_recv_age_ms: 12,
            ..XfStatus::default()
        };
        status.native_root[..6].copy_from_slice(b"anchor");
        let details = status_details(&status, true);
        assert_eq!(details["fps_milli"], 60_000);
        assert_eq!(details["capture_us_avg"], 250);
        assert_eq!(details["native"], 1);
        assert_eq!(details["live"], true);
        assert_eq!(details["native_root"], "anchor");
        assert_eq!(details["net_read_age_ms"], -1);
        assert_eq!(details["game_recv_age_ms"], 12);
    }

    #[test]
    fn call_timing_maps_to_frame_flag() {
        assert_eq!(flags(CallTiming::Immediate), 0);
        assert_eq!(flags(CallTiming::InFrame), ffi::XF_FRAME);
    }
}
