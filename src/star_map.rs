//! Static DarkOrbit star-map catalog — the same fixed map names and portal
//! links DarkBot embeds in `StarManager`. The native shared-memory feed
//! normally carries this data in `NavigationState::star_system`; these tables
//! are the fallback for backends that do not publish it (legacy pipe /
//! packet sessions) and for portal targets the runtime catalog missed.

/// `(map id, display name, short name for portal labels)`.
pub const MAPS: &[(i32, &str, &str)] = &[
    (-2, "Home Map", "Home Map"),
    (-3, "GG Escort", "GG Escort"),
    (-4, "GG Eternal", "GG Eternal"),
    (-5, "Labyrinth", "Labyrinth"),
    (-6, "GG Astral", "GG Astral"),
    (1, "1-1", "1-1"),
    (2, "1-2", "1-2"),
    (3, "1-3", "1-3"),
    (4, "1-4", "1-4"),
    (17, "1-5", "1-5"),
    (18, "1-6", "1-6"),
    (19, "1-7", "1-7"),
    (20, "1-8", "1-8"),
    (5, "2-1", "2-1"),
    (6, "2-2", "2-2"),
    (7, "2-3", "2-3"),
    (8, "2-4", "2-4"),
    (21, "2-5", "2-5"),
    (22, "2-6", "2-6"),
    (23, "2-7", "2-7"),
    (24, "2-8", "2-8"),
    (9, "3-1", "3-1"),
    (10, "3-2", "3-2"),
    (11, "3-3", "3-3"),
    (12, "3-4", "3-4"),
    (25, "3-5", "3-5"),
    (26, "3-6", "3-6"),
    (27, "3-7", "3-7"),
    (28, "3-8", "3-8"),
    (13, "4-1", "4-1"),
    (14, "4-2", "4-2"),
    (15, "4-3", "4-3"),
    (16, "4-4", "4-4"),
    (29, "4-5", "4-5"),
    (91, "5-1", "5-1"),
    (92, "5-2", "5-2"),
    (93, "5-3", "5-3"),
    (94, "5-4", "5-4"),
    (306, "1BL", "1BL"),
    (307, "2BL", "2BL"),
    (308, "3BL", "3BL"),
    (401, "Experiment Zone 1", "EZ 1"),
    (402, "Experiment Zone 2-1", "EZ 2-1"),
    (403, "Experiment Zone 2-2", "EZ 2-2"),
    (404, "Experiment Zone 2-3", "EZ 2-3"),
    (51, "GG α", "GG α"),
    (52, "GG β", "GG β"),
    (53, "GG γ", "GG γ"),
    (54, "GG NC", "GG NC"),
    (55, "GG δ", "GG δ"),
    (56, "GG Orb", "GG Orb"),
    (57, "GG Y6", "GG Y6"),
    (58, "HSG", "HSG"),
    (70, "GG ε", "GG ε"),
    (71, "GG ζ 1", "GG ζ 1"),
    (72, "GG ζ 2", "GG ζ 2"),
    (73, "GG ζ 3", "GG ζ 3"),
    (74, "GG κ", "GG κ"),
    (75, "GG λ", "GG λ"),
    (76, "GG Kronos", "GG K"),
    (77, "GG Cold Wave Easy", "GG Cold E"),
    (78, "GG Cold Wave Hard", "GG Cold H"),
    (203, "GG Hades", "Hades"),
    (223, "Devolarium Attack", "DA"),
    (225, "GG PET Attack Easy", "GG PET E"),
    (226, "GG PET Attack Hard", "GG PET H"),
    (228, "Permafrost Fissure", "GG PF"),
    (300, "GG ς 1", "GG ς 1"),
    (301, "GG ς 2", "GG ς 2"),
    (302, "GG ς 3", "GG ς 3"),
    (303, "GG ς 4", "GG ς 4"),
    (304, "GG ς 5", "GG ς 5"),
    (200, "LoW", "LoW"),
    (229, "Quarantine Zone", "QZ"),
    (227, "GG VoT 1", "GG VoT 1"),
    (230, "GG VoT 2", "GG VoT 2"),
    (231, "GG VoT 3", "GG VoT 3"),
    (232, "GG VoT 4", "GG VoT 4"),
    (233, "GG VoT 5", "GG VoT 5"),
    (234, "GG VoT 6", "GG VoT 6"),
    (235, "GG VoT 7", "GG VoT 7"),
    (236, "GG VoT 8", "GG VoT 8"),
    (305, "Compromising Invasion", "CI"),
    (410, "GoP Normal 1", "GoP N"),
    (411, "GoP Normal 2", "GoP N"),
    (412, "GoP Normal 3", "GoP N"),
    (413, "GoP Normal 4", "GoP N"),
    (414, "GoP Normal 5", "GoP N"),
    (415, "GoP Normal Final", "GoP N"),
    (469, "Plutus' Trove of Riches Normal", "PToR N"),
    (450, "GoP Easy 1", "GoP E"),
    (451, "GoP Easy 2", "GoP E"),
    (452, "GoP Easy 3", "GoP E"),
    (453, "GoP Easy 4", "GoP E"),
    (454, "GoP Easy 5", "GoP E"),
    (455, "GoP Easy Final", "GoP E"),
    (470, "Plutus' Trove of Riches Easy", "PToR E"),
    (471, "Treacherous Domain Easy", "TD E"),
    (472, "Treacherous Domain Normal", "TD N"),
    (482, "Treacherous Domain", "TD"),
    (42, "???", "???"),
    (61, "MMO Invasion", "MMO Inv"),
    (62, "EIC Invasion", "EIC Inv"),
    (63, "VRU Invasion", "VRU Inv"),
    (64, "MMO Invasion", "MMO Inv"),
    (65, "EIC Invasion", "EIC Inv"),
    (66, "VRU Invasion", "VRU Inv"),
    (67, "MMO Invasion", "MMO Inv"),
    (68, "EIC Invasion", "EIC Inv"),
    (69, "VRU Invasion", "VRU Inv"),
    (81, "TDM I", "TDM I"),
    (82, "TDM II", "TDM II"),
    (101, "JP", "JP"),
    (102, "JP", "JP"),
    (103, "JP", "JP"),
    (104, "JP", "JP"),
    (105, "JP", "JP"),
    (106, "JP", "JP"),
    (107, "JP", "JP"),
    (108, "JP", "JP"),
    (109, "JP", "JP"),
    (110, "JP", "JP"),
    (111, "JP", "JP"),
    (112, "UBA", "UBA"),
    (113, "UBA", "UBA"),
    (114, "UBA", "UBA"),
    (115, "UBA", "UBA"),
    (116, "UBA", "UBA"),
    (117, "UBA", "UBA"),
    (118, "UBA", "UBA"),
    (119, "UBA", "UBA"),
    (120, "UBA", "UBA"),
    (121, "UBA", "UBA"),
    (201, "SC-1", "SC-1"),
    (202, "SC-2", "SC-2"),
    (224, "Custom Tournament", "CT"),
    (150, "R-Zone 1", "R-Z"),
    (151, "R-Zone 2", "R-Z"),
    (152, "R-Zone 3", "R-Z"),
    (153, "R-Zone 4", "R-Z"),
    (154, "R-Zone 5", "R-Z"),
    (155, "R-Zone 6", "R-Z"),
    (156, "R-Zone 7", "R-Z"),
    (157, "R-Zone 8", "R-Z"),
    (158, "R-Zone 9", "R-Z"),
    (159, "R-Zone 10", "R-Z"),
    (420, "WarGame 1", "WG"),
    (421, "WarGame 2", "WG"),
    (422, "WarGame 3", "WG"),
    (423, "WarGame 4", "WG"),
    (424, "WarGame 5", "WG"),
    (425, "WarGame 6", "WG"),
    (430, "Escort VRU 1", "ESC-V1"),
    (431, "Escort VRU 2", "ESC-V2"),
    (432, "Escort VRU 3", "ESC-V3"),
    (433, "Escort MMO 1", "ESC-M1"),
    (434, "Escort MMO 2", "ESC-M2"),
    (435, "Escort MMO 3", "ESC-M3"),
    (436, "Escort EIC 1", "ESC-E1"),
    (437, "Escort EIC 2", "ESC-E2"),
    (438, "Escort EIC 3", "ESC-E3"),
    (460, "Eternal Blacklight", "GG ∞ BL"),
    (461, "Eternal Blacklight", "GG ∞ BL"),
    (462, "Eternal Blacklight", "GG ∞ BL"),
    (463, "Eternal Blacklight", "GG ∞ BL"),
    (464, "Eternal Blacklight", "GG ∞ BL"),
    (465, "Eternal Blacklight", "GG ∞ BL"),
    (466, "Astral Ascension", "GG Astral"),
    (467, "Astral Ascension", "GG Astral"),
    (468, "Astral Ascension", "GG Astral"),
    (439, "Eternal Gate", "GG ∞"),
    (440, "Eternal Gate", "GG ∞"),
    (441, "Eternal Gate", "GG ∞"),
    (442, "Eternal Gate", "GG ∞"),
    (443, "Eternal Gate", "GG ∞"),
    (444, "Eternal Gate", "GG ∞"),
    (445, "Eternal Gate", "GG ∞"),
];

