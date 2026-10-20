//! Keeps both versions of a conflicting slice so the creator can pick one. No merging.
use anyhow::{ensure, Result};
use miliastra_core::{
    binary::{self, Stage},
    hash, Revision, Update,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub project: String,
    pub stage: PathBuf,
    pub section: String,
    pub name: String,
    pub mine: Revision,
    pub theirs: Revision,
    pub baseline: Option<Revision>,
    pub at: i64,
    pub folder: PathBuf,
    #[serde(default)]
    pub draft: crate::runtime::Draft,
    pub latest_update: Option<Update>,
    pub can_keep_mine: bool,
    pub can_use_theirs: bool,
    pub reason: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub mine: crate::assets::Inspection,
    pub theirs: crate::assets::Inspection,
}
pub fn compare(p: &Preview) -> Result<Comparison> {
    fn read(p: &Preview, label: &str, revision: &Revision) -> Result<Vec<u8>> {
        let path = p.folder.join(format!(
            "{label}.{}",
            if p.section == "shared" { "gilh" } else { "gis" }
        ));
        let bytes = fs::read(path)?;
        ensure!(
            revision.section == p.section
                && hash(&bytes) == revision.hash
                && bytes.len() as u64 == revision.size,
            "A preserved version failed validation; review the conflict again"
        );
        ensure!(
            binary::content_hash(
                &bytes,
                &p.section,
                &binary::resource_level(&bytes, &p.section)?
            )? == revision.content_hash,
            "Preserved content failed validation"
        );
        Ok(bytes)
    }
    let baseline_path = p.folder.join(format!(
        "baseline.{}",
        if p.section == "shared" { "gilh" } else { "gis" }
    ));
    let baseline = p
        .baseline
        .as_ref()
        .filter(|_| baseline_path.exists())
        .map(|r| read(p, "baseline", r))
        .transpose()?;
    Ok(Comparison {
        mine: crate::assets::inspect(&read(p, "mine", &p.mine)?, baseline.as_deref(), &p.section)?,
        theirs: crate::assets::inspect(
            &read(p, "teammate", &p.theirs)?,
            baseline.as_deref(),
            &p.section,
        )?,
    })
}
fn durable_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
pub fn preserve(
    data: &Path,
    stage: &Stage,
    project: &str,
    mine: Revision,
    theirs: Revision,
    baseline: Option<Revision>,
    latest_update: Option<Update>,
    can_use_theirs: bool,
) -> Result<Preview> {
    miliastra_core::validate_revision(&mine)?;
    miliastra_core::validate_revision(&theirs)?;
    ensure!(
        mine.section == theirs.section,
        "Mismatched conflict sections"
    );
    let section = mine.section.clone();
    let name = mine.name.clone();
    let id = uuid::Uuid::new_v4().to_string();
    let root = data.join("conflicts");
    fs::create_dir_all(&root)?;
    let folder = root.join(&id);
    fs::create_dir(&folder)?;
    for (label, r) in [
        ("mine", Some(&mine)),
        ("teammate", Some(&theirs)),
        ("baseline", baseline.as_ref()),
    ] {
        if let Some(r) = r {
            let source = data.join("objects").join(&r.hash);
            if label == "baseline" && !source.exists() {
                continue;
            }
            let bytes = fs::read(source)?;
            ensure!(
                hash(&bytes) == r.hash && bytes.len() as u64 == r.size,
                "A conflict version failed checksum validation"
            );
            ensure!(
                binary::content_hash(
                    &bytes,
                    &r.section,
                    &binary::resource_level(&bytes, &r.section)?
                )? == r.content_hash,
                "Conflict content failed validation"
            );
            durable_new(
                &folder.join(format!(
                    "{label}.{}",
                    if section == "shared" { "gilh" } else { "gis" }
                )),
                &bytes,
            )?;
        }
    }
    // Shared settings conflicts are resolved like slices (plain copy works, tested 2026-10-17).
    let p=Preview{id,project:project.into(),stage:stage.path.clone(),section,name,mine,theirs,baseline,at:chrono::Utc::now().timestamp(),folder,draft:crate::runtime::Draft::default(),latest_update,can_keep_mine:true,can_use_theirs,reason:if !can_use_theirs{"You can share your version. To use your teammate's version, choose a recognized stage on this PC."}else{"Both versions are preserved. Choose one, or combine them manually in Miliastra and review again."}.into()};
    durable_new(
        &p.folder.join("versions.json"),
        &serde_json::to_vec_pretty(&p)?,
    )?;
    Ok(p)
}
pub fn validate(
    p: &Preview,
    stage: &Stage,
    project: &str,
    current: &[u8],
    head: &Revision,
) -> Result<()> {
    ensure!(
        p.project == project && p.stage.canonicalize()? == stage.path.canonicalize()?,
        "The project or stage changed; review again"
    );
    ensure!(
        chrono::Utc::now().timestamp() - p.at <= miliastra_core::constants::PREVIEW_TTL_SECS,
        "This review expired; preserve and review the current versions again"
    );
    ensure!(
        hash(current) == p.mine.hash,
        "Your local section changed after review; preserve and review it again"
    );
    ensure!(
        head.hash == p.theirs.hash && head.shared_dependency == p.theirs.shared_dependency,
        "The project changed after review; preserve and review the new version"
    );
    for (label, r) in [("mine", &p.mine), ("teammate", &p.theirs)] {
        let path = p.folder.join(format!(
            "{label}.{}",
            if p.section == "shared" { "gilh" } else { "gis" }
        ));
        let bytes = fs::read(path)?;
        ensure!(
            hash(&bytes) == r.hash && bytes.len() as u64 == r.size,
            "A preserved conflict version failed checksum validation"
        );
    }
    Ok(())
}
pub fn install_plan(p: &Preview) -> crate::installer::Plan {
    crate::installer::Plan {
        id: p.id.clone(),
        project: p.project.clone(),
        stage: p.stage.clone(),
        revisions: vec![p.theirs.clone()],
        originals: BTreeMap::from([(p.section.clone(), p.mine.hash.clone())]),
        previous: BTreeMap::from([(p.section.clone(), p.mine.clone())]),
        at: p.at,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflict_review_preserves_both_bytes_and_rejects_later_edits_or_new_heads() {
        let root = std::env::temp_dir().join(format!("mcollab-conflict-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("objects")).unwrap();
        let stage_root = root.join("stage");
        fs::create_dir(&stage_root).unwrap();
        fn bytes(value: u8) -> Vec<u8> {
            let payload = [8, 5, 18, 5, 8, 42, 208, 5, value];
            let mut b = vec![];
            for n in [payload.len() as u32 + 20, 1, 806, 6, payload.len() as u32] {
                b.extend(n.to_be_bytes());
            }
            b.extend(payload);
            b.extend(1657u32.to_be_bytes());
            b
        }
        let stage = Stage {
            path: stage_root,
            account: "recipient".into(),
            level: "42".into(),
            name: "Test".into(),
            sections: vec![],
            last_saved_section: None,
            settings_hash: "".into(),
            settings_version: "".into(),
            editor_version: "test".into(),
        };
        let revision = |v| {
            let b = bytes(v);
            let h = hash(&b);
            fs::write(root.join("objects").join(&h), &b).unwrap();
            Revision {
                section: "5".into(),
                name: "Boats".into(),
                hash: h.clone(),
                content_hash: binary::content_hash(&b, "5", "42").unwrap(),
                parent: None,
                artifact: miliastra_core::history_artifact(&uuid::Uuid::new_v4().to_string(), "5")
                    .unwrap(),
                size: b.len() as u64,
                shared_dependency: None,
            }
        };
        let mine = revision(2);
        let theirs = revision(3);
        let baseline = revision(1);
        let p = preserve(
            &root,
            &stage,
            "project",
            mine.clone(),
            theirs.clone(),
            Some(baseline),
            None,
            false,
        )
        .unwrap();
        assert_eq!(fs::read(p.folder.join("mine.gis")).unwrap(), bytes(2));
        assert_eq!(fs::read(p.folder.join("teammate.gis")).unwrap(), bytes(3));
        assert!(p.folder.join("baseline.gis").exists());
        let comparison = compare(&p).unwrap();
        assert!(comparison.mine.baseline_available && comparison.theirs.baseline_available);
        assert!(comparison.mine.other_changes && comparison.theirs.other_changes);
        assert!(comparison.mine.changes.is_empty() && comparison.theirs.changes.is_empty());
        fs::remove_file(p.folder.join("baseline.gis")).unwrap();
        let missing = compare(&p).unwrap();
        assert!(!missing.mine.baseline_available && !missing.theirs.baseline_available);
        fs::write(p.folder.join("baseline.gis"), bytes(1)).unwrap();
        validate(&p, &stage, "project", &bytes(2), &theirs).unwrap();
        assert!(validate(&p, &stage, "project", &bytes(4), &theirs).is_err());
        assert!(validate(&p, &stage, "project", &bytes(2), &revision(5)).is_err());
        let plan = install_plan(&p);
        assert_eq!(plan.originals["5"], mine.hash);
        assert_eq!(plan.previous["5"].hash, mine.hash);
        assert_eq!(plan.revisions[0].hash, theirs.hash);
        fs::write(p.folder.join("baseline.gis"), b"damaged").unwrap();
        assert!(
            compare(&p).is_err(),
            "Never compare against an unverified baseline"
        );
        fs::write(p.folder.join("baseline.gis"), bytes(1)).unwrap();
        fs::write(p.folder.join("teammate.gis"), b"damaged").unwrap();
        assert!(
            compare(&p).is_err(),
            "Never display a damaged preserved version"
        );
        assert!(validate(&p, &stage, "project", &bytes(2), &theirs).is_err());
        let resolved = root.canonicalize().unwrap();
        assert_eq!(
            resolved.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        assert!(resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mcollab-conflict-"));
        fs::remove_dir_all(resolved).unwrap();
    }
}
