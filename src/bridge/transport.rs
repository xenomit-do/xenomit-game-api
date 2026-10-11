use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::time::{Duration, Instant};

use crate::{ApiError, Result};

const COMMAND_LIMIT: usize = 64 * 1024 - 1;
const RESPONSE_LIMIT: usize = 1024 * 1024;

/// A single request/reply transport for the bridge wire protocol.
///
/// The Xenomit bridge handles one command per pipe connection, so each
/// exchange call must complete exactly one round trip.
pub trait BridgeTransport: Send + Sync {
    fn exchange(&self, command: &str) -> Result<String>;
}

/// Windows named-pipe transport for the xenomit_bridge named pipe.
#[derive(Clone, Debug)]
pub struct NamedPipeTransport {
    pid: u32,
}

impl NamedPipeTransport {
    pub fn new(pid: u32) -> Self {
        Self { pid }
    }

    pub fn pipe_name(&self) -> String {
        format!(r"\\.\pipe\xenomit_bridge_{}", self.pid)
    }
}

impl BridgeTransport for NamedPipeTransport {
    fn exchange(&self, command: &str) -> Result<String> {
        if command.len() > COMMAND_LIMIT {
            return Err(ApiError::InvalidCall(format!(
                "command exceeds the {}-byte pipe limit",
                COMMAND_LIMIT
            )));
        }

        #[cfg(windows)]
        {
            let mut pipe = open_named_pipe(&self.pipe_name())
                .map_err(|error| ApiError::Transport(error.to_string()))?;
            pipe.write_all(command.as_bytes())
                .map_err(|error| ApiError::Transport(error.to_string()))?;
            pipe.flush()
                .map_err(|error| ApiError::Transport(error.to_string()))?;

            // The server terminates every protocol response with a newline,
            // then disconnects the message-mode named pipe. Waiting for EOF
            // is unreliable on Windows: the disconnect can surface as
            // ERROR_BROKEN_PIPE/EINVAL even after the full reply was read.
            // Match control.py and stop at the protocol record boundary.
            let mut reader = BufReader::new(pipe);
            let mut response = Vec::new();
            loop {
                let available = reader
                    .fill_buf()
                    .map_err(|error| ApiError::Transport(error.to_string()))?;
                if available.is_empty() {
                    break;
                }
                let newline = available.iter().position(|byte| *byte == b'\n');
                let count = newline.map_or(available.len(), |index| index + 1);
                if response.len() + count > RESPONSE_LIMIT {
                    return Err(ApiError::ResponseTooLarge {
                        limit: RESPONSE_LIMIT,
                    });
                }
                response.extend_from_slice(&available[..count]);
                reader.consume(count);
                if newline.is_some() {
                    break;
                }
            }
            if !response.ends_with(b"\n") {
                return Err(ApiError::Transport(
                    "bridge closed before completing its reply".into(),
                ));
            }
            String::from_utf8(response)
                .map_err(|error| ApiError::Protocol(format!("pipe response is not UTF-8: {error}")))
        }

        #[cfg(not(windows))]
        {
            let _ = command;
            Err(ApiError::UnsupportedPlatform(
                "the xenomit reflection bridge currently exposes a Windows named pipe".into(),
            ))
        }
    }
}

#[cfg(windows)]
fn open_named_pipe(name: &str) -> std::io::Result<std::fs::File> {
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    /// The bridge queues requests on the player thread, which can stall for
    /// many seconds while the game loads; keep waiting through busy windows
    /// instead of failing every exchange after 3 s.
    const RETRY_WINDOW: Duration = Duration::from_secs(30);
    /// `WaitNamedPipe` slice — re-polled until `RETRY_WINDOW` elapses.
    const WAIT_SLICE_MS: u32 = 5_000;

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let started = Instant::now();
    let mut delay = Duration::from_millis(2);

    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
        {
            Ok(pipe) => return Ok(pipe),
            Err(error) if started.elapsed() < RETRY_WINDOW => {
                match error.raw_os_error() {
                    // ERROR_PIPE_BUSY: the server's single instance is
                    // mid-request. Block until it re-arms ConnectNamedPipe
                    // instead of spinning on CreateFile.
                    Some(231) => unsafe {
                        WaitNamedPipeW(wide.as_ptr(), WAIT_SLICE_MS);
                    },
                    // ERROR_FILE_NOT_FOUND: the client has not spawned its
                    // bridge pipe yet — once created, the instance persists.
                    Some(2) => std::thread::sleep(delay),
                    _ => return Err(error),
                }
                delay = (delay * 2).min(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}
