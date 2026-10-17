//! Native Flash state reads use the version 2 mapping; commands retain the
//! Flash STA/FIFO protocol. No STATE polling and no Ruffle NINVOKE verbs.
//! Windows sends those verbs to the ActiveX pipe. Linux sends the same verbs
//! to the Pepper host socket, which runs them in the injected bridge.
use crate::live::{LiveFrame, LiveStateReader};
use crate::{
    ActionResult, ApiError, ClientStatus, FlashClient, FunctionCall, GameClient, GameCommand,
    GameSnapshot, Inspection, Result,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct FlashNativeClient {
    /// Command bridge. Windows named pipe, or the Linux Pepper socket.
    inner: FlashClient,
    pid: u32,
    /// Linux readiness comes from the frame file. The command socket is only
    /// for reflected calls, so a missing movie must not look like a dead host.
    memory_status: bool,
    reader: Mutex<(Option<Arc<LiveStateReader>>, Instant)>,
}
impl FlashNativeClient {
    pub fn connect(pid: u32) -> Result<Self> {
        let inner = if cfg!(windows) {
            FlashClient::connect(pid)?
        } else {
            FlashClient::connect_linux_bridge(pid)?
        };
        Ok(Self {
            inner,
            pid,
            memory_status: cfg!(not(windows)),
            reader: Mutex::new((None, Instant::now())),
        })
    }
    fn live(&self) -> Option<Arc<LiveStateReader>> {
        let mut slot = self.reader.lock().ok()?;
        if slot.0.is_none() && Instant::now() >= slot.1 {
            slot.1 = Instant::now() + Duration::from_secs(1);
            slot.0 = LiveStateReader::open(self.pid).ok().map(Arc::new);
        }
        slot.0.clone()
    }
}
impl GameClient for FlashNativeClient {
    fn feature_state(&self) -> Result<crate::ClientFeatureState> {
        self.inner.feature_state()
    }
    fn poll_events(&self, after_sequence: u64) -> Result<crate::ClientEventBatch> {
        self.inner.poll_events(after_sequence)
    }
    fn status(&self) -> Result<ClientStatus> {
        if self.memory_status {
            let mut frame = LiveFrame::default();
            let live = self.read_live_frame(&mut frame);
            let mut details = BTreeMap::new();
            details.insert("live".into(), live.into());
            details.insert("native".into(), 1.into());
            return Ok(ClientStatus {
                backend: "flash_native_memory".into(),
                connected: true,
                ready: live,
                details,
            });
        }
        let mut status = self.inner.status()?;
        if status
            .details
            .get("native")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
        {
            return Err(ApiError::Unavailable(
                "Flash host uses cached capture; launch with --backend native".into(),
            ));
        }
        status
            .details
            .insert("live".into(), self.live().is_some().into());
        status.backend = "flash_native_memory".into();
        Ok(status)
    }
    fn ping(&self) -> Result<()> {
        self.inner.ping()
    }
    fn snapshot(&self, target: Option<&str>) -> Result<GameSnapshot> {
        if let Some(target) = target {
            return self.inner.snapshot(Some(target));
        }
        let mut frame = LiveFrame::default();
        if !self.read_live_frame(&mut frame) {
            return Err(ApiError::Unavailable(
                "native Flash frame unavailable or stale".into(),
            ));
        }
        Ok(frame.to_game_snapshot())
    }
    fn read_live_frame(&self, frame: &mut LiveFrame) -> bool {
        let Some(reader) = self.live() else {
            return false;
        };
        // A 60 Hz publisher can briefly own the seqlock when polling begins.
        // Yield between bounded attempts instead of reporting that collision
        // as an unavailable game. Stale successful reads still fail immediately.
        for attempt in 0..8 {
            if reader.read_into(frame) {
                return frame.is_fresh(super::wire::now_unix_ms());
            }
            if attempt < 7 {
                std::thread::yield_now();
            }
        }
        false
    }
    fn inspect(&self, target: &str) -> Result<Inspection> {
        self.inner.inspect(target)
    }
    fn call_function(&self, call: &FunctionCall) -> Result<ActionResult> {
        self.inner.call_function(call)
    }
    fn game_command(&self, command: &GameCommand) -> Result<ActionResult> {
        self.inner.game_command(command)
    }
    fn move_hero(&self, x: i32, y: i32) -> Result<ActionResult> {
        self.inner.move_hero(x, y)
    }
    fn stop_hero(&self) -> Result<ActionResult> {
        self.inner.stop_hero()
    }
    fn select_target(&self, entity_id: i32, attack: bool) -> Result<ActionResult> {
        self.inner.select_target(entity_id, attack)
    }
    fn deselect_target(&self) -> Result<ActionResult> {
        self.inner.deselect_target()
    }
    fn collect(&self, entity_id: i32, at: Option<(i32, i32)>) -> Result<ActionResult> {
        self.inner.collect(entity_id, at)
    }
    fn send_key(&self, virtual_key: u32, action: crate::KeyAction) -> Result<()> {
        self.inner.send_key(virtual_key, action)
    }
    fn send_mouse(&self, action: crate::MouseAction, x: i32, y: i32) -> Result<()> {
        self.inner.send_mouse(action, x, y)
    }
    fn set_window(&self, command: crate::WindowCommand) -> Result<()> {
        self.inner.set_window(command)
    }
    fn reload_client(&self) -> Result<()> {
        self.inner.reload_client()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BridgeTransport;
    struct Transport {
        sent: Mutex<Vec<String>>,
        reply: String,
    }
    impl BridgeTransport for Transport {
        fn exchange(&self, command: &str) -> Result<String> {
            self.sent.lock().unwrap().push(command.into());
            Ok(self.reply.clone())
        }
    }
    fn client(reply: &str) -> (FlashNativeClient, Arc<Transport>) {
        let t = Arc::new(Transport {
            sent: Mutex::new(vec![]),
            reply: reply.into(),
        });
        (
            FlashNativeClient {
                inner: FlashClient::with_transport(t.clone()),
                pid: 0,
                memory_status: false,
                reader: Mutex::new((None, Instant::now() + Duration::from_secs(60))),
            },
            t,
        )
    }
    #[test]
    fn missing_native_mapping_never_polls_as_state() {
        let (client, t) = client("ERR unexpected\n");
        assert!(matches!(
            client.snapshot(None),
            Err(ApiError::Unavailable(_))
        ));
        assert!(t.sent.lock().unwrap().is_empty());
    }
    #[test]
    fn native_state_retains_flash_command_protocol_and_errors() {
        let (client, t) = client("OK {\"status\":\"error\",\"message\":\"rejected\"}\n");
        assert!(
            client
                .call_function(&FunctionCall::new("map/7", "click").in_frame())
                .is_err()
        );
        assert!(
            client
                .game_command(&GameCommand::JumpPortal { portal_id: 7 })
                .is_err()
        );
        let sent = t.sent.lock().unwrap();
        assert!(sent[0].starts_with("FINVOKE\tmap/7\tclick\t"));
        assert!(sent[1].starts_with("FINVOKE\txenomit\tgameCommand\t"));
        assert_eq!(sent.len(), 2);
    }
    #[test]
    fn cached_host_cannot_masquerade_as_native() {
        let (client, _) = client("OK player=1 ready=1 native=0\n");
        assert!(matches!(client.status(), Err(ApiError::Unavailable(_))));
    }
    #[test]
    fn memory_status_does_not_touch_the_command_bridge() {
        let (mut client, transport) = client("OK dobot-bridge\n");
        client.memory_status = true;
        assert!(matches!(
            client.snapshot(None),
            Err(ApiError::Unavailable(_))
        ));
        let status = client.status().expect("status");
        assert_eq!(status.backend, "flash_native_memory");
        assert!(!status.ready);
        assert_eq!(status.details["native"], 1);
        assert!(transport.sent.lock().unwrap().is_empty());
    }
    #[test]
    fn reflected_commands_use_the_flash_bridge() {
        let now = crate::bridge::wire::now_unix_ms();
        let (client, transport) = client(&format!(
            "OK {{\"status\":\"ok\",\"name\":\"Map\",\"isClass\":false,\"qualified\":\"Map\",\"data\":{{\"observedAtUnixMs\":{now},\"observed_at_unix_ms\":{now},\"capabilities\":[\"events\"],\"events\":[],\"next_sequence\":0,\"dropped\":false}}}}\n"
        ));
        client.snapshot(Some("stage")).expect("reflected snapshot");
        client.inspect("stage").expect("inspect");
        client.feature_state().expect("features");
        client.poll_events(0).expect("events");
        let sent = transport.sent.lock().unwrap();
        assert_eq!(sent[0], "STATE\tstage");
        assert_eq!(sent[1], "INSPECT\tstage");
        assert!(sent[2].starts_with("FINVOKE\txenomit\tclientFeatures\t"));
        assert!(sent[3].starts_with("FINVOKE\txenomit\tclientEvents\t"));
    }
}
