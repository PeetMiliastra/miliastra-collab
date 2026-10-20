//! Keeps slice names in the stage's editor settings in line with the project's names.
use anyhow::{ensure, Result};
use miliastra_core::{
    binary::{self, Stage},
    hash, NameChange, ProjectView,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Write, path::Path};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub name: String,
    pub base_name: String,
    pub parent_version: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub edits: BTreeMap<String, Edit>,
}
/// Each row has its own draft, so saving or cancelling one row never overwrites another,
/// even when saves overlap.
pub fn save_draft(db: &crate::db::Db, project: &str, incoming: Draft) -> Result<()> {
    db.update::<Draft>(&format!("names-draft:{project}"), |draft| {
        for (id, edit) in incoming.edits {
            ensure!(
                id != "shared" && miliastra_core::valid_section(&id),
                "Unknown slice name"
            );
            ensure!(
                miliastra_core::constants::text_fits(
                    &edit.name,
                    miliastra_core::constants::MAX_SLICE_NAME_UNITS
                ) && miliastra_core::constants::text_fits(
                    &edit.base_name,
                    miliastra_core::constants::MAX_SLICE_NAME_UNITS
                ),
                "Name draft is too long"
            );
            if edit.name == edit.base_name {
                draft.edits.remove(&id);
            } else {
                draft.edits.insert(id, edit);
            }
        }
        Ok(())
    })
}
/// Clears only the names that were published; other rows and newer edits stay.
pub fn clear_published(db: &crate::db::Db, project: &str, changes: &[NameChange]) -> Result<()> {
    db.update::<Draft>(&format!("names-draft:{project}"), |draft| {
        for change in changes {
            if draft
                .edits
                .get(&change.section)
                .is_some_and(|edit| edit.name.trim() == change.after)
            {
                draft.edits.remove(&change.section);
            }
        }
        Ok(())
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub pending: Vec<NameChange>,
    pub draft: Draft,
    pub error: Option<String>,
}
pub fn effective_names(project: &ProjectView) -> BTreeMap<String, String> {
    let mut names: BTreeMap<_, _> = project
        .heads
        .iter()
        .filter(|(id, _)| id.as_str() != "shared")
        .map(|(id, r)| (id.clone(), r.name.clone()))
        .collect();
    names.extend(project.section_names.clone());
    names
}
pub fn differences(stage: &Stage, project: &ProjectView) -> Vec<NameChange> {
    let names = effective_names(project);
    stage
        .sections
        .iter()
        .filter_map(|s| {
            names
                .get(&s.id)
                .filter(|n| *n != &s.name)
                .map(|name| NameChange {
                    section: s.id.clone(),
                    before: s.name.clone(),
                    after: name.clone(),
                })
        })
        .collect()
}

/// Writes the names into the settings file (backing up just that file). Unknown fields
/// and the selection are kept, and the file is re-checked right before the swap.
pub fn synchronize(stage: &Stage, project: &ProjectView, backups: &Path) -> Result<bool> {
    let root = stage.path.canonicalize()?;
    let path = root.join("beyond_level_setting/local_setting.gils");
    ensure!(
        !path.symlink_metadata()?.file_type().is_symlink()
            && path.canonicalize()?.starts_with(&root),
        "Linked editor settings are unsupported"
    );
    let before = fs::read(&path)?;
    let (level, sections, _) = binary::settings(&before)?;
    ensure!(level == stage.level, "Settings belong to another stage");
    let canonical = effective_names(project);
    let desired: BTreeMap<_, _> = sections
        .iter()
        .filter_map(|s| {
            canonical
                .get(&s.id)
                .filter(|n| *n != &s.name)
                .map(|n| (s.id.clone(), n.clone()))
        })
        .collect();
    if desired.is_empty() {
        return Ok(false);
    }
    let patched = binary::patch_section_names(&before, &stage.level, &desired)?;
    ensure!(
        backups.is_absolute() && !backups.starts_with(&root) && !root.starts_with(backups),
        "Name backups must be separate from the stage"
    );
    fs::create_dir_all(backups)?;
    let backups = backups.canonicalize()?;
    ensure!(
        !backups.starts_with(&root) && !root.starts_with(&backups),
        "Name backups must be separate from the stage"
    );
    let folder = backups.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&folder)?;
    let mut original = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join("local_setting.gils"))?;
    original.write_all(&before)?;
    original.sync_all()?;
    drop(original);
    let mut journal = serde_json::json!({"stage":root,"project":project.id,"namesVersion":project.names_version,
        "originalHash":hash(&before),"replacementHash":hash(&patched),"names":desired,"phase":"prepared"});
    let journal_path = folder.join("journal.json");
    let save_journal = |value: &serde_json::Value| -> Result<()> {
        let mut f = fs::File::create(&journal_path)?;
        f.write_all(&serde_json::to_vec_pretty(value)?)?;
        f.sync_all()?;
        Ok(())
    };
    save_journal(&journal)?;
    let temp = path.with_file_name(format!(".mcollab-names-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(&patched)?;
        f.sync_all()?;
        drop(f);
        ensure!(
            fs::read(&path)? == before,
            "Miliastra saved while names were syncing; retrying"
        );
        fs::rename(&temp, &path)?;
        ensure!(
            fs::read(&path)? == patched,
            "Miliastra saved again while names were syncing; retrying"
        );
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    journal["phase"] = serde_json::json!(if result.is_ok() { "applied" } else { "retry" });
    save_journal(&journal)?;
    result?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct Fixture {
        root: PathBuf,
        stage: Stage,
        project: ProjectView,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("mcollab-names-{}", uuid::Uuid::new_v4()));
            let path = root.join("stage");
            fs::create_dir_all(path.join("beyond_level_setting")).unwrap();
            let stage: Stage = serde_json::from_value(serde_json::json!({
                "path":path,"level":"42","account":"test","name":"Test",
                "sections":[{"id":"5","name":"Boats"}],"lastSavedSection":"5","settingsHash":""
            }))
            .unwrap();
            let project = serde_json::from_value(serde_json::json!({
                "id":"p","name":"Test","storage":{"kind":"folder","location":""},
                "members":[],"heads":{},"locks":[],"sequence":1,"namesVersion":1,"sectionNames":{"5":"Ships"}
            })).unwrap();
            let f = Self {
                root,
                stage,
                project,
            };
            fs::write(f.path(), bytes(5, 7)).unwrap();
            f
        }
        fn path(&self) -> PathBuf {
            self.stage
                .path
                .join("beyond_level_setting/local_setting.gils")
        }
        fn backups(&self) -> PathBuf {
            self.root.join("backups")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn bytes(selected: u8, unknown: u8) -> Vec<u8> {
        let payload = [
            8, 42, 42, 25, 10, 9, 8, 5, 18, 5, 66, 111, 97, 116, 115, 10, 10, 8, 0, 18, 6, 67, 111,
            109, 109, 111, 110, 16, selected, 24, unknown,
        ];
        let mut b = Vec::new();
        for n in [payload.len() as u32 + 20, 1, 806, 8, payload.len() as u32] {
            b.extend(n.to_be_bytes());
        }
        b.extend(payload);
        b.extend(1657u32.to_be_bytes());
        b
    }
    #[test]
    fn concurrent_offline_drafts_keep_independent_versions_and_publish_only_clears_matching_values()
    {
        let f = Fixture::new();
        let db = std::sync::Arc::new(crate::db::Db::open(&f.root.join("collab.db")).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        std::thread::scope(|scope| {
            for i in 0..8 {
                let db = db.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    save_draft(
                        &db,
                        "p",
                        Draft {
                            edits: BTreeMap::from([(
                                i.to_string(),
                                Edit {
                                    name: format!("New {i}"),
                                    base_name: format!("Old {i}"),
                                    parent_version: i,
                                },
                            )]),
                        },
                    )
                    .unwrap();
                });
            }
        });
        let saved: Draft = db.get("names-draft:p").unwrap().unwrap();
        assert_eq!(saved.edits.len(), 8);
        for i in 0..8 {
            assert_eq!(saved.edits[&i.to_string()].parent_version, i);
        }
        save_draft(
            &db,
            "p",
            Draft {
                edits: BTreeMap::from([(
                    "0".into(),
                    Edit {
                        name: "Old 0".into(),
                        base_name: "Old 0".into(),
                        parent_version: 0,
                    },
                )]),
            },
        )
        .unwrap();
        clear_published(
            &db,
            "p",
            &[
                NameChange {
                    section: "1".into(),
                    before: "Old 1".into(),
                    after: "New 1".into(),
                },
                NameChange {
                    section: "2".into(),
                    before: "Old 2".into(),
                    after: "Earlier edit".into(),
                },
            ],
        )
        .unwrap();
        let saved: Draft = db.get("names-draft:p").unwrap().unwrap();
        assert_eq!(saved.edits.len(), 6);
        assert!(!saved.edits.contains_key("0") && !saved.edits.contains_key("1"));
        assert_eq!(saved.edits["2"].name, "New 2");
        assert!(db
            .get::<Draft>("names-draft:another-project")
            .unwrap()
            .is_none());
    }
    #[test]
    fn canonical_names_preserve_selection_unknown_fields_and_durable_backup() {
        let f = Fixture::new();
        let before = fs::read(f.path()).unwrap();
        let desired = BTreeMap::from([("5".into(), "Ships".into())]);
        assert!(synchronize(&f.stage, &f.project, &f.backups()).unwrap());
        let after = fs::read(f.path()).unwrap();
        assert_eq!(
            after,
            binary::patch_section_names(&before, "42", &desired).unwrap()
        );
        let (level, sections, selected) = binary::settings(&after).unwrap();
        assert_eq!(level, "42");
        assert_eq!(sections[0].name, "Ships");
        assert_eq!(selected.as_deref(), Some("5"));
        assert!(after.windows(2).any(|x| x == [24, 7]));
        let backup = fs::read_dir(f.backups())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read(backup.join("local_setting.gils")).unwrap(), before);
        let journal: serde_json::Value =
            serde_json::from_slice(&fs::read(backup.join("journal.json")).unwrap()).unwrap();
        assert_eq!(journal["phase"], "applied");
        assert_eq!(journal["originalHash"], hash(&before));
        let modified = fs::metadata(f.path()).unwrap().modified().unwrap();
        assert!(!synchronize(&f.stage, &f.project, &f.backups()).unwrap());
        assert_eq!(
            fs::metadata(f.path()).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(fs::read_dir(f.backups()).unwrap().count(), 1);
    }
    #[test]
    fn editor_saves_are_reapplied_using_the_fresh_bytes_and_selection() {
        let f = Fixture::new();
        synchronize(&f.stage, &f.project, &f.backups()).unwrap();
        let saved = bytes(0, 9);
        fs::write(f.path(), &saved).unwrap();
        assert!(synchronize(&f.stage, &f.project, &f.backups()).unwrap());
        let after = fs::read(f.path()).unwrap();
        assert_eq!(binary::settings(&after).unwrap().2.as_deref(), Some("0"));
        assert!(after.windows(2).any(|x| x == [24, 9]));
        assert_eq!(fs::read_dir(f.backups()).unwrap().count(), 2);
    }
    #[test]
    fn malformed_identity_or_names_never_replace_settings() {
        let mut f = Fixture::new();
        let before = fs::read(f.path()).unwrap();
        f.stage.level = "99".into();
        assert!(synchronize(&f.stage, &f.project, &f.backups()).is_err());
        assert_eq!(fs::read(f.path()).unwrap(), before);
        f.stage.level = "42".into();
        f.project.section_names.insert("5".into(), " ".into());
        assert!(synchronize(&f.stage, &f.project, &f.backups()).is_err());
        assert_eq!(fs::read(f.path()).unwrap(), before);
        assert!(!f.backups().exists());
        fs::write(f.path(), [0, 1, 2]).unwrap();
        assert!(synchronize(&f.stage, &f.project, &f.backups()).is_err());
        assert_eq!(fs::read(f.path()).unwrap(), [0, 1, 2]);
    }
    #[test]
    fn backup_location_inside_stage_is_rejected_without_creating_it() {
        let f = Fixture::new();
        let before = fs::read(f.path()).unwrap();
        let inside = f.stage.path.canonicalize().unwrap().join("backup");
        assert!(synchronize(&f.stage, &f.project, &inside).is_err());
        assert!(!inside.exists());
        assert_eq!(fs::read(f.path()).unwrap(), before);
    }
}
