//! Dynamic binding to `xenomit_flash.dll` (`include/xenomit_flash.h` in
//! xenomit-flash-client, `XF_API_VERSION` 1).
//!
//! The DLL hosts Flash inside this process on its own STA thread, the way
//! DarkBot loads InProcessPlayer through JNI. Every export is safe to call from
//! any thread; Flash work is queued to the host thread and waited on with a
//! bound. The library stays loaded for the life of the process because
//! Flash.ocx cannot be unloaded safely.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::{ApiError, Result};

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) const API_VERSION: u32 = 1;

pub(crate) const XF_OK: i32 = 0;
pub(crate) const XF_TIMEOUT_UNKNOWN: i32 = 1;
pub(crate) const XF_QUEUE_FULL: i32 = 2;
pub(crate) const XF_NOT_READY: i32 = 3;
pub(crate) const XF_FLASH_ERROR: i32 = 4;
pub(crate) const XF_INVALID: i32 = 5;
pub(crate) const XF_ALREADY_RUNNING: i32 = 6;
pub(crate) const XF_SHUTTING_DOWN: i32 = 7;
pub(crate) const XF_PENDING: i32 = 8;
pub(crate) const XF_START_FAILED: i32 = 9;
pub(crate) const XF_UNKNOWN_TICKET: i32 = 10;

pub(crate) const XF_FRAME: u32 = 1;
pub(crate) const XF_ASYNC: u32 = 2;
pub(crate) const XF_AT_ENTITY: i32 = i32::MIN;

#[repr(C)]
pub(crate) struct XfBuf {
    data: *mut c_char,
    len: usize,
}

pub(crate) type XfLogFn = unsafe extern "C" fn(*mut c_void, i32, *const c_char);

#[repr(C)]
pub(crate) struct XfConfig {
    pub size: u32,
    pub movie: *const c_char,
    pub base_url: *const c_char,
    pub sid: *const c_char,
    pub flash_vars: *const c_char,
    pub width: i32,
    pub height: i32,
    pub backend: *const c_char,
    pub native_discovery: *const c_char,
    pub quality: *const c_char,
    pub wmode: *const c_char,
    pub ocx_path: *const c_char,
    pub user_agent: *const c_char,
    pub title: *const c_char,
    pub hidden: i32,
    pub hidden_draw_bootstrap: i32,
    pub max_fps: i32,
    pub local_proxy_port: i32,
    pub clear_cache: *const c_char,
    pub game_ports: *const c_char,
    pub publish_named_frame: i32,
    pub pipe_server: i32,
    pub log: Option<XfLogFn>,
    pub log_context: *mut c_void,
}

/// Plain host status filled by `xf_status`; mirrors `XfStatus` in C.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XfStatus {
    pub size: u32,
    pub player: u8,
    pub bridge: u8,
    pub ready: u8,
    pub native: u8,
    pub frames: u64,
    pub fps_milli: u64,
    pub captures: u64,
    pub capture_us_avg: u64,
    pub capture_us_max: u64,
    pub state_age_ms: u64,
    pub item_updates: u64,
    pub item_refreshes: u64,
    pub bootstrap_draws: u64,
    pub bootstrap_failures: u64,
    pub parity_checks: u64,
    pub parity_mismatches: u64,
    pub native_root: [u8; 16],
    pub net_reads: u64,
    pub net_read_age_ms: u64,
    pub game_socket: u64,
    pub game_recv: u64,
    pub game_recv_bytes: u64,
    pub game_recv_age_ms: u64,
    pub vfs_hits: u64,
    pub blocked_requests: u64,
    pub cpu_percent: f64,
    pub working_set_bytes: u64,
}

type Call = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    u32,
    u32,
    *mut XfBuf,
    *mut u64,
) -> i32;

macro_rules! exports {
    ($($name:ident: $ty:ty),* $(,)?) => {
        #[allow(non_snake_case, dead_code)]
        pub(crate) struct Api {
            $(pub $name: $ty,)*
        }
        impl Api {
            #[cfg(windows)]
            unsafe fn resolve(module: windows_sys::Win32::Foundation::HMODULE) -> std::result::Result<Self, String> {
                use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
                Ok(Self {
                    $($name: {
                        let symbol = concat!(stringify!($name), "\0");
                        // SAFETY: the name is NUL-terminated; the export's C
                        // signature is the one declared in this table.
                        match unsafe { GetProcAddress(module, symbol.as_ptr()) } {
                            Some(function) => unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(function) },
                            None => return Err(format!("xenomit_flash.dll lacks {}", stringify!($name))),
                        }
                    },)*
                })
            }
        }
    };
}

