use crate::transaction::{self, Replacement};
use anyhow::{ensure, Context, Result};
use miliastra_core::{
    binary::{self, Stage},
    hash, Revision,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
// Slices can be copied between any accounts, stages and editor versions (confirmed by
// the creator on 2026-10-15). The files themselves are checked again before installing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallationPhase {
    Installing,
    AwaitingReload,
    Verified,
    Interrupted,
    Restoring,
    UndoReload,
    Undone,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compatibility {
    pub direct_install: bool,
    pub live_detection: bool,
    pub restore_versions: bool,
    pub reason: String,
}
pub fn compatibility(stage: Option<&Stage>) -> Compatibility {
    Compatibility {
        direct_install: stage.is_some(),
        live_detection: false,
        restore_versions: stage.is_some(),
        reason: if stage.is_some() {
            "Applies slice updates with a backup. Reload the stage afterward."
        } else {
            "Choose a stage to apply updates."
        }
        .into(),
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub project: String,
    pub stage: PathBuf,
    pub revisions: Vec<Revision>,
    pub originals: BTreeMap<String, String>,
    pub previous: BTreeMap<String, Revision>,
    pub at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    #[serde(default)]
    pub local_revert: bool,
    pub id: String,
    pub project: String,
    pub stage: PathBuf,
    pub journal: PathBuf,
    pub revisions: Vec<Revision>,
    pub previous: BTreeMap<String, Revision>,
    pub phase: InstallationPhase,
    pub error: Option<String>,
    #[serde(default)]
    pub undo_baselines: Option<BTreeMap<String, Revision>>,
}
impl Installation {
    pub fn pending(&self) -> bool {
        !matches!(
            self.phase,
            InstallationPhase::Verified | InstallationPhase::Undone
        )
    }
    pub fn affects(&self, id: &str) -> bool {
        self.pending() && self.revisions.iter().any(|r| r.section == id)
    }
}
pub fn matches_bytes(bytes: &[u8], revision: &Revision, cache: &Path) -> Result<bool> {
    let level = binary::resource_level(bytes, &revision.section)?;
    let content = binary::content_hash(bytes, &revision.section, &level)?;
    let source = cache.join(&revision.hash);
    if source.exists() {
        let expected = fs::read(source)?;
        ensure!(
            hash(&expected) == revision.hash && expected.len() as u64 == revision.size,
            "Cached baseline failed checksum validation"
        );
        Ok(binary::import_comparison_hash(bytes, &revision.section)?
            == binary::import_comparison_hash(&expected, &revision.section)?)
    } else {
        Ok(content == revision.content_hash)
    }
}
pub fn matches_file(stage: &Stage, revision: &Revision, cache: &Path) -> Result<bool> {
    matches_bytes(
        &fs::read(binary::resource_path(stage, &revision.section)?)?,
        revision,
        cache,
    )
}
pub fn preview(
    stage: &Stage,
    project: &str,
    heads: &BTreeMap<String, Revision>,
    installed: &BTreeMap<String, Revision>,
    cache: &Path,
) -> Result<Plan> {
    preview_files(stage, project, heads, installed, cache)
}
pub(crate) fn preview_files(
    stage: &Stage,
    project: &str,
    heads: &BTreeMap<String, Revision>,
    installed: &BTreeMap<String, Revision>,
    cache: &Path,
) -> Result<Plan> {
    let shared = heads
        .get("shared")
        .context("The project has no shared-settings baseline")?;
    let base=installed.get("shared").context("Shared settings do not match this project. Use a matching project stage before applying updates.")?;
    // Shared level settings install like a slice, as a plain copy (tested 2026-10-17: a
    // teammate's .gilh copied into another stage loads and stays). Unshared local edits to
    // them block the batch, since incoming slices depend on the team's version.
    ensure!(
        matches_file(stage, base, cache)?,
        "Your shared level settings have changes you haven't shared. Share them first, or review the conflict, before applying updates."
    );
    let mut revisions = vec![];
    let mut originals = BTreeMap::new();
    // Shared settings go first, since the slices depend on them.
    let incoming = std::iter::once(shared)
        .filter(|r| r.hash != base.hash)
        .chain(heads.values().filter(|r| {
            r.section != "shared" && installed.get(&r.section).is_none_or(|b| b.hash != r.hash)
        }));
    for revision in incoming {
        if revision.section == "shared" {
            miliastra_core::validate_revision(revision)?;
            let source = fs::read(cache.join(&revision.hash))
                .context("Shared level settings have not finished downloading")?;
            ensure!(
                source.len() as u64 == revision.size && hash(&source) == revision.hash,
                "Shared level settings failed checksum validation"
            );
            ensure!(
                binary::content_hash(
                    &source,
                    "shared",
                    &binary::resource_level(&source, "shared")?
                )? == revision.content_hash,
                "Shared level settings failed content validation"
            );
            originals.insert(
                "shared".into(),
                hash(&fs::read(binary::resource_path(stage, "shared")?)?),
            );
            revisions.push(revision.clone());
            continue;
        }
        miliastra_core::validate_revision(revision)?;
        ensure!(
            revision.shared_dependency.as_deref() == Some(shared.hash.as_str()),
            "{} requires different shared settings",
            revision.name
        );
        let baseline = installed.get(&revision.section).with_context(|| {
            format!(
                "{} has no known local baseline. Preserve or reconcile its local work first.",
                revision.name
            )
        })?;
        ensure!(
            matches_file(stage, baseline, cache)?,
            "{} contains local changes. Share or preserve them before applying updates.",
            revision.name
        );
        let source = fs::read(cache.join(&revision.hash))
            .with_context(|| format!("{} has not finished downloading", revision.name))?;
        ensure!(
            source.len() as u64 == revision.size && hash(&source) == revision.hash,
            "{} failed checksum validation",
            revision.name
        );
        let source_level = binary::resource_level(&source, &revision.section)?;
        ensure!(
            binary::content_hash(&source, &revision.section, &source_level)?
                == revision.content_hash,
            "{} failed content validation",
            revision.name
        );
        originals.insert(
            revision.section.clone(),
            hash(&fs::read(binary::resource_path(stage, &revision.section)?)?),
        );
        revisions.push(revision.clone());
    }
    ensure!(!revisions.is_empty(), "There are no new updates to apply");
    Ok(Plan {
        id: uuid::Uuid::new_v4().to_string(),
        project: project.into(),
        stage: stage.path.clone(),
        revisions,
        originals,
        previous: installed.clone(),
        at: chrono::Utc::now().timestamp(),
    })
}
pub fn prepare(plan: &Plan, stage: &Stage, cache: &Path, backups: &Path) -> Result<Installation> {
    prepare_files(plan, stage, cache, backups)
}
pub(crate) fn prepare_files(
    plan: &Plan,
    stage: &Stage,
    cache: &Path,
    backups: &Path,
) -> Result<Installation> {
    ensure!(
        plan.stage.canonicalize()? == stage.path.canonicalize()?,
        "The selected stage changed"
    );
    let items = plan
        .revisions
        .iter()
        .map(|r| {
            let path = binary::resource_path(stage, &r.section)?;
            Ok(Replacement {
                relative: path.strip_prefix(&stage.path)?.to_owned(),
                original_hash: plan
                    .originals
                    .get(&r.section)
                    .context("Missing local baseline")?
                    .clone(),
                replacement_hash: r.hash.clone(),
                verified_save_hash: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    fs::create_dir_all(backups)?;
    let journal = transaction::prepare(&stage.path, cache, backups, items)?;
    Ok(Installation {
        local_revert: false,
        id: plan.id.clone(),
        project: plan.project.clone(),
        stage: stage.path.clone(),
        journal,
        revisions: plan.revisions.clone(),
        previous: plan.previous.clone(),
        phase: crate::installer::InstallationPhase::Installing,
        error: None,
        undo_baselines: None,
    })
}
pub fn verify(
    record: &Installation,
    stage: &Stage,
    cache: &Path,
    undo: bool,
) -> Result<BTreeMap<PathBuf, String>> {
    ensure!(
        record.stage.canonicalize()? == stage.path.canonicalize()?,
        "The selected stage changed"
    );
    let mut hashes = BTreeMap::new();
    for incoming in &record.revisions {
        let expected = if undo {
            record
                .previous
                .get(&incoming.section)
                .context("Previous revision unavailable")?
        } else {
            incoming
        };
        ensure!(matches_file(stage,expected,cache)?,"{} differs from the expected import. The files on disk changed or do not match the downloaded update. Your backup is preserved.",incoming.name);
        let path = binary::resource_path(stage, &incoming.section)?;
        hashes.insert(
            path.strip_prefix(&stage.path)?.to_owned(),
            hash(&fs::read(path)?),
        );
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vi(mut n: u64) -> Vec<u8> {
        let mut out = vec![];
        loop {
            let mut b = (n & 127) as u8;
            n >>= 7;
            if n != 0 {
                b |= 128;
            }
            out.push(b);
            if n == 0 {
                return out;
            }
        }
    }
    fn framed(level: u64, section: Option<u8>, value: u8) -> Vec<u8> {
        let mut data = vec![8];
        data.extend(vi(level));
        data.extend(vi(90 * 8));
        data.push(value);
        let payload = if let Some(section) = section {
            let mut p = vec![8, section, 18];
            p.extend(vi(data.len() as u64));
            p.extend(data);
            p
        } else {
            data
        };
        let mut b = vec![];
        for n in [
            payload.len() as u32 + 20,
            1,
            806,
            if section.is_some() { 6 } else { 7 },
            payload.len() as u32,
        ] {
            b.extend(n.to_be_bytes());
        }
        b.extend(payload);
        b.extend(1657u32.to_be_bytes());
        b
    }
    struct Fixture {
        root: PathBuf,
        stage: Stage,
        cache: PathBuf,
        backups: PathBuf,
        heads: BTreeMap<String, Revision>,
        installed: BTreeMap<String, Revision>,
        old: Vec<u8>,
        incoming: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            Self::for_recipient("750933001", "1073742034", "7.1.0")
        }
        fn for_recipient(account: &str, level: &str, version: &str) -> Self {
            let level_id = level.parse().unwrap();
            let root =
                std::env::temp_dir().join(format!("mcollab-install-{}", uuid::Uuid::new_v4()));
            let path = root.join("stage");
            let cache = root.join("cache");
            let backups = root.join("backups");
            for p in [
                path.join("beyond_level_slice"),
                path.join("beyond_level_setting"),
                cache.clone(),
            ] {
                fs::create_dir_all(p).unwrap();
            }
            let stage = Stage {
                path,
                level: level.into(),
                account: account.into(),
                name: "Fixture".into(),
                sections: vec![],
                last_saved_section: None,
                settings_hash: String::new(),
                settings_version: String::new(),
                editor_version: version.into(),
            };
            let old = framed(level_id, Some(5), 1);
            let incoming = framed(1073938433, Some(5), 2);
            let shared = framed(level_id, None, 1);
            fs::write(binary::resource_path(&stage, "5").unwrap(), &old).unwrap();
            fs::write(binary::resource_path(&stage, "shared").unwrap(), &shared).unwrap();
            fs::write(
                stage.path.join("beyond_level_setting/local_setting.gils"),
                b"private selection",
            )
            .unwrap();
            fs::write(
                stage.path.join(format!("{level}.gil")),
                b"full stage unchanged",
            )
            .unwrap();
            let rev =
                |id: &str, bytes: &[u8], parent: Option<String>, dependency: Option<String>| {
                    let h = hash(bytes);
                    fs::write(cache.join(&h), bytes).unwrap();
                    Revision {
                        section: id.into(),
                        name: if id == "shared" {
                            "Shared settings"
                        } else {
                            "Boats"
                        }
                        .into(),
                        hash: h.clone(),
                        content_hash: binary::content_hash(
                            bytes,
                            id,
                            &binary::resource_level(bytes, id).unwrap(),
                        )
                        .unwrap(),
                        parent,
                        artifact: miliastra_core::history_artifact(
                            &uuid::Uuid::new_v4().to_string(),
                            id,
                        )
                        .unwrap(),
                        size: bytes.len() as u64,
                        shared_dependency: dependency,
                    }
                };
            let shared_r = rev("shared", &shared, None, None);
            let old_r = rev("5", &old, None, Some(shared_r.hash.clone()));
            let new_r = rev(
                "5",
                &incoming,
                Some(old_r.hash.clone()),
                Some(shared_r.hash.clone()),
            );
            Self {
                root,
                stage,
                cache,
                backups,
                heads: BTreeMap::from([("shared".into(), shared_r.clone()), ("5".into(), new_r)]),
                installed: BTreeMap::from([("shared".into(), shared_r), ("5".into(), old_r)]),
                old,
                incoming,
            }
        }
        fn plan(&self) -> Result<Plan> {
            preview(
                &self.stage,
                "fixture",
                &self.heads,
                &self.installed,
                &self.cache,
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.canonicalize().ok().is_some_and(|p| {
                p.starts_with(std::env::temp_dir().canonicalize().unwrap())
                    && p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("mcollab-install-")
            }) {
                let _ = fs::remove_dir_all(&self.root);
            }
        }
    }
    #[test]
    fn shared_resave_matches_the_installed_revision_without_weakening_checksums() {
        let f = Fixture::new();
        let base = f.installed.get("shared").unwrap();
        let original = fs::read(f.cache.join(&base.hash)).unwrap();
        let frame_shared = |extra: &[u8]| {
            let data = [binary::payload(&original, 7).unwrap(), extra].concat();
            let mut out = vec![];
            for n in [data.len() as u32 + 20, 1, 806, 7, data.len() as u32] {
                out.extend(n.to_be_bytes());
            }
            out.extend(data);
            out.extend(1657u32.to_be_bytes());
            out
        };
        // Save counter 41 and Resource Explorer columns 22.
        let saved = frame_shared(&[0xc8, 2, 9, 0xb2, 1, 3, 10, 1, b'A']);
        assert!(matches_bytes(&saved, base, &f.cache).unwrap());
        fs::write(binary::resource_path(&f.stage, "shared").unwrap(), &saved).unwrap();
        assert!(
            f.plan().is_ok(),
            "metadata churn must not block incoming slices"
        );
        let authored = frame_shared(&[0x5a, 3, 0x12, 1, b'X']);
        assert!(!matches_bytes(&authored, base, &f.cache).unwrap());
        fs::write(f.cache.join(&base.hash), &saved).unwrap();
        assert!(
            matches_bytes(&saved, base, &f.cache).is_err(),
            "cache corruption still rejects"
        );
    }
    #[test]
    fn slice_availability_is_independent_of_account_stage_and_version_label() {
        assert!(!compatibility(None).direct_install);
        assert!(!compatibility(None).restore_versions);
        for (account, level, version) in [
            ("750933001", "1073742034", "7.1.0"),
            ("another-creator", "1073938434", "7.1.0"),
            ("another-creator", "1073938435", "7.2.0"),
            ("another-creator", "1073938436", ""),
        ] {
            let f = Fixture::for_recipient(account, level, version);
            let c = compatibility(Some(&f.stage));
            assert!(c.direct_install);
            assert!(!c.live_detection);
            assert!(c.restore_versions);
            let plan = f.plan().unwrap();
            let record = prepare(&plan, &f.stage, &f.cache, &f.backups).unwrap();
            transaction::apply(&record.journal, &f.cache).unwrap();
            assert_eq!(
                fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
                f.incoming
            );
            assert!(!f
                .stage
                .path
                .join("beyond_level_slice/1073938433_6.gis")
                .exists());
            fs::write(
                binary::resource_path(&f.stage, "5").unwrap(),
                framed(level.parse().unwrap(), Some(5), 2),
            )
            .unwrap();
            let saved = verify(&record, &f.stage, &f.cache, false).unwrap();
            transaction::authorize_saved_files(&record.journal, &saved).unwrap();
            transaction::restore(&record.journal).unwrap();
            verify(&record, &f.stage, &f.cache, true).unwrap();
            assert_eq!(
                fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
                f.old
            );
            assert_eq!(
                fs::read(f.stage.path.join("beyond_level_setting/local_setting.gils")).unwrap(),
                b"private selection"
            );
            assert_eq!(
                fs::read(f.stage.path.join(format!("{level}.gil"))).unwrap(),
                b"full stage unchanged"
            );
        }
    }
    #[test]
    fn recognized_layout_and_section_identity_remain_required_for_other_stages() {
        let mut f = Fixture::for_recipient("another-creator", "1073938434", "7.2.0");
        let mut unknown_layout = f.incoming.clone();
        unknown_layout[8..12].copy_from_slice(&807u32.to_be_bytes());
        for bytes in [unknown_layout, framed(1073938433, Some(6), 2)] {
            let revision = f.heads.get_mut("5").unwrap();
            revision.hash = hash(&bytes);
            revision.artifact =
                miliastra_core::history_artifact(&uuid::Uuid::new_v4().to_string(), "5").unwrap();
            revision.size = bytes.len() as u64;
            fs::write(f.cache.join(&revision.hash), &bytes).unwrap();
            let error = f
                .plan()
                .err()
                .expect("Invalid slices must be rejected")
                .to_string();
            assert!(
                error.contains("Unrecognized Miliastra format")
                    || error.contains("Slice section mismatch"),
                "{error}"
            );
            assert!(!f.backups.exists());
            assert_eq!(
                fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
                f.old
            );
        }
    }
    #[test]
    fn recipient_filename_import_preserves_bytes_and_settings() {
        let f = Fixture::new();
        let plan = f.plan().unwrap();
        let record = prepare(&plan, &f.stage, &f.cache, &f.backups).unwrap();
        transaction::apply(&record.journal, &f.cache).unwrap();
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
            f.incoming
        );
        assert!(!f
            .stage
            .path
            .join("beyond_level_slice/1073938433_6.gis")
            .exists());
        assert_eq!(
            fs::read(f.stage.path.join("beyond_level_setting/local_setting.gils")).unwrap(),
            b"private selection"
        );
        assert_eq!(
            fs::read(f.stage.path.join("1073742034.gil")).unwrap(),
            b"full stage unchanged"
        );
        assert_eq!(
            fs::read(
                record
                    .journal
                    .parent()
                    .unwrap()
                    .join("stage/beyond_level_slice/1073742034_6.gis")
            )
            .unwrap(),
            f.old
        );
    }
    #[test]
    fn id_only_editor_resave_verifies_and_remains_undoable() {
        let f = Fixture::new();
        let record = prepare(&f.plan().unwrap(), &f.stage, &f.cache, &f.backups).unwrap();
        transaction::apply(&record.journal, &f.cache).unwrap();
        fs::write(
            binary::resource_path(&f.stage, "5").unwrap(),
            framed(1073742034, Some(5), 2),
        )
        .unwrap();
        let approved = verify(&record, &f.stage, &f.cache, false).unwrap();
        transaction::authorize_saved_files(&record.journal, &approved).unwrap();
        transaction::restore(&record.journal).unwrap();
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
            f.old
        );
    }
    #[test]
    fn dirty_local_work_and_corrupt_downloads_are_rejected() {
        let f = Fixture::new();
        let dirty = framed(1073742034, Some(5), 9);
        fs::write(binary::resource_path(&f.stage, "5").unwrap(), &dirty).unwrap();
        assert!(f.plan().is_err());
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
            dirty
        );
        fs::write(binary::resource_path(&f.stage, "5").unwrap(), &f.old).unwrap();
        fs::write(f.cache.join(&f.heads["5"].hash), b"corrupt").unwrap();
        assert!(f.plan().is_err());
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
            f.old
        );
    }
    #[test]
    fn wrong_dependencies_and_undownloaded_shared_settings_block_the_batch() {
        let mut f = Fixture::new();
        f.heads.get_mut("5").unwrap().shared_dependency = Some("f".repeat(64));
        assert!(f.plan().is_err());
        f.heads.get_mut("5").unwrap().shared_dependency = Some(f.installed["shared"].hash.clone());
        f.heads.get_mut("shared").unwrap().hash = "e".repeat(64);
        assert!(f.plan().is_err(), "a shared update that hasn't downloaded");
        assert!(!f.backups.exists());
    }
    /// Shared settings from another stage install together with a slice that needs them,
    /// verify after reload, and undo cleanly.
    fn with_shared_update(f: &mut Fixture) -> Vec<u8> {
        let donor = framed(1073938433, None, 2);
        let h = hash(&donor);
        fs::write(f.cache.join(&h), &donor).unwrap();
        let mut shared = f.heads["shared"].clone();
        shared.parent = Some(shared.hash.clone());
        shared.hash = h.clone();
        shared.size = donor.len() as u64;
        shared.content_hash = binary::content_hash(
            &donor,
            "shared",
            &binary::resource_level(&donor, "shared").unwrap(),
        )
        .unwrap();
        f.heads.insert("shared".into(), shared);
        f.heads.get_mut("5").unwrap().shared_dependency = Some(h);
        donor
    }
    #[test]
    fn teammate_shared_settings_install_with_dependent_slices_and_undo() {
        let mut f = Fixture::new();
        let original_shared = fs::read(binary::resource_path(&f.stage, "shared").unwrap()).unwrap();
        let donor = with_shared_update(&mut f);
        let plan = f.plan().unwrap();
        assert_eq!(plan.revisions[0].section, "shared", "shared settings first");
        assert_eq!(plan.revisions.len(), 2);
        let record = prepare(&plan, &f.stage, &f.cache, &f.backups).unwrap();
        transaction::apply(&record.journal, &f.cache).unwrap();
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "shared").unwrap()).unwrap(),
            donor,
            "plain copy of the teammate's bytes"
        );
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
            f.incoming
        );
        let saved = verify(&record, &f.stage, &f.cache, false).unwrap();
        transaction::authorize_saved_files(&record.journal, &saved).unwrap();
        transaction::restore(&record.journal).unwrap();
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "shared").unwrap()).unwrap(),
            original_shared
        );
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "5").unwrap()).unwrap(),
            f.old
        );
    }
    #[test]
    fn unshared_local_shared_settings_block_installing_a_teammates_version() {
        let mut f = Fixture::new();
        with_shared_update(&mut f);
        let local = framed(1073742034, None, 7);
        fs::write(binary::resource_path(&f.stage, "shared").unwrap(), &local).unwrap();
        let error = f.plan().err().unwrap().to_string();
        assert!(error.contains("haven't shared"), "{error}");
        assert_eq!(
            fs::read(binary::resource_path(&f.stage, "shared").unwrap()).unwrap(),
            local
        );
        assert!(!f.backups.exists());
    }
    #[test]
    fn verification_refuses_real_edits_and_preserves_backup() {
        let f = Fixture::new();
        let record = prepare(&f.plan().unwrap(), &f.stage, &f.cache, &f.backups).unwrap();
        transaction::apply(&record.journal, &f.cache).unwrap();
        fs::write(
            binary::resource_path(&f.stage, "5").unwrap(),
            framed(1073742034, Some(5), 3),
        )
        .unwrap();
        assert!(verify(&record, &f.stage, &f.cache, false).is_err());
        assert!(transaction::restore(&record.journal).is_err());
        assert!(record.journal.parent().unwrap().join("stage").exists());
    }
}
