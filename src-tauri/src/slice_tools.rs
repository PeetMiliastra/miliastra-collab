//! Local slice tools (revert, move IDs). Nothing here talks to the coordinator.
use crate::{
    assets, installer,
    runtime::{Core, LocalProject},
};
use anyhow::{ensure, Context, Result};
use miliastra_core::{binary, hash, Revision};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertPreview {
    pub id: String,
    pub section: String,
    pub name: String,
    project: String,
    stage: std::path::PathBuf,
    original: String,
    baseline: Revision,
    at: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveId {
    pub token: String,
    pub old_id: u32,
    pub new_id: u32,
    pub name: String,
    pub source: String,
    pub target: String,
    pub expires: i64,
    project: String,
    stage: std::path::PathBuf,
    source_hash: String,
    target_hash: String,
}
fn ensure_slice(p: &LocalProject, section: &str) -> Result<()> {
    ensure!(
        section != "shared" && p.stage.sections.iter().any(|s| s.id == section),
        "Choose a known slice"
    );
    Ok(())
}
fn validate_preview(preview: &RevertPreview, p: &LocalProject, current: &[u8]) -> Result<()> {
    ensure!(
        preview.project == p.project.id
            && preview.stage.canonicalize()? == p.stage.path.canonicalize()?,
        "The selected project or stage changed. Review Revert again."
    );
    ensure!(
        chrono::Utc::now().timestamp() - preview.at <= miliastra_core::constants::PREVIEW_TTL_SECS,
        "This Revert preview expired. Review it again."
    );
    ensure!(
        p.installed
            .get(&preview.section)
            .is_some_and(|r| r.hash == preview.baseline.hash
                && r.content_hash == preview.baseline.content_hash),
        "The applied version changed. Review Revert again."
    );
    ensure!(
        hash(current) == preview.original,
        "This slice changed after confirmation opened. Review Revert again."
    );
    Ok(())
}
impl Core {
    pub async fn inspect_slice(&self, section: &str) -> Result<assets::Inspection> {
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure!(miliastra_core::valid_section(section), "Unknown slice");
        let files = self.stable(&p.stage).await?;
        let current = files.get(section).context("Unknown local slice")?;
        let baseline = if let Some(revision) = p.installed.get(section) {
            let path = self.cache_path(&revision.hash)?;
            if path.exists() {
                let bytes = std::fs::read(path)?;
                ensure!(
                    hash(&bytes) == revision.hash,
                    "Cached baseline failed validation"
                );
                Some(bytes)
            } else {
                None
            }
        } else {
            None
        };
        assets::inspect(current, baseline.as_deref(), section)
    }
    pub async fn preview_revert(&self, section: &str) -> Result<RevertPreview> {
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure!(miliastra_core::valid_section(section), "Unknown slice");
        ensure!(
            self.installation(&p)?.is_none_or(|r| !r.pending()),
            "Finish applying the pending update, or recover it from History"
        );
        let baseline = p
            .installed
            .get(section)
            .context("There is no applied version to revert to")?
            .clone();
        if !self.cache_path(&baseline.hash)?.exists() {
            let bytes = self.storage(&p).get(&baseline.artifact).await?;
            ensure!(
                hash(&bytes) == baseline.hash && bytes.len() as u64 == baseline.size,
                "The applied version failed validation"
            );
            self.cache(&bytes)?;
        }
        let saved = std::fs::read(self.cache_path(&baseline.hash)?)?;
        ensure!(
            baseline.section == section
                && hash(&saved) == baseline.hash
                && saved.len() as u64 == baseline.size,
            "The applied version failed checksum validation"
        );
        let saved_level = binary::resource_level(&saved, section)?;
        ensure!(
            binary::content_hash(&saved, section, &saved_level)? == baseline.content_hash,
            "The applied version failed content validation"
        );
        let files = self.stable(&p.stage).await?;
        let current = files.get(section).context("Unknown local slice")?;
        ensure!(
            !installer::matches_bytes(current, &baseline, &self.data.join("objects"))?,
            "This slice has no unshared changes"
        );
        let preview = RevertPreview {
            id: uuid::Uuid::new_v4().to_string(),
            section: section.into(),
            name: crate::names::effective_names(&p.project)
                .get(section)
                .cloned()
                .unwrap_or(baseline.name.clone()),
            project: p.project.id.clone(),
            stage: p.stage.path.clone(),
            original: hash(current),
            baseline,
            at: chrono::Utc::now().timestamp(),
        };
        self.db
            .set(&format!("revert-preview:{}", preview.id), &preview)?;
        Ok(preview)
    }
    pub async fn revert_slice(&self, id: &str, editor_ready: bool) -> Result<()> {
        ensure!(
            editor_ready,
            "Save and unload the stage before reverting a slice"
        );
        uuid::Uuid::parse_str(id)?;
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure!(
            self.installation(&p)?.is_none_or(|r| !r.pending()),
            "Finish applying the pending update, or recover it from History"
        );
        let preview: RevertPreview = self
            .db
            .get(&format!("revert-preview:{id}"))?
            .context("Review this slice before reverting")?;
        let stage = binary::inspect(&p.stage.path)?;
        let files = self.stable(&stage).await?;
        let current = files.get(&preview.section).context("Unknown local slice")?;
        validate_preview(&preview, &p, current)?;
        // Reverts to the version last applied on this device. A later update to the shared
        // settings doesn't change that; sharing and installing check the dependency themselves.
        let local = local_revision(&preview.baseline, current, self.cache(current)?)?;
        let plan = installer::Plan {
            id: id.into(),
            project: p.project.id.clone(),
            stage: stage.path.clone(),
            revisions: vec![preview.baseline.clone()],
            originals: BTreeMap::from([(preview.section.clone(), preview.original.clone())]),
            previous: BTreeMap::from([(preview.section.clone(), local)]),
            at: preview.at,
        };
        let mut record = installer::prepare_files(
            &plan,
            &stage,
            &self.data.join("objects"),
            &self.data.join("backups"),
        )?;
        record.local_revert = true;
        record.undo_baselines = Some(p.installed.clone());
        self.save_installation(&record)?;
        let result = crate::transaction::apply(&record.journal, &self.data.join("objects"));
        record.phase = if result.is_ok() {
            crate::installer::InstallationPhase::AwaitingReload
        } else {
            crate::installer::InstallationPhase::Interrupted
        };
        record.error = result.as_ref().err().map(ToString::to_string);
        self.save_installation(&record)?;
        result?;
        self.db.remove(&format!("revert-preview:{id}"))?;
        self.finish_installation(&mut record).await
    }
    pub async fn generate_move_id(
        &self,
        source: &str,
        asset_key: &str,
        target: &str,
    ) -> Result<MoveId> {
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure_slice(&p, source)?;
        ensure_slice(&p, target)?;
        ensure!(source != target, "Choose another slice");
        let files = self.stable(&p.stage).await?;
        let current = files.get(source).context("Unknown source slice")?;
        let inspection = assets::inspect(current, None, source)?;
        let asset = inspection
            .assets
            .into_iter()
            .find(|a| a.key == asset_key)
            .context("That asset changed or is no longer in this slice")?;
        ensure!(
            asset.section.to_string() == source,
            "The asset ID does not belong to this slice"
        );
        let mut used = all_ids(&files)?;
        let now = chrono::Utc::now().timestamp();
        for raw in self
            .db
            .raw_values_with_prefix(&format!("move-id:{}:", p.project.id))?
        {
            let saved: MoveId = serde_json::from_str(&raw)?;
            if saved.expires > now {
                used.insert(saved.new_id);
            } else {
                self.db
                    .remove(&format!("move-id:{}:{}", p.project.id, saved.token))?;
            }
        }
        let new_id = assets::candidate(&asset, target.parse()?, &used)?;
        let result = MoveId {
            token: uuid::Uuid::new_v4().to_string(),
            old_id: asset.id,
            new_id,
            name: asset.name,
            source: source.into(),
            target: target.into(),
            expires: now + miliastra_core::constants::PREVIEW_TTL_SECS,
            project: p.project.id.clone(),
            stage: p.stage.path.clone(),
            source_hash: hash(current),
            target_hash: hash(files.get(target).context("Unknown target slice")?),
        };
        self.db.set(
            &format!("move-id:{}:{}", p.project.id, result.token),
            &result,
        )?;
        self.changed();
        Ok(result)
    }
    pub async fn validate_move_id(&self, token: &str) -> Result<String> {
        uuid::Uuid::parse_str(token)?;
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        let saved: MoveId = self
            .db
            .get(&format!("move-id:{}:{token}", p.project.id))?
            .context("Generate a new ID first")?;
        let files = self.stable(&p.stage).await?;
        validate_move_files(&saved, &p, &files, chrono::Utc::now().timestamp())?;
        Ok(saved.new_id.to_string())
    }
}
/// Re-checks the local files right before copying.
fn validate_move_files(
    saved: &MoveId,
    p: &LocalProject,
    files: &BTreeMap<String, Vec<u8>>,
    now: i64,
) -> Result<()> {
    ensure!(
        saved.project == p.project.id
            && saved.stage.canonicalize()? == p.stage.path.canonicalize()?
            && saved.expires > now,
        "The generated ID expired or the stage changed. Generate it again."
    );
    ensure!(
        files
            .get(&saved.source)
            .is_some_and(|b| hash(b) == saved.source_hash)
            && files
                .get(&saved.target)
                .is_some_and(|b| hash(b) == saved.target_hash),
        "One of these slices changed. Generate a new ID."
    );
    ensure!(
        !all_ids(&files)?.contains(&saved.new_id),
        "This ID is already in use. Generate another ID."
    );
    Ok(())
}
fn all_ids(files: &BTreeMap<String, Vec<u8>>) -> Result<BTreeSet<u32>> {
    let mut used = BTreeSet::new();
    for (section, bytes) in files {
        used.extend(assets::used_ids(bytes, section)?);
    }
    Ok(used)
}
fn local_revision(baseline: &Revision, bytes: &[u8], fingerprint: String) -> Result<Revision> {
    let level = binary::resource_level(bytes, &baseline.section)?;
    Ok(Revision {
        hash: fingerprint,
        content_hash: binary::content_hash(bytes, &baseline.section, &level)?,
        size: bytes.len() as u64,
        ..baseline.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn vi(mut n: u64) -> Vec<u8> {
        let mut o = vec![];
        while n >= 128 {
            o.push(n as u8 | 128);
            n >>= 7;
        }
        o.push(n as u8);
        o
    }
    fn field(n: u64, b: &[u8]) -> Vec<u8> {
        [vi(n << 3 | 2), vi(b.len() as u64), b.to_vec()].concat()
    }
    fn bytes(label: &str) -> Vec<u8> {
        let body = [vi(8), vi(501), field(90, label.as_bytes())].concat();
        let data = [vi(8), vi(5), field(2, &body)].concat();
        let mut o = vec![];
        for n in [data.len() as u32 + 20, 1, 806, 6, data.len() as u32] {
            o.extend(n.to_be_bytes());
        }
        o.extend(data);
        o.extend(1657u32.to_be_bytes());
        o
    }
    fn setup() -> (std::path::PathBuf, LocalProject, Revision, Vec<u8>) {
        let root = std::env::temp_dir().join(format!("mcollab-revert-{}", uuid::Uuid::new_v4()));
        let stage = root.join("501");
        fs::create_dir_all(stage.join("beyond_level_slice")).unwrap();
        let saved = bytes("Last applied");
        let current = bytes("Unshared accidental edit");
        fs::write(stage.join("beyond_level_slice/501_6.gis"), &current).unwrap();
        fs::write(stage.join("sibling"), b"Other slices and settings stay").unwrap();
        let stage = binary::Stage {
            path: stage,
            level: "501".into(),
            account: "Synthetic".into(),
            name: "Stage".into(),
            sections: vec![],
            last_saved_section: None,
            settings_hash: "".into(),
            settings_version: "".into(),
            editor_version: "".into(),
        };
        let baseline = Revision {
            section: "5".into(),
            name: "Boats".into(),
            hash: hash(&saved),
            content_hash: binary::content_hash(&saved, "5", "501").unwrap(),
            parent: None,
            artifact: "ignored".into(),
            size: saved.len() as u64,
            shared_dependency: None,
        };
        let p=LocalProject{endpoint:"http://127.0.0.1:9".into(),project:serde_json::from_value(serde_json::json!({"id":"project","name":"Synthetic","storage":{"kind":"folder","location":""},"members":[],"locks":[],"heads":{},"sequence":1})).unwrap(),member:"me".into(),stage,folder:None,installed:BTreeMap::from([("5".into(),baseline.clone())])};
        fs::create_dir(root.join("cache")).unwrap();
        fs::create_dir(root.join("backups")).unwrap();
        fs::write(root.join("cache").join(&baseline.hash), saved).unwrap();
        (root, p, baseline, current)
    }
    fn preview(p: &LocalProject, b: &Revision, current: &[u8]) -> RevertPreview {
        RevertPreview {
            id: uuid::Uuid::new_v4().to_string(),
            section: "5".into(),
            name: "Boats".into(),
            project: p.project.id.clone(),
            stage: p.stage.path.clone(),
            original: hash(current),
            baseline: b.clone(),
            at: chrono::Utc::now().timestamp(),
        }
    }
    #[test]
    fn local_id_copy_checks_context_and_saved_bytes_without_shared_state() {
        let (root, mut p, _, current) = setup();
        let target = bytes("Target saved bytes");
        let files = BTreeMap::from([("5".into(), current.clone()), ("6".into(), target.clone())]);
        let mut proposal = MoveId {
            token: uuid::Uuid::new_v4().to_string(),
            old_id: 1078263809,
            new_id: 1078329345,
            name: "Saved asset".into(),
            source: "5".into(),
            target: "6".into(),
            expires: 200,
            project: p.project.id.clone(),
            stage: p.stage.path.clone(),
            source_hash: hash(&current),
            target_hash: hash(&target),
        };
        assert!(p.project.heads.is_empty() && p.project.locks.is_empty());
        assert!(validate_move_files(&proposal, &p, &files, 100).is_ok());
        let mut changed = files.clone();
        changed.insert("6".into(), bytes("Later edit"));
        assert!(validate_move_files(&proposal, &p, &changed, 100).is_err());
        changed = files.clone();
        changed.insert("5".into(), bytes("Later source edit"));
        assert!(validate_move_files(&proposal, &p, &changed, 100).is_err());
        assert!(validate_move_files(&proposal, &p, &files, 201).is_err());
        p.project.id = "another project".into();
        assert!(validate_move_files(&proposal, &p, &files, 100).is_err());
        p.project.id = proposal.project.clone();
        proposal.stage = root.clone();
        assert!(validate_move_files(&proposal, &p, &files, 100).is_err());
        assert!(
            root.starts_with(std::env::temp_dir())
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("mcollab-revert-")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn revert_previews_reject_later_edits_different_projects_baselines_and_expiry() {
        let (root, mut p, b, current) = setup();
        let mut v = preview(&p, &b, &current);
        assert!(validate_preview(&v, &p, &current).is_ok());
        assert!(validate_preview(&v, &p, &bytes("Later edit")).is_err());
        v.at -= 601;
        assert!(validate_preview(&v, &p, &current).is_err());
        v.at += 601;
        p.project.id = "another".into();
        assert!(validate_preview(&v, &p, &current).is_err());
        p.project.id = v.project.clone();
        p.installed.get_mut("5").unwrap().hash = "a".repeat(64);
        assert!(validate_preview(&v, &p, &current).is_err());
        assert!(
            root.starts_with(std::env::temp_dir())
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("mcollab-revert-")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn revert_backup_restores_unshared_edits_without_changing_other_files_or_baseline() {
        let (root, p, b, current) = setup();
        let cache = root.join("cache");
        let local = local_revision(&b, &current, hash(&current)).unwrap();
        fs::write(cache.join(&local.hash), &current).unwrap();
        let v = preview(&p, &b, &current);
        let plan = installer::Plan {
            id: v.id,
            project: p.project.id.clone(),
            stage: p.stage.path.clone(),
            revisions: vec![b.clone()],
            originals: BTreeMap::from([("5".into(), hash(&current))]),
            previous: BTreeMap::from([("5".into(), local)]),
            at: v.at,
        };
        let mut record =
            installer::prepare_files(&plan, &p.stage, &cache, &root.join("backups")).unwrap();
        record.local_revert = true;
        record.undo_baselines = Some(p.installed.clone());
        crate::transaction::apply(&record.journal, &cache).unwrap();
        let checked = installer::verify(&record, &p.stage, &cache, false).unwrap();
        crate::transaction::authorize_saved_files(&record.journal, &checked).unwrap();
        assert_eq!(
            fs::read(binary::resource_path(&p.stage, "5").unwrap()).unwrap(),
            bytes("Last applied")
        );
        assert_eq!(p.installed["5"].hash, b.hash);
        crate::transaction::restore(&record.journal).unwrap();
        installer::verify(&record, &p.stage, &cache, true).unwrap();
        assert_eq!(
            fs::read(binary::resource_path(&p.stage, "5").unwrap()).unwrap(),
            current
        );
        assert_eq!(
            fs::read(p.stage.path.join("sibling")).unwrap(),
            b"Other slices and settings stay"
        );
        assert_eq!(record.undo_baselines.unwrap()["5"].hash, b.hash);
        assert!(
            root.starts_with(std::env::temp_dir())
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("mcollab-revert-")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