exports! {
    xf_api_version: unsafe extern "C" fn() -> u32,
    xf_free: unsafe extern "C" fn(*mut XfBuf),
    xf_start: unsafe extern "C" fn(*const XfConfig, *mut XfBuf) -> i32,
    xf_stop: unsafe extern "C" fn(u32) -> i32,
    xf_wait: unsafe extern "C" fn(u32, *mut i32) -> i32,
    xf_reload: unsafe extern "C" fn(*mut XfBuf) -> i32,
    xf_status: unsafe extern "C" fn(*mut XfStatus) -> i32,
    xf_live_frame: unsafe extern "C" fn(*mut *const u8, *mut usize) -> i32,
    xf_state: unsafe extern "C" fn(*const c_char, *mut XfBuf) -> i32,
    xf_inspect: unsafe extern "C" fn(*const c_char, *mut XfBuf) -> i32,
    xf_invoke: Call,
    xf_game_command: unsafe extern "C" fn(*const c_char, u32, u32, *mut XfBuf, *mut u64) -> i32,
    xf_take_result: unsafe extern "C" fn(u64, *mut XfBuf) -> i32,
    xf_move: unsafe extern "C" fn(i32, i32, u32, *mut XfBuf, *mut u64) -> i32,
    xf_stop_hero: unsafe extern "C" fn(u32, *mut XfBuf, *mut u64) -> i32,
    xf_select: unsafe extern "C" fn(i32, i32, u32, *mut XfBuf, *mut u64) -> i32,
    xf_deselect: unsafe extern "C" fn(u32, *mut XfBuf, *mut u64) -> i32,
    xf_collect: unsafe extern "C" fn(i32, i32, i32, u32, *mut XfBuf, *mut u64) -> i32,
    xf_use_item: unsafe extern "C" fn(*const c_char, i32, u32, *mut XfBuf, *mut u64) -> i32,
    xf_refine: unsafe extern "C" fn(i32, u64, u32, *mut XfBuf, *mut u64) -> i32,
    xf_notify: unsafe extern "C" fn(*const c_char, *const c_char, u32, *mut XfBuf, *mut u64) -> i32,
    xf_caps: unsafe extern "C" fn(*mut XfBuf) -> i32,
    xf_mouse: unsafe extern "C" fn(i32, i32, i32) -> i32,
    xf_key: unsafe extern "C" fn(u32, i32) -> i32,
    xf_text: unsafe extern "C" fn(*const c_char) -> i32,
    xf_paste: unsafe extern "C" fn(*const c_char) -> i32,
    xf_set_user_input: unsafe extern "C" fn(i32) -> i32,
    xf_set_visible: unsafe extern "C" fn(i32) -> i32,
    xf_set_position: unsafe extern "C" fn(i32, i32) -> i32,
    xf_set_size: unsafe extern "C" fn(i32, i32) -> i32,
    xf_set_client_size: unsafe extern "C" fn(i32, i32) -> i32,
    xf_set_min_client_size: unsafe extern "C" fn(i32, i32) -> i32,
    xf_set_minimized: unsafe extern "C" fn(i32) -> i32,
    xf_set_transparency: unsafe extern "C" fn(i32) -> i32,
    xf_set_max_fps: unsafe extern "C" fn(i32) -> i32,
    xf_set_volume: unsafe extern "C" fn(i32) -> i32,
    xf_set_quality: unsafe extern "C" fn(*const c_char) -> i32,
    xf_set_backend: unsafe extern "C" fn(*const c_char, *mut XfBuf) -> i32,
    xf_set_blocking_patterns: unsafe extern "C" fn(*const *const c_char, usize) -> i32,
    xf_clear_cache: unsafe extern "C" fn(*const c_char, *mut u32) -> i32,
    xf_trim: unsafe extern "C" fn() -> i32,
    xf_read: unsafe extern "C" fn(u64, *mut c_void, usize) -> i32,
    xf_query_bytes: unsafe extern "C" fn(*const u8, *const u8, usize, *mut u64, usize, *mut usize) -> i32,
    xf_exchange: unsafe extern "C" fn(*const c_char, *mut XfBuf) -> i32,
}

static LIBRARY: OnceLock<std::result::Result<Api, String>> = OnceLock::new();

