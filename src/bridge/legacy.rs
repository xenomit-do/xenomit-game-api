//! Legacy pipe backend: the original AS3-reflection path, kept for
//! compatibility and debugging.
//!
//! Every state read runs `stateAS` inside the injected `DobotBridge` on the
//! player thread, which stalls the game for ~200 ms per snapshot. Prefer
//! [`super::native::NativeClient`] unless the client is too old to publish
//! shared-memory state or a reflected target is required.

use std::sync::Arc;

use crate::bridge::wire::{self, SnapshotEnvelope};
use crate::bridge::{BridgeTransport, NamedPipeTransport, Pipe, protocol};
use crate::model::{
    ActionResult, ClientStatus, FunctionCall, GameClient, GameSnapshot, Inspection, StageInfo,
};
use crate::{ApiError, Result};

/// Client that speaks the legacy bridge commands (`STATUS`, `BRIDGE`,
/// `STATE`, `INSPECT`, `INVOKE`, `FINVOKE`) over the named pipe. It never
/// touches shared memory or the native `N*` commands; every operation runs
/// AS3 reflection on the player thread and may stall the game.
pub struct LegacyPipeClient {
    pub(crate) pipe: Pipe,
}

impl LegacyPipeClient {
    /// Connect to the bridge pipe of a running xenomit-client process.
    pub fn connect(pid: u32) -> Result<Self> {
        if !cfg!(windows) {
            return Err(ApiError::UnsupportedPlatform(
                "the xenomit reflection bridge currently exposes a Windows named pipe".into(),
            ));
        }
        Ok(Self::with_transport(Arc::new(NamedPipeTransport::new(pid))))
    }

    /// Build a client over a custom transport (tests, alternate channels).
    pub fn with_transport(transport: Arc<dyn BridgeTransport>) -> Self {
        Self {
            pipe: Pipe::new(transport),
        }
    }
}

impl GameClient for LegacyPipeClient {
    fn feature_state(&self) -> Result<crate::ClientFeatureState> {
        crate::features::feature_state(self)
    }
    fn poll_events(&self, after_sequence: u64) -> Result<crate::ClientEventBatch> {
        crate::features::poll_events(self, after_sequence)
    }
    fn game_command(&self, command: &crate::GameCommand) -> Result<ActionResult> {
        self.call_function(
            &FunctionCall::new("xenomit", "gameCommand")
                .with_arguments(vec![
                    serde_json::to_value(command)
                        .map_err(|e| ApiError::InvalidCall(e.to_string()))?,
                ])
                .in_frame(),
        )
    }
    fn status(&self) -> Result<ClientStatus> {
        super::status(&self.pipe, "xenomit_pipe_legacy", |_| {})
    }

    fn ping(&self) -> Result<()> {
        super::ping(&self.pipe)
    }

    fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
        let target = target.unwrap_or("stage");
        protocol::validate_token(target, "target")?;
        let wire: SnapshotEnvelope = self.pipe.json_command(&format!("STATE\t{target}"))?;
        let observed_at_unix_ms = wire::now_unix_ms();
        let data = wire.into_data()?;
        Ok(GameSnapshot {
            navigation: Default::default(),
            observed_at_unix_ms,
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

    fn inspect(&self, target: &str) -> Result<Inspection> {
        super::inspect(&self.pipe, target)
    }

    fn call_function(&self, call: &FunctionCall) -> Result<ActionResult> {
        super::call_function(&self.pipe, call, "INVOKE", "FINVOKE")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records every command and answers with a canned reply.
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
    fn legacy_sends_only_legacy_commands() {
        let transport = FakeTransport::new("OK {\"status\":\"ok\",\"data\":{}}\n");
        let client = LegacyPipeClient::with_transport(transport.clone());

        client
            .snapshot(None)
            .expect("legacy snapshot should decode");
        client
            .call_function(&FunctionCall::new("root", "toString"))
            .expect("invoke");
        client
            .call_function(&FunctionCall::new("root", "toString").in_frame())
            .expect("finvoke");

        let sent = transport.sent();
        assert_eq!(sent[0], "STATE\tstage");
        assert!(sent[1].starts_with("INVOKE\t"), "{}", sent[1]);
        assert!(sent[2].starts_with("FINVOKE\t"), "{}", sent[2]);
        assert!(
            sent.iter()
                .all(|command| !command.starts_with('N') || command.starts_with("NUDGE")),
            "legacy backend must never send native commands: {sent:?}"
        );
    }

    #[test]
    fn legacy_snapshot_decodes_entities() {
        let reply = "OK {\"status\":\"ok\",\"data\":{\"stageWidth\":100.0,\"entities\":[{\"path\":\"map/1\",\"className\":\"Box\",\"x\":5.0,\"y\":6.0,\"bounds\":{\"x\":0.0,\"y\":0.0,\"width\":0.0,\"height\":0.0},\"fields\":{\"id\":1}}],\"visited\":1,\"truncated\":false}}\n";
        let transport = FakeTransport::new(reply);
        let client = LegacyPipeClient::with_transport(transport);
        let snapshot = client.snapshot(Some("map")).expect("snapshot");
        assert_eq!(snapshot.entities.len(), 1);
        assert_eq!(snapshot.entities[0].class_name, "Box");
    }
}