/// Position-matched portals: `(on_map, x, y, target_map)`.
pub const PORTALS: &[(i32, i32, i32, i32)] = &[
    // MMO
    (1, 18500, 11500, 2),
    (1, 10500, 6750, 401),
    (2, 2000, 2000, 1),
    (2, 18500, 2000, 3),
    (2, 18500, 11500, 4),
    (3, 2000, 11500, 2),
    (3, 18500, 11500, 4),
    (3, 18500, 2000, 7),
    (4, 2000, 2000, 2),
    (4, 18500, 2000, 3),
    (4, 19000, 6000, 13),
    (4, 18500, 11500, 12),
    (17, 19000, 6000, 16),
    (17, 10000, 12000, 29),
    (17, 2000, 2000, 18),
    (17, 2000, 11500, 19),
    (17, 10500, 6750, 402),
    (18, 18500, 11500, 17),
    (18, 2000, 11500, 20),
    (19, 2000, 2000, 20),
    (19, 18500, 2000, 17),
    (20, 18500, 2000, 18),
    (20, 18500, 11500, 19),
    (20, 11084, 11084, 306),
    // EIC
    (5, 2000, 11500, 6),
    (5, 10500, 6750, 401),
    (6, 2000, 11500, 7),
    (6, 18500, 11500, 8),
    (6, 18500, 2000, 5),
    (7, 2000, 11500, 3),
    (7, 18500, 11500, 8),
    (7, 18500, 2000, 6),
    (8, 2000, 2000, 6),
    (8, 18500, 2000, 7),
    (8, 2000, 11500, 11),
    (8, 10000, 12000, 14),
    (21, 2000, 11500, 16),
    (21, 18500, 11500, 29),
    (21, 2000, 2000, 22),
    (21, 18500, 2000, 23),
    (21, 10500, 6750, 403),
    (22, 2000, 11500, 21),
    (22, 18500, 2000, 24),
    (23, 2000, 11500, 21),
    (23, 18500, 2000, 24),
    (24, 2000, 11500, 22),
    (24, 18500, 11500, 23),
    (24, 11084, 11084, 307),
    // VRU
    (9, 2000, 2000, 10),
    (9, 10500, 6750, 401),
    (10, 18500, 2000, 11),
    (10, 2000, 2000, 12),
    (10, 18500, 11500, 9),
    (11, 2000, 2000, 8),
    (11, 2000, 11500, 12),
    (11, 18500, 11500, 10),
    (12, 2000, 2000, 4),
    (12, 10000, 1500, 15),
    (12, 18500, 2000, 11),
    (12, 18500, 11500, 10),
    (25, 2000, 2000, 16),
    (25, 16500, 1500, 29),
    (25, 2000, 11500, 26),
    (25, 18500, 11500, 27),
    (25, 10500, 6750, 404),
    (26, 2000, 2000, 25),
    (26, 18500, 11500, 28),
    (27, 2000, 11500, 25),
    (27, 18500, 11500, 28),
    (28, 2000, 2000, 27),
    (28, 2000, 11500, 26),
    (28, 11084, 11084, 308),
    // Battle maps
    (13, 1500, 6000, 4),
    (13, 18500, 2000, 14),
    (13, 18500, 11500, 15),
    (13, 10500, 6750, 16),
    (14, 10000, 1500, 8),
    (14, 2000, 11500, 13),
    (14, 18500, 11500, 15),
    (14, 10500, 6750, 16),
    (15, 19000, 6000, 12),
    (15, 2000, 2000, 14),
    (15, 2000, 11500, 13),
    (15, 10500, 6750, 16),
    (16, 7000, 13500, 17),
    (16, 28000, 1376, 21),
    (16, 28000, 25124, 25),
    (16, 19200, 13500, 13),
    (16, 21900, 11941, 14),
    (16, 21900, 14559, 15),
    (29, 7000, 13500, 17),
    (29, 28000, 1376, 21),
    (29, 28000, 25624, 25),
    (29, 12200, 13300, 91),
    (29, 25000, 6300, 91),
    (29, 25000, 20700, 91),
    // Pirates
    (91, 5200, 6800, 92),
    (91, 2900, 13500, 92),
    (91, 5200, 20600, 92),
    (92, 2800, 3600, 93),
    (92, 1300, 6750, 93),
    (92, 2800, 10900, 93),
    (92, 19300, 1400, 94),
    (92, 19300, 3400, 94),
    (92, 19300, 5400, 94),
    (93, 2000, 9500, 16),
    (93, 2000, 13500, 16),
    (93, 2000, 17500, 16),
    (94, 3000, 2000, 92),
    (94, 3000, 3500, 92),
    (94, 3000, 5000, 92),
    // Blacklight
    (306, 786, 11458, 20),
    (306, 7589, 1456, 307),
    (306, 20072, 11732, 308),
    (307, 9893, 862, 306),
    (307, 593, 5884, 24),
    (307, 20377, 7996, 308),
    (308, 1545, 12210, 306),
    (308, 19400, 11854, 307),
    (308, 14027, 3181, 28),
    // Experiment zones
    (401, 1200, 1100, -2),
    (402, 1200, 1100, 17),
    (403, 1200, 1100, 21),
    (404, 1200, 1100, 25),
];

