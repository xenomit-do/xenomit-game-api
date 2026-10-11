use super::layout;
use super::{
    LiveEntity, LiveFrame, LiveKind, LiveStateReader, layout::LiveEntityRecord, layout::LiveHeader,
};
use crate::model::EntityCategory;

/// Write a complete frame into a raw buffer like `state_shm.rs` does:
/// seq odd, payload, seq even.
fn write_frame(
    buffer: &mut [u8],
    header: &LiveHeader,
    entities: &[LiveEntityRecord],
    strings: &[u8],
) {
    let mut header = *header;
    header.magic = layout::MAGIC;
    header.version = layout::VERSION;
    header.entity_count = entities.len() as u32;
    header.string_bytes = strings.len() as u32;

    unsafe {
        let base = buffer.as_mut_ptr();
        let seq = &mut *base.add(layout::SEQ_OFFSET).cast::<u32>();
        *seq = header.seq | 1;
        std::ptr::write(base.cast::<LiveHeader>(), header);
        std::ptr::copy_nonoverlapping(
            entities.as_ptr(),
            base.add(layout::HEADER_LEN).cast::<LiveEntityRecord>(),
            entities.len(),
        );
        std::ptr::copy_nonoverlapping(
            strings.as_ptr(),
            base.add(layout::STRINGS_OFFSET),
            strings.len(),
        );
        *seq = header.seq.wrapping_add(2);
    }
}

fn blank_header() -> LiveHeader {
    LiveHeader::default()
}

#[test]
fn navigation_payload_is_bounded_cached_and_invalidated_when_missing_or_stale() {
    let mut buffer = vec![0u8; layout::MAPPING_LEN];
    let navigation = crate::NavigationState {
        map_id: Some(8),
        geometry_ready: true,
        ..Default::default()
    };
    let payload = serde_json::to_vec(&navigation).unwrap();
    let mut header = LiveHeader {
        map_id: 8,
        navigation_revision: 1,
        navigation_len: payload.len() as u32,
        ..Default::default()
    };
    write_frame(&mut buffer, &header, &[], &payload);
    let reader = unsafe { LiveStateReader::from_raw(buffer.as_ptr(), buffer.len()) };
    let mut frame = LiveFrame::default();
    assert!(reader.read_into(&mut frame));
    assert!(frame.navigation.geometry_ready);
    assert_eq!(frame.to_game_snapshot().navigation, navigation);
    header.navigation_len = 0;
    write_frame(&mut buffer, &header, &[], &[]);
    assert!(reader.read_into(&mut frame));
    assert!(!frame.navigation.geometry_ready);
    assert_eq!(frame.navigation.map_id, Some(8));
    // Same revision becoming available again must recover after a dropped blob.
    header.navigation_len = payload.len() as u32;
    write_frame(&mut buffer, &header, &[], &payload);
    assert!(reader.read_into(&mut frame));
    assert!(frame.navigation.geometry_ready);
    header.map_id = 9;
    header.navigation_revision = 2;
    write_frame(&mut buffer, &header, &[], &payload);
    assert!(reader.read_into(&mut frame));
    assert!(!frame.navigation.geometry_ready);
    assert_eq!(frame.navigation.map_id, Some(9));
    header.navigation_off = u32::MAX;
    header.navigation_len = u32::MAX;
    header.navigation_revision = 3;
    write_frame(&mut buffer, &header, &[], &[]);
    assert!(reader.read_into(&mut frame));
    assert!(!frame.navigation.geometry_ready);
}

#[test]
fn layout_offsets_are_stable() {
    use std::mem::offset_of;
    assert_eq!(layout::MAGIC, u32::from_le_bytes(*b"XSTM"));
    assert_eq!(size_of::<LiveHeader>(), layout::HEADER_LEN);
    assert_eq!(size_of::<LiveEntityRecord>(), layout::ENTITY_LEN);
    assert_eq!(layout::ENTITY_LEN, 72);
    assert_eq!(layout::SEQ_OFFSET, offset_of!(LiveHeader, seq));
    assert_eq!(offset_of!(LiveHeader, entity_count), 80);
    assert_eq!(offset_of!(LiveHeader, string_bytes), 84);
    assert_eq!(offset_of!(LiveEntityRecord, x), 8);
    assert_eq!(offset_of!(LiveEntityRecord, hp), 24);
    assert_eq!(offset_of!(LiveEntityRecord, faction), 48);
    assert_eq!(offset_of!(LiveEntityRecord, name_off), 64);
    assert_eq!(
        layout::STRINGS_OFFSET + layout::MAX_STRING_LEN,
        layout::MAPPING_LEN
    );
}

