use crate::{
    conflicts, installer,
    runtime::{Core, Draft},
};
use anyhow::{ensure, Context, Result};
use miliastra_core::{binary, *};
use std::{collections::BTreeMap, fs};
impl Core {
    pub async fn inspect_conflict(&self, id: &str) -> Result<conflicts::Comparison> {
        uuid::Uuid::parse_str(id)?;
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        let preview = self
            .db
            .get::<conflicts::Preview>(&format!("conflict-preview:{id}"))?
            .context("Review the current versions first")?;
        ensure!(
            preview.project == p.project.id
                && preview.stage.canonicalize()? == p.stage.path.canonicalize()?,
            "The project or stage changed; review again"
        );
        conflicts::compare(&preview)
    }

    pub async fn preview_conflict(&self, section: &str) -> Result<conflicts::Preview> {
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure!(
            self.installation(&p)?.is_none_or(|r| !r.pending()),
            "Finish applying the pending update first"
        );
        ensure!(valid_section(section), "Unknown section");
        let stage = binary::inspect(&p.stage.path)?;
        let head = p
            .project
            .heads
            .get(section)
            .context("Project version unavailable")?
            .clone();
        let baseline = p.installed.get(section).cloned();
        let files = self.stable(&stage).await?;
        let bytes = files.get(section).context("Unknown local section")?;
        ensure!(
            baseline.as_ref().is_none_or(|b| !self.matches(&stage, b))
                && baseline.as_ref().is_none_or(|b| b.hash != head.hash),
            "This section no longer has conflicting versions; refresh the project"
        );
        ensure!(
            self.cache_path(&head.hash)?.exists(),
            "The teammate version has not downloaded yet. Try again after it downloads."
        );
        let h = self.cache(bytes)?;
        let mine = Revision {
            section: section.into(),
            name: crate::names::effective_names(&p.project)
                .get(section)
                .cloned()
                .unwrap_or_else(|| "Shared level settings".into()),
            hash: h.clone(),
            content_hash: binary::content_hash(
                bytes,
                section,
                &binary::resource_level(bytes, section)?,
            )?,
            parent: baseline.as_ref().map(|r| r.hash.clone()),
            artifact: history_artifact(&uuid::Uuid::new_v4().to_string(), section)?,
            size: bytes.len() as u64,
            shared_dependency: p.installed.get("shared").map(|r| r.hash.clone()),
        };
        let update = self.db.history(&p.project.id)?.into_iter().find(|u| {
            u.revisions
                .iter()
                .any(|r| r.section == section && r.hash == head.hash)
        });
        let mut preview = conflicts::preserve(
            &self.data,
            &stage,
            &p.project.id,
            mine,
            head,
            baseline,
            update,
            installer::compatibility(Some(&stage)).direct_install || self.is_practice(&p),
        )?;
        preview.draft = self
            .db
            .get(&format!("conflict-draft:{}:{}", p.project.id, section))?
            .unwrap_or_default();
        self.db
            .set(&format!("conflict-preview:{}", preview.id), &preview)?;
        Ok(preview)
    }
    pub async fn resolve_conflict(
        &self,
        id: &str,
        choice: &str,
        draft: Draft,
        editor_ready: bool,
    ) -> Result<()> {
        uuid::Uuid::parse_str(id)?;
        ensure!(
            ["mine", "theirs"].contains(&choice),
            "Choose a version to keep"
        );
        ensure!(
            editor_ready,
            "Save your work and unload the stage in Miliastra before resolving the conflict"
        );
        let _guard = self.operation.lock().await;
        // Keeping mine publishes it. Using the teammate's version is a normal local install
        // with backup and Undo.
        if choice == "mine" {
            self.sync(true).await?;
        }
        let mut p = self.active()?;
        let preview = self
            .db
            .get::<conflicts::Preview>(&format!("conflict-preview:{id}"))?
            .context("Review the current versions first")?;
        ensure!(
            preview.project == p.project.id,
            "The selected project changed"
        );
        let key = format!("conflict-publish:{id}");
        if let Some(req) = self.db.get::<PublishRequest>(&key)? {
            if self
                .db
                .history(&p.project.id)?
                .iter()
                .any(|u| u.id == req.id && u.author_id == p.member)
            {
                ensure!(
                    choice == "mine",
                    "This review already published your version"
                );
                self.db.set(&format!("conflict-resolved:{id}"), &"mine")?;
                return Ok(());
            }
            ensure!(choice == "mine", "A previous share is still awaiting confirmation. Reconnect before changing that choice.");
        }
        if let Some(done) = self.db.get::<String>(&format!("conflict-resolved:{id}"))? {
            ensure!(
                done == choice,
                "This review was already resolved another way"
            );
            return Ok(());
        }
        ensure!(
            self.installation(&p)?.is_none_or(|r| !r.pending()),
            "Finish applying the pending update first"
        );
        let stage = binary::inspect(&p.stage.path)?;
        let files = self.stable(&stage).await?;
        let head = p
            .project
            .heads
            .get(&preview.section)
            .context("Project version unavailable")?;
        conflicts::validate(
            &preview,
            &stage,
            &p.project.id,
            files
                .get(&preview.section)
                .context("Local section unavailable")?,
            head,
        )?;
        let is_shared = preview.section == "shared";
        let shared = p
            .project
            .heads
            .get("shared")
            .context("Shared baseline unavailable")?;
        // A slice needs the team's shared settings installed first, unless the conflict is
        // on the shared settings themselves.
        ensure!(
            is_shared
                || p.installed
                    .get("shared")
                    .is_some_and(|base| base.hash == shared.hash && self.matches(&stage, base))
                    && head.shared_dependency.as_deref() == Some(shared.hash.as_str()),
            "Shared settings changed; resolve their dependencies before choosing a slice"
        );
        if choice == "mine" {
            ensure!(
                !draft.summary.trim().is_empty(),
                "Explain why you are keeping your version"
            );
            self.db.set(
                &format!("conflict-draft:{}:{}", p.project.id, preview.section),
                &draft,
            )?;
            let lease = self.acquire(&preview.section, false).await?;
            let mut revision = preview.mine.clone();
            revision.parent = Some(head.hash.clone());
            revision.shared_dependency = (!is_shared).then(|| shared.hash.clone());
            revision.name = crate::names::effective_names(&p.project)
                .get(&preview.section)
                .cloned()
                .unwrap_or_else(|| "Shared level settings".into());
            let old = self.db.get::<PublishRequest>(&key)?;
            let mut req = if let Some(old) = old.filter(|r| {
                r.summary == draft.summary
                    && r.details == draft.details
                    && r.revisions
                        .first()
                        .is_some_and(|r| r.hash == revision.hash && r.parent == revision.parent)
            }) {
                PublishRequest {
                    fences: BTreeMap::from([(preview.section.clone(), lease.fence)]),
                    ..old
                }
            } else {
                PublishRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    summary: draft.summary,
                    details: draft.details,
                    revisions: vec![revision],
                    fences: BTreeMap::from([(preview.section.clone(), lease.fence)]),
                    restores: None,
                }
            };
            req.revisions[0].artifact = history_artifact(&req.id, &preview.section)?;
            self.db.set(&key, &req)?;
            let storage = self.storage(&p);
            storage
                .put(
                    &req.revisions[0].artifact,
                    fs::read(self.cache_path(&preview.mine.hash)?)?,
                )
                .await?;
            storage.put(&format!("History/{}/pending.json",req.id),serde_json::to_vec_pretty(&serde_json::json!({"id":req.id,"authorId":p.member,"summary":req.summary,"details":req.details,"revisions":req.revisions,"restores":req.restores}))?).await?;
            let update: Update = self
                .api(
                    &p,
                    reqwest::Method::POST,
                    "/publish",
                    Some(serde_json::to_value(&req)?),
                )
                .await?;
            self.db.cache(&p.project.id, &update)?;
            for r in &update.revisions {
                p.installed.insert(r.section.clone(), r.clone());
            }
            self.persist(&p)?;
            self.db.set(&format!("conflict-resolved:{id}"), &"mine")?;
            if self.sync(false).await.is_err() {
                self.online
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
            self.changed();
        } else {
            ensure!(
                preview.can_use_theirs,
                "Applying a teammate's slice hasn't been verified for this stage"
            );
            let plan = conflicts::install_plan(&preview);
            let mut record = if self.is_practice(&p) {
                installer::prepare_files(
                    &plan,
                    &stage,
                    &self.data.join("objects"),
                    &self.data.join("backups"),
                )?
            } else {
                installer::prepare(
                    &plan,
                    &stage,
                    &self.data.join("objects"),
                    &self.data.join("backups"),
                )?
            };
            record.undo_baselines = Some(p.installed.clone());
            self.save_installation(&record)?;
            let result = crate::transaction::apply(&record.journal, &self.data.join("objects"));
            record.phase = if result.is_ok() {
                crate::installer::InstallationPhase::AwaitingReload
            } else {
                crate::installer::InstallationPhase::Interrupted
            };
            record.error = result.as_ref().err().map(|e| e.to_string());
            self.save_installation(&record)?;
            result?;
            self.finish_installation(&mut record).await?;
            self.db.set(&format!("conflict-resolved:{id}"), &"theirs")?;
        }
        Ok(())
    }
}
