//! Slice installs and undos with a journal, so an interrupted change can be recovered.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JournalPhase {
    Prepared,
    Applying,
    Installed,
    Restoring,
    Restored,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Replacement {
    pub relative: PathBuf,
    pub original_hash: String,
    pub replacement_hash: String,
    #[serde(default)]
    pub verified_save_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Journal {
    pub root: PathBuf,
    pub backup: PathBuf,
    pub items: Vec<Replacement>,
    pub phase: JournalPhase,
}
fn target(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "Invalid install path"
    );
    let root = root.canonicalize()?;
    let p = root.join(relative);
    let mut cur = root.clone();
    for part in relative.components() {
        cur.push(part);
        if cur.exists() {
            ensure!(
                !cur.symlink_metadata()?.file_type().is_symlink(),
                "Linked game paths are not supported"
            );
            ensure!(
                cur.canonicalize()?.starts_with(&root),
                "Install path escapes stage"
            );
        }
    }
    Ok(p)
}
fn durable(path: &Path, b: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    f.write_all(b)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&temp, path)?;
    Ok(())
}
fn save(path: &Path, j: &Journal) -> Result<()> {
    durable(path, &serde_json::to_vec_pretty(j)?)
}
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for item in fs::read_dir(source)? {
        let item = item?;
        ensure!(
            !item.file_type()?.is_symlink(),
            "Linked files cannot be backed up safely"
        );
        let to = dest.join(item.file_name());
        if item.file_type()?.is_dir() {
            copy_tree(&item.path(), &to)?;
        } else {
            fs::copy(item.path(), &to)?;
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&to)?
                .sync_all()?;
        }
    }
    Ok(())
}
pub fn prepare(
    root: &Path,
    cache: &Path,
    backups: &Path,
    items: Vec<Replacement>,
) -> Result<PathBuf> {
    ensure!(!items.is_empty(), "No updates selected");
    let root = root.canonicalize()?;
    let backups = backups.canonicalize()?;
    ensure!(
        !backups.starts_with(&root) && !root.starts_with(&backups),
        "Backups must be separate from game saves"
    );
    for item in &items {
        ensure!(
            [&item.original_hash, &item.replacement_hash]
                .iter()
                .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            "Invalid artifact checksum"
        );
        let p = target(&root, &item.relative)?;
        ensure!(
            miliastra_core::hash(&fs::read(p)?) == item.original_hash,
            "Your local files changed, so applying was cancelled"
        );
        let b = fs::read(cache.join(&item.replacement_hash))?;
        ensure!(
            miliastra_core::hash(&b) == item.replacement_hash,
            "Downloaded artifact is corrupt"
        );
    }
    let folder = backups.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&folder)?;
    let backup = folder.join("stage");
    copy_tree(&root, &backup)?;
    for item in &items {
        ensure!(
            miliastra_core::hash(&fs::read(target(&backup, &item.relative)?)?)
                == item.original_hash,
            "The stage changed during backup or its backup failed validation"
        );
    }
    let path = folder.join("journal.json");
    save(
        &path,
        &Journal {
            root,
            backup,
            items,
            phase: crate::transaction::JournalPhase::Prepared,
        },
    )?;
    Ok(path)
}
pub fn apply(path: &Path, cache: &Path) -> Result<()> {
    let mut j: Journal = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        j.phase == crate::transaction::JournalPhase::Prepared,
        "Transaction is not ready"
    );
    for item in &j.items {
        ensure!(
            miliastra_core::hash(&fs::read(target(&j.root, &item.relative)?)?)
                == item.original_hash,
            "Local files changed after backup"
        );
        ensure!(
            miliastra_core::hash(&fs::read(target(&j.backup, &item.relative)?)?)
                == item.original_hash,
            "The backup failed its check, so nothing was applied"
        );
    }
    j.phase = crate::transaction::JournalPhase::Applying;
    save(path, &j)?;
    for item in &j.items {
        let b = fs::read(cache.join(&item.replacement_hash))?;
        ensure!(
            miliastra_core::hash(&b) == item.replacement_hash,
            "Artifact changed after backup"
        );
        let p = target(&j.root, &item.relative)?;
        ensure!(
            miliastra_core::hash(&fs::read(&p)?) == item.original_hash,
            "Your local files changed while the update was being applied"
        );
        durable(&p, &b)?;
        #[cfg(debug_assertions)]
        crate::reliability::checkpoint("install-file", &item.relative.to_string_lossy())?;
    }
    j.phase = crate::transaction::JournalPhase::Installed;
    save(path, &j)?;
    Ok(())
}
pub fn authorize_saved_files(
    path: &Path,
    expected: &std::collections::BTreeMap<PathBuf, String>,
) -> Result<()> {
    let mut j: Journal = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        matches!(
            j.phase,
            JournalPhase::Prepared
                | JournalPhase::Applying
                | JournalPhase::Installed
                | JournalPhase::Restoring
        ),
        "No active installation journal"
    );
    for item in &mut j.items {
        let approved = expected
            .get(&item.relative)
            .ok_or_else(|| anyhow::anyhow!("Missing verified slice"))?;
        ensure!(
            miliastra_core::hash(&fs::read(target(&j.root, &item.relative)?)?) == *approved,
            "Files changed during verification"
        );
        item.verified_save_hash = Some(approved.clone());
    }
    save(path, &j)
}
pub fn restore(path: &Path) -> Result<()> {
    let mut j: Journal = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        matches!(
            j.phase,
            JournalPhase::Prepared
                | JournalPhase::Applying
                | JournalPhase::Installed
                | JournalPhase::Restoring
        ),
        "There is no update to undo"
    );
    for item in &j.items {
        let b = fs::read(target(&j.root, &item.relative)?)?;
        let hash = miliastra_core::hash(&b);
        ensure!(
            hash == item.original_hash
                || hash == item.replacement_hash
                || item.verified_save_hash.as_ref() == Some(&hash),
            "New local edits exist; preserve them before restoring"
        );
        let original = fs::read(target(&j.backup, &item.relative)?)?;
        ensure!(
            miliastra_core::hash(&original) == item.original_hash,
            "Backup failed validation"
        );
    }
    j.phase = crate::transaction::JournalPhase::Restoring;
    save(path, &j)?;
    for item in &j.items {
        let original = fs::read(target(&j.backup, &item.relative)?)?;
        let path = target(&j.root, &item.relative)?;
        let current = miliastra_core::hash(&fs::read(&path)?);
        ensure!(
            current == item.original_hash
                || current == item.replacement_hash
                || item.verified_save_hash.as_ref() == Some(&current),
            "New local edits appeared during restoration"
        );
        durable(&path, &original)?;
    }
    j.phase = crate::transaction::JournalPhase::Restored;
    save(path, &j)?;
    Ok(())
}
/// Each backup is a full stage copy. Keeps the newest `keep` backups plus any that a
/// pending install or a possible Undo still needs.
pub fn prune_backups(backups: &Path, referenced: &[String], keep: usize) -> Result<usize> {
    if !backups.exists() {
        return Ok(0);
    }
    let mut folders = vec![];
    for item in fs::read_dir(backups)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if item.file_type()?.is_dir() && uuid::Uuid::parse_str(&name).is_ok() {
            folders.push((item.metadata()?.modified()?, name, item.path()));
        }
    }
    folders.sort_by(|a, b| b.0.cmp(&a.0));
    let mut removed = 0;
    for (_, name, path) in folders.into_iter().skip(keep) {
        if referenced.iter().any(|r| r.contains(&name)) {
            continue;
        }
        fs::remove_dir_all(path)?;
        removed += 1;
    }
    Ok(removed)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pruning_keeps_newest_and_referenced_backups() {
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let mut ids = vec![];
        for _ in 0..5 {
            let id = uuid::Uuid::new_v4().to_string();
            fs::create_dir_all(base.join(&id).join("stage")).unwrap();
            ids.push(id);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        fs::create_dir_all(base.join("not-a-backup")).unwrap();
        let referenced = vec![format!(
            r#"{{"journal":"C:/x/backups/{}/journal.json"}}"#,
            ids[0]
        )];
        assert_eq!(prune_backups(&base, &referenced, 2).unwrap(), 2);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(base.join(id).exists(), i == 0 || i >= 3, "{i}");
        }
        assert!(base.join("not-a-backup").exists());
        fs::remove_dir_all(base).unwrap();
    }
    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        cache: PathBuf,
        backups: PathBuf,
        items: Vec<Replacement>,
    }
    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
            let root = base.join("stage");
            let cache = base.join("cache");
            let backups = base.join("backups");
            for p in [&root, &cache, &backups] {
                fs::create_dir_all(p).unwrap();
            }
            let mut items = vec![];
            for (name, old, new) in [
                ("a", b"old a".as_slice(), b"new a".as_slice()),
                ("b", b"old b".as_slice(), b"new b".as_slice()),
            ] {
                fs::write(root.join(name), old).unwrap();
                let h = miliastra_core::hash(new);
                fs::write(cache.join(&h), new).unwrap();
                items.push(Replacement {
                    relative: name.into(),
                    original_hash: miliastra_core::hash(old),
                    replacement_hash: h,
                    verified_save_hash: None,
                });
            }
            Self {
                base,
                root,
                cache,
                backups,
                items,
            }
        }
        fn prepare(&self) -> PathBuf {
            prepare(&self.root, &self.cache, &self.backups, self.items.clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.base.canonicalize().ok().is_some_and(|p| {
                std::env::temp_dir()
                    .canonicalize()
                    .ok()
                    .is_some_and(|temp| p.starts_with(temp))
                    && p.file_name()
                        .is_some_and(|n| uuid::Uuid::parse_str(&n.to_string_lossy()).is_ok())
            }) {
                let _ = fs::remove_dir_all(&self.base);
            }
        }
    }
    #[test]
    fn install_and_undo_preserve_files() {
        let f = Fixture::new();
        let j = f.prepare();
        apply(&j, &f.cache).unwrap();
        assert_eq!(fs::read(f.root.join("a")).unwrap(), b"new a");
        restore(&j).unwrap();
        assert_eq!(fs::read(f.root.join("b")).unwrap(), b"old b");
    }
    #[test]
    fn interrupted_multi_file_install_recovers() {
        let f = Fixture::new();
        let j = f.prepare();
        let mut journal: Journal = serde_json::from_slice(&fs::read(&j).unwrap()).unwrap();
        journal.phase = crate::transaction::JournalPhase::Applying;
        save(&j, &journal).unwrap();
        fs::write(f.root.join("a"), b"new a").unwrap();
        restore(&j).unwrap();
        assert_eq!(fs::read(f.root.join("a")).unwrap(), b"old a");
        assert_eq!(fs::read(f.root.join("b")).unwrap(), b"old b");
    }
    #[test]
    fn dirty_files_are_never_overwritten() {
        let f = Fixture::new();
        let j = f.prepare();
        fs::write(f.root.join("a"), b"my work").unwrap();
        assert!(apply(&j, &f.cache).is_err());
        assert_eq!(fs::read(f.root.join("a")).unwrap(), b"my work");
    }
    #[test]
    fn traversal_rejected() {
        let f = Fixture::new();
        assert!(target(&f.root, Path::new("../other")).is_err());
    }
    #[test]
    fn damaged_backup_cancels_install_before_any_file_changes() {
        let f = Fixture::new();
        let path = f.prepare();
        let journal: Journal = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        fs::write(journal.backup.join("a"), b"damaged backup").unwrap();
        assert!(apply(&path, &f.cache).is_err());
        assert_eq!(fs::read(f.root.join("a")).unwrap(), b"old a");
        assert_eq!(fs::read(f.root.join("b")).unwrap(), b"old b");
    }
}