#[test]
fn reads_a_hand_written_frame() {
    let mut buffer = vec![0u8; layout::MAPPING_LEN];
    let name = "Murderous Mordon";
    let entity = LiveEntityRecord {
        id: 42121,
        kind: layout::kind::NPC,
        flags: layout::entity_flags::NPC | layout::entity_flags::HAS_HP,
        x: 1234.5,
        y: -77.25,
        hp: 8000,
        max_hp: 10000,
        shield: 500,
        max_shield: 2000,
        nanohull: 12,
        nanohull_max: 50,
        faction: 2,
        clan: 77,
        clan_diplomacy: 1,
        name_off: 0,
        name_len: name.len() as u32,
        ..LiveEntityRecord::default()
    };
    let header = LiveHeader {
        frame: 42,
        captured_unix_ms: 1_700_000_000_000,
        capture_us: 123,
        map_id: 8,
        map_width: 21000.0,
        map_height: 13100.0,
        hero_id: 7,
        hero_x: 100.0,
        hero_y: 200.0,
        hero_dest_x: 300.0,
        hero_dest_y: 400.0,
        hero_flags: layout::hero_flags::PRESENT | layout::hero_flags::MOVING,
        hero_faction: 3,
        hero_clan: 99,
        ..blank_header()
    };
    write_frame(&mut buffer, &header, &[entity], name.as_bytes());

    let reader = unsafe { LiveStateReader::from_raw(buffer.as_ptr(), buffer.len()) };
    let frame = reader.read().expect("complete frame must decode");

    assert_eq!(frame.frame, 42);
    assert_eq!(frame.captured_unix_ms, 1_700_000_000_000);
    assert_eq!(frame.capture_us, 123);
    let map = frame.map.expect("map present");
    assert_eq!((map.id, map.width, map.height), (8, 21000.0, 13100.0));
    let hero = frame.hero.expect("hero present");
    assert_eq!(hero.id, 7);
    assert_eq!(hero.dest, Some(crate::Point { x: 300.0, y: 400.0 }));
    assert_eq!(hero.faction, 3);
    assert_eq!(hero.clan, 99);

    assert_eq!(frame.entities.len(), 1);
    let entity = &frame.entities[0];
    assert_eq!(entity.id, 42121);
    assert_eq!(entity.kind, LiveKind::Npc);
    assert!(entity.is_npc && !entity.is_hero && entity.has_hp);
    assert!(!entity.dest.is_some());
    assert_eq!(entity.hp, 8000);
    assert_eq!(entity.max_hp, 10000);
    assert_eq!(entity.nanohull, 12);
    assert_eq!(entity.faction, 2);
    assert_eq!(entity.clan, 77);
    assert_eq!(entity.clan_diplomacy, 1);
    assert_eq!(entity.name, name);
}

#[test]
fn read_into_reuses_allocations() {
    let mut buffer = vec![0u8; layout::MAPPING_LEN];
    let entity = LiveEntityRecord {
        id: 1,
        kind: layout::kind::BOX,
        flags: layout::entity_flags::COLLECTABLE,
        name_off: 0,
        name_len: 9,
        ..LiveEntityRecord::default()
    };
    write_frame(&mut buffer, &blank_header(), &[entity], b"box_bonus");

    let reader = unsafe { LiveStateReader::from_raw(buffer.as_ptr(), buffer.len()) };
    let mut frame = LiveFrame::default();
    assert!(reader.read_into(&mut frame));
    assert_eq!(frame.entities.len(), 1);
    assert_eq!(frame.entities[0].name, "box_bonus");

    // Shrink the next frame; the entity vec must reflect the new count.
    write_frame(&mut buffer, &blank_header(), &[], b"");
    assert!(reader.read_into(&mut frame));
    assert!(frame.entities.is_empty());
}

