//! Reads asset names and IDs from slice files (read-only). Unknown content still counts.
use anyhow::{ensure, Context, Result};
use miliastra_core::{
    binary::{self, Field},
    hash,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
// Engine GUID: 3 family bits, 7 type bits, 6 slice bits, 16 local-ID bits.
const GUID_FAMILY_SHIFT: u32 = 29;
const GUID_SLICE_FAMILY: u32 = 2;
const GUID_SLICE_SHIFT: u32 = 16;
const GUID_SLICE_MASK: u32 = 0x3f;
const GUID_PREFIX_MASK: u32 = 0xffc0_0000;
const GUID_LOCAL_MASK: u32 = 0xffff;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub key: String,
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub section: u8,
    pub can_generate_id: bool,
    #[serde(skip)]
    pub fingerprint: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub asset: Asset,
    pub status: String,
    pub before_name: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub section: String,
    pub assets: Vec<Asset>,
    pub changes: Vec<Change>,
    pub other_changes: bool,
    pub baseline_available: bool,
}
struct Index {
    assets: BTreeMap<String, Asset>,
    // Fingerprints are kept even when there's no readable name.
    records: BTreeMap<String, String>,
    other: Vec<u8>,
}
fn integer(f: &Field) -> Result<u32> {
    ensure!(f.wire == 0, "Expected asset ID");
    let mut n = 0u64;
    for (i, b) in f.value.iter().enumerate() {
        ensure!(i < 10, "Invalid integer");
        n |= ((b & 127) as u64) << (i * 7);
        if b & 128 == 0 {
            return Ok(u32::try_from(n)?);
        }
    }
    anyhow::bail!("Truncated integer")
}
fn child(bytes: &[u8], path: &[u64]) -> Result<Vec<u8>> {
    if path.is_empty() {
        return Ok(bytes.to_vec());
    }
    let fields = binary::fields(bytes)?;
    let found: Vec<_> = fields.iter().filter(|f| f.number == path[0]).collect();
    ensure!(found.len() == 1, "Missing or ambiguous asset field");
    if path.len() > 1 {
        ensure!(found[0].wire == 2, "Expected nested asset field");
    }
    if path.len() == 1 {
        return Ok(found[0].value.clone());
    }
    child(&found[0].value, &path[1..])
}
fn record_id(bytes: &[u8], path: &[u64]) -> Result<u32> {
    let parent = child(bytes, &path[..path.len() - 1])?;
    let fields = binary::fields(&parent)?;
    let f = fields
        .iter()
        .find(|f| f.number == path[path.len() - 1])
        .context("Missing asset ID")?;
    integer(f)
}
fn vi(mut n: u64, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push(n as u8 | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn field(number: u64, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    vi(number << 3 | 2, &mut out);
    vi(bytes.len() as u64, &mut out);
    out.extend(bytes);
    out
}
// Layouts checked against real editor saves. Strings anywhere else aren't asset names.
fn layout(n: u64) -> Option<(&'static str, &'static [u64], &'static [u64], &'static [u64])> {
    Some(match n {
        4 => ("Prefab", &[1], &[1], &[6, 11, 1]),
        5 => ("Entity", &[1], &[1], &[5, 11, 1]),
        7 => ("Terrain", &[1], &[1], &[2, 1]),
        10 => ("Node graph", &[1, 1], &[1, 5], &[2]),
        15 => ("Configuration", &[1], &[1], &[4, 11, 1]),
        18 => ("Camera", &[1], &[1], &[2, 1]),
        9 => ("Interface", &[502], &[501], &[505, 12, 501]),
        _ => return None,
    })
}
// Configuration record field 2 is its explicit type enum. These values were
// matched to editor-labelled assets in LOADTEST_C17_1_1 (7.1.0); GUID type bits
// are a separate namespace and do not always have the same numeric value.
const CONFIGURATION_KINDS: &[(u32, &str)] = &[
    (1, "Unit Status"),
    (4, "Class"),
    (5, "Growth Curve"),
    (6, "Character Skill"),
    (9, "Item"),
    (12, "Inventory Template"),
    (18, "Scan Tag Template"),
    (22, "Shield"),
    (26, "Environment"),
    (31, "Attribute Group"),
    (32, "VFX Tool"),
    (36, "Character Control Skill"),
];
const CREATION_GUID_TYPE: u32 = 2;
const PREFAB_PROJECTILE_SUBTYPE: u32 = 1;
const PREFAB_LOOT_SUBTYPE: u32 = 2;

fn asset_kind<'a>(container_kind: &'a str, record: &[u8], id: u32) -> &'a str {
    match container_kind {
        "Configuration" => scalar_field(record, 2)
            .and_then(|kind| CONFIGURATION_KINDS.iter().find(|(value, _)| *value == kind))
            .map_or(container_kind, |(_, name)| *name),
        // Prefab field 9 distinguishes projectile/loot definitions. Creations
        // have GUID type 2; some creation records omit field 9 entirely.
        "Prefab" => match scalar_field(record, 9) {
            Some(PREFAB_PROJECTILE_SUBTYPE) => "Projectile",
            Some(PREFAB_LOOT_SUBTYPE) => "Loot",
            _ if (id >> GUID_TYPE_SHIFT) & GUID_TYPE_MASK == CREATION_GUID_TYPE => "Creation",
            _ => container_kind,
        },
        _ => container_kind,
    }
}

