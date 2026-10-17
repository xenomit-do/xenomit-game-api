//! Wire layout for the per-frame live state shared-memory mapping.
//!
//! The player (`xenomit-client`) is the single writer; bot/UI processes open
//! the mapping read-only. Everything is little-endian `repr(C)` POD copied
//! under a seqlock (`seq` is odd while a write is in progress), so readers
//! never synchronize with the player thread beyond retrying a torn frame.
//!
//! This module intentionally depends only on `std` so the exact same layout
//! is shared between the writer and every reader.

use std::mem::offset_of;

/// Mapping name tag: "XSTM" little-endian.
pub const MAGIC: u32 = 0x4D54_5358;
/// Bump when the layout changes; readers must reject other versions.
pub const VERSION: u32 = 2;

/// Byte size of the frame header; entities start at this offset. Kept a
/// multiple of 64 so the entity array is aligned.
pub const HEADER_LEN: usize = 128;
/// Byte size of one entity record.
pub const ENTITY_LEN: usize = size_of::<LiveEntityRecord>();
/// Hard cap on entities per frame; extras are truncated.
pub const MAX_ENTITIES: usize = 8192;
/// Total size of the file mapping.
pub const MAPPING_LEN: usize = 2 * 1024 * 1024;
/// Offset of the UTF-8 string table (after the fixed-size entity array).
pub const STRINGS_OFFSET: usize = HEADER_LEN + MAX_ENTITIES * ENTITY_LEN;
/// Maximum bytes available for the string table.
pub const MAX_STRING_LEN: usize = MAPPING_LEN - STRINGS_OFFSET;

/// `seq` field offset inside the header; read/written atomically.
pub const SEQ_OFFSET: usize = offset_of!(LiveHeader, seq);

/// `Local\xenomit_state_<pid>` — opened by readers via `OpenFileMappingW`.
pub fn mapping_name(pid: u32) -> String {
    format!(r"Local\xenomit_state_{pid}")
}

/// Linux frame files, in the order the Pepper host creates them.
pub fn frame_file_candidates(pid: u32) -> [String; 2] {
    [
        format!("/dev/shm/xenomit_state_{pid}"),
        format!("/tmp/xenomit_state_{pid}"),
    ]
}

/// Entity kinds; `LiveEntityRecord::kind`.
pub mod kind {
    pub const OTHER: u8 = 0;
    pub const HERO: u8 = 1;
    pub const PLAYER: u8 = 2;
    pub const NPC: u8 = 3;
    pub const PET: u8 = 4;
    pub const BOX: u8 = 5;
    pub const ORE: u8 = 6;
    pub const PORTAL: u8 = 7;
    pub const BASE: u8 = 8;
    pub const STATION: u8 = 9;
    pub const BEACON: u8 = 10;
    pub const MINE: u8 = 11;
}

/// `LiveEntityRecord::flags` bits.
pub mod entity_flags {
    /// Ship with `_isNPC` set.
    pub const NPC: u8 = 1;
    /// Entity whose id equals `Hero.userID`.
    pub const HERO: u8 = 2;
    /// MoveTrait `_destination` is set.
    pub const MOVING: u8 = 4;
    /// hp/shield/nanohull fields are meaningful.
    pub const HAS_HP: u8 = 8;
    /// `type_id` is meaningful.
    pub const HAS_TYPE_ID: u8 = 16;
    /// Collectable: `name` carries the collectable hash.
    pub const COLLECTABLE: u8 = 32;
}

/// `LiveHeader::hero_flags` bits.
pub mod hero_flags {
    pub const PRESENT: u32 = 1;
    pub const MOVING: u32 = 2;
    /// `target_id` carries the selected (locked) entity id; 0 means none.
    pub const TARGET: u32 = 4;
    /// `attack_target_id` carries the hero's current attack target; 0 means none.
    pub const ATTACK: u32 = 8;
    /// `hero_speed` is meaningful.
    pub const SPEED: u32 = 16;
}

/// Frame header, padded to [`HEADER_LEN`].
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LiveHeader {
    pub magic: u32,
    pub version: u32,
    /// Seqlock sequence counter: odd while a write is in progress.
    pub seq: u32,
    pub _pad: u32,
    /// Capture counter, incremented once per published frame.
    pub frame: u64,
    /// Unix time in milliseconds when the capture ran.
    pub captured_unix_ms: u64,
    /// Wall time the capture took, microseconds.
    pub capture_us: u32,
    /// -1 when no map is active.
    pub map_id: i32,
    pub map_width: f32,
    pub map_height: f32,
    pub hero_id: i32,
    pub hero_x: f32,
    pub hero_y: f32,
    pub hero_dest_x: f32,
    pub hero_dest_y: f32,
    pub hero_flags: u32,
    pub hero_faction: i32,
    pub hero_clan: i32,
    /// Number of valid [`LiveEntityRecord`]s in the entity array.
    pub entity_count: u32,
    /// Number of valid bytes in the string table.
    pub string_bytes: u32,
    pub navigation_revision: u64,
    /// JSON navigation/manager payload in the same bounded string table.
    pub navigation_off: u32,
    pub navigation_len: u32,
    /// Selected entity id; valid when [`hero_flags::TARGET`] is set.
    pub target_id: i32,
    /// Hero attack target (AttackerTrait); valid when [`hero_flags::ATTACK`] is set.
    pub attack_target_id: i32,
    /// Hero move speed; valid when [`hero_flags::SPEED`] is set.
    pub hero_speed: i32,
    pub _reserved: [u8; 12],
}

const _: () = assert!(size_of::<LiveHeader>() == HEADER_LEN);

impl Default for LiveHeader {
    fn default() -> Self {
        Self {
            magic: 0,
            version: 0,
            seq: 0,
            _pad: 0,
            frame: 0,
            captured_unix_ms: 0,
            capture_us: 0,
            map_id: -1,
            map_width: 0.0,
            map_height: 0.0,
            hero_id: 0,
            hero_x: 0.0,
            hero_y: 0.0,
            hero_dest_x: 0.0,
            hero_dest_y: 0.0,
            hero_flags: 0,
            hero_faction: 0,
            hero_clan: 0,
            entity_count: 0,
            string_bytes: 0,
            navigation_revision: 0,
            navigation_off: 0,
            navigation_len: 0,
            target_id: 0,
            attack_target_id: 0,
            hero_speed: 0,
            _reserved: [0; 12],
        }
    }
}

/// Fixed-size entity record. `name_off`/`name_len` index the string table.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct LiveEntityRecord {
    pub id: i32,
    pub kind: u8,
    pub flags: u8,
    pub _pad: u16,
    pub x: f32,
    pub y: f32,
    /// Destination when `flags & entity_flags::MOVING` is set.
    pub dest_x: f32,
    pub dest_y: f32,
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
    pub name_off: u32,
    pub name_len: u32,
}

const _: () = assert!(size_of::<LiveEntityRecord>() % 8 == 0);
