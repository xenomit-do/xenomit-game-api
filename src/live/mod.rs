//! Live per-frame game state read from the player's shared-memory mapping.
//!
//! `xenomit-client` publishes `Local\xenomit_state_<pid>` every frame
//! (see `state_shm.rs` there); this module maps it read-only and decodes
//! frames with a seqlock, so polling never touches the player thread.

pub mod layout;

use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Map, Value};

use crate::model::{
    Bounds, EntityCategory, GameEntity, GameMapInfo, GameSnapshot, Point, SafePlace, StageInfo,
};
use layout::LiveEntityRecord;

/// Maximum seqlock retries before a read reports failure.
const MAX_READ_ATTEMPTS: usize = 8;

/// How old a frame may be before `snapshot` falls back to the pipe.
pub const FRESHNESS_MS: u64 = 2000;

/// Entity classification written by the player.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum LiveKind {
    #[default]
    Other = 0,
    Hero = 1,
    Player = 2,
    Npc = 3,
    Pet = 4,
    Box = 5,
    Ore = 6,
    Portal = 7,
    Base = 8,
    Station = 9,
    Beacon = 10,
    Mine = 11,
}

impl LiveKind {
    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Hero,
            2 => Self::Player,
            3 => Self::Npc,
            4 => Self::Pet,
            5 => Self::Box,
            6 => Self::Ore,
            7 => Self::Portal,
            8 => Self::Base,
            9 => Self::Station,
            10 => Self::Beacon,
            11 => Self::Mine,
            _ => Self::Other,
        }
    }

    /// Representative simple class name, keeping `class_name`-based
    /// behavior selectors working like they did with AS3 state.
    pub fn class_name(self) -> &'static str {
        match self {
            Self::Hero | Self::Player | Self::Npc => "Ship",
            Self::Pet => "Pet",
            Self::Box => "Box",
            Self::Ore => "Ore",
            Self::Portal => "Portal",
            Self::Base => "BaseStation",
            Self::Station => "BattleStation",
            Self::Beacon => "Beacon",
            Self::Mine => "Mine",
            Self::Other => "MapEntity",
        }
    }

    /// Same category the bridge adapter's `classify_entity` produced for the
    /// corresponding AS3 class.
    pub fn category(self) -> EntityCategory {
        match self {
            Self::Hero => EntityCategory::Player,
            Self::Player => EntityCategory::Ship,
            Self::Npc => EntityCategory::Npc,
            Self::Portal => EntityCategory::Portal,
            _ => EntityCategory::Unknown,
        }
    }
}

/// One decoded frame of live map state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveFrame {
    pub navigation: crate::NavigationState,
    pub navigation_revision: u64,
    /// Capture counter from the player.
    pub frame: u64,
    /// Unix time in milliseconds when the capture ran.
    pub captured_unix_ms: u64,
    /// Capture duration in microseconds.
    pub capture_us: u32,
    /// Present when the game is on a map.
    pub map: Option<LiveMap>,
    /// Present when the hero ship is on the map.
    pub hero: Option<LiveHero>,
    pub entities: Vec<LiveEntity>,
}

impl LiveFrame {
    /// `captured_unix_ms` is within [`FRESHNESS_MS`] of `now_unix_ms`.
    pub fn is_fresh(&self, now_unix_ms: u64) -> bool {
        now_unix_ms.abs_diff(self.captured_unix_ms) <= FRESHNESS_MS
    }

