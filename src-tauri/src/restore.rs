//! Restores a slice to an older version by sharing that version again as a new update.
//! History isn't changed, and everyone applies it like any other update.
use crate::runtime::Core;
use anyhow::{ensure, Context, Result};
use miliastra_core::{hash, history_artifact, valid_section, PublishRequest, Revision, Update};
use std::{collections::BTreeMap, sync::atomic::Ordering};

/// Why a restore can't start, if it can't.
pub(crate) fn restore_blocker(
    target: &Revision,
    head: Option<&Revision>,
    local_changes: bool,
    holder: Option<&str>,
) -> Option<String> {
    if head.is_some_and(|h| h.hash == target.hash) {
        return Some(format!("{} is already at this version.", target.name));
    }
    if local_changes {
        return Some(format!(
            "You have unshared changes to {}. Share them first; restoring would replace them when you apply it.",
            target.name
        ));
    }
    holder.map(|name| {
        format!(
            "{name} has {} reserved. Ask them to finish first, or take it over.",
            target.name
        )
    })
}

impl Core {
    pub async fn restore_version(
        &self,
        update_id: &str,
        section: &str,
        reason: &str,
    ) -> Result<Update> {
        uuid::Uuid::parse_str(update_id)?;
        ensure!(valid_section(section), "Unknown slice");
        let reason = reason.trim();
        ensure!(
            !reason.is_empty()
                && miliastra_core::constants::text_fits(
                    reason,
                    miliastra_core::constants::MAX_SUMMARY_UNITS
                ),
            "Say why you're restoring this version"
        );
        let _guard = self.operation.lock().await;
        ensure!(
            self.online.load(Ordering::Relaxed),
            "Reconnect before restoring a version"
        );
        let mut p = self.active()?;
        p.project = self.api(&p, reqwest::Method::GET, "", None).await?;
        self.persist(&p)?;
        self.details_ready(&p)?;
        let source = self
            .db
            .history(&p.project.id)?
            .into_iter()
            .find(|u| u.id == update_id)
            .context("That update isn't in this project's history")?;
        let target = source
            .revisions
            .iter()
            .find(|r| r.section == section)
            .context("That update didn't change this slice")?
            .clone();
        let now = chrono::Utc::now().timestamp();
        let holder = p
            .project
            .locks
            .iter()
            .find(|l| l.section == section && l.member != p.member && l.expires > now)
            .map(|l| l.name.clone());
        if let Some(why) = restore_blocker(
            &target,
            p.project.heads.get(section),
            self.has_local_changes(&p, section),
            holder.as_deref(),
        ) {
            anyhow::bail!(why);
        }
        let head = p
            .project
            .heads
            .get(section)
            .context("This slice has no current version")?
            .clone();
        let bytes = match std::fs::read(self.cache_path(&target.hash)?) {
            Ok(bytes) => bytes,
            Err(_) => self.storage(&p).get(&target.artifact).await?,
        };
        ensure!(
            hash(&bytes) == target.hash && bytes.len() as u64 == target.size,
            "The stored version failed its checksum; it can't be restored"
        );
        self.cache(&bytes)?;
        let name = crate::names::effective_names(&p.project)
            .get(section)
            .cloned()
            .unwrap_or_else(|| "Shared level settings".into());
        let lease = self.acquire(section, false).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let revision = Revision {
            section: section.into(),
            name: name.clone(),
            hash: target.hash.clone(),
            content_hash: target.content_hash.clone(),
            parent: Some(head.hash.clone()),
            artifact: history_artifact(&id, section)?,
            size: target.size,
            // A restored slice joins the team's current shared settings.
            shared_dependency: (section != "shared")
                .then(|| p.project.heads.get("shared").map(|r| r.hash.clone()))
                .flatten(),
        };
        let req = PublishRequest {
            id,
            summary: reason.into(),
            details: format!(
                "Restored {name} to the version from “{}” by {}.",
                source.summary, source.author
            ),
            revisions: vec![revision],
            fences: BTreeMap::from([(section.to_string(), lease.fence)]),
            restores: Some(source.id.clone()),
        };
        let storage = self.storage(&p);
        storage.put(&req.revisions[0].artifact, bytes).await?;
        storage
            .put(
                &format!("History/{}/pending.json", req.id),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "id": req.id, "authorId": p.member, "summary": req.summary,
                    "details": req.details, "revisions": req.revisions, "restores": req.restores
                }))?,
            )
            .await?;
        let update: Update = self
            .api(
                &p,
                reqwest::Method::POST,
                "/publish",
                Some(serde_json::to_value(&req)?),
            )
            .await?;
        self.db.cache(&p.project.id, &update)?;
        let _ = storage
            .put(
                &format!("History/{}/changelog.json", update.id),
                serde_json::to_vec_pretty(&update)?,
            )
            .await;
        // Your stage still has the newer version until you apply this update.
        let _ = self.release_held(&[section.to_string()]).await;
        if self.sync(false).await.is_err() {
            self.online.store(false, Ordering::Relaxed);
        }
        self.changed();
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rev(hash: char) -> Revision {
        Revision {
            section: "5".into(),
            name: "Boats".into(),
            hash: hash.to_string().repeat(64),
            content_hash: "c".repeat(64),
            parent: None,
            artifact: String::new(),
            size: 1,
            shared_dependency: None,
        }
    }
    #[test]
    fn restores_need_a_different_version_and_no_unshared_or_reserved_work() {
        let old = rev('a');
        assert!(restore_blocker(&old, Some(&rev('a')), false, None)
            .unwrap()
            .contains("already at this version"));
        assert!(restore_blocker(&old, Some(&rev('b')), true, None)
            .unwrap()
            .contains("unshared changes"));
        assert!(restore_blocker(&old, Some(&rev('b')), false, Some("Alex"))
            .unwrap()
            .contains("Alex has Boats reserved"));
        assert_eq!(restore_blocker(&old, Some(&rev('b')), false, None), None);
    }
}
