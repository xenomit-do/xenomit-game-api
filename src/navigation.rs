//! World-space navigation and manager observations captured by native clients.
use crate::{Bounds, Point};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NavigationState {
    /// Slow supplementary observations, cached separately from hot frame state.
    pub features: Option<crate::ClientFeatureState>,
    pub map_id: Option<i32>,
    /// False when geometry was unavailable/truncated. Never navigate on stale
    /// geometry from a previous map. Empty, ready geometry is valid.
    pub geometry_ready: bool,
    pub obstacles: Vec<Obstacle>,
    pub portals: Vec<PortalInfo>,
    pub star_system: StarSystem,
    pub hero: HeroState,
    /// Optional manager facts under the documented ModuleContext field names.
    pub entity_fields: BTreeMap<i32, Map<String, Value>>,
    /// Galaxy-gate builder rows, indexed by the game's 1-based gate id.
    pub galaxy_gates: Vec<GalaxyGateInfo>,
    pub group: GroupState,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeroState {
    pub owns_pet: bool,
    pub pet_active: bool,
    pub pet_destroyed: bool,
    pub pet_gear: Option<i32>,
    pub pet_gears: Vec<i32>,
    pub configuration: Option<u8>,
    pub killed: bool,
    /// Offered, affordable, cooldown-free base repair; never a paid option.
    pub free_repair_type: Option<i32>,
    pub level: Option<u32>,
    /// Faction id from the hero ship, used to resolve `x-1` / `x-8` homes.
    pub faction: Option<i32>,
    pub items: BTreeMap<String, ItemState>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemState {
    pub quantity: Option<f64>,
    pub available: bool,
    pub activatable: bool,
    pub selected: bool,
    pub blocked: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Obstacle {
    pub id: i32,
    pub name: String,
    pub shape: ObstacleShape,
    /// Geometry class, when the client can distinguish it. Legacy zone
    /// observations default to no-access geometry and remain fail-closed.
    #[serde(default)]
    pub kind: ObstacleKind,
    /// An inverted no-access zone bounds a playable interior.
    #[serde(default)]
    pub inverted: bool,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObstacleKind {
    #[default]
    NoAccess,
    Mine,
    Cbs,
    Radiation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObstacleShape {
    Circle { center: Point, radius: f64 },
    Rectangle { bounds: Bounds },
    Polygon { points: Vec<Point> },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortalInfo {
    pub id: i32,
    pub position: Point,
    /// Portal design id (`Portal.designId`), used when a placed gate is not in the star graph.
    #[serde(default)]
    pub type_id: i32,
    pub target_map: Option<i32>,
    pub jumping: bool,
}
/// One row from `GalaxyGateBuilderModel.gates`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GalaxyGateInfo {
    pub id: i32,
    pub parts: i32,
    pub total_parts: i32,
    pub lives: i32,
    pub life_price: i32,
    pub current_wave: i32,
    pub total_waves: i32,
    pub finished: bool,
    pub multiplier: i32,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupState {
    pub members: Vec<GroupMemberInfo>,
    /// `Some(false)` is an observed solo state; `None` is unknown.
    pub active: Option<bool>,
    pub leader_id: Option<i32>,
    pub invites_open: Option<bool>,
    pub pending_invites: Vec<GroupInvite>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupMemberInfo {
    pub id: i32,
    pub name: String,
    pub map_id: i32,
    pub leader: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupInvite {
    pub player_id: i32,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StarSystem {
    pub maps: Vec<MapDescriptor>,
    /// Directed links, one per portal. No reverse link is inferred.
    pub links: Vec<MapLink>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapDescriptor {
    pub id: i32,
    pub name: String,
    pub min_level: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapLink {
    pub from: i32,
    pub to: i32,
    pub portal_id: i32,
}

impl StarSystem {
    /// Parse the game's loaded map catalog and spacemap portal configuration.
    /// Spacemap icon coordinates are not world coordinates and are not used.
    pub fn from_game_xml(maps_xml: &str, space_xml: &str) -> Result<Self, String> {
        let maps = roxmltree::Document::parse(maps_xml).map_err(|e| format!("map XML: {e}"))?;
        let space =
            roxmltree::Document::parse(space_xml).map_err(|e| format!("spacemap XML: {e}"))?;
        let mut catalog = BTreeMap::new();
        for node in maps.descendants().filter(|n| n.has_tag_name("map")) {
            let Some(id) = integer(node.attribute("id")) else {
                continue;
            };
            let name = node.attribute("name").unwrap_or("").to_owned();
            let min_level = integer(node.attribute("minLevel"))
                .or_else(|| child_integer(node, "minLevel"))
                .unwrap_or(0)
                .max(0) as u32;
            catalog.insert(
                id,
                MapDescriptor {
                    id,
                    name,
                    min_level,
                },
            );
        }
        for node in space.descendants().filter(|n| n.has_tag_name("map")) {
            let Some(id) = integer(node.attribute("id")) else {
                continue;
            };
            catalog.entry(id).or_insert_with(|| MapDescriptor {
                id,
                name: node.attribute("name").unwrap_or("").into(),
                min_level: 0,
            });
        }
        let mut links = Vec::new();
        for node in space.descendants().filter(|n| n.has_tag_name("jumpgate")) {
            if let (Some(portal_id), Some(from), Some(to)) = (
                integer(node.attribute("key")),
                integer(node.attribute("ref")),
                child_integer(node, "leadsToMap"),
            ) {
                if from != to && catalog.contains_key(&from) && catalog.contains_key(&to) {
                    let link = MapLink {
                        from,
                        to,
                        portal_id,
                    };
                    if !links.contains(&link) {
                        links.push(link);
                    }
                }
            }
        }
        if catalog.is_empty() {
            return Err("game map catalog is empty".into());
        }
        Ok(Self {
            maps: catalog.into_values().collect(),
            links,
        })
    }
    pub fn map(&self, id: i32) -> Option<&MapDescriptor> {
        self.maps.iter().find(|m| m.id == id)
    }

    /// Parse the CDN `maps-config`/`spacemap-config` documents directly:
    /// expands their `<reference>` templates the way the client's
    /// `XMLUtils.resolveReferences` does before delegating to
    /// [`Self::from_game_xml`]. This is the dynamic form of the bundled
    /// `star_map` catalog for backends without shared-memory navigation.
    pub fn from_config_xml(maps_xml: &str, space_xml: &str) -> Result<Self, String> {
        let maps = resolve_references(maps_xml).map_err(|e| format!("maps-config: {e}"))?;
        let space = resolve_references(space_xml).map_err(|e| format!("spacemap-config: {e}"))?;
        Self::from_game_xml(&maps, &space)
    }
}
fn integer(text: Option<&str>) -> Option<i32> {
    text?.trim().parse().ok()
}
fn child_integer(node: roxmltree::Node<'_, '_>, name: &str) -> Option<i32> {
    integer(node.children().find(|n| n.has_tag_name(name))?.text())
}

#[cfg(test)]
mod native_navigation_tests {
    use super::*;
    #[test]
    fn loaded_game_xml_yields_names_directed_links_and_no_icon_coordinates() {
        let system = StarSystem::from_game_xml(r#"<maps><map id="1" name="1-1"/><map id="2" name="1-2"><minLevel>5</minLevel></map></maps>"#,
            r#"<spacemap><maps><map id="3" name="1-3"/></maps><jumpgates><jumpgate key="12" ref="1"><leadsToMap>2</leadsToMap><position x="17" y="18"/></jumpgate><jumpgate key="12" ref="1"><leadsToMap>2</leadsToMap></jumpgate><jumpgate key="99" ref="1"><leadsToMap>999</leadsToMap></jumpgate></jumpgates></spacemap>"#).unwrap();
        assert_eq!(system.maps.len(), 3);
        assert_eq!(system.map(2).unwrap().min_level, 5);
        assert_eq!(
            system.links,
            vec![MapLink {
                from: 1,
                to: 2,
                portal_id: 12
            }]
        );
        assert!(StarSystem::from_game_xml("<broken>", "<maps/>").is_err());
        assert!(StarSystem::from_game_xml("<maps/>", "<maps/>").is_err());
    }
    #[test]
    fn typed_commands_and_navigation_round_trip() {
        let mut state = NavigationState {
            map_id: Some(3),
            geometry_ready: true,
            ..Default::default()
        };
        state.hero.free_repair_type = Some(1);
        state.obstacles.push(Obstacle {
            id: -2,
            name: "NOA".into(),
            shape: ObstacleShape::Circle {
                center: Point { x: 2.0, y: 3.0 },
                radius: 50.0,
            },
            kind: ObstacleKind::NoAccess,
            inverted: true,
        });
        assert_eq!(
            serde_json::from_value::<NavigationState>(serde_json::to_value(&state).unwrap())
                .unwrap(),
            state
        );
        for command in [
            GameCommand::JumpPortal { portal_id: 12 },
            GameCommand::SetPet { enabled: false },
            GameCommand::SelectPetGear {
                gear_type: 6,
                target: 42,
            },
            GameCommand::SelectConfiguration { configuration: 2 },
            GameCommand::UseItem {
                item_id: "laser".into(),
                activate: false,
            },
            GameCommand::Revive { repair_type: 1 },
            GameCommand::ReviveAt {
                repair_type: 1,
                location: crate::ReviveLocation::Base,
            },
            GameCommand::RepairDrones,
            GameCommand::SpinGalaxyGate {
                gate_id: 1,
                spin_amount: 5,
                use_multiplier: true,
            },
            GameCommand::PlaceGalaxyGate { gate_id: 1 },
            GameCommand::BuyGalaxyLife { gate_id: 1 },
            GameCommand::AcceptGroupInvite { inviter_id: 9 },
            GameCommand::InviteGroupMember { player_id: 10 },
            GameCommand::LeaveGroup,
            GameCommand::KickGroupMember { player_id: 11 },
            GameCommand::SetGroupInvites { open: true },
        ] {
            assert_eq!(
                serde_json::from_value::<GameCommand>(serde_json::to_value(&command).unwrap())
                    .unwrap(),
                command
            );
        }
    }
}

/// Owned XML node for `resolve_references` — `roxmltree` trees are
/// immutable, and the template pass splices cloned nodes into parents.
#[derive(Clone, Debug)]
enum XmlNode {
    Element(XmlElem),
    Text(String),
}
#[derive(Clone, Debug)]
struct XmlElem {
    /// Local tag name (namespace prefix stripped).
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<XmlNode>,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn to_elem(node: roxmltree::Node<'_, '_>) -> XmlElem {
    XmlElem {
        name: local_name(node.tag_name().name()).to_owned(),
        attributes: node
            .attributes()
            .filter(|attr| !attr.name().starts_with("xmlns"))
            .map(|attr| (local_name(attr.name()).to_owned(), attr.value().to_owned()))
            .collect(),
        children: node
            .children()
            .filter_map(|child| {
                if child.is_element() {
                    Some(XmlNode::Element(to_elem(child)))
                } else if child.is_text() {
                    child.text().map(|t| XmlNode::Text(t.to_owned()))
                } else {
                    None
                }
            })
            .collect(),
    }
}

fn escape_xml(text: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\'' if attribute => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn write_xml(elem: &XmlElem, out: &mut String) {
    out.push('<');
    out.push_str(&elem.name);
    for (key, value) in &elem.attributes {
        out.push_str(&format!(" {key}=\"{}\"", escape_xml(value, true)));
    }
    if elem.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for child in &elem.children {
        match child {
            XmlNode::Element(e) => write_xml(e, out),
            XmlNode::Text(t) => out.push_str(&escape_xml(t, false)),
        }
    }
    out.push_str(&format!("</{}>", elem.name));
}

fn elem_children(elem: &XmlElem) -> impl Iterator<Item = &XmlElem> {
    elem.children.iter().filter_map(|n| match n {
        XmlNode::Element(e) => Some(e),
        _ => None,
    })
}
fn attr<'a>(elem: &'a XmlElem, name: &str) -> Option<&'a str> {
    elem.attributes
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// `XMLUtils.selectNode`: `"a>b>c[WHERE attr==value]"` walks direct-child
/// chains from `root` (`"this"` selects the root itself), then filters the
/// matched elements by the attribute equality or `*`.
fn select_paths(root: &XmlElem, selector: &str) -> Vec<Vec<usize>> {
    if selector == "this" {
        return vec![Vec::new()];
    }
    let (path, condition) = match selector.split_once("WHERE") {
        Some((path, cond)) => (path, cond.replace(' ', "")),
        None => (selector, "*".to_owned()),
    };
    let mut level: Vec<(&XmlElem, Vec<usize>)> = vec![(root, Vec::new())];
    for tag in path.split('>').map(|t| t.replace(' ', "")) {
        if tag.is_empty() {
            continue;
        }
        let mut next = Vec::new();
        for (elem, elem_path) in level {
            for (index, child) in elem.children.iter().enumerate() {
                if let XmlNode::Element(c) = child {
                    if c.name == tag {
                        let mut p = elem_path.clone();
                        p.push(index);
                        next.push((c, p));
                    }
                }
            }
        }
        level = next;
    }
    let (key, value) = condition.split_once("==").unwrap_or(("", ""));
    level
        .into_iter()
        .filter(|(elem, _)| {
            condition == "*" || condition.is_empty() || attr(elem, key) == Some(value)
        })
        .map(|(_, path)| path)
        .collect()
}

fn elem_at<'a>(mut elem: &'a XmlElem, path: &[usize]) -> Option<&'a XmlElem> {
    for &index in path {
        match elem.children.get(index) {
            Some(XmlNode::Element(child)) => elem = child,
            _ => return None,
        }
    }
    Some(elem)
}

fn elem_at_mut<'a>(mut elem: &'a mut XmlElem, path: &[usize]) -> Option<&'a mut XmlElem> {
    for &index in path {
        match elem.children.get_mut(index) {
            Some(XmlNode::Element(child)) => elem = child,
            _ => return None,
        }
    }
    Some(elem)
}

/// `XMLUtils.processReference`: clone the template node and apply the
/// reference's `<replace selector attributes values>` and `<append
/// selector>` directives to the clone.
fn process_reference(template: &XmlElem, reference: &XmlElem) -> XmlElem {
    let mut clone = template.clone();
    for directive in elem_children(reference) {
        let Some(selector) = attr(directive, "selector") else {
            continue;
        };
        match directive.name.as_str() {
            "replace" => {
                let keys: Vec<&str> = attr(directive, "attributes")
                    .unwrap_or_default()
                    .split(',')
                    .collect();
                let values: Vec<&str> = attr(directive, "values")
                    .unwrap_or_default()
                    .split(',')
                    .collect();
                for path in select_paths(&clone, selector) {
                    if let Some(target) = elem_at_mut(&mut clone, &path) {
                        for (key, value) in keys.iter().zip(values.iter()) {
                            if let Some(existing) =
                                target.attributes.iter_mut().find(|(k, _)| k == key)
                            {
                                existing.1 = (*value).to_owned();
                            } else {
                                target
                                    .attributes
                                    .push(((*key).to_owned(), (*value).to_owned()));
                            }
                        }
                    }
                }
            }
            "append" => {
                for path in select_paths(&clone, selector) {
                    if let Some(target) = elem_at_mut(&mut clone, &path) {
                        target.children.extend(directive.children.iter().cloned());
                    }
                }
            }
            _ => {}
        }
    }
    clone
}

/// `XMLUtils.resolveReferences`: replace every `<reference selector=…>`
/// with processed copies of the nodes its selector matches in the whole
/// document. With `children="false"` the processed node itself is appended
/// to the reference's parent; otherwise only its children are. References
/// are resolved in reverse document order, matching AS3's index loop.
pub fn resolve_references(xml: &str) -> Result<String, String> {
    let document = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
    let Some(root_node) = document.root_element().into() else {
        return Err("empty document".into());
    };
    let mut root = to_elem(root_node);
    // Paths of every <reference> in document order (child indices).
    let mut references: Vec<Vec<usize>> = Vec::new();
    fn collect(elem: &XmlElem, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (index, child) in elem.children.iter().enumerate() {
            let XmlNode::Element(c) = child else { continue };
            path.push(index);
            if c.name == "reference" {
                out.push(path.clone());
            }
            collect(c, path, out);
            path.pop();
        }
    }
    collect(&root, &mut Vec::new(), &mut references);
    for ref_path in references.into_iter().rev() {
        let (parent_path, index) = ref_path.split_at(ref_path.len() - 1);
        let index = index[0];
        let Some(parent) = elem_at_mut(&mut root, parent_path) else {
            continue;
        };
        let reference = if index < parent.children.len() {
            match parent.children.remove(index) {
                XmlNode::Element(reference) if reference.name == "reference" => reference,
                other => {
                    parent.children.insert(index, other);
                    continue;
                }
            }
        } else {
            continue;
        };
        let selector = attr(&reference, "selector").unwrap_or_default().to_owned();
        let children_only = attr(&reference, "children") != Some("false");
        for template_path in select_paths(&root, &selector) {
            let Some(template) = elem_at(&root, &template_path).cloned() else {
                continue;
            };
            let resolved = process_reference(&template, &reference);
            let Some(parent) = elem_at_mut(&mut root, parent_path) else {
                continue;
            };
            if children_only {
                parent.children.extend(resolved.children);
            } else {
                parent.children.push(XmlNode::Element(resolved));
            }
        }
    }
    let mut out = String::new();
    write_xml(&root, &mut out);
    Ok(out)
}

/// Typed manager commands. Native Ruffle executes these through the same
/// game model/request handlers used by its UI, without guessed hotkeys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum GameCommand {
    /// Backend-owned noninteractive text, in stage pixels. Never mutates game UI.
    SetOverlay {
        id: String,
        text: String,
        x: f64,
        y: f64,
        color: u32,
    },
    RemoveOverlay {
        id: String,
    },
    /// Resolve a currently observed GUI identity and dispatch its actual click.
    ClickGui {
        name: String,
    },
    /// Close through a currently observed window's own close handler.
    CloseGui {
        name: String,
    },
    /// Production amount, using the game's OreTypeModule numeric material ID.
    RefineOre {
        ore_type: i32,
        amount: u64,
    },
    SellOre {
        ore_type: i32,
        amount: u64,
    },
    /// Select only a difficulty offered by an observed gate selection UI.
    SelectGateDifficulty {
        gui: String,
        difficulty: i32,
    },
    JumpPortal {
        portal_id: i32,
    },
    SetPet {
        enabled: bool,
    },
    SelectPetGear {
        gear_type: i32,
        #[serde(default)]
        target: i32,
    },
    SelectConfiguration {
        configuration: u8,
    },
    UseItem {
        item_id: String,
        activate: bool,
    },
    /// Only the offered free/base repair is automated by the safety manager.
    Revive {
        repair_type: i32,
    },
    /// Select a repair option only after its matching location is observed on
    /// the current kill screen. A command cannot invent or purchase an offer.
    ReviveAt {
        repair_type: i32,
        location: crate::ReviveLocation,
    },
    /// Repair drones using the game's currently available repair request.
    RepairDrones,
    /// `GalaxyGateBuilderMultiEnergyRequest` — spin amount is 1, 5, 10, or 100.
    SpinGalaxyGate {
        gate_id: i32,
        spin_amount: u8,
        use_multiplier: bool,
    },
    /// `GalaxyGateBuilderSetupGateRequest`.
    PlaceGalaxyGate {
        gate_id: i32,
    },
    /// `GalaxyGateBuilderBuyLifeRequest`.
    BuyGalaxyLife {
        gate_id: i32,
    },
    /// `InvitationAcceptRequest`. `inviter_id` is the inviting player's user id.
    AcceptGroupInvite {
        inviter_id: i32,
    },
    InviteGroupMember {
        player_id: i32,
    },
    LeaveGroup,
    KickGroupMember {
        player_id: i32,
    },
    SetGroupInvites {
        open: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_expand_templates_with_replace_and_append() {
        // Mirror of XMLUtils.resolveReferences: the reference clones every
        // node matching `templates>template`, applies the replace/append
        // directives, and drops the placeholder.
        let xml = r#"<root>
            <templates>
                <template><map id="0" name="X" minLevel="1"/></template>
            </templates>
            <maps>
                <map id="1" name="1-1"/>
                <reference selector="templates>template">
                    <replace selector="map" attributes="id,name" values="7,4-2"/>
                    <append selector="map"><link to="9"/></append>
                </reference>
            </maps>
        </root>"#;
        let out = resolve_references(xml).unwrap();
        let doc = roxmltree::Document::parse(&out).unwrap();
        let maps: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("map"))
            .collect();
        // Existing map, template expansion (children of the template clone),
        // and the one inside <templates> itself.
        assert_eq!(maps.len(), 3);
        let expanded = maps
            .iter()
            .find(|n| n.attribute("id") == Some("7"))
            .expect("reference expansion should add map 7");
        assert_eq!(expanded.attribute("name"), Some("4-2"));
        assert!(expanded.children().any(|c| c.has_tag_name("link")));
    }

    #[test]
    fn reference_children_false_keeps_the_template_node() {
        let xml = r#"<root>
            <templates><template><jumpgate key="5"><leadsToMap>3</leadsToMap></jumpgate></template></templates>
            <gates><reference selector="templates>template" children="false"/></gates>
        </root>"#;
        let out = resolve_references(xml).unwrap();
        let doc = roxmltree::Document::parse(&out).unwrap();
        // children="false" appends the whole <template> clone — the jumpgate
        // still resolves because lookups descend the tree.
        let gates: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name("jumpgate"))
            .collect();
        assert_eq!(gates.len(), 2);
    }

    #[test]
    fn where_clause_filters_selected_templates() {
        let xml = r#"<root>
            <templates>
                <template kind="a"><map id="10" name="A"/></template>
                <template kind="b"><map id="20" name="B"/></template>
            </templates>
            <maps><reference selector="templates>template WHERE kind==b"/></maps>
        </root>"#;
        let out = resolve_references(xml).unwrap();
        let doc = roxmltree::Document::parse(&out).unwrap();
        // Only template "b" expanded: <maps> gains a second <map id="20">.
        let maps_node = doc.descendants().find(|n| n.has_tag_name("maps")).unwrap();
        let ids: Vec<_> = maps_node
            .descendants()
            .filter(|n| n.has_tag_name("map"))
            .filter_map(|n| n.attribute("id"))
            .collect();
        assert_eq!(ids, ["20"]);
    }

    #[test]
    fn config_xml_builds_the_star_system() {
        let maps = r#"<maps>
            <map id="1" name="1-1" minLevel="1"/>
            <map id="3" name="1-3" minLevel="5"/>
        </maps>"#;
        let space = r#"<spacemap>
            <templates>
                <template><jumpgate key="42" ref="1"><leadsToMap>3</leadsToMap></jumpgate></template>
            </templates>
            <gates><reference selector="templates>template"/></gates>
        </spacemap>"#;
        let system = StarSystem::from_config_xml(maps, space).unwrap();
        assert_eq!(system.map(3).map(|m| m.name.as_str()), Some("1-3"));
        assert_eq!(
            system.links,
            vec![MapLink {
                from: 1,
                to: 3,
                portal_id: 42
            }]
        );
    }
}