/// Load `xenomit_flash.dll` once. Search order: `XENOMIT_FLASH_DLL`, beside
/// the current executable, then `xenomit-flash-client/build` beside the
/// executable or any of its ancestors (the sibling-repository layout).
pub(crate) fn library() -> Result<&'static Api> {
    LIBRARY
        .get_or_init(load)
        .as_ref()
        .map_err(|error| ApiError::Unavailable(error.clone()))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("XENOMIT_FLASH_DLL") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("xenomit_flash.dll"));
            for ancestor in dir.ancestors() {
                paths.push(sibling_build(ancestor));
            }
        }
    }
    paths
}

#[cfg_attr(not(windows), allow(dead_code))]
fn sibling_build(dir: &Path) -> PathBuf {
    dir.join("xenomit-flash-client")
        .join("build")
        .join("xenomit_flash.dll")
}

#[cfg(windows)]
fn load() -> std::result::Result<Api, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LoadLibraryExW,
    };
    let path = candidates()
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| "xenomit_flash.dll not found; set XENOMIT_FLASH_DLL".to_owned())?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path; the module is never freed.
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    };
    if module.is_null() {
        return Err(format!(
            "could not load {}: Windows error {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: exported names and signatures follow xenomit_flash.h.
    let api = unsafe { Api::resolve(module) }?;
    // SAFETY: no arguments.
    let version = unsafe { (api.xf_api_version)() };
    if version != API_VERSION {
        return Err(format!(
            "{} implements API {version}, expected {API_VERSION}",
            path.display()
        ));
    }
    Ok(api)
}

#[cfg(not(windows))]
fn load() -> std::result::Result<Api, String> {
    Err("the in-process Flash host requires Windows x64".into())
}

impl XfBuf {
    pub(crate) fn new() -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
        }
    }
}

/// Owns an `XfBuf` and frees it through the DLL.
pub(crate) struct Buffer<'a> {
    api: &'a Api,
    raw: XfBuf,
}

impl<'a> Buffer<'a> {
    pub(crate) fn new(api: &'a Api) -> Self {
        Self {
            api,
            raw: XfBuf::new(),
        }
    }
    pub(crate) fn as_mut_ptr(&mut self) -> *mut XfBuf {
        &mut self.raw
    }
    pub(crate) fn text(&self) -> String {
        if self.raw.data.is_null() || self.raw.len == 0 {
            return String::new();
        }
        // SAFETY: the DLL wrote `len` bytes at `data`.
        let bytes = unsafe { std::slice::from_raw_parts(self.raw.data.cast::<u8>(), self.raw.len) };
        String::from_utf8_lossy(bytes).into_owned()
    }
}

impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        // SAFETY: the buffer was produced by this DLL (or is empty).
        unsafe { (self.api.xf_free)(&mut self.raw) };
    }
}

/// Owned NUL-terminated argument; rejects interior NUL bytes.
pub(crate) fn c_text(value: &str, what: &str) -> Result<CString> {
    CString::new(value).map_err(|_| ApiError::InvalidCall(format!("{what} contains a NUL byte")))
}

pub(crate) fn optional(value: &Option<CString>) -> *const c_char {
    value
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr())
}

/// Map an `XfCode` and its text to the crate's error model.
pub(crate) fn check(code: i32, text: String) -> Result<String> {
    match code {
        XF_OK => Ok(text),
        XF_TIMEOUT_UNKNOWN => Err(ApiError::Transport(format!(
            "{text}; action outcome may be unknown; no automatic retry"
        ))),
        XF_QUEUE_FULL => Err(ApiError::Transport(or(text, "Flash call queue full"))),
        XF_NOT_READY | XF_SHUTTING_DOWN => {
            Err(ApiError::Unavailable(or(text, "Flash host not ready")))
        }
        XF_FLASH_ERROR => Err(ApiError::ActionFailed {
            message: or(text, "Flash call failed"),
            stack: None,
        }),
        XF_INVALID => Err(ApiError::InvalidCall(or(
            text,
            "rejected by the Flash host",
        ))),
        XF_ALREADY_RUNNING => Err(ApiError::Unavailable(or(
            text,
            "a Flash host is already running in this process",
        ))),
        XF_START_FAILED => Err(ApiError::Unavailable(or(
            text,
            "Flash host failed to start",
        ))),
        XF_UNKNOWN_TICKET => Err(ApiError::InvalidCall("unknown or expired ticket".into())),
        XF_PENDING => Err(ApiError::Unavailable("result still pending".into())),
        other => Err(ApiError::Protocol(format!(
            "unknown Flash host code {other}: {text}"
        ))),
    }
}

