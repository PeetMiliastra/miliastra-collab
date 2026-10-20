//! Names, changelogs and update manifests stored in the team folder, matched up with the
//! coordinator's receipts.
use crate::{
    runtime::{Core, LocalProject},
    storage::Storage,
};
use anyhow::{ensure, Context, Result};
use miliastra_core::constants::{text_fits, MAX_DETAILS_UNITS, MAX_NAME_UNITS, MAX_SUMMARY_UNITS};
use miliastra_core::{binary::Stage, control as wire, *};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
pub const WAITING:&str="Waiting for update details from the team folder. Cached history and your saved work are preserved.";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectDocument {
    pub version: u32,
    pub nonce: String,
    pub project: String,
    pub name: String,
    pub storage: StorageConfig,
    pub owner: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub nonce: String,
    pub project: String,
    pub member: String,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub nonce: String,
    pub project: String,
    pub update: Update,
    pub controls: Vec<wire::Revision>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Head {
    control: wire::Revision,
    file: Revision,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Hydration {
    sequence: u64,
    heads: BTreeMap<String, Head>,
    names: BTreeMap<String, String>,
    version: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct MetadataDownload {
    path: String,
    hash: String,
    retry: i64,
    attempts: u32,
    issue: Option<crate::recovery::StorageIssue>,
}
#[derive(Clone, Serialize, Deserialize)]
struct PendingMembership {
    joined: wire::Joined,
    config: StorageConfig,
}

pub(crate) fn view_binding(project: &ProjectView) -> Result<String> {
    Ok(hash(&serde_json::to_vec(
        &json!({"sequence":project.sequence,"heads":project.heads,"names":project.section_names,"version":project.names_version}),
    )?))
}
pub fn manifest_path(id: &str) -> Result<String> {
    uuid::Uuid::parse_str(id)?;
    Ok(format!("History/{id}/manifest.json"))
}
pub fn profile_path(id: &str, hash: &str) -> Result<String> {
    uuid::Uuid::parse_str(id)?;
    wire::fingerprint(hash)?;
    Ok(format!("Members/{id}/{hash}.json"))
}
/// Checks a manifest against the coordinator's receipt, including every revision link.
pub fn verify_manifest(project: &str, receipt: &wire::Update, bytes: &[u8]) -> Result<Manifest> {
    ensure!(
        hash(bytes) == receipt.manifest_hash,
        "Update details failed fingerprint validation"
    );
    let manifest: Manifest = serde_json::from_slice(bytes)?;
    uuid::Uuid::parse_str(&manifest.nonce)?;
    ensure!(
        manifest.version == 1 && manifest.project == project,
        "Update details belong to another project"
    );
    let u = &manifest.update;
    ensure!(
        u.id == receipt.id
            && u.author_id == receipt.author_id
            && u.at == 0
            && u.sequence == 0
            && u.restores == receipt.restores
            && u.names_parent == receipt.names_parent,
        "Publication receipt does not match its manifest"
    );
    ensure!(
        !u.summary.trim().is_empty()
            && text_fits(&u.summary, MAX_SUMMARY_UNITS)
            && text_fits(&u.details, MAX_DETAILS_UNITS)
            && !u.author.trim().is_empty()
            && text_fits(&u.author, MAX_NAME_UNITS),
        "Invalid update details"
    );
    ensure!(
        manifest.controls == receipt.revisions && u.revisions.len() == receipt.revisions.len(),
        "Publication revisions do not match"
    );
    for (r, c) in u.revisions.iter().zip(&manifest.controls) {
        validate_revision(r)?;
        wire::revision(c)?;
        ensure!(
            r.artifact == history_artifact(&u.id, &r.section)?,
            "Invalid immutable artifact"
        );
    }
    ensure!(
        u.name_changes.len() == receipt.name_sections.len(),
        "Name changes do not match"
    );
    for c in &u.name_changes {
        ensure!(
            c.section != "shared" && valid_section(&c.section),
            "Invalid slice name"
        );
        validate_name(&c.before)?;
        validate_name(&c.after)?;
    }
    ensure!(
        u.names_parent.is_some() == !u.name_changes.is_empty()
            && u.name_changes.is_empty() == !u.revisions.is_empty(),
        "Invalid publication kind"
    );
    Ok(manifest)
}
impl Core {
    fn metadata_nonce(&self, key: &str) -> Result<String> {
        let key = format!("metadata-nonce:{key}");
        if let Some(nonce) = self.db.get::<String>(&key)? {
            return Ok(nonce);
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        self.db.set(&key, &nonce)?;
        Ok(nonce)
    }
    pub fn metadata_storage_issue(
        &self,
        project: &str,
    ) -> Result<Option<crate::recovery::StorageIssue>> {
        for value in self
            .db
            .raw_values_with_prefix(&format!("metadata-download:{project}:"))?
        {
            let item: MetadataDownload = serde_json::from_str(&value)?;
            if item.issue.is_some() {
                return Ok(item.issue);
            }
        }
        Ok(None)
    }
    pub fn retry_team_metadata(&self, project: &str) -> Result<()> {
        for value in self
            .db
            .raw_values_with_prefix(&format!("metadata-download:{project}:"))?
        {
            let mut item: MetadataDownload = serde_json::from_str(&value)?;
            item.retry = 0;
            self.db
                .set(&format!("metadata-download:{project}:{}", item.hash), &item)?;
        }
        Ok(())
    }
    pub async fn download_team_metadata(&self, p: &LocalProject) -> Result<bool> {
        let mut downloaded = false;
        for value in self
            .db
            .raw_values_with_prefix(&format!("metadata-download:{}:", p.project.id))?
        {
            let mut item: MetadataDownload = serde_json::from_str(&value)?;
            if item.retry > chrono::Utc::now().timestamp() {
                continue;
            }
            if !self
                .active
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|a| a.project.id == p.project.id && a.endpoint == p.endpoint)
            {
                break;
            }
            let key = format!("metadata-download:{}:{}", p.project.id, item.hash);
            let result = async {
                let bytes = self.storage(p).get(&item.path).await?;
                ensure!(
                    bytes.len() <= miliastra_core::constants::MAX_METADATA_BYTES
                        && hash(&bytes) == item.hash,
                    "Update details failed fingerprint validation"
                );
                self.cache(&bytes)?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            match result {
                Ok(()) => {
                    self.db.remove(&key)?;
                    downloaded = true;
                }
                Err(error) => {
                    item.issue = Some(crate::recovery::classify(&error));
                    item.attempts = item.attempts.saturating_add(1);
                    item.retry = chrono::Utc::now().timestamp() + (2i64 << item.attempts.min(5));
                    self.db.set(&key, &item)?;
                }
            }
        }
        Ok(downloaded)
    }
    pub fn complete_metadata_view(&self, p: &LocalProject) -> Result<()> {
        if self
            .db
            .get::<String>(&format!("verified-view:{}", p.project.id))?
            .as_ref()
            == Some(&view_binding(&p.project)?)
        {
            self.db
                .set(&format!("details-pending:{}", p.project.id), &false)?;
        }
        Ok(())
    }
    pub fn metadata_view_pending(&self, p: &LocalProject) -> Result<bool> {
        Ok(self
            .db
            .get::<bool>(&format!("details-pending:{}", p.project.id))?
            .unwrap_or(false)
            || self
                .db
                .get::<String>(&format!("verified-view:{}", p.project.id))?
                .as_ref()
                != Some(&view_binding(&p.project)?))
    }
    pub fn details_ready(&self, p: &LocalProject) -> Result<()> {
        ensure!(!self.metadata_view_pending(p)?, "{WAITING}");
        Ok(())
    }
    async fn metadata_bytes(
        &self,
        p: &LocalProject,
        path: &str,
        expected: &str,
    ) -> Result<Vec<u8>> {
        wire::fingerprint(expected)?;
        if let Ok(bytes) = std::fs::read(self.cache_path(expected)?) {
            if hash(&bytes) == expected {
                return Ok(bytes);
            }
        }
        if p.project.sequence > 0 {
            let key = format!("metadata-download:{}:{expected}", p.project.id);
            if self.db.get::<MetadataDownload>(&key)?.is_none() {
                self.db.set(
                    &key,
                    &MetadataDownload {
                        path: path.into(),
                        hash: expected.into(),
                        retry: 0,
                        attempts: 0,
                        issue: None,
                    },
                )?;
            }
            anyhow::bail!("{WAITING}");
        }
        let bytes = self.storage(p).get(path).await?;
        ensure!(
            bytes.len() <= miliastra_core::constants::MAX_METADATA_BYTES
                && hash(&bytes) == expected,
            "Shared metadata failed fingerprint validation"
        );
        self.cache(&bytes)?;
        Ok(bytes)
    }
    pub async fn store_profile(
        &self,
        storage: &Storage,
        project: &str,
        member: &str,
        name: &str,
    ) -> Result<String> {
        ensure!(
            !name.trim().is_empty() && text_fits(name, MAX_NAME_UNITS),
            "Enter your display name"
        );
        let bytes = serde_json::to_vec(&Profile {
            version: 1,
            nonce: self.metadata_nonce(&format!("profile:{project}:{member}"))?,
            project: project.into(),
            member: member.into(),
            name: name.into(),
        })?;
        let h = hash(&bytes);
        storage
            .put(&profile_path(member, &h)?, bytes.clone())
            .await?;
        self.cache(&bytes)?;
        Ok(h)
    }
    fn manifest_update(&self, p: &LocalProject, req: &PublishRequest) -> Update {
        Update {
            id: req.id.clone(),
            author: p
                .project
                .members
                .iter()
                .find(|m| m.id == p.member)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Creator".into()),
            author_id: p.member.clone(),
            summary: req.summary.clone(),
            details: req.details.clone(),
            at: 0,
            sequence: 0,
            revisions: req.revisions.clone(),
            restores: req.restores.clone(),
            name_changes: vec![],
            names_parent: None,
        }
    }
    async fn stage_manifest(&self, p: &LocalProject, update: Update) -> Result<Manifest> {
        let key = format!("staged-manifest:{}:{}", p.project.id, update.id);
        if let Some(saved) = self.db.get::<Manifest>(&key)? {
            ensure!(
                serde_json::to_value(&saved.update)? == serde_json::to_value(&update)?,
                "Update ID already used for different details"
            );
            return Ok(saved);
        }
        let heads = self
            .db
            .get::<BTreeMap<String, Head>>(&format!("control-heads:{}", p.project.id))?
            .unwrap_or_default();
        let mut ids = BTreeMap::new();
        for r in &update.revisions {
            ids.insert(r.section.clone(), uuid::Uuid::new_v4().to_string());
        }
        let keys = self.privacy_keys(&p.endpoint, &p.project.id)?;
        let mut controls = vec![];
        for r in &update.revisions {
            validate_revision(r)?;
            let parent = heads.get(&r.section);
            ensure!(
                parent.map(|h| h.file.hash.clone()) == r.parent,
                "This slice changed since you started; your draft is preserved"
            );
            let dependency =
                if let Some(shared) = update.revisions.iter().find(|s| s.section == "shared") {
                    ensure!(
                        r.section == "shared" || r.shared_dependency.as_ref() == Some(&shared.hash),
                        "Shared settings changed"
                    );
                    ids.get("shared").cloned()
                } else {
                    let shared = heads.get("shared");
                    ensure!(
                        r.section == "shared"
                            || r.shared_dependency == shared.map(|h| h.file.hash.clone()),
                        "Shared settings changed"
                    );
                    shared.map(|h| h.control.revision.clone())
                };
            controls.push(wire::Revision {
                section: keys.resource(&p.endpoint, &p.project.id, &r.section)?,
                revision: ids[&r.section].clone(),
                parent: parent.map(|h| h.control.revision.clone()),
                shared_dependency: if r.section == "shared" {
                    None
                } else {
                    dependency
                },
            });
        }
        let manifest = Manifest {
            version: 1,
            nonce: uuid::Uuid::new_v4().to_string(),
            project: p.project.id.clone(),
            update,
            controls,
        };
        self.db.set(&key, &manifest)?;
        Ok(manifest)
    }
    async fn upload_manifest(&self, p: &LocalProject, manifest: &Manifest) -> Result<String> {
        let bytes = serde_json::to_vec(manifest)?;
        self.storage(p)
            .put(&manifest_path(&manifest.update.id)?, bytes.clone())
            .await?;
        let fingerprint = self.cache(&bytes)?;
        Ok(fingerprint)
    }
    pub async fn prepare_control_create(
        &self,
        ep: &str,
        raw: &CreateProject,
        storage: &Storage,
        stage: Stage,
        folder: Option<PathBuf>,
    ) -> Result<wire::CreateProject> {
        let profile_hash = self
            .store_profile(storage, &raw.id, &raw.member_id, &raw.display_name)
            .await?;
        let document = ProjectDocument {
            version: 1,
            nonce: self.metadata_nonce(&format!("project:{}", raw.id))?,
            project: raw.id.clone(),
            name: raw.name.clone(),
            storage: raw.storage.clone(),
            owner: raw.member_id.clone(),
        };
        ensure!(
            !document.name.trim().is_empty() && text_fits(&document.name, MAX_NAME_UNITS),
            "Enter your project name"
        );
        let bytes = serde_json::to_vec(&document)?;
        let project_hash = hash(&bytes);
        storage.put("Metadata/project.json", bytes.clone()).await?;
        self.cache(&bytes)?;
        let project = ProjectView {
            id: raw.id.clone(),
            name: raw.name.clone(),
            storage: raw.storage.clone(),
            members: vec![Member {
                id: raw.member_id.clone(),
                name: raw.display_name.clone(),
                owner: true,
                joined: 0,
            }],
            locks: vec![],
            heads: BTreeMap::new(),
            sequence: 0,
            section_names: BTreeMap::new(),
            names_version: 0,
            presence: vec![],
        };
        let p = LocalProject {
            endpoint: ep.into(),
            project,
            member: raw.member_id.clone(),
            stage,
            folder,
            installed: BTreeMap::new(),
        };
        let req = PublishRequest {
            id: raw.baseline_id.clone(),
            summary: raw.summary.clone(),
            details: raw.details.clone(),
            revisions: raw.baseline.clone(),
            fences: BTreeMap::new(),
            restores: None,
        };
        let manifest = self
            .stage_manifest(&p, self.manifest_update(&p, &req))
            .await?;
        let manifest_hash = self.upload_manifest(&p, &manifest).await?;
        let bootstrap =
            self.privacy_keys(ep, &raw.id)?
                .seal(ep, &raw.id, "storage-bootstrap", &raw.storage)?;
        Ok(wire::CreateProject {
            id: raw.id.clone(),
            member_id: raw.member_id.clone(),
            profile_hash,
            project_hash,
            bootstrap,
            baseline: manifest.controls,
            baseline_id: raw.baseline_id.clone(),
            manifest_hash,
        })
    }
    fn pending_membership_key(ep: &str, project: &str) -> String {
        format!("pending-membership:{ep}:{project}")
    }
    pub fn pending_membership(
        &self,
        ep: &str,
        project: &str,
    ) -> Result<Option<(wire::Joined, StorageConfig)>> {
        let Some(mut saved) = self
            .db
            .get::<PendingMembership>(&Self::pending_membership_key(ep, project))?
        else {
            return Ok(None);
        };
        saved.joined.token = crate::oauth::secret_get(&format!("membership:{ep}:{project}"))?;
        Ok(Some((saved.joined, saved.config)))
    }
    pub fn finish_membership(&self, ep: &str, project: &str) -> Result<()> {
        self.db.remove(&Self::pending_membership_key(ep, project))
    }
    pub async fn hydrate_joined(
        &self,
        ep: &str,
        wire: wire::Joined,
        config: StorageConfig,
        stage: Stage,
        folder: Option<PathBuf>,
    ) -> Result<Joined> {
        let mut saved = wire.clone();
        saved.token.clear();
        self.db.set(
            &Self::pending_membership_key(ep, &wire.project.id),
            &PendingMembership {
                joined: saved,
                config: config.clone(),
            },
        )?;
        let project = ProjectView {
            id: wire.project.id.clone(),
            name: String::new(),
            storage: config,
            members: vec![],
            locks: vec![],
            heads: BTreeMap::new(),
            sequence: 0,
            section_names: BTreeMap::new(),
            names_version: 0,
            presence: vec![],
        };
        let p = LocalProject {
            endpoint: ep.into(),
            project,
            member: wire.member.clone(),
            stage,
            folder,
            installed: BTreeMap::new(),
        };
        let latest: wire::ProjectView = serde_json::from_value(
            self.control_http(&p, reqwest::Method::GET, "", None)
                .await?,
        )?;
        let project = self.hydrate_control_view(&p, latest).await?;
        ensure!(
            project.members.iter().any(|m| m.id == wire.member),
            "Not a member"
        );
        ensure!(
            self.db
                .get::<String>(&format!("verified-view:{}", p.project.id))?
                .as_ref()
                == Some(&view_binding(&project)?),
            "{WAITING}"
        );
        Ok(Joined {
            project,
            member: wire.member,
            token: wire.token,
            invitation: wire.invitation,
        })
    }
    pub async fn control_command(
        &self,
        p: &LocalProject,
        route: &str,
        value: Value,
    ) -> Result<Value> {
        if route == "/invite" {
            return Ok(
                json!({"bootstrap":self.privacy_keys(&p.endpoint,&p.project.id)?.seal(&p.endpoint,&p.project.id,"storage-bootstrap",&p.project.storage)?}),
            );
        }
        if route == "/join" {
            let member = value["memberId"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let name = value["displayName"]
                .as_str()
                .context("Enter your display name")?;
            let profile_hash = self
                .store_profile(&self.storage(p), &p.project.id, &member, name)
                .await?;
            return Ok(
                json!({"invite":value["invite"],"memberId":member,"profileHash":profile_hash}),
            );
        }
        if route == "/publish" || route == "/names" {
            self.details_ready(p)?;
            let (update, fences) = if route == "/publish" {
                let req: PublishRequest = serde_json::from_value(value)?;
                (self.manifest_update(p, &req), req.fences)
            } else {
                let req: PublishNames = serde_json::from_value(value)?;
                (
                    Update {
                        id: req.id,
                        author: p
                            .project
                            .members
                            .iter()
                            .find(|m| m.id == p.member)
                            .context("Not a member")?
                            .name
                            .clone(),
                        author_id: p.member.clone(),
                        summary: req.summary,
                        details: req.details,
                        at: 0,
                        sequence: 0,
                        revisions: vec![],
                        restores: None,
                        name_changes: req.changes,
                        names_parent: Some(req.parent_version),
                    },
                    BTreeMap::new(),
                )
            };
            ensure!(
                !update.summary.trim().is_empty()
                    && text_fits(&update.summary, MAX_SUMMARY_UNITS)
                    && text_fits(&update.details, MAX_DETAILS_UNITS),
                "Write what changed"
            );
            let manifest = self.stage_manifest(p, update).await?;
            let manifest_hash = self.upload_manifest(p, &manifest).await?;
            let keys = self.privacy_keys(&p.endpoint, &p.project.id)?;
            return if route == "/publish" {
                Ok(serde_json::to_value(wire::PublishRequest {
                    id: manifest.update.id,
                    manifest_hash,
                    revisions: manifest.controls,
                    fences: fences
                        .into_iter()
                        .map(|(s, f)| Ok((keys.resource(&p.endpoint, &p.project.id, &s)?, f)))
                        .collect::<Result<_>>()?,
                    restores: manifest.update.restores,
                })?)
            } else {
                Ok(serde_json::to_value(wire::PublishNames {
                    id: manifest.update.id,
                    manifest_hash,
                    parent_version: manifest
                        .update
                        .names_parent
                        .context("Missing names version")?,
                    sections: manifest
                        .update
                        .name_changes
                        .iter()
                        .map(|c| keys.resource(&p.endpoint, &p.project.id, &c.section))
                        .collect::<Result<_>>()?,
                })?)
            };
        }
        let mut value = value;
        if route.starts_with("/locks/") {
            let section = value["section"].as_str().context("Missing slice")?;
            value["section"] = self
                .privacy_keys(&p.endpoint, &p.project.id)?
                .resource(&p.endpoint, &p.project.id, section)?
                .into();
        }
        Ok(value)
    }
    async fn read_manifest(&self, p: &LocalProject, receipt: &wire::Update) -> Result<Manifest> {
        let bytes = self
            .metadata_bytes(p, &manifest_path(&receipt.id)?, &receipt.manifest_hash)
            .await?;
        let manifest = verify_manifest(&p.project.id, receipt, &bytes)?;
        let keys = self.privacy_keys(&p.endpoint, &p.project.id)?;
        for (raw, control) in manifest.update.revisions.iter().zip(&receipt.revisions) {
            ensure!(
                keys.resource(&p.endpoint, &p.project.id, &raw.section)? == control.section,
                "Manifest resource changed"
            );
        }
        let sections = manifest
            .update
            .name_changes
            .iter()
            .map(|c| keys.resource(&p.endpoint, &p.project.id, &c.section))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            sections == receipt.name_sections,
            "Manifest name resources changed"
        );
        Ok(manifest)
    }
    fn cache_receipt(&self, p: &LocalProject, r: &wire::Update) -> Result<()> {
        let key = format!("receipt:{}:{}", p.project.id, r.sequence);
        if let Some(old) = self.db.get::<wire::Update>(&key)? {
            ensure!(old == *r, "The service changed published history");
        }
        self.db.set(&key, r)?;
        Ok(())
    }
    async fn accepted_update(&self, p: &LocalProject, receipt: wire::Update) -> Result<Update> {
        self.cache_receipt(p, &receipt)?;
        let manifest = self.read_manifest(p, &receipt).await?;
        let mut update = manifest.update;
        update.at = receipt.at;
        update.sequence = receipt.sequence;
        let mut heads = self
            .db
            .get::<BTreeMap<String, Head>>(&format!("control-heads:{}", p.project.id))?
            .unwrap_or_default();
        for (file, control) in update.revisions.iter().zip(&receipt.revisions) {
            heads.insert(
                file.section.clone(),
                Head {
                    control: control.clone(),
                    file: file.clone(),
                },
            );
        }
        self.db
            .set(&format!("control-heads:{}", p.project.id), &heads)?;
        self.db.cache(&p.project.id, &update)?;
        // If archiving the receipt fails, archive_history retries it later.
        Ok(update)
    }
    async fn receipt_history(&self, p: &LocalProject, sequence: u64) -> Result<Vec<wire::Update>> {
        let mut receipts = self
            .db
            .raw_values_with_prefix(&format!("receipt:{}:", p.project.id))?
            .into_iter()
            .map(|s| Ok(serde_json::from_str::<wire::Update>(&s)?))
            .collect::<Result<Vec<_>>>()?;
        receipts.sort_by_key(|r| r.sequence);
        let mut known = 0;
        for receipt in &receipts {
            if receipt.sequence == known + 1 {
                known += 1
            } else {
                break;
            }
        }
        if known < sequence {
            let mut before = None;
            loop {
                let route = before
                    .map(|n| format!("/history?before={n}"))
                    .unwrap_or_else(|| "/history".into());
                let page: wire::HistoryPage = serde_json::from_value(
                    self.control_http(p, reqwest::Method::GET, &route, None)
                        .await?,
                )?;
                let oldest = page.entries.last().map(|r| r.sequence).unwrap_or(0);
                for r in page.entries {
                    self.cache_receipt(p, &r)?;
                    if !receipts.iter().any(|x| x.id == r.id) {
                        receipts.push(r);
                    }
                }
                before = page.next;
                if before.is_none() || oldest <= known + 1 {
                    break;
                }
            }
        }
        receipts.retain(|r| r.sequence <= sequence);
        receipts.sort_by_key(|r| r.sequence);
        ensure!(
            receipts.len() as u64 == sequence
                && receipts
                    .iter()
                    .enumerate()
                    .all(|(i, r)| r.sequence == i as u64 + 1),
            "Published history is incomplete"
        );
        Ok(receipts)
    }
    pub async fn hydrate_control_view(
        &self,
        p: &LocalProject,
        view: wire::ProjectView,
    ) -> Result<ProjectView> {
        ensure!(view.id == p.project.id, "Project identity changed");
        let document = match self
            .metadata_bytes(p, "Metadata/project.json", &view.project_hash)
            .await
        {
            Ok(bytes) => bytes,
            Err(_) if p.project.sequence > 0 => {
                self.db
                    .set(&format!("details-pending:{}", p.project.id), &true)?;
                self.db.remove(&format!("verified-view:{}", p.project.id))?;
                *self.message.lock().unwrap() = WAITING.into();
                let mut cached = p.project.clone();
                cached.sequence = view.sequence;
                cached.locks = view
                    .locks
                    .into_iter()
                    .map(|l| self.expand_lease(p, &p.project, l))
                    .collect::<Result<_>>()?;
                return Ok(cached);
            }
            Err(error) => return Err(error),
        };
        let doc: ProjectDocument = serde_json::from_slice(&document)?;
        ensure!(
            doc.version == 1
                && doc.project == view.id
                && doc.storage == p.project.storage
                && !doc.name.trim().is_empty(),
            "Project storage binding changed"
        );
        let mut result = p.project.clone();
        result.name = doc.name;
        result.sequence = view.sequence;
        let receipts = self.receipt_history(p, view.sequence).await?;
        let checkpoint_key = format!("hydration:{}", p.project.id);
        let checkpoint = self
            .db
            .get::<Hydration>(&checkpoint_key)?
            .unwrap_or_default();
        ensure!(
            checkpoint.sequence <= view.sequence,
            "The service returned older project state; cached history is preserved"
        );
        let mut last = checkpoint.sequence;
        let mut heads = checkpoint.heads;
        let mut names = checkpoint.names;
        let mut version = checkpoint.version;
        let mut pending = false;
        for receipt in receipts.iter().filter(|r| r.sequence > checkpoint.sequence) {
            let manifest = match self.read_manifest(p, receipt).await {
                Ok(m) => m,
                Err(_) => {
                    pending = true;
                    break;
                }
            };
            for (raw, control) in manifest.update.revisions.iter().zip(&receipt.revisions) {
                ensure!(
                    heads.get(&raw.section).map(|h| h.control.revision.clone()) == control.parent
                        && heads.get(&raw.section).map(|h| h.file.hash.clone()) == raw.parent,
                    "Revision parent does not match its manifest"
                );
                let shared = manifest
                    .update
                    .revisions
                    .iter()
                    .zip(&receipt.revisions)
                    .find(|(s, _)| s.section == "shared")
                    .map(|(s, c)| (s.hash.clone(), c.revision.clone()))
                    .or_else(|| {
                        heads
                            .get("shared")
                            .map(|h| (h.file.hash.clone(), h.control.revision.clone()))
                    });
                if raw.section != "shared" {
                    ensure!(
                        raw.shared_dependency == shared.as_ref().map(|x| x.0.clone())
                            && control.shared_dependency == shared.map(|x| x.1),
                        "Shared dependency does not match its manifest"
                    );
                }
            }
            if let Some(parent) = manifest.update.names_parent {
                ensure!(parent == version, "Name history is incomplete");
                for change in &manifest.update.name_changes {
                    ensure!(
                        names.get(&change.section) == Some(&change.before),
                        "Name history parent changed"
                    );
                    names.insert(change.section.clone(), change.after.clone());
                }
                version += 1;
            }
            let mut u = manifest.update;
            for (file, control) in u.revisions.iter().zip(&receipt.revisions) {
                names
                    .entry(file.section.clone())
                    .or_insert_with(|| file.name.clone());
                heads.insert(
                    file.section.clone(),
                    Head {
                        control: control.clone(),
                        file: file.clone(),
                    },
                );
            }
            u.sequence = receipt.sequence;
            u.at = receipt.at;
            self.db.cache(&p.project.id, &u)?;
            last = receipt.sequence;
        }
        self.db.set(
            &checkpoint_key,
            &Hydration {
                sequence: last,
                heads: heads.clone(),
                names: names.clone(),
                version,
            },
        )?;
        if !pending {
            let keys = self.privacy_keys(&p.endpoint, &p.project.id)?;
            let compared = heads
                .iter()
                .map(|(section, h)| {
                    Ok((
                        keys.resource(&p.endpoint, &p.project.id, section)?,
                        h.control.clone(),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            ensure!(
                compared == view.heads && version == view.names_version,
                "Coordinator heads do not match published receipts"
            );
            result.heads = heads
                .iter()
                .map(|(s, h)| (s.clone(), h.file.clone()))
                .collect();
            names.remove("shared");
            result.section_names = names;
            result.names_version = version;
            self.db
                .set(&format!("control-heads:{}", p.project.id), &heads)?;
            if self
                .db
                .get::<bool>(&format!("details-pending:{}", p.project.id))?
                .unwrap_or(false)
            {
                *self.message.lock().unwrap() = "Update details downloaded.".into();
            }
        } else {
            *self.message.lock().unwrap() = WAITING.into();
        }
        if pending {
            self.db
                .set(&format!("details-pending:{}", p.project.id), &true)?;
            self.db.remove(&format!("verified-view:{}", p.project.id))?;
        }
        result.members.clear();
        for member in view.members {
            let name = match self
                .metadata_bytes(
                    p,
                    &profile_path(&member.id, &member.profile_hash)?,
                    &member.profile_hash,
                )
                .await
            {
                Ok(bytes) => {
                    let profile: Profile = serde_json::from_slice(&bytes)?;
                    ensure!(
                        profile.version == 1
                            && profile.project == p.project.id
                            && profile.member == member.id
                            && !profile.name.trim().is_empty()
                            && text_fits(&profile.name, MAX_NAME_UNITS),
                        "Invalid creator profile"
                    );
                    profile.name
                }
                Err(_) => p
                    .project
                    .members
                    .iter()
                    .find(|m| m.id == member.id)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "Teammate".into()),
            };
            result.members.push(Member {
                id: member.id,
                name,
                owner: member.owner,
                joined: member.joined,
            });
        }
        result.locks = view
            .locks
            .into_iter()
            .map(|l| self.expand_lease(p, &result, l))
            .collect::<Result<_>>()?;
        result.presence =
            serde_json::from_value(self.privacy_keys(&p.endpoint, &p.project.id)?.open_events(
                &p.endpoint,
                &p.project.id,
                serde_json::to_value(view.presence)?,
            )?)?;
        if !pending {
            self.db.set(
                &format!("verified-view:{}", p.project.id),
                &view_binding(&result)?,
            )?;
        }
        Ok(result)
    }
    fn expand_lease(
        &self,
        p: &LocalProject,
        project: &ProjectView,
        l: wire::Lease,
    ) -> Result<Lease> {
        Ok(Lease {
            section: self
                .privacy_keys(&p.endpoint, &p.project.id)?
                .resource_name(&p.endpoint, &p.project.id, &l.section)?,
            name: project
                .members
                .iter()
                .find(|m| m.id == l.member)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Teammate".into()),
            member: l.member,
            fence: l.fence,
            expires: l.expires,
        })
    }
    pub async fn control_response(
        &self,
        p: &LocalProject,
        route: &str,
        raw: Value,
    ) -> Result<Value> {
        if route.is_empty() {
            return Ok(serde_json::to_value(
                self.hydrate_control_view(p, serde_json::from_value(raw)?)
                    .await?,
            )?);
        }
        if route == "/publish" || route == "/names" {
            return Ok(serde_json::to_value(
                self.accepted_update(p, serde_json::from_value(raw)?)
                    .await?,
            )?);
        }
        if route.starts_with("/history") {
            let page: wire::HistoryPage = serde_json::from_value(raw)?;
            let mut entries = vec![];
            for receipt in page.entries {
                self.cache_receipt(p, &receipt)?;
                if let Ok(manifest) = self.read_manifest(p, &receipt).await {
                    let mut u = manifest.update;
                    u.at = receipt.at;
                    u.sequence = receipt.sequence;
                    entries.push(u);
                }
            }
            return Ok(serde_json::to_value(HistoryPage {
                entries,
                next: page.next,
            })?);
        }
        if route == "/locks/acquire" || route == "/locks/renew" {
            return Ok(serde_json::to_value(self.expand_lease(
                p,
                &p.project,
                serde_json::from_value(raw)?,
            )?)?);
        }
        if route == "/join" {
            let joined: wire::Joined = serde_json::from_value(raw)?;
            let project = self.hydrate_control_view(p, joined.project).await?;
            return Ok(serde_json::to_value(Joined {
                project,
                member: joined.member,
                token: joined.token,
                invitation: joined.invitation,
            })?);
        }
        let mut raw = raw;
        if let Some(id) = raw["holder"].as_str() {
            raw["holder"] = p
                .project
                .members
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Teammate".into())
                .into();
        }
        Ok(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_metadata_keeps_only_a_private_recovery_category() {
        let item = MetadataDownload {
            path: "History/test/manifest.json".into(),
            hash: "a".repeat(64),
            retry: 50,
            attempts: 2,
            issue: Some(crate::recovery::classify(&anyhow::anyhow!(
                "private error https://example.invalid?token=secret"
            ))),
        };
        let encoded = serde_json::to_string(&item).unwrap();
        assert!(!encoded.contains("token") && !encoded.contains("private error"));
        let restored: MetadataDownload = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.issue, Some(crate::recovery::StorageIssue::Unknown));
        assert_eq!(restored.attempts, 2);
    }
    #[test]
    fn readiness_binds_actual_heads_and_names_not_only_the_observed_sequence() {
        let mut project = ProjectView {
            id: "project".into(),
            name: "World".into(),
            storage: StorageConfig {
                kind: "folder".into(),
                location: String::new(),
                drive_id: None,
            },
            members: vec![],
            locks: vec![],
            heads: BTreeMap::new(),
            sequence: 2,
            section_names: BTreeMap::new(),
            names_version: 0,
            presence: vec![],
        };
        let baseline = view_binding(&project).unwrap();
        project.locks.push(Lease {
            section: "5".into(),
            member: "creator".into(),
            name: "Name".into(),
            fence: 1,
            expires: 100,
        });
        assert_eq!(
            baseline,
            view_binding(&project).unwrap(),
            "Reservations must remain independent of metadata transfer"
        );
        project.section_names.insert("5".into(), "Boats".into());
        assert_ne!(
            baseline,
            view_binding(&project).unwrap(),
            "A cached view with the same observed sequence is not a verified new view"
        );
    }
    #[test]
    fn fingerprints_and_identity_are_bound_before_metadata_can_enter_history() {
        let id = uuid::Uuid::new_v4().to_string();
        let update = Update {
            id: id.clone(),
            author: "Creator".into(),
            author_id: uuid::Uuid::new_v4().to_string(),
            summary: "A factual change".into(),
            details: String::new(),
            at: 0,
            sequence: 0,
            revisions: vec![Revision {
                section: "5".into(),
                name: "Boats".into(),
                hash: "a".repeat(64),
                content_hash: "b".repeat(64),
                parent: None,
                artifact: history_artifact(&id, "5").unwrap(),
                size: 10,
                shared_dependency: None,
            }],
            restores: None,
            name_changes: vec![],
            names_parent: None,
        };
        let control = wire::Revision {
            section: "c".repeat(64),
            revision: uuid::Uuid::new_v4().to_string(),
            parent: None,
            shared_dependency: None,
        };
        let manifest = Manifest {
            version: 1,
            nonce: uuid::Uuid::new_v4().to_string(),
            project: "project".into(),
            update,
            controls: vec![control.clone()],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let mut receipt = wire::Update {
            id,
            author_id: manifest.update.author_id.clone(),
            manifest_hash: hash(&bytes),
            at: 10,
            sequence: 1,
            revisions: vec![control],
            restores: None,
            name_sections: vec![],
            names_parent: None,
        };
        assert!(verify_manifest("project", &receipt, &bytes).is_ok());
        for (summary, details, author, valid) in [
            (
                "界".repeat(500),
                "詳".repeat(10_000),
                "名".repeat(100),
                true,
            ),
            ("😀".repeat(250), String::new(), "Creator".into(), true),
            ("界".repeat(501), String::new(), "Creator".into(), false),
            ("😀".repeat(251), String::new(), "Creator".into(), false),
        ] {
            let mut unicode = manifest.clone();
            unicode.update.summary = summary;
            unicode.update.details = details;
            unicode.update.author = author;
            let bytes = serde_json::to_vec(&unicode).unwrap();
            let mut receipt = receipt.clone();
            receipt.manifest_hash = hash(&bytes);
            assert_eq!(verify_manifest("project", &receipt, &bytes).is_ok(), valid);
        }
        assert!(verify_manifest("another", &receipt, &bytes).is_err());
        assert!(verify_manifest("project", &receipt, b"altered").is_err());
        receipt.revisions[0].revision = uuid::Uuid::new_v4().to_string();
        assert!(verify_manifest("project", &receipt, &bytes).is_err());
    }
}
