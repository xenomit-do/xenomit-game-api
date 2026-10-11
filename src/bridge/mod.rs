//! Named-pipe bridge plumbing and the two client backends.
//!
//! Both backends ([`legacy`] and [`native`]) speak the same
//! one-command-per-connection wire protocol; the transport, reply envelope,
//! STATUS parsing, and shared envelope decoding live here and in [`wire`] so
//! neither backend duplicates them.

pub mod flash;
pub mod flash_native;
pub mod legacy;
pub mod native;
pub(crate) mod protocol;
mod transport;
pub(crate) mod wire;

pub use flash::FlashClient;
pub use flash_native::FlashNativeClient;
pub use legacy::LegacyPipeClient;
pub use native::NativeClient;
pub use transport::{BridgeTransport, NamedPipeTransport};

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::model::{ActionResult, CallTiming, ClientStatus, FunctionCall, Inspection};
use crate::{ApiError, Result};

/// A bridge pipe plus the lock the one-command-per-connection protocol needs.
pub(crate) struct Pipe {
    transport: Arc<dyn BridgeTransport>,
    exchange_lock: Mutex<()>,
}

impl Pipe {
    pub(crate) fn new(transport: Arc<dyn BridgeTransport>) -> Self {
        Self {
            transport,
            exchange_lock: Mutex::new(()),
        }
    }

    /// One serialized round trip over the transport.
    pub(crate) fn exchange(&self, command: &str) -> Result<String> {
        let _guard = self
            .exchange_lock
            .lock()
            .map_err(|error| ApiError::Transport(format!("bridge request lock failed: {error}")))?;
        self.transport.exchange(command)
    }

    /// Exchange plus `OK `/`ERR ` envelope stripping.
    pub(crate) fn command(&self, command: &str) -> Result<String> {
        let response = self.exchange(command)?;
        protocol::unwrap_reply(&response)
    }

    /// Exchange plus JSON decode of an `OK` payload.
    pub(crate) fn json_command<T: serde::de::DeserializeOwned>(&self, command: &str) -> Result<T> {
        let payload = self.command(command)?;
        serde_json::from_str(&payload)
            .map_err(|error| ApiError::Protocol(format!("{error}; response: {payload}")))
    }
}

/// Shared `STATUS` exchange. `extra` merges backend-specific detail keys
/// into the reply map before numeric coercion.
pub(crate) fn status(
    pipe: &Pipe,
    backend: &'static str,
    extra: impl FnOnce(&mut Map<String, Value>),
) -> Result<ClientStatus> {
    let response = pipe.exchange("STATUS")?;
    let line = protocol::unwrap_reply(&response)?;
    let (connected, ready, mut values) = wire::parse_status(&line)?;
    extra(&mut values);
    let details = values
        .into_iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .and_then(|text| text.parse::<u64>().ok())
                .map(Value::from)
                .unwrap_or(value);
            (key, value)
        })
        .collect();
    Ok(ClientStatus {
        backend: backend.into(),
        connected,
        ready,
        details,
    })
}

/// Shared `BRIDGE` ping.
pub(crate) fn ping(pipe: &Pipe) -> Result<()> {
    let reply = pipe.command("BRIDGE")?;
    if reply == "dobot-bridge" {
        Ok(())
    } else {
        Err(ApiError::Protocol(format!(
            "unexpected ping response: {reply}"
        )))
    }
}

/// Shared `INSPECT` exchange — AS3-side member reflection, so it runs on the
/// player thread in both backends.
pub(crate) fn inspect(pipe: &Pipe, target: &str) -> Result<Inspection> {
    protocol::validate_token(target, "target")?;
    let wire: wire::InspectionEnvelope = pipe.json_command(&format!("INSPECT\t{target}"))?;
    wire.into_data()
}

/// Shared `*INVOKE` exchange; `immediate`/`in_frame` are the verb names the
/// backend uses (`INVOKE`/`FINVOKE` or `NINVOKE`/`NFINVOKE`).
pub(crate) fn call_function(
    pipe: &Pipe,
    call: &FunctionCall,
    immediate: &str,
    in_frame: &str,
) -> Result<ActionResult> {
    protocol::validate_token(&call.target, "target")?;
    protocol::validate_token(&call.function, "function")?;
    let args = serde_json::to_string(&call.arguments)
        .map_err(|error| ApiError::InvalidCall(error.to_string()))?;
    let verb = match call.timing {
        CallTiming::Immediate => immediate,
        CallTiming::InFrame => in_frame,
    };
    let payload = pipe.command(&format!(
        "{verb}\t{}\t{}\t{args}",
        call.target, call.function
    ))?;
    let envelope: wire::ActionEnvelope = serde_json::from_str(&payload)
        .map_err(|error| ApiError::Protocol(format!("{error}; response: {payload}")))?;
    envelope.into_data()
}
