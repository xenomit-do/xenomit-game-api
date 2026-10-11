//! Boundary for behavior engines that operate on a client-agnostic game API.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::GameClient;

/// A selected behavior module and its engine-defined settings payload.
///
/// `settings` is JSON so a behavior engine can own the per-module schema and
/// evolve it independently of desktop clients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleConfig {
    pub module_id: String,
    #[serde(default)]
    pub settings: Value,
}

/// Immutable discovery metadata shared by engines and schema-driven clients.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub settings: Vec<SettingDescriptor>,
    /// Services this module requires from its host (e.g. galaxy_gate.spin).
    #[serde(default)]
    pub required_services: Vec<String>,
}

/// Display and ownership metadata for a module or behavior plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default = "plugin_enabled_by_default")]
    pub default_enabled: bool,
    #[serde(default)]
    pub module_ids: Vec<String>,
    #[serde(default)]
    pub behavior_ids: Vec<String>,
    #[serde(default)]
    pub required_plugins: Vec<String>,
}

const fn plugin_enabled_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingDescriptor {
    /// Dotted object path in ModuleConfig.settings. Paths are unique per module.
    pub key: String,
    pub label: String,
    pub description: String,
    pub group: String,
    pub kind: SettingKind,
    pub default: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SettingKind {
    Boolean,
    Integer {
        min: i64,
        max: i64,
    },
    Number {
        min: f64,
        max: f64,
    },
    Text,
    /// Current map (null), numeric map ID, or a legacy display name resolved
    /// against the client's star-system catalog.
    Map,
    Choice {
        options: Vec<SettingChoice>,
    },
    BooleanMap {
        discover: Option<crate::EntityCategory>,
    },
    Json,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingChoice {
    pub label: String,
    pub value: Value,
}

impl SettingDescriptor {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        kind: SettingKind,
        default: Value,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            description: String::new(),
            group: "General".into(),
            kind,
            default,
        }
    }
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
    pub fn validate(&self, value: &Value) -> Result<(), String> {
        let valid = match &self.kind {
            SettingKind::Boolean => value.is_boolean(),
            SettingKind::Integer { min, max } => {
                value.as_i64().is_some_and(|n| (*min..=*max).contains(&n))
            }
            SettingKind::Number { min, max } => {
                min.is_finite()
                    && max.is_finite()
                    && value
                        .as_f64()
                        .is_some_and(|n| n.is_finite() && (*min..=*max).contains(&n))
            }
            SettingKind::Text => value.is_string(),
            SettingKind::Map => {
                value.is_null()
                    || value
                        .as_i64()
                        .is_some_and(|n| (1..=i64::from(i32::MAX)).contains(&n))
                    || value.as_str().is_some_and(|name| !name.trim().is_empty())
            }
            SettingKind::Choice { options } => options.iter().any(|o| o.value == *value),
            SettingKind::BooleanMap { .. } => value
                .as_object()
                .is_some_and(|m| m.values().all(Value::is_boolean)),
            SettingKind::Json => true,
        };
        if valid {
            Ok(())
        } else {
            Err(format!("{} has an invalid value", self.label))
        }
    }
}

impl ModuleDescriptor {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            settings: Vec::new(),
            required_services: Vec::new(),
        }
    }
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
    pub fn setting(mut self, setting: SettingDescriptor) -> Self {
        self.settings.push(setting);
        self
    }
    pub fn requires_service(mut self, name: impl Into<String>) -> Self {
        self.required_services.push(name.into());
        self
    }
    pub fn prepare_settings(&self, mut settings: Value) -> Result<Value, String> {
        if settings.is_null() {
            settings = serde_json::json!({});
        }
        if !settings.is_object() {
            return Err("module settings must be an object".into());
        }
        for field in &self.settings {
            let value = setting_value(&settings, &field.key).unwrap_or(&field.default);
            field.validate(value)?;
            if setting_value(&settings, &field.key).is_none() {
                set_setting_value(&mut settings, &field.key, field.default.clone())?;
            }
        }
        Ok(settings)
    }
}

pub fn setting_value<'a>(settings: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(settings, |value, part| value.as_object()?.get(part))
}

pub fn set_setting_value(settings: &mut Value, path: &str, value: Value) -> Result<(), String> {
    if path.split('.').any(str::is_empty) {
        return Err("setting path contains an empty segment".into());
    }
    let mut parts = path.split('.').peekable();
    let mut current = settings;
    while let Some(part) = parts.next() {
        if part.is_empty() {
            return Err("setting path contains an empty segment".into());
        }
        let object = current
            .as_object_mut()
            .ok_or_else(|| format!("{path} crosses a non-object value"))?;
        if parts.peek().is_none() {
            object.insert(part.into(), value);
            return Ok(());
        }
        current = object.entry(part).or_insert_with(|| serde_json::json!({}));
    }
    Err("setting path must not be empty".into())
}

/// Engine integration boundary used by desktop controllers.
///
/// Implementations own behavior scheduling and execution. They receive only
/// the shared `GameClient` interface, so the engine does not need to know
/// about the Backpage transport, a specific game client, or a UI framework.
pub trait BehaviorRuntime: Send + Sync + 'static {
    fn start(&self, client: Arc<dyn GameClient>, config: ModuleConfig) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    /// Queried off the UI thread once at attachment. The catalog is immutable
    /// for this runtime's lifetime, including while a module is running.
    fn modules(&self) -> Result<Vec<ModuleDescriptor>, String> {
        Ok(Vec::new())
    }

    /// Available plugin metadata, queried off the UI thread during startup.
    fn plugin_descriptors(&self) -> Result<Vec<PluginDescriptor>, String> {
        Ok(Vec::new())
    }

    /// Nonfatal manifest discovery errors for display in the plugin settings page.
    fn plugin_diagnostics(&self) -> Vec<String> {
        Vec::new()
    }

    /// Objective descriptors visible for a persisted plugin selection.
    fn modules_for(
        &self,
        _enabled: &std::collections::BTreeMap<String, bool>,
    ) -> Result<Vec<ModuleDescriptor>, String> {
        self.modules()
    }

    /// Latest human-readable worker state for display in a controller GUI.
    ///
    /// Engines should update this on every tick so a controller can show why
    /// the module is idle (missing player, no target in range, bridge
    /// failure, ...). `None` while no module is running or the engine has
    /// nothing to report yet.
    fn report(&self) -> Option<String> {
        None
    }

    /// True while the module's worker thread is alive.
    ///
    /// Controllers can poll this to notice a worker that exited on its own
    /// instead of leaving a stale "running" state forever.
    fn is_running(&self) -> bool {
        false
    }

    /// Push an updated module configuration to a running worker so settings
    /// changed in the GUI (target lists, feature flags) take effect without
    /// restarting the module. Best-effort: runtimes that cannot reconfigure
    /// in place keep the default no-op.
    fn update(&self, _module: &ModuleConfig) {}

    /// General rules may opt into stopped ticks. Hosts call this on a background
    /// executor with a recent snapshot; implementations serialize with Start.
    fn tick_stopped(
        &self,
        _client: Arc<dyn GameClient>,
        _config: &ModuleConfig,
        _snapshot: Option<&crate::GameSnapshot>,
    ) -> Result<(), String> {
        Ok(())
    }
}