/// Whether the ID belongs to the GUID family that carries a slice number. Doesn't say
/// whether Miliastra lets you edit that asset's ID.
fn has_slice_id(id: u32) -> bool {
    id >> GUID_FAMILY_SHIFT == GUID_SLICE_FAMILY
}
/// Finds the Name component (type 1) among the repeated components.
fn record_name(bytes: &[u8], path: &[u64]) -> Result<Vec<u8>> {
    if let [components, 11, 1] = path {
        const NAME_COMPONENT_TYPE: u32 = 1;
        let mut selected = None;
        for component in binary::fields(bytes)? {
            if component.number != *components || component.wire != 2 {
                continue;
            }
            let fields = binary::fields(&component.value)?;
            let types: Vec<_> = fields.iter().filter(|f| f.number == 1).collect();
            if types.len() != 1 || integer(types[0]).ok() != Some(NAME_COMPONENT_TYPE) {
                continue;
            }
            ensure!(selected.is_none(), "Ambiguous name component");
            selected = Some(component.value);
        }
        child(&selected.context("Missing name component")?, &[11, 1])
    } else {
        child(bytes, path)
    }
}
fn entries(
    bytes: &[u8],
    path: &[u64],
    kind: &str,
    id_path: &[u64],
    name_path: &[u64],
    assets: &mut BTreeMap<String, Asset>,
    records: &mut BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let mut other = vec![];
    for f in binary::fields(bytes)? {
        if f.number != path[0] || f.wire != 2 {
            other.extend(f.raw);
            continue;
        }
        if path.len() > 1 {
            let rest = entries(
                &f.value,
                &path[1..],
                kind,
                id_path,
                name_path,
                assets,
                records,
            )?;
            if !rest.is_empty() {
                other.extend(field(f.number, &rest));
            }
            continue;
        }
        let id = match record_id(&f.value, id_path) {
            Ok(id) if id >= 1 << 30 && id < 1 << 31 => id,
            _ => {
                other.extend(f.raw);
                continue;
            }
        };
        // Identity follows the physical record namespace, independently of its
        // display category. Different namespaces can reuse numeric GUIDs.
        let key = format!("{kind}:{id}");
        let kind = asset_kind(kind, &f.value, id);
        let fingerprint = hash(&f.value);
        ensure!(
            records.insert(key.clone(), fingerprint.clone()).is_none(),
            "Duplicate asset identity"
        );
        let parsed = (|| -> Result<Asset> {
            let name = String::from_utf8(record_name(&f.value, name_path)?)?;
            ensure!(
                !name.trim().is_empty()
                    && name.len() <= 1024
                    && !name.chars().any(char::is_control),
                "Invalid asset name"
            );
            Ok(Asset {
                key,
                id,
                name,
                kind: kind.into(),
                section: ((id >> GUID_SLICE_SHIFT) & GUID_SLICE_MASK) as u8,
                can_generate_id: has_slice_id(id),
                fingerprint,
            })
        })();
        match parsed {
            Ok(asset) => {
                ensure!(!assets.contains_key(&asset.key), "Duplicate asset identity");
                assets.insert(asset.key.clone(), asset);
            }
            Err(_) => other.extend(f.raw), // An unsupported schema never becomes an invented name.
        }
    }
    Ok(other)
}
// These model/type pairs and definition/instance references are from editor saves.
// Names are deliberately not used: templates can be renamed or localized.
const PLAYER_TEMPLATE_MODEL: u32 = 1_000_000;
const PLAYER_CHARACTER_MODEL: u32 = 1_000_001;
const GUID_TYPE_SHIFT: u32 = 22;
const GUID_TYPE_MASK: u32 = 0x7f;
const PLAYER_TEMPLATE_GUID_TYPE: u32 = 3;
const PLAYER_CHARACTER_GUID_TYPE: u32 = 4;
const PLAYER_TEMPLATE_KIND: &str = "Player Template";