#[test]
fn odd_sequence_retries_then_gives_up() {
    let mut buffer = vec![0u8; layout::MAPPING_LEN];
    let header = LiveHeader {
        seq: 3, // odd: writer permanently mid-write
        ..blank_header()
    };
    unsafe {
        std::ptr::write(buffer.as_mut_ptr().cast::<LiveHeader>(), header);
    }
    let reader = unsafe { LiveStateReader::from_raw(buffer.as_ptr(), buffer.len()) };
    assert!(
        reader.read().is_none(),
        "a never-settled seq must not decode"
    );
}

#[test]
fn garbage_magic_does_not_decode() {
    let mut buffer = vec![0xABu8; layout::MAPPING_LEN];
    unsafe {
        *buffer.as_mut_ptr().add(layout::SEQ_OFFSET).cast::<u32>() = 0;
    }
    let reader = unsafe { LiveStateReader::from_raw(buffer.as_ptr(), buffer.len()) };
    assert!(reader.read().is_none());
}

#[test]
fn snapshot_shapes_match_state_contract() {
    let mut frame = LiveFrame {
        navigation: Default::default(),
        navigation_revision: 0,
        frame: 5,
        captured_unix_ms: 10,
        capture_us: 1,
        map: Some(super::LiveMap {
            id: 8,
            width: 21000.0,
            height: 13100.0,
        }),
        hero: Some(super::LiveHero {
            id: 7,
            x: 100.0,
            y: 200.0,
            dest: None,
            faction: 1,
            clan: 9,
        }),
        entities: Vec::new(),
    };
    frame.entities.push(LiveEntity {
        id: 7,
        kind: LiveKind::Hero,
        x: 100.0,
        y: 200.0,
        is_hero: true,
        has_hp: true,
        hp: 150000,
        max_hp: 256000,
        name: "hero-name".into(),
        ..LiveEntity::default()
    });
    frame.entities.push(LiveEntity {
        id: 12,
        kind: LiveKind::Box,
        x: 400.0,
        y: 500.0,
        is_collectable: true,
        name: "box_bonus".into(),
        ..LiveEntity::default()
    });
    frame.entities.push(LiveEntity {
        id: 30,
        kind: LiveKind::Portal,
        x: 100.0,
        y: 200.0,
        name: "portal".into(),
        ..LiveEntity::default()
    });

    let snapshot = frame.to_game_snapshot();
    let map = snapshot.map.expect("map");
    assert_eq!(map.id, Some(8));
    assert_eq!(
        map.player_position,
        Some(crate::Point { x: 100.0, y: 200.0 })
    );
    // Hero sits on the portal: in a safe place.
    assert_eq!(map.in_safe_place, Some(true));
    assert_eq!(map.safe_places.len(), 1);
    assert_eq!(map.safe_places[0].kind, "portal");
    assert_eq!(map.safe_places[0].radius, 250.0);

    let hero = &snapshot.entities[0];
    assert_eq!(hero.path, "map/7");
    assert_eq!(hero.category, EntityCategory::Player);
    assert_eq!(hero.field("isHero"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(hero.field("health"), Some(&serde_json::json!(150000)));
    assert_eq!(hero.field("worldX"), Some(&serde_json::json!(100.0)));
    assert_eq!(hero.health, Some(150000.0));

    let boxx = &snapshot.entities[1];
    assert_eq!(boxx.field("entityKind"), Some(&serde_json::json!("box")));
    assert_eq!(boxx.field("isCollectable"), Some(&serde_json::json!(true)));
    assert_eq!(boxx.field("hash"), Some(&serde_json::json!("box_bonus")));

    let portal = &snapshot.entities[2];
    assert_eq!(portal.category, EntityCategory::Portal);
}

#[test]
fn from_game_snapshot_maps_hero_and_entities() {
    use crate::model::{GameEntity, GameMapInfo, GameSnapshot, Point, StageInfo};
    use serde_json::{Map, Value, json};

    let fields = |pairs: &[(&str, Value)]| {
        pairs
            .iter()
            .cloned()
            .map(|(k, v)| (k.into(), v))
            .collect::<Map<String, Value>>()
    };
    let entity = |id: i32,
                  class: &str,
                  category: EntityCategory,
                  x: f64,
                  y: f64,
                  fields: Map<String, Value>| GameEntity {
        path: format!("map/{id}"),
        class_name: class.into(),
        name: String::new(),
        category,
        position: Point { x, y },
        bounds: crate::Bounds {
            x,
            y,
            width: 0.0,
            height: 0.0,
        },
        id: Some(json!(id)),
        health: None,
        max_health: None,
        shield: None,
        max_shield: None,
        fields,
    };

    let snapshot = GameSnapshot {
        navigation: Default::default(),
        observed_at_unix_ms: 1_700_000_000_000,
        stage: StageInfo::default(),
        map: Some(GameMapInfo {
            id: Some(8),
            name: None,
            width: Some(21000.0),
            height: Some(13100.0),
            player_position: Some(Point { x: 100.0, y: 200.0 }),
            safe_places: Vec::new(),
            in_safe_place: Some(false),
        }),
        entities: vec![
            entity(
                7,
                "Ship",
                EntityCategory::Player,
                0.0,
                0.0,
                fields(&[
                    ("isHero", json!(true)),
                    ("worldX", json!(100.0)),
                    ("worldY", json!(200.0)),
                    ("health", json!(150000.0)),
                    ("maxHealth", json!(256000.0)),
                    ("factionID", json!(1)),
                ]),
            ),
            entity(
                30,
                "Portal",
                EntityCategory::Portal,
                0.0,
                0.0,
                fields(&[("worldX", json!(500.0)), ("worldY", json!(600.0))]),
            ),
            entity(
                42,
                "Box",
                EntityCategory::Unknown,
                0.0,
                0.0,
                fields(&[
                    ("entityKind", json!("box")),
                    ("isCollectable", json!(true)),
                    ("hash", json!("box_bonus")),
                    ("worldX", json!(700.0)),
                    ("worldY", json!(800.0)),
                ]),
            ),
        ],
        visited: 3,
        truncated: false,
    };

    let frame = LiveFrame::from_game_snapshot(&snapshot);
    assert_eq!(frame.frame, 1_700_000_000_000);
    assert_eq!(frame.capture_us, 0);
    let map = frame.map.expect("map");
    assert_eq!((map.id, map.width, map.height), (8, 21000.0, 13100.0));
    let hero = frame.hero.expect("hero from player_position");
    assert_eq!((hero.id, hero.x, hero.y), (7, 100.0, 200.0));
    assert_eq!(hero.faction, 1);

    assert_eq!(frame.entities.len(), 3);
    let hero_entity = &frame.entities[0];
    assert_eq!(hero_entity.kind, LiveKind::Hero);
    assert!(hero_entity.is_hero && hero_entity.has_hp);
    assert_eq!((hero_entity.hp, hero_entity.max_hp), (150000, 256000));
    assert_eq!(frame.entities[1].kind, LiveKind::Portal);
    let boxx = &frame.entities[2];
    assert_eq!(boxx.kind, LiveKind::Box);
    assert!(boxx.is_collectable);
    assert_eq!((boxx.x, boxx.y), (700.0, 800.0));
}

#[test]
fn frame_file_roundtrip_keeps_geometry_unset() {
    let dir = std::env::temp_dir().join(format!("xenomit-frame-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("frame");
    let mut buffer = vec![0u8; layout::MAPPING_LEN];
    let navigation = br#"{"map_id":8,"geometry_ready":false}"#;
    let now = crate::bridge::wire::now_unix_ms();
    let header = LiveHeader {
        map_id: 8,
        map_width: 1000.0,
        map_height: 800.0,
        hero_id: 3,
        hero_x: 10.0,
        hero_y: 20.0,
        hero_flags: layout::hero_flags::PRESENT,
        navigation_revision: 1,
        navigation_len: navigation.len() as u32,
        captured_unix_ms: now,
        frame: 4,
        ..LiveHeader::default()
    };
    write_frame(&mut buffer, &header, &[], navigation);
    std::fs::write(&path, &buffer).unwrap();
    let reader = LiveStateReader::open_frame_file(&path).unwrap();
    let frame = reader.read().expect("frame");
    assert!(frame.is_fresh(now));
    assert!(!frame.navigation.geometry_ready);
    assert_eq!(frame.navigation.map_id, Some(8));
    assert_eq!(frame.map.expect("map").id, 8);
    drop(reader);
    let _ = std::fs::remove_dir_all(&dir);
}