    /// Shape this frame like the `STATE` snapshot the behavior engine and
    /// bot already consume.
    pub fn to_game_snapshot(&self) -> GameSnapshot {
        let mut safe_places = Vec::new();
        for entity in &self.entities {
            let (kind, radius) = match entity.kind {
                LiveKind::Portal => ("portal", 250.0),
                LiveKind::Station => ("battle_station", 250.0),
                // The AS3 bridge used 1250 for Headquarters; without a
                // sub-kind, 1500 keeps behavior identical for other bases.
                LiveKind::Base => ("base", 1500.0),
                _ => continue,
            };
            let name = if entity.name.is_empty() {
                entity.kind.class_name().to_owned()
            } else {
                entity.name.clone()
            };
            safe_places.push(SafePlace {
                kind: kind.to_owned(),
                name,
                x: f64::from(entity.x),
                y: f64::from(entity.y),
                radius,
            });
        }

        let player_position = self.hero.as_ref().map(|hero| Point {
            x: f64::from(hero.x),
            y: f64::from(hero.y),
        });
        let in_safe_place = player_position.map(|position| {
            safe_places.iter().any(|place| {
                let dx = position.x - place.x;
                let dy = position.y - place.y;
                dx * dx + dy * dy <= place.radius * place.radius
            })
        });

        let map = self.map.as_ref().map(|map| GameMapInfo {
            id: Some(map.id),
            name: Some(
                self.navigation
                    .star_system
                    .map(map.id)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
            ),
            width: Some(f64::from(map.width)),
            height: Some(f64::from(map.height)),
            player_position,
            safe_places,
            in_safe_place,
        });

        GameSnapshot {
            navigation: self.navigation.clone(),
            observed_at_unix_ms: self.captured_unix_ms,
            stage: StageInfo::default(),
            map,
            entities: self
                .entities
                .iter()
                .map(|entity| {
                    let mut result = entity.to_game_entity();
                    if let Some(fields) = self.navigation.entity_fields.get(&entity.id) {
                        result.fields.extend(fields.clone());
                    }
                    result
                })
                .collect(),
            visited: self.entities.len(),
            truncated: self.entities.len() >= layout::MAX_ENTITIES,
        }
    }