fn scalar_field(bytes: &[u8], number: u64) -> Option<u32> {
    let fields = binary::fields(bytes).ok()?;
    let mut found = fields.iter().filter(|f| f.number == number);
    let value = found.next()?;
    if found.next().is_some() {
        return None;
    }
    integer(value).ok()
}
fn template_role(model: u32, id: u32) -> Option<bool> {
    if !has_slice_id(id) {
        return None;
    }
    match (model, (id >> GUID_TYPE_SHIFT) & GUID_TYPE_MASK) {
        (PLAYER_TEMPLATE_MODEL, PLAYER_TEMPLATE_GUID_TYPE) => Some(false),
        (PLAYER_CHARACTER_MODEL, PLAYER_CHARACTER_GUID_TYPE) => Some(true),
        _ => None,
    }
}
fn template_for_character(id: u32) -> u32 {
    (id & !(GUID_TYPE_MASK << GUID_TYPE_SHIFT)) | (PLAYER_TEMPLATE_GUID_TYPE << GUID_TYPE_SHIFT)
}
/// One editor-visible player template has a definition, its character-edit
/// definition, and saved instances. Fold all of their fingerprints into the
/// primary asset so hiding those implementation records never hides changes.
fn group_player_templates(body: &[u8], index: &mut Index) -> Result<()> {
    let mut definitions = BTreeMap::<u32, BTreeSet<String>>::new();
    let mut characters = BTreeSet::new();
    let mut instances = vec![];
    for container in binary::fields(body)? {
        if container.wire != 2 || !matches!(container.number, 4 | 5) {
            continue;
        }
        for record in binary::fields(&container.value)? {
            if record.number != 1 || record.wire != 2 {
                continue;
            }
            let Some(id) = scalar_field(&record.value, 1) else {
                continue;
            };
            if container.number == 4 {
                let Some(character) =
                    scalar_field(&record.value, 2).and_then(|model| template_role(model, id))
                else {
                    continue;
                };
                if character {
                    characters.insert(id);
                } else {
                    definitions
                        .entry(id)
                        .or_default()
                        .insert(format!("Prefab:{id}"));
                }
            } else {
                let Some(character) =
                    scalar_field(&record.value, 8).and_then(|model| template_role(model, id))
                else {
                    continue;
                };
                if let Ok(reference) = record_id(&record.value, &[2, 1]) {
                    // A known template model alone is insufficient to collapse an
                    // entity: require a reference to the matching definition.
                    instances.push((id, reference, character));
                }
            }
        }
    }
    for id in &characters {
        // The character-edit definition has the same slice/local GUID identity.
        // A template's selected character reference can point to a different
        // template, so it must not be used as ownership of this companion.
        if let Some(parts) = definitions.get_mut(&template_for_character(*id)) {
            parts.insert(format!("Prefab:{id}"));
        }
    }
    for (id, reference, character) in instances {
        let owner = if character && characters.contains(&reference) {
            template_for_character(reference)
        } else if !character {
            reference
        } else {
            continue;
        };
        if let Some(parts) = definitions.get_mut(&owner) {
            parts.insert(format!("Entity:{id}"));
        }
    }
    for (id, parts) in definitions {
        let key = format!("{PLAYER_TEMPLATE_KIND}:{id}");
        let mut primary = index.assets.remove(&format!("Prefab:{id}"));
        let mut fingerprints = BTreeMap::new();
        for part in parts {
            index.assets.remove(&part);
            if let Some(fingerprint) = index.records.remove(&part) {
                fingerprints.insert(part, fingerprint);
            }
        }
        let fingerprint = hash(&serde_json::to_vec(&fingerprints)?);
        index.records.insert(key.clone(), fingerprint.clone());
        if let Some(asset) = &mut primary {
            asset.key = key.clone();
            asset.kind = PLAYER_TEMPLATE_KIND.into();
            asset.fingerprint = fingerprint;
        }
        if let Some(asset) = primary {
            index.assets.insert(key, asset);
        }
    }
    Ok(())
}

