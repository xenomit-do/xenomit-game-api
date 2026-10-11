//! Supplementary client APIs. These do not change the shared-memory frame ABI.
//! Optional observations remain unknown until the backend actually reads them.
use crate::{ApiError, Bounds, GalaxyGateInfo, Point, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

/// Decode the supplementary surface only from a successful invocation result.
/// An old bridge returning undefined must not appear to support this API.
pub(crate) fn decode_result<T: serde::de::DeserializeOwned>(
    result: crate::ActionResult,
) -> Result<T> {
    let value = result.data.ok_or_else(|| {
        ApiError::Unavailable("client did not return supplementary API data".into())
    })?;
    serde_json::from_value(value).map_err(|error| ApiError::Protocol(error.to_string()))
}

pub(crate) fn feature_state(client: &dyn crate::GameClient) -> Result<ClientFeatureState> {
    let state: ClientFeatureState = decode_result(
        client.call_function(&crate::FunctionCall::new("xenomit", "clientFeatures").in_frame())?,
    )?;
    let now = crate::bridge::wire::now_unix_ms();
    if state.observed_at_unix_ms == 0
        || now.saturating_sub(state.observed_at_unix_ms) > 4000
        || state.observed_at_unix_ms > now.saturating_add(1000)
    {
        return Err(ApiError::Unavailable(
            "supplementary client state is stale or missing its capture timestamp".into(),
        ));
    }
    Ok(state)
}

pub(crate) fn poll_events(
    client: &dyn crate::GameClient,
    after_sequence: u64,
) -> Result<ClientEventBatch> {
    // AS3 Number cannot exactly represent integers above this bound.
    if after_sequence > 9_007_199_254_740_991 {
        return Err(ApiError::InvalidCall(
            "event cursor exceeds the bridge integer range".into(),
        ));
    }
    let batch: ClientEventBatch = decode_result(
        client.call_function(
            &crate::FunctionCall::new("xenomit", "clientEvents")
                .with_arguments(vec![Value::from(after_sequence)])
                .in_frame(),
        )?,
    )?;
    if batch.next_sequence < after_sequence || batch.events.len() > 4096 {
        return Err(ApiError::Protocol(
            "invalid client event batch cursor or length".into(),
        ));
    }
    let mut previous = after_sequence;
    for event in &batch.events {
        if event.sequence <= previous || event.sequence > batch.next_sequence {
            return Err(ApiError::Protocol(
                "client events are not ordered within their cursor range".into(),
            ));
        }
        previous = event.sequence;
    }
    Ok(batch)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientFeatureState {
    pub observed_at_unix_ms: u64,
    /// Implemented feature names, not promises that optional values are known.
    pub capabilities: Vec<String>,
    pub gui: Option<Vec<GuiElementState>>,
    pub pet: Option<PetFeatureState>,
    pub gate: Option<GateFeatureState>,
    pub event: Option<EventFeatureState>,
    pub economy: Option<EconomyState>,
    /// Active booster categories (the game's `BoosterProxy` model). Unknown
    /// until the backend has seen a booster update; `Some(vec![])` means none
    /// are active.
    pub boosters: Option<Vec<BoosterState>>,
}

/// One active booster category, as the booster window summarises it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BoosterState {
    /// Game category id, e.g. `HP` or `DAMAGE`.
    pub category: String,
    /// Summed boost percentage for the category.
    pub percent: f64,
    /// Seconds until the soonest-expiring timed booster in the category
    /// ends; `None` for untimed boosters or when the backend cannot tell.
    pub seconds_left: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiElementState {
    /// Backend-stable identity. Actions must resolve and revalidate it.
    pub id: String,
    pub visible: bool,
    pub bounds: Option<Bounds>,
    pub text: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PetFeatureState {
    pub id: Option<i32>,
    pub owned: Option<bool>,
    pub active: Option<bool>,
    pub destroyed: Option<bool>,
    pub position: Option<Point>,
    pub health: Option<f64>,
    pub max_health: Option<f64>,
    pub shield: Option<f64>,
    pub max_shield: Option<f64>,
    pub fuel: Option<f64>,
    pub max_fuel: Option<f64>,
    pub level: Option<i32>,
    pub gear_type: Option<i32>,
    pub gears: Option<Vec<i32>>,
    pub idle: Option<bool>,
    pub repair_available: Option<bool>,
    pub data: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GateFeatureState {
    pub gate_id: Option<i32>,
    pub wave: Option<i32>,
    pub total_waves: Option<i32>,
    pub lives: Option<i32>,
    pub difficulty: Option<i32>,
    pub completed: Option<bool>,
    pub builder: Vec<GalaxyGateInfo>,
    /// Additional authoritative per-gate/event facts retained without guessing.
    pub data: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventFeatureState {
    pub server_time_unix_ms: Option<u64>,
    pub active: Option<Vec<String>>,
    pub team: Option<i32>,
    pub direction: Option<i32>,
    pub data: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EconomyState {
    pub ores: BTreeMap<i32, f64>,
    pub cargo: Option<f64>,
    pub max_cargo: Option<f64>,
    pub credits: Option<f64>,
    pub uridium: Option<f64>,
    pub trade_available: Option<bool>,
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientEvent {
    pub sequence: u64,
    pub observed_at_unix_ms: u64,
    pub kind: String,
    pub text: Option<String>,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientEventBatch {
    pub events: Vec<ClientEvent>,
    /// Cursor for the next poll, including when this batch is empty.
    pub next_sequence: u64,
    /// The requested cursor predates the oldest retained event.
    pub dropped: bool,
}

/// Bounded replay journal. Independent consumers keep independent cursors;
/// duplicate text represents distinct events and must not be deduplicated.
pub struct ClientEventJournal {
    capacity: usize,
    sequence: u64,
    events: VecDeque<ClientEvent>,
}
impl Default for ClientEventJournal {
    fn default() -> Self {
        Self::new(512)
    }
}
impl ClientEventJournal {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.clamp(1, 4096),
            sequence: 0,
            events: VecDeque::new(),
        }
    }
    pub fn push(
        &mut self,
        observed_at_unix_ms: u64,
        kind: impl Into<String>,
        text: Option<String>,
        data: Value,
    ) {
        self.sequence = self.sequence.saturating_add(1);
        self.events.push_back(ClientEvent {
            sequence: self.sequence,
            observed_at_unix_ms,
            kind: kind.into(),
            text,
            data,
        });
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
    }
    pub fn poll(&self, after_sequence: u64) -> Result<ClientEventBatch> {
        if after_sequence > self.sequence {
            return Err(ApiError::InvalidCall(
                "event cursor is ahead of this client session".into(),
            ));
        }
        Ok(ClientEventBatch {
            events: self
                .events
                .iter()
                .filter(|e| e.sequence > after_sequence)
                .cloned()
                .collect(),
            next_sequence: self.sequence,
            dropped: self
                .events
                .front()
                .is_some_and(|e| after_sequence < e.sequence.saturating_sub(1)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_cursors_replay_duplicates_and_report_overflow() {
        let mut journal = ClientEventJournal::new(2);
        journal.push(1, "log", Some("Reward".into()), Value::Null);
        journal.push(2, "log", Some("Reward".into()), Value::Null);
        let first = journal.poll(0).unwrap();
        assert_eq!(first.events.len(), 2);
        assert_eq!(first, journal.poll(0).unwrap());
        journal.push(3, "log", None, Value::Null);
        assert!(journal.poll(0).unwrap().dropped);
        assert!(!journal.poll(1).unwrap().dropped);
        assert!(journal.poll(3).unwrap().events.is_empty());
        assert!(journal.poll(4).is_err());
    }
    #[test]
    fn absent_feature_fields_remain_unknown() {
        let state: ClientFeatureState = serde_json::from_str("{}").unwrap();
        assert!(state.pet.is_none() && state.event.is_none() && state.gui.is_none());
        let pet: PetFeatureState = serde_json::from_str("{}").unwrap();
        assert_eq!(pet.owned, None);
        assert_eq!(pet.health, None);
    }
}