    /// Best-effort conversion of a `STATE` snapshot into a live frame so map
    /// rendering and the hero trail work on the legacy pipe backend too.
    /// Fields the AS3 snapshot does not carry (destination, clan/diplomacy,
    /// capture timing) are left empty; `frame` counts snapshot millis.
    pub fn from_game_snapshot(snapshot: &GameSnapshot) -> LiveFrame {
        let entities: Vec<LiveEntity> = snapshot
            .entities
            .iter()
            .map(LiveEntity::from_game_entity)
            .collect();
        let hero_entity = entities.iter().find(|entity| entity.is_hero);
        let hero = snapshot
            .map
            .as_ref()
            .and_then(|map| map.player_position)
            .map(|position| LiveHero {
                id: hero_entity.map(|entity| entity.id).unwrap_or_default(),
                x: position.x as f32,
                y: position.y as f32,
                dest: None,
                faction: hero_entity.map(|entity| entity.faction).unwrap_or_default(),
                clan: hero_entity.map(|entity| entity.clan).unwrap_or_default(),
            });
        let map = snapshot.map.as_ref().map(|map| LiveMap {
            id: map.id.unwrap_or(-1),
            width: map.width.unwrap_or(21_000.0) as f32,
            height: map.height.unwrap_or(13_100.0) as f32,
        });
        LiveFrame {
            navigation: snapshot.navigation.clone(),
            navigation_revision: 0,
            frame: snapshot.observed_at_unix_ms,
            captured_unix_ms: snapshot.observed_at_unix_ms,
            capture_us: 0,
            map,
            hero,
            entities,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveMap {
    pub id: i32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveHero {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub dest: Option<Point>,
    pub faction: i32,
    pub clan: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveEntity {
    pub id: i32,
    pub kind: LiveKind,
    pub x: f32,
    pub y: f32,
    pub dest: Option<Point>,
    pub hp: i32,
    pub max_hp: i32,
    pub shield: i32,
    pub max_shield: i32,
    pub nanohull: i32,
    pub nanohull_max: i32,
    pub faction: i32,
    pub clan: i32,
    pub clan_diplomacy: i32,
    pub type_id: i32,
    pub name: String,
    /// `entity_flags::NPC` (ship with `_isNPC` set).
    pub is_npc: bool,
    /// `entity_flags::HERO` (id equals `Hero.userID`).
    pub is_hero: bool,
    /// `entity_flags::HAS_HP`: hp/shield fields are meaningful.
    pub has_hp: bool,
    /// `entity_flags::HAS_TYPE_ID`: `type_id` is meaningful.
    pub has_type_id: bool,
    /// `entity_flags::COLLECTABLE`: `name` is the collectable hash.
    pub is_collectable: bool,
}

impl LiveEntity {
    fn to_game_entity(&self) -> GameEntity {
        let mut fields = Map::new();
        fields.insert("id".into(), Value::from(self.id));
        fields.insert("faction".into(), Value::from(self.faction));
        fields.insert("clan".into(), Value::from(self.clan));
        fields.insert("clanDiplomacy".into(), Value::from(self.clan_diplomacy));
        fields.insert("worldX".into(), Value::from(f64::from(self.x)));
        fields.insert("worldY".into(), Value::from(f64::from(self.y)));
        fields.insert("isMoving".into(), Value::from(self.dest.is_some()));
        if let Some(destination) = self.dest {
            fields.insert("destinationX".into(), Value::from(destination.x));
            fields.insert("destinationY".into(), Value::from(destination.y));
        }
        if matches!(
            self.kind,
            LiveKind::Hero | LiveKind::Player | LiveKind::Npc | LiveKind::Pet
        ) {
            fields.insert("isNpc".into(), Value::from(self.is_npc));
            fields.insert("isHero".into(), Value::from(self.is_hero));
        }
        if self.has_hp {
            fields.insert("health".into(), Value::from(self.hp));
            fields.insert("maxHealth".into(), Value::from(self.max_hp));
            fields.insert("shield".into(), Value::from(self.shield));
            fields.insert("maxShield".into(), Value::from(self.max_shield));
            fields.insert("nanohullHP".into(), Value::from(self.nanohull));
            fields.insert("nanohullMaxHP".into(), Value::from(self.nanohull_max));
        }
        if self.is_collectable {
            fields.insert("isCollectable".into(), Value::from(true));
            fields.insert("hash".into(), Value::from(self.name.clone()));
        }
        if self.has_type_id {
            fields.insert("typeID".into(), Value::from(self.type_id));
        }
        match self.kind {
            LiveKind::Box => {
                fields.insert("entityKind".into(), Value::from("box"));
            }
            LiveKind::Ore => {
                fields.insert("entityKind".into(), Value::from("ore"));
            }
            LiveKind::Beacon => {
                fields.insert("entityKind".into(), Value::from("beacon"));
            }
            _ => {}
        }

        GameEntity {
            path: format!("map/{}", self.id),
            class_name: self.kind.class_name().to_owned(),
            name: self.name.clone(),
            category: self.kind.category(),
            position: Point {
                x: f64::from(self.x),
                y: f64::from(self.y),
            },
            bounds: Bounds {
                x: f64::from(self.x),
                y: f64::from(self.y),
                width: 0.0,
                height: 0.0,
            },
            id: Some(Value::from(self.id)),
            health: self.has_hp.then_some(f64::from(self.hp)),
            max_health: self.has_hp.then_some(f64::from(self.max_hp)),
            shield: self.has_hp.then_some(f64::from(self.shield)),
            max_shield: self.has_hp.then_some(f64::from(self.max_shield)),
            fields,
        }
    }

    /// Reverse of [`LiveEntity::to_game_entity`], best effort. `worldX`/
    /// `worldY` fields win over `position`, which holds stage pixels in some
    /// snapshots.
    fn from_game_entity(entity: &GameEntity) -> LiveEntity {
        let fields = &entity.fields;
        let flag = |name: &str| fields.get(name).and_then(Value::as_bool).unwrap_or(false);
        let number = |aliases: &[&str]| {
            aliases.iter().find_map(|alias| {
                fields.get(*alias).and_then(|value| {
                    value
                        .as_f64()
                        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
                })
            })
        };
        let int = |aliases: &[&str]| number(aliases).map(|value| value as i32);

        let is_hero = flag("isHero");
        let is_npc = flag("isNpc");
        let kind = if is_hero {
            LiveKind::Hero
        } else if is_npc {
            LiveKind::Npc
        } else {
            match fields.get("entityKind").and_then(Value::as_str) {
                Some("box") => LiveKind::Box,
                Some("ore") => LiveKind::Ore,
                Some("beacon") => LiveKind::Beacon,
                _ => match entity.category {
                    EntityCategory::Npc => LiveKind::Npc,
                    EntityCategory::Portal => LiveKind::Portal,
                    EntityCategory::Player | EntityCategory::Ship => LiveKind::Player,
                    _ => match entity.class_name.as_str() {
                        "Pet" => LiveKind::Pet,
                        "Box" => LiveKind::Box,
                        "Ore" => LiveKind::Ore,
                        "Beacon" => LiveKind::Beacon,
                        name if name.contains("Portal") => LiveKind::Portal,
                        name if name.contains("BattleStation") => LiveKind::Station,
                        name if name.contains("Headquarters")
                            || name.contains("BaseStation")
                            || name.contains("BaseHeadquarters")
                            || name.contains("BaseSpot") =>
                        {
                            LiveKind::Base
                        }
                        name if name.contains("Mine") => LiveKind::Mine,
                        name if name.contains("Ship") => LiveKind::Player,
                        _ => LiveKind::Other,
                    },
                },
            }
        };

        let x = number(&["worldX", "world_x"]).unwrap_or(entity.position.x);
        let y = number(&["worldY", "world_y"]).unwrap_or(entity.position.y);

        let hp = entity
            .health
            .or_else(|| number(&["health", "hp"]))
            .map(|value| value as i32);
        let max_hp = entity
            .max_health
            .or_else(|| number(&["maxHealth", "maxHp"]))
            .map(|value| value as i32);
        let shield = entity
            .shield
            .or_else(|| number(&["shield", "currentShield"]))
            .map(|value| value as i32);
        let max_shield = entity
            .max_shield
            .or_else(|| number(&["maxShield"]))
            .map(|value| value as i32);
        let nanohull = int(&["nanohullHP"]).unwrap_or_default();
        let nanohull_max = int(&["nanohullMaxHP"]).unwrap_or_default();

        let id = entity
            .id
            .as_ref()
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
            })
            .map(|value| value as i32)
            .unwrap_or_default();

        LiveEntity {
            id,
            kind,
            x: x as f32,
            y: y as f32,
            dest: None,
            hp: hp.unwrap_or_default(),
            max_hp: max_hp.unwrap_or_default(),
            shield: shield.unwrap_or_default(),
            max_shield: max_shield.unwrap_or_default(),
            nanohull,
            nanohull_max,
            faction: int(&["factionID", "faction"]).unwrap_or_default(),
            clan: int(&["clanID", "clan"]).unwrap_or_default(),
            clan_diplomacy: int(&["clanDiplomacy", "clan_diplomacy"]).unwrap_or_default(),
            type_id: int(&["typeID"]).unwrap_or_default(),
            name: entity.name.clone(),
            is_npc,
            is_hero,
            has_hp: [hp, max_hp, shield, max_shield].iter().any(Option::is_some),
            has_type_id: int(&["typeID"]).is_some(),
            is_collectable: flag("isCollectable") || fields.contains_key("hash"),
        }
    }
}

/// Read-only handle to a player's `Local\xenomit_state_<pid>` mapping.
///
/// `read`/`read_into` copy out under a seqlock and are safe to call at any
/// rate from any thread; they never synchronize with the writer.
pub struct LiveStateReader {
    base: *const u8,
    len: usize,
    /// Keeps the mapped view alive; `None` for `from_raw` test readers.
    _view: Option<ViewGuard>,
}

// SAFETY: the mapping is read-only for us; reads use volatile copies and the
// seqlock, so concurrent access is sound.
unsafe impl Send for LiveStateReader {}
// SAFETY: see above.
unsafe impl Sync for LiveStateReader {}

impl LiveStateReader {
    /// Open `Local\xenomit_state_<pid>` for reading.
    #[cfg(windows)]
    pub fn open(pid: u32) -> crate::Result<Self> {
        imp::open(pid)
    }

    /// Open the Linux frame file for `pid` (`/dev/shm`, then `/tmp`).
    #[cfg(not(windows))]
    pub fn open(pid: u32) -> crate::Result<Self> {
        let mut last = crate::ApiError::Unavailable(format!("no live frame file for {pid}"));
        for candidate in layout::frame_file_candidates(pid) {
            match Self::open_frame_file(std::path::Path::new(&candidate)) {
                Ok(reader) => return Ok(reader),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    /// Map a version 2 frame file. The file must already be [`layout::MAPPING_LEN`] bytes.
    pub fn open_frame_file(path: &std::path::Path) -> crate::Result<Self> {
        imp::open_frame_file(path)
    }

    /// Wrap an existing buffer. Used by tests to exercise the seqlock
    /// decoder without a real file mapping.
    ///
    /// # Safety
    /// `base` must point to `len` readable bytes that outlive the reader and
    /// follow the `layout` module's wire format.
    #[doc(hidden)]
    pub unsafe fn from_raw(base: *const u8, len: usize) -> Self {
        Self {
            base,
            len,
            _view: None,
        }
    }

    /// Decode the newest complete frame; `None` while the writer is
    /// mid-publish or the mapping is not yet initialized.
    pub fn read(&self) -> Option<LiveFrame> {
        let mut frame = LiveFrame::default();
        self.read_into(&mut frame).then_some(frame)
    }

    /// Decode into `frame`, reusing its entity/string allocations. Suitable
    /// for 60 Hz polling. Returns false when no complete frame was read.
    pub fn read_into(&self, frame: &mut LiveFrame) -> bool {
        // SAFETY: `base`/`len` come from `open` (mapped view) or `from_raw`
        // (caller contract); all access is through volatile copies.
        unsafe { read_into_at(self.base, self.len, frame) }
    }
}

/// Decode one frame from `base` into `frame`. Retries while `seq` is odd or
/// changes mid-copy; returns false after `MAX_READ_ATTEMPTS` torn reads.
///
/// # Safety
/// `base` must point to at least `len` readable bytes and `len` must be at
/// least [`layout::HEADER_LEN`].
unsafe fn read_into_at(base: *const u8, len: usize, frame: &mut LiveFrame) -> bool {
    if len < layout::STRINGS_OFFSET {
        return false;
    }
    // SAFETY: `base + SEQ_OFFSET` is inside the mapping and 4-aligned.
    let seq = unsafe { &*base.add(layout::SEQ_OFFSET).cast::<AtomicU32>() };
    for _ in 0..MAX_READ_ATTEMPTS {
        let start = seq.load(Ordering::Acquire);
        if start % 2 != 0 {
            std::hint::spin_loop();
            continue;
        }
        // SAFETY: seq is even, so the writer is idle; a torn copy is caught
        // by the second seq read below.
        let header = unsafe { std::ptr::read_volatile(base.cast::<layout::LiveHeader>()) };
        if header.magic != layout::MAGIC || header.version != layout::VERSION {
            return false;
        }
        // Clamp to the mapping bounds so a corrupt header can never push
        // reads past `len`.
        let count = (header.entity_count as usize)
            .min(layout::MAX_ENTITIES)
            .min((len - layout::HEADER_LEN) / layout::ENTITY_LEN);
        let string_len = (header.string_bytes as usize)
            .min(layout::MAX_STRING_LEN)
            .min(len.saturating_sub(layout::STRINGS_OFFSET));

        // Copy entity records and strings while the frame is stable.
        let records = unsafe {
            std::slice::from_raw_parts(
                base.add(layout::HEADER_LEN).cast::<LiveEntityRecord>(),
                count,
            )
        };
        if frame.entities.len() < count {
            frame.entities.resize_with(count, LiveEntity::default);
        }
        let strings =
            unsafe { std::slice::from_raw_parts(base.add(layout::STRINGS_OFFSET), string_len) };
        for (record, entity) in records.iter().zip(frame.entities.iter_mut()) {
            apply_record(record, strings, entity);
        }
        frame.entities.truncate(count);

        // Decode before the final seqlock check. A writer changing this payload
        // while it is copied must invalidate the entire frame, not just entities.
        let navigation = if header.navigation_len == 0
            || !frame.navigation.geometry_ready
            || frame.navigation_revision != header.navigation_revision
            || frame.navigation.map_id != (header.map_id >= 0).then_some(header.map_id)
        {
            let nav_start = header.navigation_off as usize;
            let nav_end = nav_start.checked_add(header.navigation_len as usize);
            Some(
                nav_end
                    .and_then(|end| strings.get(nav_start..end))
                    .and_then(|bytes| serde_json::from_slice::<crate::NavigationState>(bytes).ok())
                    .filter(|nav| nav.map_id == (header.map_id >= 0).then_some(header.map_id))
                    .unwrap_or_else(|| crate::NavigationState {
                        map_id: (header.map_id >= 0).then_some(header.map_id),
                        ..Default::default()
                    }),
            )
        } else {
            None
        };

        std::sync::atomic::fence(Ordering::Acquire);
        if seq.load(Ordering::Acquire) == start {
            if let Some(navigation) = navigation {
                frame.navigation = navigation;
                frame.navigation_revision = header.navigation_revision;
            }
            frame.frame = header.frame;
            frame.captured_unix_ms = header.captured_unix_ms;
            frame.capture_us = header.capture_us;
            frame.map = (header.map_id >= 0).then_some(LiveMap {
                id: header.map_id,
                width: header.map_width,
                height: header.map_height,
            });
            frame.hero =
                (header.hero_flags & layout::hero_flags::PRESENT != 0).then_some(LiveHero {
                    id: header.hero_id,
                    x: header.hero_x,
                    y: header.hero_y,
                    dest: (header.hero_flags & layout::hero_flags::MOVING != 0).then_some(Point {
                        x: f64::from(header.hero_dest_x),
                        y: f64::from(header.hero_dest_y),
                    }),
                    faction: header.hero_faction,
                    clan: header.hero_clan,
                });
            return true;
        }
    }
    false
}

fn apply_record(record: &LiveEntityRecord, strings: &[u8], entity: &mut LiveEntity) {
    let flags = record.flags;
    entity.id = record.id;
    entity.kind = LiveKind::from_u8(record.kind);
    entity.x = record.x;
    entity.y = record.y;
    entity.dest = (flags & layout::entity_flags::MOVING != 0).then(|| Point {
        x: f64::from(record.dest_x),
        y: f64::from(record.dest_y),
    });
    entity.is_npc = flags & layout::entity_flags::NPC != 0;
    entity.is_hero = flags & layout::entity_flags::HERO != 0;
    entity.has_hp = flags & layout::entity_flags::HAS_HP != 0;
    entity.has_type_id = flags & layout::entity_flags::HAS_TYPE_ID != 0;
    entity.is_collectable = flags & layout::entity_flags::COLLECTABLE != 0;
    entity.hp = record.hp;
    entity.max_hp = record.max_hp;
    entity.shield = record.shield;
    entity.max_shield = record.max_shield;
    entity.nanohull = record.nanohull;
    entity.nanohull_max = record.nanohull_max;
    entity.faction = record.faction;
    entity.clan = record.clan;
    entity.clan_diplomacy = record.clan_diplomacy;
    entity.type_id = record.type_id;
    entity.name.clear();
    let start = record.name_off as usize;
    let end = start.saturating_add(record.name_len as usize);
    if let Some(bytes) = strings.get(start..end) {
        entity.name.push_str(&String::from_utf8_lossy(bytes));
    }
}

#[cfg(windows)]
struct ViewGuard {
    mapping: windows_sys::Win32::Foundation::HANDLE,
    base: *mut core::ffi::c_void,
}

#[cfg(unix)]
struct ViewGuard {
    base: *mut u8,
    len: usize,
}

#[cfg(unix)]
impl Drop for ViewGuard {
    fn drop(&mut self) {
        unsafe {
            munmap(self.base, self.len);
        }
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn open(path: *const i8, flags: i32) -> i32;
    fn close(fd: i32) -> i32;
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
}

#[cfg(windows)]
impl Drop for ViewGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Memory::UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS { Value: self.base },
            );
            windows_sys::Win32::Foundation::CloseHandle(self.mapping);
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::{LiveStateReader, ViewGuard};
    use crate::live::layout;
    use crate::{ApiError, Result};
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Memory::{FILE_MAP_READ, MapViewOfFile, OpenFileMappingW};

    pub fn open(pid: u32) -> Result<LiveStateReader> {
        let name: Vec<u16> = layout::mapping_name(pid)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: name is a valid NUL-terminated UTF-16 string.
        let mapping = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if mapping.is_null() {
            let error = unsafe { GetLastError() };
            return Err(ApiError::Transport(format!(
                "OpenFileMappingW failed for {}: {error}",
                layout::mapping_name(pid)
            )));
        }
        // SAFETY: mapping is a valid handle; we map the full view read-only.
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0) };
        let base = view.Value;
        if base.is_null() {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(mapping) };
            return Err(ApiError::Transport(format!(
                "MapViewOfFile failed for {}: {error}",
                layout::mapping_name(pid)
            )));
        }
        Ok(LiveStateReader {
            base: base.cast::<u8>(),
            len: layout::MAPPING_LEN,
            _view: Some(ViewGuard { mapping, base }),
        })
    }

    pub fn open_frame_file(path: &std::path::Path) -> Result<LiveStateReader> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::Memory::{CreateFileMappingW, PAGE_READONLY};

        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let meta = std::fs::metadata(path).map_err(|error| {
            ApiError::Transport(format!("frame file {}: {error}", path.display()))
        })?;
        if meta.len() < layout::MAPPING_LEN as u64 {
            return Err(ApiError::Unavailable(format!(
                "frame file is shorter than {} bytes: {}",
                layout::MAPPING_LEN,
                path.display()
            )));
        }
        // SAFETY: wide is NUL-terminated. Share read and write so the publisher can keep the file open.
        let file = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0x8000_0000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if file.is_null() || file == INVALID_HANDLE_VALUE {
            let error = unsafe { GetLastError() };
            return Err(ApiError::Transport(format!(
                "CreateFileW failed for {}: {error}",
                path.display()
            )));
        }
        // SAFETY: file is a valid handle. The mapping is closed with the view.
        let mapping = unsafe {
            CreateFileMappingW(file, std::ptr::null(), PAGE_READONLY, 0, 0, std::ptr::null())
        };
        unsafe { CloseHandle(file) };
        if mapping.is_null() {
            let error = unsafe { GetLastError() };
            return Err(ApiError::Transport(format!(
                "CreateFileMappingW failed for {}: {error}",
                path.display()
            )));
        }
        // SAFETY: mapping is valid; map the full view read-only.
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0) };
        let base = view.Value;
        if base.is_null() {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(mapping) };
            return Err(ApiError::Transport(format!(
                "MapViewOfFile failed for {}: {error}",
                path.display()
            )));
        }
        Ok(LiveStateReader {
            base: base.cast::<u8>(),
            len: layout::MAPPING_LEN,
            _view: Some(ViewGuard { mapping, base }),
        })
    }
}