fn or(text: String, fallback: &str) -> String {
    if text.is_empty() {
        fallback.to_owned()
    } else {
        text
    }
}

/// `native_root` is a NUL-terminated C string inside a fixed array.
pub(crate) fn fixed_text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Forward host log lines to stderr with a prefix; never blocks the host.
/// Receives host log lines: level 0 info, 1 warning, 2 error.
pub type FlashLogSink = std::sync::Arc<dyn Fn(i32, &str) + Send + Sync>;

static LOG_SINK: std::sync::RwLock<Option<FlashLogSink>> = std::sync::RwLock::new(None);

/// Route host log lines to `sink` (`None` restores stderr). The host calls
/// it from its own threads; the sink must not block or call back into the host.
pub fn set_log_sink(sink: Option<FlashLogSink>) {
    if let Ok(mut slot) = LOG_SINK.write() {
        *slot = sink;
    }
}

/// Forward host log lines to the installed sink, or to stderr with a prefix.
pub(crate) unsafe extern "C" fn log_to_stderr(
    _context: *mut c_void,
    level: i32,
    text: *const c_char,
) {
    if text.is_null() {
        return;
    }
    // SAFETY: the host passes a NUL-terminated UTF-8 string valid for this call.
    let line = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let sink = LOG_SINK.read().ok().and_then(|slot| slot.clone());
    if let Some(sink) = sink {
        sink(level, &line);
        return;
    }
    let tag = match level {
        0 => "info",
        1 => "warn",
        _ => "error",
    };
    eprintln!("[flash {tag}] {line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_layout_matches_the_c_header() {
        // uint32 size + 4 flag bytes, 12 counters, char[16], 8 network
        // counters, double, uint64: 8 + 96 + 16 + 64 + 8 + 8.
        assert_eq!(std::mem::size_of::<XfStatus>(), 200);
        assert_eq!(std::mem::offset_of!(XfStatus, frames), 8);
        assert_eq!(std::mem::offset_of!(XfStatus, native_root), 104);
        assert_eq!(std::mem::offset_of!(XfStatus, net_reads), 120);
        assert_eq!(std::mem::offset_of!(XfStatus, cpu_percent), 184);
    }

    #[test]
    fn config_layout_matches_the_c_header() {
        // 4 pointers after a padded u32, two i32, seven pointers, four i32,
        // two pointers, two i32, two pointers.
        assert_eq!(std::mem::offset_of!(XfConfig, movie), 8);
        assert_eq!(std::mem::offset_of!(XfConfig, width), 40);
        assert_eq!(std::mem::offset_of!(XfConfig, backend), 48);
        assert_eq!(std::mem::offset_of!(XfConfig, hidden), 104);
        assert_eq!(std::mem::offset_of!(XfConfig, clear_cache), 120);
        assert_eq!(std::mem::offset_of!(XfConfig, publish_named_frame), 136);
        assert_eq!(std::mem::offset_of!(XfConfig, log), 144);
        assert_eq!(std::mem::size_of::<XfConfig>(), 160);
    }

    #[test]
    fn codes_map_to_api_errors() {
        assert_eq!(check(XF_OK, "x".into()).unwrap(), "x");
        assert!(
            matches!(check(XF_TIMEOUT_UNKNOWN, "t".into()), Err(ApiError::Transport(m)) if m.contains("no automatic retry"))
        );
        assert!(matches!(
            check(XF_NOT_READY, String::new()),
            Err(ApiError::Unavailable(_))
        ));
        assert!(
            matches!(check(XF_FLASH_ERROR, "boom".into()), Err(ApiError::ActionFailed { message, .. }) if message == "boom")
        );
        assert!(matches!(
            check(XF_INVALID, String::new()),
            Err(ApiError::InvalidCall(_))
        ));
        assert!(matches!(
            check(99, String::new()),
            Err(ApiError::Protocol(_))
        ));
    }

    #[test]
    fn fixed_text_stops_at_nul() {
        let mut root = [0u8; 16];
        root[..6].copy_from_slice(b"anchor");
        assert_eq!(fixed_text(&root), "anchor");
        assert_eq!(fixed_text(b"pattern"), "pattern");
    }

    #[test]
    fn sibling_build_layout() {
        let candidates = candidates();
        assert!(candidates.iter().any(|p| {
            p.ends_with("xenomit-flash-client/build/xenomit_flash.dll")
                || p.ends_with(r"xenomit-flash-client\build\xenomit_flash.dll")
        }));
    }
}
