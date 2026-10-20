use crate::hash;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Field {
    pub number: u64,
    pub wire: u8,
    pub value: Vec<u8>,
    pub raw: Vec<u8>,
}
fn varint(b: &[u8], i: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for s in (0..70).step_by(7) {
        let x = *b.get(*i).context("Truncated varint")?;
        *i += 1;
        ensure!(s < 63 || x <= 1, "Varint overflow");
        v |= ((x & 127) as u64) << s;
        if x < 128 {
            return Ok(v);
        }
    }
    bail!("Invalid varint")
}
pub fn fields(b: &[u8]) -> Result<Vec<Field>> {
    let mut i = 0;
    let mut out = vec![];
    while i < b.len() {
        let start = i;
        let key = varint(b, &mut i)?;
        let wire = (key & 7) as u8;
        let number = key >> 3;
        ensure!(number > 0 && number < 1 << 29, "Invalid field number");
        let value = match wire {
            0 => {
                let j = i;
                varint(b, &mut i)?;
                b[j..i].to_vec()
            }
            1 | 5 => {
                let n = if wire == 1 { 8 } else { 4 };
                let end = i.checked_add(n).context("Length overflow")?;
                let v = b.get(i..end).context("Truncated fixed value")?.to_vec();
                i = end;
                v
            }
            2 => {
                let n = usize::try_from(varint(b, &mut i)?)?;
                let end = i.checked_add(n).context("Length overflow")?;
                let v = b.get(i..end).context("Truncated data")?.to_vec();
                i = end;
                v
            }
            _ => bail!("Unsupported protobuf wire type {wire}"),
        };
        out.push(Field {
            number,
            wire,
            value,
            raw: b[start..i].to_vec(),
        });
    }
    Ok(out)
}
fn scalar(f: &Field) -> Result<u64> {
    ensure!(f.wire == 0, "Expected integer");
    varint(&f.value, &mut 0)
}
fn one(fs: &[Field], n: u64) -> Result<&Field> {
    let mut it = fs.iter().filter(|f| f.number == n);
    let f = it.next().context(format!("Missing field {n}"))?;
    ensure!(it.next().is_none(), "Duplicate scalar field");
    Ok(f)
}
pub fn payload(b: &[u8], kind: u32) -> Result<&[u8]> {
    ensure!(
        b.len() >= 24 && b.len() as u64 <= crate::constants::MAX_ARTIFACT_BYTES,
        "Invalid file size"
    );
    let u = |i| u32::from_be_bytes(b[i..i + 4].try_into().unwrap());
    ensure!(
        u(0) as usize == b.len() - 4 && u(16) as usize == b.len() - 24,
        "Invalid framing lengths"
    );
    ensure!(
        u(4) == 1 && u(8) == 806 && u(12) == kind,
        "Unrecognized Miliastra format"
    );
    ensure!(u(b.len() - 4) == 1657, "Unrecognized footer");
    Ok(&b[20..b.len() - 4])
}
/// The stage loaded most recently, from field 3 of the account's
/// `Beyond_Local_Save_Player.gip`. The editor rewrites that file when a stage is loaded
/// or saved, but not when it's unloaded.
pub fn last_loaded_level(b: &[u8]) -> Result<Option<String>> {
    let fs = fields(payload(b, 1)?)?;
    match fs.iter().find(|f| f.number == 3) {
        Some(f) => Ok(Some(scalar(f)?.to_string())),
        None => Ok(None),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub path: PathBuf,
    pub level: String,
    pub account: String,
    pub name: String,
    pub sections: Vec<Section>,
    pub last_saved_section: Option<String>,
    pub settings_hash: String,
    #[serde(default)]
    pub settings_version: String,
    #[serde(default)]
    pub editor_version: String,
}
pub fn settings(b: &[u8]) -> Result<(String, Vec<Section>, Option<String>)> {
    let root = fields(payload(b, 8)?)?;
    let level = scalar(one(&root, 1)?)?.to_string();
    let container = fields(&one(&root, 5)?.value)?;
    let mut sections = vec![];
    for row in container.iter().filter(|f| f.number == 1) {
        let r = fields(&row.value)?;
        let id = r
            .iter()
            .find(|f| f.number == 1)
            .map(scalar)
            .transpose()?
            .unwrap_or(0)
            .to_string();
        ensure!(
            crate::domain::valid_section(&id) && !sections.iter().any(|s: &Section| s.id == id),
            "Invalid section ID"
        );
        let name = String::from_utf8(one(&r, 2)?.value.clone())?;
        sections.push(Section { id, name });
    }
    let selected = container
        .iter()
        .find(|f| f.number == 2)
        .map(scalar)
        .transpose()?
        .map(|n| n.to_string());
    ensure!(
        selected
            .as_ref()
            .is_none_or(|s| sections.iter().any(|x| &x.id == s)),
        "Unknown selection"
    );
    Ok((level, sections, selected))
}
fn put_varint(mut n: u64, out: &mut Vec<u8>) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn length_field(number: u64, value: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    put_varint((number << 3) | 2, &mut out);
    put_varint(value.len() as u64, &mut out);
    out.extend(value);
    out
}
/// Rewrites slice names only; every other field is kept byte-for-byte. The editor may
/// need a reload to show the new names.
pub fn patch_section_names(
    bytes: &[u8],
    expected_level: &str,
    names: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    let (level, sections, selected) = settings(bytes)?;
    ensure!(level == expected_level, "Settings belong to another stage");
    for (id, name) in names {
        ensure!(
            id != "shared" && sections.iter().any(|s| &s.id == id),
            "Unknown section name target"
        );
        crate::domain::validate_name(name)?;
    }
    let root = fields(payload(bytes, 8)?)?;
    let container = one(&root, 5)?;
    ensure!(container.wire == 2, "Invalid settings container");
    let mut rows_out = vec![];
    for row in fields(&container.value)? {
        if row.number != 1 {
            rows_out.extend(row.raw);
            continue;
        }
        ensure!(row.wire == 2, "Invalid section row");
        let fields = fields(&row.value)?;
        let ids: Vec<_> = fields.iter().filter(|f| f.number == 1).collect();
        ensure!(ids.len() <= 1, "Duplicate section identity");
        let id = ids
            .first()
            .map(|f| scalar(f))
            .transpose()?
            .unwrap_or(0)
            .to_string();
        let name = one(&fields, 2)?;
        ensure!(name.wire == 2, "Invalid section name");
        if let Some(new) = names.get(&id).filter(|new| new.as_bytes() != name.value) {
            let mut new_row = vec![];
            for field in fields {
                if field.number == 2 {
                    new_row.extend(length_field(2, new.as_bytes()));
                } else {
                    new_row.extend(field.raw);
                }
            }
            rows_out.extend(length_field(1, &new_row));
        } else {
            rows_out.extend(row.raw);
        }
    }
    let mut data = vec![];
    for f in root {
        if f.number == 5 {
            data.extend(length_field(5, &rows_out));
        } else {
            data.extend(f.raw);
        }
    }
    let mut out = bytes[..20].to_vec();
    out[..4].copy_from_slice(&(data.len() as u32 + 20).to_be_bytes());
    out[16..20].copy_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend(data);
    out.extend(&bytes[bytes.len() - 4..]);
    let (new_level, new_sections, new_selected) = settings(&out)?;
    ensure!(
        new_level == level && new_selected == selected && new_sections.len() == sections.len(),
        "Name patch changed settings identity"
    );
    Ok(out)
}
pub fn inspect(path: &Path) -> Result<Stage> {
    let path = path.canonicalize()?;
    let b = fs::read(path.join("beyond_level_setting/local_setting.gils"))?;
    let (level, sections, last_saved_section) = settings(&b)?;
    ensure!(
        path.file_name().is_some_and(|n| n == level.as_str()),
        "Stage folder identity mismatch"
    );
    let main = fs::read(path.join(format!("{level}.gil")))?;
    let fs = fields(payload(&main, 2)?)?;
    ensure!(
        scalar(one(&fs, 1)?)?.to_string() == level,
        "Level identity mismatch"
    );
    let name = fs
        .iter()
        .find(|f| f.number == 2)
        .and_then(|f| String::from_utf8(f.value.clone()).ok())
        .unwrap_or_else(|| level.clone());
    let account = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or_default();
    let modified = std::fs::metadata(path.join("beyond_level_setting/local_setting.gils"))?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let settings_hash = hash(&b);
    let settings_version = format!("{settings_hash}:{modified}");
    let editor_version = fs
        .iter()
        .find(|f| f.number == 43)
        .and_then(|f| String::from_utf8(f.value.clone()).ok())
        .unwrap_or_default();
    Ok(Stage {
        path,
        level,
        account,
        name,
        sections,
        last_saved_section,
        settings_hash,
        settings_version,
        editor_version,
    })
}
pub fn resource_path(stage: &Stage, section: &str) -> Result<PathBuf> {
    ensure!(crate::domain::valid_section(section), "Invalid section");
    let name = if section == "shared" {
        format!("{}.gilh", stage.level)
    } else {
        format!("{}_{}.gis", stage.level, section.parse::<u8>()? + 1)
    };
    Ok(stage.path.join("beyond_level_slice").join(name))
}
pub fn resource_level(b: &[u8], section: &str) -> Result<String> {
    let root = fields(payload(b, if section == "shared" { 7 } else { 6 })?)?;
    let data = if section == "shared" {
        root
    } else {
        fields(&one(&root, 2)?.value)?
    };
    Ok(scalar(one(&data, 1)?)?.to_string())
}
pub fn content_hash(b: &[u8], section: &str, level: &str) -> Result<String> {
    let root = fields(payload(b, if section == "shared" { 7 } else { 6 })?)?;
    let data = if section == "shared" {
        root
    } else {
        ensure!(
            root.iter().filter(|f| f.number == 1).count() <= 1,
            "Duplicate section identity"
        );
        let id = root
            .iter()
            .find(|f| f.number == 1)
            .map(scalar)
            .transpose()?
            .unwrap_or(0);
        ensure!(id.to_string() == section, "Slice section mismatch");
        fields(&one(&root, 2)?.value)?
    };
    ensure!(
        scalar(one(&data, 1)?)?.to_string() == level,
        "Slice level mismatch"
    );
    // These two fields are save timestamps. Everything else, including unknown fields, counts.
    let canonical: Vec<u8> = data
        .iter()
        .filter(|f| f.number != 40 && f.number != 53)
        .flat_map(|f| f.raw.clone())
        .collect();
    Ok(hash(&canonical))
}
/// Builds a filtered copy of a nested message for comparison (the file itself is not
/// changed). Unknown fields and wire types are kept; messages left empty are dropped.
fn comparison_message(
    field: &Field,
    project: impl Fn(&Field) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    if field.wire != 2 {
        return Ok(field.raw.clone());
    }
    let mut out = vec![];
    for child in fields(&field.value)? {
        out.extend(project(&child)?);
    }
    Ok(if out.is_empty() {
        vec![]
    } else {
        length_field(field.number, &out)
    })
}

/// For local change detection only; transfers and published hashes still cover the
/// original bytes. Strips the parts editor 7.1.0 rebuilds on every import or resave
/// (seen 2026-10-18, see the editor-compatibility notes). Folders, graphs, definitions
/// and unknown fields are kept.
pub fn comparison_field(field: &Field, section: &str) -> Result<Vec<u8>> {
    if [1, 40, 53].contains(&field.number) {
        return Ok(vec![]);
    }
    if section != "shared" {
        return Ok(field.raw.clone());
    }
    match (field.number, field.wire) {
        // Recipient stage name, save counter and GUID bookkeeping.
        (2, 2) | (41, 0) | (46, _) => Ok(vec![]),
        // The default "Uncategorized" folder picks up references to existing slice assets on
        // resave. Named folders, their names/IDs and unknown children are kept.
        (6, 2) => comparison_message(field, |row| {
            if row.number != 1 {
                return Ok(row.raw.clone());
            }
            comparison_message(row, |folder| {
                if folder.number != 3 || folder.wire != 2 {
                    return Ok(folder.raw.clone());
                }
                let parts = fields(&folder.value)?;
                let names: Vec<_> = parts.iter().filter(|p| p.number == 1).collect();
                let ids: Vec<_> = parts.iter().filter(|p| p.number == 3).collect();
                let default_bucket = names.len() == 1
                    && names[0].wire == 2
                    && names[0].value == b"Uncategorized Tab"
                    && ids.len() == 1
                    && ids[0].wire == 0
                    && scalar(ids[0])? == 2;
                if !default_bucket {
                    return Ok(folder.raw.clone());
                }
                comparison_message(folder, |member| {
                    // Only plain (type, ID) references are known to be derived.
                    let reference = if member.number == 5 && member.wire == 2 {
                        let r = fields(&member.value)?;
                        r.len() == 2
                            && r[0].number == 1
                            && r[0].wire == 0
                            && r[1].number == 2
                            && r[1].wire == 0
                    } else {
                        false
                    };
                    Ok(if reference {
                        vec![]
                    } else {
                        member.raw.clone()
                    })
                })
            })
        }),
        // Field 5 of shared graphs holds regenerated type/port summaries (2, 3). Graph bodies
        // (1, 2, 4) and structure definitions (6) still count.
        (10, 2) => comparison_message(field, |child| {
            if child.number != 5 {
                return Ok(child.raw.clone());
            }
            comparison_message(child, |summary| {
                Ok(if [2, 3].contains(&summary.number) && summary.wire == 2 {
                    vec![]
                } else {
                    summary.raw.clone()
                })
            })
        }),
        // Some editor state we don't understand. Only the leaf that changes on resave is
        // ignored; the rest of the block still counts.
        (21, 2) => comparison_message(field, |child| {
            Ok(if child.number == 1 && child.wire == 2 {
                vec![]
            } else {
                child.raw.clone()
            })
        }),
        // Resource Explorer column names and visibility; not game content.
        (22, 2) => comparison_message(field, |child| {
            Ok(if [1, 2].contains(&child.number) && child.wire == 2 {
                vec![]
            } else {
                child.raw.clone()
            })
        }),
        _ => Ok(field.raw.clone()),
    }
}
/// Hash for comparing an imported resource with the saved one. Published checksums
/// are not affected.
pub fn import_comparison_hash(b: &[u8], section: &str) -> Result<String> {
    let level = resource_level(b, section)?;
    content_hash(b, section, &level)?;
    let root = fields(payload(b, if section == "shared" { 7 } else { 6 })?)?;
    let data = if section == "shared" {
        root.clone()
    } else {
        fields(&one(&root, 2)?.value)?
    };
    let mut canonical = vec![];
    for field in &data {
        canonical.extend(comparison_field(field, section)?);
    }
    if section != "shared" {
        // Unknown top-level fields count too (field 2 is handled above).
        canonical.extend(
            root.iter()
                .filter(|f| f.number != 2)
                .flat_map(|f| f.raw.clone()),
        );
    }
    Ok(hash(&canonical))
}
pub fn fingerprints(stage: &Stage) -> Result<BTreeMap<String, String>> {
    stage
        .sections
        .iter()
        .map(|s| s.id.clone())
        .chain(std::iter::once("shared".into()))
        .map(|id| {
            let b = fs::read(resource_path(stage, &id)?)?;
            // Imported shared settings keep the sender's level ID, like slices do.
            let level = resource_level(&b, &id)?;
            let h = content_hash(&b, &id, &level)?;
            Ok((id, h))
        })
        .collect()
}
pub fn discover(root: &Path) -> Vec<Stage> {
    let mut out = vec![];
    if let Ok(accounts) = fs::read_dir(root) {
        for a in accounts.flatten() {
            if let Ok(levels) = fs::read_dir(a.path().join("Beyond_Local_Save_Level")) {
                for l in levels.flatten() {
                    if let Ok(stage) = inspect(&l.path()) {
                        out.push(stage);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn frame(kind: u32, p: &[u8]) -> Vec<u8> {
        let mut b = vec![];
        for n in [p.len() as u32 + 20, 1, 806, kind, p.len() as u32] {
            b.extend(n.to_be_bytes());
        }
        b.extend(p);
        b.extend(1657u32.to_be_bytes());
        b
    }
    #[test]
    fn last_loaded_level_reads_player_file_field_3() {
        // field 1 (message), field 3 = 1073742034, field 4 = 1
        let mut p = vec![0x0a, 0x00, 0x18];
        let mut v = 1073742034u64;
        while v >= 128 {
            p.push((v as u8 & 127) | 128);
            v >>= 7;
        }
        p.push(v as u8);
        p.extend([0x20, 0x01]);
        assert_eq!(
            last_loaded_level(&frame(1, &p)).unwrap().as_deref(),
            Some("1073742034")
        );
        assert_eq!(last_loaded_level(&frame(1, &[0x20, 0x01])).unwrap(), None);
        assert!(last_loaded_level(&frame(2, &p)).is_err());
    }
    #[test]
    fn malformed_files_rejected() {
        for b in [vec![], vec![0; 24], frame(6, &[0])] {
            assert!(payload(&b, 8).is_err());
        }
        assert!(fields(&[10, 255]).is_err());
        assert!(fields(&[0]).is_err());
    }
    #[test]
    fn timestamp_only_saves_are_unchanged() {
        let a = frame(7, &[8, 1, 192, 2, 1, 168, 3, 1]);
        let b = frame(7, &[8, 1, 192, 2, 9, 168, 3, 9]);
        assert_eq!(
            content_hash(&a, "shared", "1").unwrap(),
            content_hash(&b, "shared", "1").unwrap()
        );
    }
    #[test]
    fn shared_settings_ignore_per_save_bookkeeping_but_not_real_changes() {
        // field 1 = level, field 46 = per-save bookkeeping, field 6 = level-wide content
        let save = |bookkeeping: u8, content: u8| {
            frame(
                7,
                &[8, 1, 0xF2, 0x02, 2, 8, bookkeeping, 0x32, 2, 8, content],
            )
        };
        let compare = |b: &[u8]| import_comparison_hash(b, "shared").unwrap();
        assert_eq!(
            compare(&save(1, 5)),
            compare(&save(2, 5)),
            "a plain re-save"
        );
        assert_ne!(compare(&save(1, 5)), compare(&save(1, 6)), "a real change");
        // Published checksums are unchanged: they still cover field 46.
        assert_ne!(
            content_hash(&save(1, 5), "shared", "1").unwrap(),
            content_hash(&save(2, 5), "shared", "1").unwrap()
        );
    }
    fn number(n: u64, v: u64) -> Vec<u8> {
        let mut out = vec![];
        put_varint(n << 3, &mut out);
        put_varint(v, &mut out);
        out
    }
    // Sanitized shape from the imported stage before/after a no-edit editor save.
    fn shared_resave(regenerated: bool, faction: &str) -> Vec<u8> {
        let reference = length_field(5, &[number(1, 800), number(2, 1074462721)].concat());
        let folder = [
            length_field(1, b"Uncategorized Tab"),
            number(3, 2),
            if regenerated { reference } else { vec![] },
        ]
        .concat();
        frame(
            7,
            &[
                number(1, if regenerated { 501 } else { 701 }),
                length_field(2, if regenerated { b"Recipient" } else { b"Donor" }),
                length_field(
                    6,
                    &length_field(1, &[number(1, 4), length_field(3, &folder)].concat()),
                ),
                length_field(
                    10,
                    &[
                        length_field(3, &number(1, 2)),
                        if regenerated {
                            length_field(
                                5,
                                &[
                                    length_field(2, &number(5, 1611333633)),
                                    length_field(3, &length_field(3, b"Composite summary")),
                                ]
                                .concat(),
                            )
                        } else {
                            vec![]
                        },
                        number(7, 1),
                    ]
                    .concat(),
                ),
                length_field(
                    11,
                    &length_field(2, &length_field(1, &length_field(1, faction.as_bytes()))),
                ),
                length_field(
                    21,
                    &length_field(
                        1,
                        if regenerated {
                            &[23, 95, 24, 49]
                        } else {
                            &[23]
                        },
                    ),
                ),
                length_field(
                    22,
                    &if regenerated {
                        [length_field(1, b"PropertyTransform"), length_field(2, &[1])].concat()
                    } else {
                        vec![]
                    },
                ),
                number(40, if regenerated { 200 } else { 100 }),
                number(41, if regenerated { 3 } else { 1 }),
                length_field(46, &number(1, if regenerated { 999 } else { 111 })),
            ]
            .concat(),
        )
    }
    #[test]
    fn regenerated_shared_metadata_is_not_an_edit_but_faction_renames_are() {
        let before = shared_resave(false, "Initial faction");
        let saved = shared_resave(true, "Initial faction");
        let renamed = shared_resave(true, "Renamed faction");
        let compare = |b: &[u8]| import_comparison_hash(b, "shared").unwrap();
        assert_eq!(compare(&before), compare(&saved));
        assert_ne!(compare(&saved), compare(&renamed));
        assert_ne!(
            hash(&before),
            hash(&saved),
            "transport SHA covers every byte"
        );
        assert_ne!(
            content_hash(&before, "shared", "701").unwrap(),
            content_hash(&saved, "shared", "501").unwrap(),
            "published content hashes stay unchanged"
        );
    }
    #[test]
    fn folder_organization_and_unknown_reference_data_stay_significant() {
        let folder = |name: &[u8], id: u64, members: &[u8]| {
            frame(
                7,
                &[
                    number(1, 1),
                    length_field(
                        6,
                        &length_field(
                            1,
                            &length_field(
                                3,
                                &[length_field(1, name), number(3, id), members.to_vec()].concat(),
                            ),
                        ),
                    ),
                ]
                .concat(),
            )
        };
        let reference = length_field(5, &[number(1, 800), number(2, 123)].concat());
        let compare = |b: &[u8]| import_comparison_hash(b, "shared").unwrap();
        assert_ne!(
            compare(&folder(b"Boats", 3, &[])),
            compare(&folder(b"Boats", 3, &reference))
        );
        assert_ne!(
            compare(&folder(b"Uncategorized Tab", 2, &[])),
            compare(&folder(b"Boats", 2, &[]))
        );
        let unknown = length_field(5, &[number(1, 800), number(2, 123), number(99, 1)].concat());
        assert_ne!(
            compare(&folder(b"Uncategorized Tab", 2, &[])),
            compare(&folder(b"Uncategorized Tab", 2, &unknown))
        );
    }
    #[test]
    fn authored_graphs_settings_and_unknown_metadata_children_are_preserved() {
        let baseline = shared_resave(true, "Faction");
        let compare = |b: &[u8]| import_comparison_hash(b, "shared").unwrap();
        for field in [
            length_field(10, &length_field(1, b"Executable graph")),
            length_field(10, &length_field(6, b"Structure definition")),
            length_field(10, &length_field(5, &number(99, 1))),
            length_field(21, &number(99, 1)),
            length_field(22, &number(99, 1)),
            length_field(6, &number(99, 1)),
            length_field(36, b"Global setting"),
            number(99, 1),
        ] {
            let changed = frame(7, &[payload(&baseline, 7).unwrap(), &field].concat());
            assert_ne!(compare(&baseline), compare(&changed));
        }
    }
    #[test]
    fn shared_projection_never_hides_ordinary_slice_content() {
        let slice =
            |body: &[u8]| frame(6, &length_field(2, &[number(1, 1), body.to_vec()].concat()));
        for field in [
            number(41, 3),
            length_field(22, &length_field(1, b"Column")),
            length_field(21, &length_field(1, &[23])),
            length_field(10, &length_field(5, &length_field(2, b"Data"))),
        ] {
            assert_ne!(
                import_comparison_hash(&slice(&[]), "0").unwrap(),
                import_comparison_hash(&slice(&field), "0").unwrap()
            );
        }
    }
    #[test]
    fn unknown_fields_remain_significant() {
        let a = frame(7, &[8, 1, 16, 2]);
        let b = frame(7, &[8, 1, 16, 3]);
        assert_ne!(
            content_hash(&a, "shared", "1").unwrap(),
            content_hash(&b, "shared", "1").unwrap()
        );
        assert!(content_hash(&a, "shared", "2").is_err());
    }
    #[test]
    fn duplicate_identity_rejected() {
        assert!(content_hash(&frame(7, &[8, 1, 8, 2]), "shared", "1").is_err());
    }
    #[test]
    fn import_comparison_normalizes_only_level_identity_and_save_times() {
        let a = frame(6, &[8, 5, 18, 8, 8, 1, 208, 5, 7, 192, 2, 1]);
        let b = frame(6, &[8, 5, 18, 8, 8, 2, 208, 5, 7, 192, 2, 9]);
        assert_ne!(
            content_hash(&a, "5", "1").unwrap(),
            content_hash(&b, "5", "2").unwrap()
        );
        assert_eq!(
            import_comparison_hash(&a, "5").unwrap(),
            import_comparison_hash(&b, "5").unwrap()
        );
        let changed = frame(6, &[8, 5, 18, 8, 8, 2, 208, 5, 8, 192, 2, 9]);
        assert_ne!(
            import_comparison_hash(&a, "5").unwrap(),
            import_comparison_hash(&changed, "5").unwrap()
        );
        let mut outer = vec![8, 5, 18, 8, 8, 2, 208, 5, 7, 192, 2, 9];
        outer.extend([208, 5, 1]);
        assert_ne!(
            import_comparison_hash(&a, "5").unwrap(),
            import_comparison_hash(&frame(6, &outer), "5").unwrap()
        );
    }
    #[test]
    fn duplicate_outer_section_identity_is_rejected() {
        let b = frame(6, &[8, 5, 8, 6, 18, 2, 8, 1]);
        assert!(content_hash(&b, "5", "1").is_err());
    }
    #[test]
    fn names_patch_preserves_recipient_selection_order_and_unknown_fields() {
        let row0 = [length_field(2, b"Common"), vec![152, 6, 7]].concat();
        let row5 = [vec![8, 5], length_field(2, b"Boats"), vec![152, 6, 9]].concat();
        let container = [
            length_field(1, &row5),
            vec![16, 5, 24, 1],
            length_field(1, &row0),
        ]
        .concat();
        let b = frame(
            8,
            &[
                vec![8, 42, 152, 6, 3],
                length_field(5, &container),
                vec![160, 6, 8],
            ]
            .concat(),
        );
        let names = BTreeMap::from([
            ("5".into(), "Bateaux 海".into()),
            ("0".into(), "Common".into()),
        ]);
        let patched = patch_section_names(&b, "42", &names).unwrap();
        let (level, sections, selected) = settings(&patched).unwrap();
        assert_eq!(level, "42");
        assert_eq!(selected.as_deref(), Some("5"));
        assert_eq!(sections[0].name, "Bateaux 海");
        assert_eq!(sections[1].id, "0");
        let root = fields(payload(&patched, 8).unwrap()).unwrap();
        assert_eq!(root[1].raw, vec![152, 6, 3]);
        assert_eq!(root[3].raw, vec![160, 6, 8]);
        let rows = fields(&one(&root, 5).unwrap().value).unwrap();
        assert_eq!(rows[1].raw, vec![16, 5]);
        assert_eq!(rows[2].raw, vec![24, 1]);
        assert_eq!(rows[3].value, row0);
        assert!(rows[0].value.ends_with(&[152, 6, 9]));
        assert_eq!(
            patch_section_names(&patched, "42", &names).unwrap(),
            patched
        );
        assert_eq!(
            patch_section_names(
                &patched,
                "42",
                &BTreeMap::from([("5".into(), "Boats".into())])
            )
            .unwrap(),
            b
        );
    }
    #[test]
    fn names_patch_rejects_wrong_identity_unknown_slots_and_malformed_rows() {
        let row = [vec![8, 5], length_field(2, b"Boats")].concat();
        let b = frame(
            8,
            &[vec![8, 42], length_field(5, &length_field(1, &row))].concat(),
        );
        assert!(patch_section_names(&b, "43", &BTreeMap::new()).is_err());
        assert!(
            patch_section_names(&b, "42", &BTreeMap::from([("6".into(), "Islands".into())]))
                .is_err()
        );
        assert!(
            patch_section_names(&b, "42", &BTreeMap::from([("5".into(), "\n".into())])).is_err()
        );
        let duplicate = [vec![8, 5, 8, 6], length_field(2, b"Boats")].concat();
        let bad = frame(
            8,
            &[vec![8, 42], length_field(5, &length_field(1, &duplicate))].concat(),
        );
        assert!(patch_section_names(&bad, "42", &BTreeMap::new()).is_err());
    }
}
