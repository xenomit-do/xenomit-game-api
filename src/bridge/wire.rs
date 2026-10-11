//! Wire envelopes and line parsing shared by both bridge backends.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::model::{
    ActionResult, Bounds, EntityCategory, GameEntity, GameMapInfo, Inspection, Point, SafePlace,
};
use crate::{ApiError, Result};

/// Parse the `key=value` token stream of a `STATUS` reply.
/// Returns `(connected, ready, remaining values)`.
pub(crate) fn parse_status(line: &str) -> Result<(bool, bool, Map<String, Value>)> {
    let mut values = Map::new();
    for part in line.split_whitespace() {
        let Some((key, value)) = part.split_once('=') else {
            return Err(ApiError::Protocol(format!(
                "unexpected bridge status token: {part}"
            )));
        };
        values.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    let flag = |key: &str| -> Result<bool> {
        match values.get(key).and_then(Value::as_str) {
            Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(value) => Err(ApiError::Protocol(format!(
                "invalid status flag {key}={value}"
            ))),
            None => Err(ApiError::Protocol(format!("missing status field {key}"))),
        }
    };
    Ok((flag("player")?, flag("ready")?, values))
}

/// Unix time in milliseconds, saturated at `u64::MAX`.
pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[derive(Debug, Deserialize)]
pub(crate) struct InspectionEnvelope {
    status: String,
    name: Option<String>,
    #[serde(rename = "isClass")]
    is_class: Option<bool>,
    qualified: Option<String>,
    #[serde(default)]
    methods: Vec<crate::model::MemberMethod>,
    #[serde(default)]
    accessors: Vec<crate::model::MemberProperty>,
    #[serde(default)]
    variables: Vec<crate::model::MemberVariable>,
    message: Option<String>,
    stack: Option<String>,
}

impl InspectionEnvelope {
    pub(crate) fn into_data(self) -> Result<Inspection> {
        if self.status != "ok" {
            return Err(action_error(self.message, self.stack));
        }
        Ok(Inspection {
            name: required(self.name, "inspection.name")?,
            is_class: self.is_class.unwrap_or(false),
            qualified: required(self.qualified, "inspection.qualified")?,
            methods: self.methods,
            accessors: self.accessors,
            variables: self.variables,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ActionEnvelope {
    status: String,
    data: Option<Value>,
    note: Option<String>,
    message: Option<String>,
    stack: Option<String>,
}

impl ActionEnvelope {
    pub(crate) fn into_data(self) -> Result<ActionResult> {
        if self.status != "ok" {
            return Err(action_error(self.message, self.stack));
        }
        Ok(ActionResult {
            data: self.data,
            note: self.note,
        })
    }
}

// --- STATE snapshot envelope -------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SnapshotEnvelope {
    status: String,
    data: Option<SnapshotData>,
    message: Option<String>,
    stack: Option<String>,
}

impl SnapshotEnvelope {
    pub(crate) fn into_data(self) -> Result<SnapshotData> {
        if self.status != "ok" {
            return Err(action_error(self.message, self.stack));
        }
        self.data
            .ok_or_else(|| ApiError::Protocol("snapshot response has no data".into()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotData {
    #[serde(default)]
    pub navigation: crate::NavigationState,
    pub observed_at_unix_ms: Option<u64>,
    pub stage_width: Option<f64>,
    pub stage_height: Option<f64>,
    pub map: Option<WireMapInfo>,
    #[serde(default)]
    pub entities: Vec<WireEntity>,
    #[serde(default)]
    pub visited: usize,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireMapInfo {
    pub id: Option<i32>,
    pub name: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub player_position: Option<Point>,
    #[serde(default)]
    pub safe_places: Vec<WireSafePlace>,
    pub in_safe_place: Option<bool>,
}

impl From<WireMapInfo> for GameMapInfo {
    fn from(map: WireMapInfo) -> Self {
        Self {
            id: map.id,
            name: map.name,
            width: map.width,
            height: map.height,
            player_position: map.player_position,
            safe_places: map.safe_places.into_iter().map(Into::into).collect(),
            in_safe_place: map.in_safe_place,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireSafePlace {
    kind: String,
    #[serde(default)]
    name: String,
    x: f64,
    y: f64,
    radius: f64,
}

impl From<WireSafePlace> for SafePlace {
    fn from(place: WireSafePlace) -> Self {
        Self {
            kind: place.kind,
            name: place.name,
            x: place.x,
            y: place.y,
            radius: place.radius,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct WireEntity {
    path: String,
    #[serde(rename = "className")]
    class_name: String,
    #[serde(default)]
    name: String,
    x: f64,
    y: f64,
    #[serde(default)]
    bounds: Bounds,
    #[serde(default)]
    fields: Map<String, Value>,
}

impl From<WireEntity> for GameEntity {
    fn from(entity: WireEntity) -> Self {
        let category = classify_entity(&entity.class_name, &entity.name, &entity.fields);
        let id = field_value(&entity.fields, &["id", "userId"]);
        let health = field_number(
            &entity.fields,
            &["health", "currentHealth", "healthPoints", "hp", "hitPoints"],
        );
        let max_health = field_number(&entity.fields, &["maxHealth", "maxHp", "maxHitPoints"]);
        let shield = field_number(&entity.fields, &["shield", "currentShield"]);
        let max_shield = field_number(&entity.fields, &["maxShield"]);
        GameEntity {
            path: entity.path,
            class_name: entity.class_name,
            name: entity.name,
            category,
            position: Point {
                x: entity.x,
                y: entity.y,
            },
            bounds: entity.bounds,
            id,
            health,
            max_health,
            shield,
            max_shield,
            fields: entity.fields,
        }
    }
}

fn field_value(fields: &Map<String, Value>, aliases: &[&str]) -> Option<Value> {
    aliases.iter().find_map(|alias| fields.get(*alias)).cloned()
}

fn field_number(fields: &Map<String, Value>, aliases: &[&str]) -> Option<f64> {
    aliases.iter().find_map(|alias| {
        fields.get(*alias).and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
        })
    })
}

fn classify_entity(class_name: &str, name: &str, fields: &Map<String, Value>) -> EntityCategory {
    let class = class_name.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    if fields.get("isHero").and_then(Value::as_bool) == Some(true) {
        EntityCategory::Player
    } else if fields.get("isNpc").and_then(Value::as_bool) == Some(true) {
        EntityCategory::Npc
    } else if fields.contains_key("portalId") || class.contains("portal") || name.contains("portal")
    {
        EntityCategory::Portal
    } else if class.contains("projectile")
        || class.contains("rocket")
        || class.contains("laser")
        || class.contains("munition")
    {
        EntityCategory::Projectile
    } else if class.contains("npc") {
        EntityCategory::Npc
    } else if class.contains("ownship") || class.contains("playerhero") || class.contains("hero") {
        EntityCategory::Player
    } else if class.contains("ship") {
        EntityCategory::Ship
    } else {
        EntityCategory::Unknown
    }
}

fn required<T>(value: Option<T>, name: &str) -> Result<T> {
    value.ok_or_else(|| ApiError::Protocol(format!("response is missing {name}")))
}

pub(crate) fn action_error(message: Option<String>, stack: Option<String>) -> ApiError {
    ApiError::ActionFailed {
        message: message.unwrap_or_else(|| "client operation failed".into()),
        stack,
    }
}