/// Galaxy-gate / event portals matched by portal `type_id` on specific maps:
/// `(type_id, on_maps, x or -1, y or -1, target_map)`.
/// Entries with `x == -1` match anywhere on the listed maps.
pub const GATES: &[(i32, &[i32], i32, i32, i32)] = &[
    // Home maps = {1, 5, 9}; outposts = {20, 24, 28}; bases = home + outposts;
    // BL = {306, 307, 308}.
    (54, &[1, 20, 5, 24, 9, 28], 10500, 6500, -3), // GG Escort
    (86, &[1, 5, 9], 10500, 5800, -6),             // GG Astral
    (2, &[1, 5, 9], -1, -1, 51),                   // GG α
    (3, &[1, 5, 9], -1, -1, 52),                   // GG β
    (4, &[1, 5, 9], -1, -1, 53),                   // GG γ
    (5, &[1, 5, 9], -1, -1, 55),                   // GG δ
    (16, &[1, 5, 9], -1, -1, 57),                  // GG Y6
    (53, &[1, 5, 9], -1, -1, 70),                  // GG ε
    (54, &[1, 5, 9], -1, -1, 71),                  // GG ζ 1
    (54, &[71], -1, -1, 72),                       // GG ζ 2
    (54, &[72], -1, -1, 73),                       // GG ζ 3
    (54, &[306, 307, 308], -1, -1, -4),            // GG Eternal
    (70, &[1, 5, 9], -1, -1, 74),                  // GG κ
    (71, &[1, 5, 9], -1, -1, 75),                  // GG λ
    (72, &[1, 5, 9], -1, -1, 76),                  // GG Kronos
    (73, &[4, 8, 12], -1, -1, 77),                 // GG Cold Wave (easy/hard)
    (74, &[1, 5, 9], -1, -1, 203),                 // GG Hades
    (34, &[1, 5, 9], -1, -1, 223),                 // Devolarium Attack
    (34, &[3, 7, 11], -1, -1, 200),                // LoW
    (82, &[1, 5, 9], -1, -1, 300),                 // GG ς 1
    (82, &[300], -1, -1, 301),
    (82, &[301], -1, -1, 302),
    (82, &[302], -1, -1, 303),
    (82, &[303], -1, -1, 304),
    (81, &[4, 8, 12], -1, -1, 227), // GG VoT
    (81, &[227], -1, -1, 230),
    (81, &[230], -1, -1, 231),
    (81, &[231], -1, -1, 232),
    (81, &[232], -1, -1, 233),
    (81, &[233], -1, -1, 234),
    (81, &[234], -1, -1, 235),
    (81, &[235], -1, -1, 236),
    (83, &[1, 5, 9], -1, -1, -5),     // Labyrinth
    (84, &[19, 23, 27], -1, -1, 229), // Quarantine Zone
    (24, &[20, 24, 28], -1, -1, 410), // Gate of Prometheus N
    (24, &[410], -1, -1, 411),
    (24, &[411], -1, -1, 412),
    (24, &[412], -1, -1, 413),
    (24, &[413], -1, -1, 414),
    (24, &[414], -1, -1, 415),
    (235, &[1, 5, 9], -1, -1, 450), // Gate of Prometheus E
    (235, &[450], -1, -1, 451),
    (235, &[451], -1, -1, 452),
    (235, &[452], -1, -1, 453),
    (235, &[453], -1, -1, 454),
    (235, &[454], -1, -1, 455),
    (238, &[1, 5, 9], -1, -1, 471),             // Treacherous Domain E
    (238, &[20, 24, 28], -1, -1, 472),          // Treacherous Domain N
    (239, &[1, 20, 5, 24, 9, 28], -1, -1, 482), // Treacherous Domain
    // GG exit portals (type 1) back to the faction home map.
    (
        1,
        &[
            51, 52, 53, 55, 70, 71, 72, 73, 203, 300, 301, 302, 303, 304, 227, 230, 231, 232, 233,
            234, 235, 236, 410, 411, 412, 413, 414, 415, 450, 451, 452, 453, 454, 455, 469, 470,
            439, 440, 441, 442, 443, 444, 445, 460, 461, 462, 463, 464, 465,
        ],
        -1,
        -1,
        -2,
    ),
];

