use crate::{runtime::Core, storage::Storage};
use anyhow::{ensure, Context, Result};
use miliastra_core::{binary, Invitation, Revision};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// The starting stage in team storage, which new teammates import first.
pub const BASE_STAGE: &str = "Base/stage.gil";
pub const BASE_MANIFEST: &str = "Base/stage.json";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseStage {
    pub name: String,
    pub size: u64,
    pub hash: String,
    /// Publication fingerprints; importing changes the stage ID contained in these hashes.
    pub slices: BTreeMap<String, String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCheck {
    /// The team folder has a starting stage this teammate can download.
    pub base_stage: bool,
}

/// Upload the stage's level file and its fingerprints as the starting stage.
pub(crate) async fn publish_base(
    storage: &Storage,
    stage: &binary::Stage,
    revisions: &BTreeMap<String, Revision>,
) -> Result<()> {
    let level = stage.path.join(format!("{}.gil", stage.level));
    ensure!(
        std::fs::metadata(&level)?.len() <= 128 * 1024 * 1024,
        "The stage is too large to share as a starting stage"
    );
    let bytes = std::fs::read(&level)?;
    let manifest = BaseStage {
        name: stage.name.clone(),
        size: bytes.len() as u64,
        hash: miliastra_core::hash(&bytes),
        slices: revisions
            .iter()
            .map(|(id, r)| (id.clone(), r.content_hash.clone()))
            .collect(),
    };
    storage.put_base(BASE_STAGE, bytes).await?;
    storage
        .put_base(BASE_MANIFEST, serde_json::to_vec_pretty(&manifest)?)
        .await
}

/// Compare complete level content using the same known editor-metadata exclusions
/// as shared-resource comparison. Imported stages receive a different local ID.
fn stage_import_hash(bytes: &[u8]) -> Result<String> {
    let fields = binary::fields(binary::payload(bytes, 2)?)?;
    let identity: Vec<_> = fields.iter().filter(|field| field.number == 1).collect();
    ensure!(
        identity.len() == 1 && identity[0].wire == 0,
        "Invalid stage identity"
    );
    let mut content = Vec::new();
    for field in &fields {
        content.extend(binary::comparison_field(field, "shared")?);
    }
    Ok(miliastra_core::hash(&content))
}
fn verify_base_stage(manifest: &BaseStage, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() as u64 <= miliastra_core::constants::MAX_ARTIFACT_BYTES
            && bytes.len() as u64 == manifest.size
            && miliastra_core::hash(bytes) == manifest.hash,
        "The starting stage failed its checksum. Ask the owner to share it again."
    );
    Ok(())
}
/// Only select a unique complete-content match. A matching name or empty slice
/// cannot establish that a local stage belongs to this project.
pub(crate) fn best_match(expected_hash: &str, stages: &[binary::Stage]) -> Option<PathBuf> {
    let mut matches = stages.iter().filter_map(|stage| {
        let bytes = std::fs::read(stage.path.join(format!("{}.gil", stage.level))).ok()?;
        (stage_import_hash(&bytes).ok()?.as_str() == expected_hash).then(|| stage.path.clone())
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// The provider's HTTP status behind a storage error, for the error message.
fn provider_reason(e: &anyhow::Error) -> String {
    e.chain()
        .find_map(|c| c.downcast_ref::<reqwest::Error>())
        .and_then(|r| r.status())
        .map(|status| format!(" (the storage service answered {status})"))
        .unwrap_or_default()
}

impl Core {
    /// Keep verified base bytes in the ordinary object cache, so import polling
    /// and exporting the download do not repeatedly transfer the level file.
    async fn starting_stage_bytes(
        &self,
        storage: &Storage,
        manifest: &BaseStage,
    ) -> Result<Vec<u8>> {
        let path = self.cache_path(&manifest.hash)?;
        if let Ok(bytes) = std::fs::read(path) {
            if verify_base_stage(manifest, &bytes).is_ok() {
                return Ok(bytes);
            }
        }
        let bytes = storage.get(BASE_STAGE).await?;
        verify_base_stage(manifest, &bytes)?;
        self.cache(&bytes)?;
        Ok(bytes)
    }
    fn invitation_folder(
        &self,
        invitation: &Invitation,
        folder: Option<PathBuf>,
    ) -> Option<PathBuf> {
        folder.or_else(|| {
            self.db
                .get::<Vec<crate::runtime::LocalProject>>("connections")
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .find(|p| p.project.id == invitation.project && p.endpoint == invitation.endpoint)
                .and_then(|p| p.folder)
        })
    }
    async fn invitation_storage(
        &self,
        invitation: &Invitation,
        folder: Option<PathBuf>,
    ) -> Result<Storage> {
        let info = self.invite_details(invitation.clone()).await?;
        Ok(Storage {
            config: info.storage,
            folder: self.invitation_folder(invitation, folder),
            oauth: self.settings.lock().unwrap().oauth.clone(),
        })
    }
    /// Checks team-folder access before joining. No stage is needed yet: a new teammate may
    /// still have to download and import the starting stage.
    pub async fn check_invitation_storage(
        &self,
        invitation: Invitation,
        folder: Option<PathBuf>,
        stage_path: Option<PathBuf>,
    ) -> Result<StorageCheck> {
        let storage = self.invitation_storage(&invitation, folder).await?;
        crate::storage::ensure_provider_available(&storage.config.kind)?;
        let settings = self.settings.lock().unwrap().clone();
        if storage.config.kind == "folder" {
            let root = storage
                .folder
                .as_ref()
                .context("Choose the project's shared folder")?;
            let protected = match &stage_path {
                Some(path) => Some(binary::inspect(path)?.path),
                None => settings.save_root.canonicalize().ok(),
            };
            if let Some(protected) = protected {
                super::runtime::validate_exchange(root, &protected, &settings.save_root)?;
            }
        }
        let marker: serde_json::Value = serde_json::from_slice(
            &storage.get("project.json").await.map_err(|e| {
                anyhow::anyhow!(
                    "Cannot read the team folder{}. Reconnect your account or ask the owner for folder access, then retry.",
                    provider_reason(&e)
                )
            })?,
        )?;
        ensure!(
            marker["id"].as_str() == Some(&invitation.project),
            "This folder belongs to a different project. Choose the invited project's folder."
        );
        storage.verify().await.map_err(|e| {
            anyhow::anyhow!(
                "Cannot write to the team folder{}. Ask the owner for edit access, then retry.",
                provider_reason(&e)
            )
        })?;
        Ok(StorageCheck {
            base_stage: storage.get(BASE_MANIFEST).await.is_ok(),
        })
    }
    /// Save the starting stage to Downloads and show it in Explorer, ready to import.
    pub async fn download_base_stage(
        &self,
        invitation: Invitation,
        folder: Option<PathBuf>,
    ) -> Result<PathBuf> {
        let storage = self.invitation_storage(&invitation, folder).await?;
        let manifest: BaseStage = serde_json::from_slice(&storage.get(BASE_MANIFEST).await?)?;
        let bytes = self.starting_stage_bytes(&storage, &manifest).await?;
        use tauri::Manager;
        let downloads = self
            .app
            .path()
            .download_dir()
            .unwrap_or_else(|_| self.data.join("exports"));
        #[cfg(debug_assertions)]
        let downloads = if std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some() {
            self.data.join("exports")
        } else {
            downloads
        };
        std::fs::create_dir_all(&downloads)?;
        let stem: String = manifest
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || " -_".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .take(80)
            .collect();
        let stem = if stem.trim().is_empty() {
            "Starting stage".into()
        } else {
            stem
        };
        let mut target = downloads.join(format!("{stem}.gil"));
        for n in 2.. {
            if !target.exists() {
                break;
            }
            target = downloads.join(format!("{stem} ({n}).gil"));
        }
        std::fs::write(&target, bytes)?;
        Ok(target)
    }
    /// The local stage that matches the project's starting stage, if one is imported.
    pub async fn find_project_stage(
        &self,
        invitation: Invitation,
        folder: Option<PathBuf>,
    ) -> Result<Option<PathBuf>> {
        let storage = self.invitation_storage(&invitation, folder).await?;
        let Ok(raw) = storage.get(BASE_MANIFEST).await else {
            return Ok(None);
        };
        let manifest: BaseStage = serde_json::from_slice(&raw)?;
        let bytes = self.starting_stage_bytes(&storage, &manifest).await?;
        let root = self.settings.lock().unwrap().save_root.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let expected_hash = stage_import_hash(&bytes)?;
            let stages = binary::discover(&root);
            Ok(best_match(&expected_hash, &stages))
        })
        .await?
    }
    /// Any member: share the current stage as the starting stage for new teammates.
    pub async fn publish_base_stage(&self) -> Result<()> {
        let p = self.active()?;
        let stage = binary::inspect(&p.stage.path)?;
        publish_base(&self.storage(&p), &stage, &p.project.heads).await?;
        self.changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn number(field: u64, value: u64) -> Vec<u8> {
        let mut out = varint(field << 3);
        out.append(&mut varint(value));
        out
    }
    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        while value >= 128 {
            out.push(value as u8 | 128);
            value >>= 7;
        }
        out.push(value as u8);
        out
    }
    fn text(field: u64, value: &[u8]) -> Vec<u8> {
        [
            varint((field << 3) | 2),
            varint(value.len() as u64),
            value.to_vec(),
        ]
        .concat()
    }
    fn level(id: u64, bookkeeping: u64, name: &str, asset: &str) -> Vec<u8> {
        let payload = [
            number(1, id),
            text(2, name.as_bytes()),
            text(4, &text(1, asset.as_bytes())),
            text(6, &number(8, 1)),
            number(40, bookkeeping),
            number(41, bookkeeping),
            text(46, &number(1, bookkeeping)),
            number(53, bookkeeping),
            text(90, b"unknown content"),
        ]
        .concat();
        frame(&payload)
    }
    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [payload.len() as u32 + 20, 1, 806, 2, payload.len() as u32] {
            bytes.extend(value.to_be_bytes());
        }
        bytes.extend(payload);
        bytes.extend(1657u32.to_be_bytes());
        bytes
    }
    fn stage(root: &std::path::Path, id: &str, bytes: &[u8]) -> binary::Stage {
        let path = root.join(id);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(format!("{id}.gil")), bytes).unwrap();
        binary::Stage {
            path,
            level: id.into(),
            account: "local".into(),
            name: "Same name".into(),
            sections: vec![],
            last_saved_section: None,
            settings_hash: String::new(),
            settings_version: String::new(),
            editor_version: "7.1.0".into(),
        }
    }
    #[test]
    fn imports_match_after_local_identity_and_editor_bookkeeping_change() {
        let original = level(1073742105, 1, "Starting stage", "Boat");
        let imported = level(1074462734, 9, "Renamed local copy", "Boat");
        assert_eq!(
            stage_import_hash(&original).unwrap(),
            stage_import_hash(&imported).unwrap()
        );
    }
    #[test]
    fn actual_asset_shared_and_unknown_changes_remain_significant() {
        let original = level(1, 1, "Same name", "Boat");
        let expected = stage_import_hash(&original).unwrap();
        assert_ne!(
            expected,
            stage_import_hash(&level(2, 2, "Same name", "Island")).unwrap()
        );
        let mut fields = binary::fields(binary::payload(&original, 2).unwrap()).unwrap();
        for field in [6, 90] {
            let saved = fields
                .iter()
                .find(|f| f.number == field)
                .unwrap()
                .raw
                .clone();
            fields.iter_mut().find(|f| f.number == field).unwrap().raw = text(field, &number(8, 2));
            let changed = frame(
                &fields
                    .iter()
                    .flat_map(|f| f.raw.clone())
                    .collect::<Vec<_>>(),
            );
            assert_ne!(
                expected,
                stage_import_hash(&changed).unwrap(),
                "field {field}"
            );
            fields.iter_mut().find(|f| f.number == field).unwrap().raw = saved;
        }
    }
    #[test]
    fn unique_full_content_match_is_required_even_when_names_match() {
        let root = std::env::temp_dir().join(format!("mcollab-import-{}", uuid::Uuid::new_v4()));
        let expected = stage_import_hash(&level(1, 1, "Same name", "Boat")).unwrap();
        let unrelated = stage(&root, "2", &level(2, 1, "Same name", "Island"));
        let imported = stage(&root, "3", &level(3, 2, "Same name", "Boat"));
        assert!(best_match(&expected, &[]).is_none());
        assert!(best_match(&expected, &[unrelated.clone()]).is_none());
        assert_eq!(
            best_match(&expected, &[unrelated, imported.clone()]),
            Some(imported.path.clone())
        );
        let duplicate = stage(&root, "4", &level(4, 3, "Same name", "Boat"));
        assert!(best_match(&expected, &[imported, duplicate]).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn partial_or_invalid_saves_are_skipped_until_a_complete_save() {
        let root = std::env::temp_dir().join(format!("mcollab-import-{}", uuid::Uuid::new_v4()));
        let bytes = level(1, 1, "World", "Boat");
        let expected = stage_import_hash(&bytes).unwrap();
        let partial = stage(&root, "2", &bytes[..bytes.len() - 1]);
        assert!(best_match(&expected, std::slice::from_ref(&partial)).is_none());
        std::fs::write(partial.path.join("2.gil"), &bytes).unwrap();
        assert_eq!(
            best_match(&expected, std::slice::from_ref(&partial)),
            Some(partial.path)
        );
        assert!(stage_import_hash(&frame(&text(4, b"no identity"))).is_err());
        assert!(stage_import_hash(&frame(&[number(1, 1), number(1, 2)].concat())).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn starting_stage_bytes_must_still_match_the_published_checksum_and_size() {
        let bytes = level(1, 1, "World", "Boat");
        let manifest = BaseStage {
            name: "World".into(),
            size: bytes.len() as u64,
            hash: miliastra_core::hash(&bytes),
            slices: BTreeMap::new(),
        };
        assert!(verify_base_stage(&manifest, &bytes).is_ok());
        assert!(verify_base_stage(&manifest, &level(2, 2, "World", "Boat")).is_err());
        assert!(verify_base_stage(&manifest, &bytes[..bytes.len() - 1]).is_err());
    }
}
