//! Native backend: shared-memory state reads plus `NINVOKE`/`NFINVOKE`
//! calls, both resolved natively inside the player.
//!
//! `snapshot(None)` decodes the per-frame capture the player publishes to
//! `Local\xenomit_state_<pid>` — it never touches the pipe, so polling does
//! not stall the game. Calls use the native invoke commands only: a player
//! that answers `unknown command` is too old for this backend; use
//! [`super::legacy::LegacyPipeClient`] instead. `inspect` and explicit
//! `snapshot` targets still delegate to the AS3 reflection commands because
//! member/target reflection is inherently AS3-side.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use crate::bridge::wire;
use crate::bridge::{BridgeTransport, LegacyPipeClient, NamedPipeTransport, Pipe};
use crate::live::{LiveFrame, LiveStateReader};
use crate::model::{
    ActionResult, ClientStatus, FunctionCall, GameClient, GameSnapshot, Inspection,
};
use crate::{ApiError, Result};

/// Native client. `connect` attaches to a running xenomit-client process;
/// `with_transport` uses a custom pipe transport (tests, alternate clients).
pub struct NativeClient {
    inner: LegacyPipeClient,
    /// Player process id, when attached to a local process.
    pid: Option<u32>,
    live: Mutex<LiveSlot>,
}

/// Lazily opened shared-memory reader; retried at most once a second while
/// the player has not created the mapping yet.
struct LiveSlot {
    reader: Option<Arc<LiveStateReader>>,
    next_retry: Instant,
}

impl NativeClient {
    /// Connect to the bridge pipe of a running xenomit-client process.
    pub fn connect(pid: u32) -> Result<Self> {
        if !cfg!(windows) {
            return Err(ApiError::UnsupportedPlatform(
                "the xenomit reflection bridge currently exposes a Windows named pipe".into(),
            ));
        }
        Ok(Self::with_transport_and_pid(
            Arc::new(NamedPipeTransport::new(pid)),
            Some(pid),
        ))
    }

    /// Build a client over a custom transport; without a pid the shared
    /// memory reader is unavailable, so `snapshot`/`read_live_frame` fail.
    pub fn with_transport(transport: Arc<dyn BridgeTransport>) -> Self {
        Self::with_transport_and_pid(transport, None)
    }

    fn with_transport_and_pid(transport: Arc<dyn BridgeTransport>, pid: Option<u32>) -> Self {
        Self {
            inner: LegacyPipeClient {
                pipe: Pipe::new(transport),
            },
            pid,
            live: Mutex::new(LiveSlot {
                reader: None,
                next_retry: Instant::now(),
            }),
        }
    }

    /// The shared-memory reader, when the player publishes one.
    /// Opens `Local\xenomit_state_<pid>` on first use.
    pub fn live(&self) -> Option<Arc<LiveStateReader>> {
        let pid = self.pid?;
        let mut slot = self.live.lock().ok()?;
        if slot.reader.is_none() {
            if Instant::now() < slot.next_retry {
                return None;
            }
            slot.next_retry = Instant::now() + std::time::Duration::from_secs(1);
            slot.reader = LiveStateReader::open(pid).ok().map(Arc::new);
        }
        slot.reader.clone()
    }
}

impl GameClient for NativeClient {
    fn feature_state(&self) -> Result<crate::ClientFeatureState> {
        crate::features::feature_state(self)
    }
    fn poll_events(&self, after_sequence: u64) -> Result<crate::ClientEventBatch> {
        crate::features::poll_events(self, after_sequence)
    }
    fn game_command(&self, command: &crate::GameCommand) -> Result<ActionResult> {
        let call = FunctionCall::new("xenomit", "gameCommand").with_arguments(vec![
            serde_json::to_value(command).map_err(|e| ApiError::Protocol(e.to_string()))?,
        ]);
        self.call_function(&call)
    }
    fn status(&self) -> Result<ClientStatus> {
        super::status(&self.inner.pipe, "xenomit_native", |values| {
            values.insert("live".into(), Value::from(self.live().is_some()));
        })
    }

    fn ping(&self) -> Result<()> {
        self.inner.ping()
    }