/// Position tolerance (world units) when matching a portal entity to a
/// catalog portal. Game portal positions are fixed and exact, so a modest
/// radius keeps matches unambiguous.
const PORTAL_MATCH_RADIUS: f32 = 500.0;

/// Full display name for a map id from the static catalog.
pub fn map_name(id: i32) -> Option<&'static str> {
    MAPS.iter()
        .find(|(m, ..)| *m == id)
        .map(|(_, name, _)| *name)
}

/// Short name for a map id — used inside portal labels.
pub fn map_short_name(id: i32) -> Option<&'static str> {
    MAPS.iter()
        .find(|(m, ..)| *m == id)
        .map(|(_, _, short)| *short)
}

/// Resolve where a portal entity on `map` at `(x, y)` leads to.
/// `type_id` is the portal's game type (`_typeID`), `0` when unknown.
///
/// Order matches DarkBot: exact-position catalog portals first, then
/// positioned gate types, then type-only gates.
pub fn portal_target(map: i32, x: f32, y: f32, type_id: i32) -> Option<i32> {
    let mut best: Option<(i32, f32)> = None;
    for &(m, px, py, to) in PORTALS {
        if m != map {
            continue;
        }
        let d = (px as f32 - x).hypot(py as f32 - y);
        if d <= PORTAL_MATCH_RADIUS && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((to, d));
        }
    }
    if let Some((to, _)) = best {
        return Some(to);
    }
    let mut type_match: Option<i32> = None;
    for &(t, maps, gx, gy, to) in GATES {
        if type_id <= 0 || t != type_id || !maps.contains(&map) {
            continue;
        }
        if gx >= 0 {
            // Positioned gate (e.g. Escort on base maps) — must match both.
            if (gx as f32 - x).hypot(gy as f32 - y) <= PORTAL_MATCH_RADIUS {
                return Some(to);
            }
        } else {
            type_match = type_match.or(Some(to));
        }
    }
    type_match
}
