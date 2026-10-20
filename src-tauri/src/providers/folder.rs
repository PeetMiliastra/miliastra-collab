use super::*;
use crate::storage::{key, publish_file};
use anyhow::ensure;
use miliastra_core::hash;
use std::path::{Path, PathBuf};
pub struct SharedFolder;
fn local(s: &Storage, k: &str) -> Result<PathBuf> {
    key(k)?;
    let root = s
        .folder
        .as_ref()
        .context("Choose this project's shared folder")?
        .canonicalize()?;
    let mut cur = root.clone();
    for c in Path::new(k).components() {
        cur.push(c);
        if cur.exists() {
            ensure!(
                !cur.symlink_metadata()?.file_type().is_symlink()
                    && cur.canonicalize()?.starts_with(&root),
                "Linked storage paths are not supported"
            );
        }
    }
    Ok(cur)
}
impl StorageBackend for SharedFolder {
    fn get<'a>(&'a self, s: &'a Storage, k: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let p = local(s, k)?;
            ensure!(
                p.metadata()?.len() <= miliastra_core::constants::MAX_ARTIFACT_BYTES,
                "Artifact too large"
            );
            return Ok(tokio::fs::read(p).await?);
        })
    }
    fn write<'a>(
        &'a self,
        s: &'a Storage,
        k: &'a str,
        bytes: Vec<u8>,
        replace: bool,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let p = local(s, k)?;
            if p.exists() && !replace {
                ensure!(
                    hash(&tokio::fs::read(&p).await?) == hash(&bytes),
                    "Immutable storage object changed"
                );
                return Ok(());
            }
            tokio::fs::create_dir_all(p.parent().unwrap()).await?;
            let temp = p
                .parent()
                .unwrap()
                .join(format!(".{}.pending", uuid::Uuid::new_v4()));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            use std::io::Write;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            let result = publish_file(&temp, &p, replace);
            let _ = std::fs::remove_file(&temp);
            if let Err(e) = result {
                if !replace && p.exists() {
                    ensure!(
                        hash(&tokio::fs::read(&p).await?) == hash(&bytes),
                        "Immutable storage collision"
                    );
                } else {
                    return Err(e);
                }
            }
            return Ok(());
        })
    }
    fn copy_latest<'a>(
        &'a self,
        s: &'a Storage,
        _source: &'a str,
        target: &'a str,
        bytes: Vec<u8>,
        _journal: &'a crate::db::Db,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { s.put_latest(target, bytes).await })
    }
    fn ensure_directory<'a>(
        &'a self,
        s: &'a Storage,
        directory: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            std::fs::create_dir_all(local(s, directory)?)?;
            Ok(())
        })
    }
    fn remove_check<'a>(&'a self, s: &'a Storage, k: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            std::fs::remove_file(local(s, k)?)?;
            Ok(())
        })
    }
}