    /// `None` reads the shared-memory frame only — never the pipe, which is
    /// what froze the game. Explicit targets delegate to the legacy `STATE`
    /// command because reflected targets are inherently AS3-side.
    fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
        if let Some(target) = target {
            return self.inner.snapshot(Some(target));
        }
        let reader = self.live().ok_or_else(|| {
            ApiError::Unavailable("live state shared memory not published yet".into())
        })?;
        let mut frame = LiveFrame::default();
        if !reader.read_into(&mut frame) {
            return Err(ApiError::Unavailable(
                "no complete live frame has been published yet".into(),
            ));
        }
        if !frame.is_fresh(wire::now_unix_ms()) {
            return Err(ApiError::Unavailable(
                "the newest live frame is stale".into(),
            ));
        }
        Ok(frame.to_game_snapshot())
    }

    fn inspect(&self, target: &str) -> Result<Inspection> {
        self.inner.inspect(target)
    }

    /// Native calls only. No automatic `INVOKE` fallback — an old player
    /// surfaces `ERR unknown command` here, so pick `LegacyPipeClient` for
    /// those.
    fn call_function(&self, call: &FunctionCall) -> Result<ActionResult> {
        super::call_function(&self.inner.pipe, call, "NINVOKE", "NFINVOKE")
    }

    fn read_live_frame(&self, frame: &mut LiveFrame) -> bool {
        let Some(reader) = self.live() else {
            return false;
        };
        reader.read_into(frame) && frame.is_fresh(wire::now_unix_ms())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakeTransport {
        commands: Mutex<Vec<String>>,
        reply: String,
    }

    impl FakeTransport {
        fn new(reply: &str) -> Arc<Self> {
            Arc::new(Self {
                commands: Mutex::new(Vec::new()),
                reply: reply.to_owned(),
            })
        }

        fn sent(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }
    }

    impl BridgeTransport for FakeTransport {
        fn exchange(&self, command: &str) -> Result<String> {
            self.commands.lock().unwrap().push(command.to_owned());
            Ok(self.reply.clone())
        }
    }

    #[test]
    fn native_calls_send_only_native_commands() {
        let transport = FakeTransport::new("OK {\"status\":\"ok\",\"data\":null}\n");
        let client = NativeClient::with_transport(transport.clone());

        client
            .call_function(&FunctionCall::new("root", "toString"))
            .expect("ninvoke");
        client
            .call_function(&FunctionCall::new("root", "toString").in_frame())
            .expect("nfinvoke");

        let sent = transport.sent();
        assert!(sent[0].starts_with("NINVOKE\t"), "{}", sent[0]);
        assert!(sent[1].starts_with("NFINVOKE\t"), "{}", sent[1]);
        assert!(
            sent.iter().all(|command| !command.starts_with("INVOKE\t")
                && !command.starts_with("FINVOKE\t")
                && !command.starts_with("STATE")),
            "native calls must never fall back: {sent:?}"
        );
    }
    #[test]
    fn typed_commands_use_native_dispatch_and_surface_game_errors() {
        let transport = FakeTransport::new("OK {\"status\":\"ok\",\"data\":null}\n");
        let client = NativeClient::with_transport(transport.clone());
        client
            .game_command(&crate::GameCommand::JumpPortal { portal_id: 12 })
            .unwrap();
        let sent = transport.sent();
        assert!(sent[0].starts_with("NINVOKE\txenomit\tgameCommand\t"));
        assert!(sent[0].contains("jump_portal"));
        let transport = FakeTransport::new("ERR Portal disappeared\n");
        assert!(
            NativeClient::with_transport(transport.clone())
                .game_command(&crate::GameCommand::JumpPortal { portal_id: 12 })
                .is_err()
        );
        assert_eq!(transport.sent().len(), 1);
    }

    #[test]
    fn native_snapshot_errors_without_shared_memory() {
        let transport = FakeTransport::new("OK {}\n");
        let client = NativeClient::with_transport(transport.clone());
        let error = client.snapshot(None).expect_err("must fail without shm");
        assert!(matches!(error, ApiError::Unavailable(_)), "{error:?}");
        // Crucially: the failure must not have touched the pipe at all.
        assert!(transport.sent().is_empty(), "{:?}", transport.sent());
    }

    #[test]
    fn native_explicit_target_uses_legacy_state() {
        let transport =
            FakeTransport::new("OK {\"status\":\"ok\",\"data\":{\"entities\":[],\"visited\":0}}\n");
        let client = NativeClient::with_transport(transport.clone());
        client.snapshot(Some("map")).expect("targeted snapshot");
        assert_eq!(transport.sent(), vec!["STATE\tmap".to_owned()]);
    }

    #[test]
    fn native_unknown_command_surfaces_without_retry() {
        let transport = FakeTransport::new("ERR unknown command\n");
        let client = NativeClient::with_transport(transport.clone());
        let error = client
            .call_function(&FunctionCall::new("root", "toString"))
            .expect_err("unknown command must surface");
        assert!(matches!(error, ApiError::Transport(_)), "{error:?}");
        assert_eq!(
            transport.sent().len(),
            1,
            "no silent retry: {:?}",
            transport.sent()
        );
    }

    #[test]
    fn native_status_reports_live_detail() {
        let transport = FakeTransport::new("OK bridge=1 ready=1 player=1 pending=0\n");
        let client = NativeClient::with_transport(transport.clone());
        let status = client.status().expect("status");
        assert_eq!(status.backend, "xenomit_native");
        assert!(status.ready);
        assert_eq!(status.details["live"], json!(false));
        assert_eq!(transport.sent(), vec!["STATUS".to_owned()]);
    }
}