#[cfg(unix)]
mod imp {
    use super::{LiveStateReader, ViewGuard, close, mmap, open};
    use crate::live::layout;
    use crate::{ApiError, Result};
    use std::os::unix::ffi::OsStrExt;

    pub fn open_frame_file(path: &std::path::Path) -> Result<LiveStateReader> {
        let meta = std::fs::metadata(path).map_err(|error| {
            ApiError::Transport(format!("frame file {}: {error}", path.display()))
        })?;
        if meta.len() < layout::MAPPING_LEN as u64 {
            return Err(ApiError::Unavailable(format!(
                "frame file is shorter than {} bytes: {}",
                layout::MAPPING_LEN,
                path.display()
            )));
        }
        let mut bytes = path.as_os_str().as_bytes().to_vec();
        bytes.push(0);
        // SAFETY: path is a local NUL-terminated byte string. The mapping is dropped with the reader.
        unsafe {
            let fd = open(bytes.as_ptr().cast(), 0);
            if fd < 0 {
                return Err(ApiError::Transport(format!(
                    "open failed for {}",
                    path.display()
                )));
            }
            let base = mmap(std::ptr::null_mut(), layout::MAPPING_LEN, 1, 1, fd, 0);
            close(fd);
            if base == usize::MAX as *mut u8 {
                return Err(ApiError::Transport(format!(
                    "mmap failed for {}",
                    path.display()
                )));
            }
            Ok(LiveStateReader {
                base,
                len: layout::MAPPING_LEN,
                _view: Some(ViewGuard {
                    base,
                    len: layout::MAPPING_LEN,
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests;