fn index(bytes: &[u8], section: &str) -> Result<Index> {
    let level = binary::resource_level(bytes, section)?;
    binary::content_hash(bytes, section, &level)?;
    let root = binary::fields(binary::payload(
        bytes,
        if section == "shared" { 7 } else { 6 },
    )?)?;
    let body = if section == "shared" {
        binary::payload(bytes, 7)?.to_vec()
    } else {
        child(binary::payload(bytes, 6)?, &[2])?
    };
    let mut assets = BTreeMap::new();
    let mut records = BTreeMap::new();
    let mut other = vec![];
    for f in binary::fields(&body)? {
        let compared = binary::comparison_field(&f, section)?;
        if compared.is_empty() {
            continue;
        }
        if section != "shared" {
            if let Some((kind, path, id_path, name_path)) = layout(f.number) {
                match entries(
                    &f.value,
                    path,
                    kind,
                    id_path,
                    name_path,
                    &mut assets,
                    &mut records,
                ) {
                    Ok(rest) => {
                        if !rest.is_empty() {
                            other.extend(field(f.number, &rest));
                        }
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        other.extend(compared);
    }
    if section != "shared" {
        for f in root {
            if f.number != 2 {
                other.extend(f.raw);
            }
        }
    }
    let mut index = Index {
        assets,
        records,
        other,
    };
    if section != "shared" {
        group_player_templates(&body, &mut index)?;
    }
    Ok(index)
}
pub fn inspect(current: &[u8], baseline: Option<&[u8]>, section: &str) -> Result<Inspection> {
    let now = index(current, section)?;
    let before = baseline.map(|b| index(b, section)).transpose()?;
    let mut changes = vec![];
    if let Some(old) = &before {
        for (key, asset) in &now.assets {
            let prev = old.assets.get(key);
            let previous_record = old.records.get(key);
            if previous_record.is_none_or(|fingerprint| fingerprint != &asset.fingerprint) {
                changes.push(Change {
                    asset: asset.clone(),
                    status: if previous_record.is_some() {
                        "Modified"
                    } else {
                        "Added"
                    }
                    .into(),
                    before_name: prev
                        .filter(|p| p.name != asset.name)
                        .map(|p| p.name.clone()),
                });
            }
        }
        for (key, asset) in &old.assets {
            if !now.records.contains_key(key) {
                changes.push(Change {
                    asset: asset.clone(),
                    status: "Deleted".into(),
                    before_name: None,
                });
            }
        }
    }
    let other_changes = before.as_ref().is_none_or(|old| old.other != now.other);
    Ok(Inspection {
        section: section.into(),
        assets: now.assets.into_values().collect(),
        changes,
        other_changes,
        baseline_available: before.is_some(),
    })
}
/// Every ID-sized number in the file, to avoid collisions when picking a new ID. Extra
/// matches only rule out candidates.
pub fn used_ids(bytes: &[u8], section: &str) -> Result<BTreeSet<u32>> {
    fn walk(bytes: &[u8], depth: u8, budget: &mut usize, out: &mut BTreeSet<u32>) -> Result<()> {
        ensure!(
            depth <= 32,
            "Asset inspection exceeds the supported nesting depth"
        );
        let Ok(fields) = binary::fields(bytes) else {
            return Ok(());
        };
        for f in fields {
            ensure!(
                *budget > 0,
                "Asset inspection exceeds the supported complexity"
            );
            *budget -= 1;
            if f.wire == 0 {
                if let Ok(id) = integer(&f) {
                    if id >= 1 << 30 && id < 1 << 31 {
                        out.insert(id);
                    }
                }
            } else if f.wire == 2 {
                walk(&f.value, depth + 1, budget, out)?;
            }
        }
        Ok(())
    }
    let mut ids = BTreeSet::new();
    walk(
        binary::payload(bytes, if section == "shared" { 7 } else { 6 })?,
        0,
        &mut 1_000_000,
        &mut ids,
    )?;
    Ok(ids)
}
pub fn candidate(asset: &Asset, target: u8, used: &BTreeSet<u32>) -> Result<u32> {
    ensure!(
        has_slice_id(asset.id),
        "This ID does not use a recognized slice layout"
    );
    ensure!(
        target < miliastra_core::constants::SLICE_COUNT && target != asset.section,
        "Choose another slice"
    );
    let prefix = (asset.id & GUID_PREFIX_MASK) | ((target as u32) << GUID_SLICE_SHIFT);
    let preferred = asset.id & GUID_LOCAL_MASK;
    if preferred > 0 && !used.contains(&(prefix | preferred)) {
        return Ok(prefix | preferred);
    }
    (1..=GUID_LOCAL_MASK)
        .map(|i| prefix | i)
        .find(|id| !used.contains(id))
        .context("No unused IDs remain in that slice for this asset type")
}
#[cfg(test)]
mod tests {
    use super::*;
    fn scalar(n: u64, v: u64) -> Vec<u8> {
        let mut o = vec![];
        vi(n << 3, &mut o);
        vi(v, &mut o);
        o
    }
    fn entity(id: u32, name: &str, value: u64) -> Vec<u8> {
        [
            scalar(1, id as u64),
            field(
                5,
                &[scalar(1, 1), field(11, &field(1, name.as_bytes()))].concat(),
            ),
            scalar(98, value),
        ]
        .concat()
    }
    fn slice(records: Vec<Vec<u8>>, extra: u64, time: u64) -> Vec<u8> {
        let body = [
            scalar(1, 501),
            field(
                5,
                &records
                    .into_iter()
                    .flat_map(|r| field(1, &r))
                    .collect::<Vec<_>>(),
            ),
            scalar(99, extra),
            scalar(40, time),
        ]
        .concat();
        let data = [scalar(1, 6), field(2, &body)].concat();
        let mut out = vec![];
        for n in [data.len() as u32 + 20, 1, 806, 6, data.len() as u32] {
            out.extend(n.to_be_bytes());
        }
        out.extend(data);
        out.extend(1657u32.to_be_bytes());
        out
    }
    #[test]
    fn shared_inspection_uses_the_same_metadata_projection_as_dirty_detection() {
        let shared = |churn: u64, faction: &str| {
            let body = [
                scalar(1, 501),
                scalar(41, churn),
                field(22, &field(1, format!("Editor column {churn}").as_bytes())),
                field(11, &field(2, &field(1, faction.as_bytes()))),
            ]
            .concat();
            let mut out = vec![];
            for n in [body.len() as u32 + 20, 1, 806, 7, body.len() as u32] {
                out.extend(n.to_be_bytes());
            }
            out.extend(body);
            out.extend(1657u32.to_be_bytes());
            out
        };
        let original = shared(1, "Faction");
        assert!(
            !inspect(&shared(2, "Faction"), Some(&original), "shared")
                .unwrap()
                .other_changes
        );
        assert!(
            inspect(&shared(2, "Renamed faction"), Some(&original), "shared")
                .unwrap()
                .other_changes
        );
    }
    #[test]
    fn names_are_record_scoped_and_changes_include_add_modify_delete_and_rename() {
        let a = 1078329345;
        let b = a + 1;
        let c = a + 2;
        let before = slice(
            vec![entity(a, "Platform", 1), entity(b, "Deleted", 1)],
            0,
            1,
        );
        let after = slice(
            vec![entity(a, "Renamed platform", 2), entity(c, "New boat", 1)],
            0,
            2,
        );
        let d = inspect(&after, Some(&before), "6").unwrap();
        assert_eq!(d.changes.len(), 3);
        assert!(!d.other_changes);
        let renamed = d.changes.iter().find(|c| c.asset.id == a).unwrap();
        assert_eq!(renamed.status, "Modified");
        assert_eq!(renamed.before_name.as_deref(), Some("Platform"));
        assert!(d.changes.iter().any(|c| c.status == "Deleted"));
        assert_eq!(d.assets.len(), 2);
    }
    #[test]
    fn unknown_changes_stay_visible_and_absent_baselines_never_invent_changes() {
        let a = slice(vec![entity(1078329345, "Platform", 1)], 0, 1);
        let b = slice(vec![entity(1078329345, "Platform", 1)], 1, 3);
        let d = inspect(&b, Some(&a), "6").unwrap();
        assert!(d.other_changes);
        assert!(d.changes.is_empty());
        let d = inspect(&b, None, "6").unwrap();
        assert!(!d.baseline_available);
        assert!(d.changes.is_empty());
        assert!(inspect(&b[..20], None, "6").is_err());
    }
    #[test]
    fn unreadable_names_do_not_turn_existing_assets_into_deleted_or_added_assets() {
        let id = 1078329345;
        let named = slice(vec![entity(id, "Boat deck", 1)], 0, 1);
        for unreadable in [
            scalar(1, id as u64),
            [
                scalar(1, id as u64),
                field(5, &[scalar(1, 1), field(11, &field(1, &[255]))].concat()),
            ]
            .concat(),
        ] {
            let unnamed = slice(vec![unreadable], 0, 2);
            let lost_name = inspect(&unnamed, Some(&named), "6").unwrap();
            assert!(
                lost_name.changes.is_empty(),
                "An existing ID must not be reported as deleted when its name cannot be read"
            );
            assert!(lost_name.other_changes);
            assert!(lost_name.assets.is_empty());
            let recovered_name = inspect(&named, Some(&unnamed), "6").unwrap();
            assert_eq!(recovered_name.changes.len(), 1);
            assert_eq!(
                recovered_name.changes[0].status, "Modified",
                "Recovering a readable name must not invent a new asset"
            );
        }
        let removed = inspect(&slice(vec![], 0, 3), Some(&named), "6").unwrap();
        assert_eq!(removed.changes.len(), 1);
        assert_eq!(removed.changes[0].status, "Deleted");
    }
    #[test]
    fn supported_namespaces_are_separate_and_reference_strings_are_not_names() {
        let id = 1090519041u32;
        let config = field(
            15,
            &field(
                1,
                &[
                    scalar(1, id as u64),
                    scalar(2, 4), // Class, independent of the same ID in Entity records.
                    field(
                        4,
                        &[scalar(1, 1), field(11, &field(1, b"Custom Class"))].concat(),
                    ),
                ]
                .concat(),
            ),
        );
        let entity = field(5, &field(1, &entity(id, "Entity using the same number", 1)));
        let graph = field(
            10,
            &field(
                1,
                &field(
                    1,
                    &[field(1, &scalar(5, 1073741825)), field(2, b"Graph Name")].concat(),
                ),
            ),
        );
        let body = [
            scalar(1, 501),
            config,
            entity,
            graph,
            field(91, b"Unrelated text must not be an asset"),
        ]
        .concat();
        let data = [scalar(1, 6), field(2, &body)].concat();
        let mut bytes = vec![];
        for n in [data.len() as u32 + 20, 1, 806, 6, data.len() as u32] {
            bytes.extend(n.to_be_bytes());
        }
        bytes.extend(data);
        bytes.extend(1657u32.to_be_bytes());
        let d = inspect(&bytes, None, "6").unwrap();
        assert_eq!(d.assets.len(), 3);
        assert!(d
            .assets
            .iter()
            .any(|asset| asset.id == id && asset.kind == "Class"));
        assert!(d
            .assets
            .iter()
            .any(|a| a.name == "Custom Class" && a.can_generate_id));
        assert!(d
            .assets
            .iter()
            .any(|a| a.name == "Graph Name" && a.can_generate_id));
        assert!(!d.assets.iter().any(|a| a.name.starts_with("Unrelated")));
    }
    fn framed_slice(body: &[u8]) -> Vec<u8> {
        framed_slice_for(11, body)
    }
    fn framed_slice_for(section: u8, body: &[u8]) -> Vec<u8> {
        let data = [
            scalar(1, section as u64),
            field(2, &[scalar(1, 501), body.to_vec()].concat()),
        ]
        .concat();
        let mut bytes = vec![];
        for n in [data.len() as u32 + 20, 1, 806, 6, data.len() as u32] {
            bytes.extend(n.to_be_bytes());
        }
        bytes.extend(data);
        bytes.extend(1657u32.to_be_bytes());
        bytes
    }
    fn component(list: u64, kind: u64, name: &str) -> Vec<u8> {
        field(
            list,
            &[scalar(1, kind), field(11, &field(1, name.as_bytes()))].concat(),
        )
    }
    #[test]
    fn repeated_components_reveal_prefabs_world_objects_and_configurations() {
        let mut body = vec![];
        for (record_kind, list, name) in [
            (4, 6, "Water prefab"),
            (5, 5, "World platform"),
            (15, 4, "Swim configuration"),
        ] {
            body.extend(field(
                record_kind,
                &field(
                    1,
                    &[
                        scalar(1, 1078657038),
                        component(list, 13, "Not the asset name"),
                        component(list, 1, name),
                        component(list, 14, "Another property"),
                    ]
                    .concat(),
                ),
            ));
        }
        let saved = framed_slice(&body);
        let inspected = inspect(&saved, None, "11").unwrap();
        assert_eq!(inspected.assets.len(), 3);
        for name in ["Water prefab", "World platform", "Swim configuration"] {
            assert!(inspected
                .assets
                .iter()
                .any(|a| a.name == name && a.can_generate_id));
        }
        // Unknown component edits still change the whole asset fingerprint.
        let changed = framed_slice(&field(
            5,
            &field(
                1,
                &[
                    scalar(1, 1078657038),
                    component(5, 1, "World platform"),
                    component(5, 14, "Edited property"),
                ]
                .concat(),
            ),
        ));
        let changes = inspect(&changed, Some(&saved), "11").unwrap();
        assert!(changes
            .changes
            .iter()
            .any(|c| c.asset.name == "World platform" && c.status == "Modified"));
    }
    #[test]
    fn missing_ambiguous_or_unreadable_name_components_never_invent_names() {
        for components in [
            component(5, 13, "Unrelated name"),
            [component(5, 1, "First"), component(5, 1, "Second")].concat(),
            field(5, &[scalar(1, 1), field(11, &field(1, &[255]))].concat()),
        ] {
            let saved = framed_slice(&field(
                5,
                &field(1, &[scalar(1, 1078657038), components].concat()),
            ));
            let inspected = inspect(&saved, None, "11").unwrap();
            assert!(inspected.assets.is_empty());
            assert!(inspected.other_changes);
        }
    }
    fn configuration(id: u32, kind: Option<u32>, name: &str, edit: u64) -> Vec<u8> {
        field(
            15,
            &field(
                1,
                &[
                    scalar(1, id as u64),
                    kind.map(|kind| scalar(2, kind as u64)).unwrap_or_default(),
                    component(4, 1, name),
                    scalar(99, edit),
                ]
                .concat(),
            ),
        )
    }
    #[test]
    fn configuration_categories_use_the_saved_enum_not_names_or_guid_type_bits() {
        for (kind, expected) in CONFIGURATION_KINDS {
            let saved = framed_slice(&configuration(1077936129, Some(*kind), "Renamed asset", 1));
            let result = inspect(&saved, None, "11").unwrap();
            assert_eq!(result.assets.len(), 1);
            assert_eq!(result.assets[0].kind, *expected);
            assert_eq!(result.assets[0].key, "Configuration:1077936129");
            assert_eq!(result.assets[0].name, "Renamed asset");
            assert!(result.assets[0].can_generate_id);
        }
        for kind in [None, Some(999)] {
            let saved = framed_slice(&configuration(1077936129, kind, "Custom Skill", 1));
            assert_eq!(
                inspect(&saved, None, "11").unwrap().assets[0].kind,
                "Configuration"
            );
        }
    }
    #[test]
    fn categorized_configuration_edits_renames_and_unknown_data_keep_their_identity() {
        let before = framed_slice(&configuration(1077936129, Some(22), "Original shield", 1));
        let changed = framed_slice(&configuration(1077936129, Some(22), "Renamed shield", 2));
        let result = inspect(&changed, Some(&before), "11").unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].status, "Modified");
        assert_eq!(result.changes[0].asset.kind, "Shield");
        assert_eq!(
            result.changes[0].before_name.as_deref(),
            Some("Original shield")
        );
        assert!(!result.other_changes);
        let retyped = framed_slice(&configuration(1077936129, Some(36), "Control skill", 1));
        let result = inspect(&retyped, Some(&before), "11").unwrap();
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].status, "Modified");
        assert_eq!(result.changes[0].asset.kind, "Character Control Skill");
    }
    fn specialized_prefab(id: u32, subtype: Option<u32>, name: &str) -> Vec<u8> {
        field(
            4,
            &field(
                1,
                &[
                    scalar(1, id as u64),
                    component(6, 1, name),
                    subtype
                        .map(|kind| scalar(9, kind as u64))
                        .unwrap_or_default(),
                ]
                .concat(),
            ),
        )
    }
    #[test]
    fn creations_projectiles_and_loot_use_prefab_types_without_guessing_from_names() {
        for (id, subtype, name, expected) in [
            (1082261506, None, "Renamed creation", "Creation"),
            (1082261507, Some(4), "Renamed slime", "Creation"),
            (1078067201, Some(1), "Renamed projectile", "Projectile"),
            (1078067208, Some(2), "Renamed loot", "Loot"),
            (1078067209, Some(3), "Prefab group", "Prefab"),
            (1078067210, None, "Large Hydro Slime", "Prefab"),
            (1078067211, Some(99), "Challenge Coin Loot", "Prefab"),
        ] {
            let result = inspect(
                &framed_slice(&specialized_prefab(id, subtype, name)),
                None,
                "11",
            )
            .unwrap();
            assert_eq!(result.assets.len(), 1);
            assert_eq!(result.assets[0].kind, expected);
            assert_eq!(result.assets[0].key, format!("Prefab:{id}"));
        }
        // GUID type 2 in a configuration is not a creation: enums are namespaced.
        let config = framed_slice(&configuration(1082261506, Some(22), "Shield", 1));
        assert_eq!(
            inspect(&config, None, "11").unwrap().assets[0].kind,
            "Shield"
        );
    }
    #[test]
    fn display_categories_do_not_allow_duplicate_identities_in_a_record_namespace() {
        let saved = framed_slice(
            &[
                configuration(1077936129, Some(6), "Skill", 1),
                configuration(1077936129, Some(9), "Item", 1),
            ]
            .concat(),
        );
        assert!(inspect(&saved, None, "11").is_err());
        let saved = framed_slice(
            &[
                specialized_prefab(1078067201, Some(1), "Projectile"),
                specialized_prefab(1078067201, Some(2), "Loot"),
            ]
            .concat(),
        );
        assert!(inspect(&saved, None, "11").is_err());
    }
    fn player_definition(
        id: u32,
        model: u32,
        name: &str,
        selected_character: u32,
        edit: u64,
    ) -> Vec<u8> {
        field(
            4,
            &field(
                1,
                &[
                    scalar(1, id as u64),
                    scalar(2, model as u64),
                    component(6, 1, name),
                    field(
                        6,
                        &[
                            scalar(1, 3),
                            field(12, &scalar(5, selected_character as u64)),
                        ]
                        .concat(),
                    ),
                    scalar(99, edit),
                ]
                .concat(),
            ),
        )
    }
    fn player_instance(id: u32, definition: u32, model: u32, name: &str, edit: u64) -> Vec<u8> {
        field(
            5,
            &field(
                1,
                &[
                    scalar(1, id as u64),
                    field(2, &scalar(1, definition as u64)),
                    component(5, 1, name),
                    scalar(8, model as u64),
                    scalar(99, edit),
                ]
                .concat(),
            ),
        )
    }
    fn player_records(
        primary: u32,
        character: u32,
        character_edit: u64,
        instance_edit: u64,
    ) -> Vec<u8> {
        [
            player_definition(
                primary,
                PLAYER_TEMPLATE_MODEL,
                "Renamed player",
                1090519041,
                1,
            ),
            player_definition(
                character,
                PLAYER_CHARACTER_MODEL,
                "Localized character name",
                0,
                character_edit,
            ),
            player_instance(
                primary,
                primary,
                PLAYER_TEMPLATE_MODEL,
                "Renamed player",
                instance_edit,
            ),
            player_instance(
                character,
                character,
                PLAYER_CHARACTER_MODEL,
                "Localized character name",
                1,
            ),
        ]
        .concat()
    }
    #[test]
    fn player_templates_are_one_logical_asset_not_prefabs_or_world_instances() {
        let primary = 1086455809;
        let character = 1090650113;
        let saved = framed_slice_for(2, &player_records(primary, character, 1, 1));
        let result = inspect(&saved, None, "2").unwrap();
        assert_eq!(result.assets.len(), 1);
        let asset = &result.assets[0];
        assert_eq!(
            (asset.id, asset.name.as_str(), asset.kind.as_str()),
            (primary, "Renamed player", PLAYER_TEMPLATE_KIND)
        );
        assert_eq!(asset.key, format!("Player Template:{primary}"));
        assert!(asset.can_generate_id);
        let ids = used_ids(&saved, "2").unwrap();
        assert!(ids.contains(&primary) && ids.contains(&character));
        let allocated = candidate(asset, 4, &ids).unwrap();
        assert_eq!(allocated & GUID_PREFIX_MASK, primary & GUID_PREFIX_MASK);
        assert_eq!((allocated >> GUID_SLICE_SHIFT) & GUID_SLICE_MASK, 4);
    }
    #[test]
    fn hidden_character_instance_and_instance_count_changes_still_modify_the_template() {
        let primary = 1086455809;
        let character = 1090650113;
        let before = framed_slice_for(2, &player_records(primary, character, 1, 1));
        for records in [
            player_records(primary, character, 2, 1),
            player_records(primary, character, 1, 2),
            [
                player_records(primary, character, 1, 1),
                player_instance(
                    primary + 1,
                    primary,
                    PLAYER_TEMPLATE_MODEL,
                    "Another saved instance",
                    1,
                ),
            ]
            .concat(),
            player_definition(
                primary,
                PLAYER_TEMPLATE_MODEL,
                "Renamed player",
                1090519041,
                1,
            ),
        ] {
            let change = inspect(&framed_slice_for(2, &records), Some(&before), "2").unwrap();
            assert_eq!(change.changes.len(), 1);
            assert_eq!(change.changes[0].asset.id, primary);
            assert_eq!(change.changes[0].status, "Modified");
            assert!(!change.other_changes);
        }
        let removed = inspect(&framed_slice_for(2, &[]), Some(&before), "2").unwrap();
        assert_eq!(removed.changes.len(), 1);
        assert_eq!(removed.changes[0].status, "Deleted");
    }
    #[test]
    fn repeated_default_instances_collapse_by_reference_not_by_name() {
        let primary = 1086324737;
        let character = 1090519041;
        let mut records = player_records(primary, character, 1, 1);
        for offset in 1..9 {
            records.extend(player_instance(
                primary + offset,
                primary,
                PLAYER_TEMPLATE_MODEL,
                "Default Template",
                offset as u64,
            ));
        }
        records.extend(field(
            5,
            &field(1, &entity(1077936129, "Default Template", 1)),
        ));
        records.extend(field(
            5,
            &field(1, &entity(1077936130, "Default Template", 1)),
        ));
        let saved = framed_slice_for(0, &records);
        let result = inspect(&saved, Some(&saved), "0").unwrap();
        assert_eq!(
            result
                .assets
                .iter()
                .filter(|asset| asset.kind == PLAYER_TEMPLATE_KIND)
                .count(),
            1
        );
        assert_eq!(
            result
                .assets
                .iter()
                .filter(|asset| asset.kind == "Entity")
                .count(),
            2
        );
        assert!(result.changes.is_empty() && !result.other_changes);
        for offset in 0..9 {
            assert!(used_ids(&saved, "0").unwrap().contains(&(primary + offset)));
        }
    }
    #[test]
    fn character_ownership_uses_its_identity_not_the_selected_character_reference() {
        let default = 1086324737;
        let default_character = 1090519041;
        let custom = 1086455809;
        let custom_character = 1090650113;
        let before = framed_slice_for(
            2,
            &[
                player_records(default, default_character, 1, 1),
                player_records(custom, custom_character, 1, 1),
            ]
            .concat(),
        );
        let after = framed_slice_for(
            2,
            &[
                player_records(default, default_character, 1, 1),
                player_records(custom, custom_character, 2, 1),
            ]
            .concat(),
        );
        let result = inspect(&after, Some(&before), "2").unwrap();
        assert_eq!(result.assets.len(), 2);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].asset.id, custom);
    }
    #[test]
    fn unreadable_primary_names_preserve_template_identity_and_recovery_is_not_addition() {
        let primary = 1086455809;
        let character = 1090650113;
        let named = framed_slice_for(2, &player_records(primary, character, 1, 1));
        let unreadable = framed_slice_for(
            2,
            &[
                player_definition(primary, PLAYER_TEMPLATE_MODEL, "", 1090519041, 1),
                player_definition(
                    character,
                    PLAYER_CHARACTER_MODEL,
                    "Still-readable internal name",
                    0,
                    1,
                ),
            ]
            .concat(),
        );
        let result = inspect(&unreadable, Some(&named), "2").unwrap();
        assert!(result.assets.is_empty() && result.changes.is_empty() && result.other_changes);
        let recovered = inspect(&named, Some(&unreadable), "2").unwrap();
        assert_eq!(recovered.changes.len(), 1);
        assert_eq!(recovered.changes[0].status, "Modified");
    }
    #[test]
    fn template_like_names_or_model_numbers_without_matching_types_never_merge_assets() {
        let records = [
            player_definition(1077936129, PLAYER_TEMPLATE_MODEL, "Default Template", 0, 1),
            player_definition(1086324737, 20001221, "Custom Player Template", 0, 1),
            player_instance(
                1077936130,
                1086324737,
                PLAYER_TEMPLATE_MODEL,
                "Default Template",
                1,
            ),
        ]
        .concat();
        let result = inspect(&framed_slice_for(0, &records), None, "0").unwrap();
        assert_eq!(result.assets.len(), 3);
        assert!(!result
            .assets
            .iter()
            .any(|asset| asset.kind == PLAYER_TEMPLATE_KIND));
    }
    #[test]
    fn id_generation_is_independent_of_asset_kind_and_preserves_every_prefix_bit() {
        for (kind, id_type) in [
            ("Prefab", 1),
            ("Entity", 2),
            ("Camera", 0),
            ("Node graph", 0),
            ("Configuration", 127),
            ("Interface", 0),
            ("Terrain", 0),
        ] {
            let id = (GUID_SLICE_FAMILY << GUID_FAMILY_SHIFT) | (id_type << 22) | (11 << 16) | 7;
            let asset = Asset {
                key: format!("{kind}:{id}"),
                id,
                name: "Saved asset".into(),
                kind: kind.into(),
                section: 11,
                can_generate_id: has_slice_id(id),
                fingerprint: String::new(),
            };
            assert!(asset.can_generate_id);
            let occupied = (id & GUID_PREFIX_MASK) | (5 << 16) | 7;
            let new_id = candidate(&asset, 5, &BTreeSet::from([occupied])).unwrap();
            assert_ne!(new_id, occupied);
            assert_eq!(new_id & GUID_PREFIX_MASK, id & GUID_PREFIX_MASK);
            assert_eq!((new_id >> GUID_SLICE_SHIFT) & GUID_SLICE_MASK, 5);
        }
        assert!(!has_slice_id(0x60000001));
        assert!(!has_slice_id(1));
    }
    #[test]
    fn allocation_preserves_type_and_avoids_reference_collisions_and_invalid_types() {
        let bytes = slice(vec![entity(1078329345, "Platform", 1)], 0, 1);
        let d = inspect(&bytes, None, "6").unwrap();
        let a = &d.assets[0];
        let mut used = used_ids(&bytes, "6").unwrap();
        let next = candidate(a, 5, &used).unwrap();
        assert_eq!(next, 1078263809);
        assert_eq!((next >> 22) & 127, (a.id >> 22) & 127);
        used.insert(next);
        assert_ne!(candidate(a, 5, &used).unwrap(), next);
        assert!(candidate(a, 6, &used).is_err());
        let mut graph = a.clone();
        graph.kind = "Node graph".into();
        assert!(
            candidate(&graph, 5, &used).is_ok(),
            "asset kind never gates ID generation"
        );
        graph.id = 0x60000001;
        graph.can_generate_id = true;
        assert!(
            candidate(&graph, 5, &used).is_err(),
            "recheck the actual ID layout"
        );
    }
}
