use crate::retry::SendRetrying;
use crate::{
    oauth::{self, OAuthConfig},
    providers,
};
use anyhow::{ensure, Context, Result};
use futures_util::StreamExt;
use miliastra_core::StorageConfig;
use std::path::{Path, PathBuf};
static CLOUD_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub fn cloud_providers() -> Vec<&'static str> {
    providers::all()
        .iter()
        .filter(|p| p.info().cloud && p.info().enabled)
        .map(|p| p.id)
        .collect()
}
pub fn ensure_provider_available(kind: &str) -> Result<()> {
    providers::get(kind)?.ensure_available()
}
#[derive(Clone)]
pub struct Storage {
    pub config: StorageConfig,
    pub folder: Option<PathBuf>,
    pub oauth: OAuthConfig,
}
pub(crate) fn key(k: &str) -> Result<&str> {
    ensure!(
        !k.is_empty()
            && k.len() <= 200
            && !k.contains("..")
            && k.split('/').all(|s| !s.is_empty() && s != ".")
            && k.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b)),
        "Invalid storage reference"
    );
    Ok(k)
}
pub(crate) fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}
pub(crate) fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .build()?)
}
#[cfg(windows)]
pub(crate) fn publish_file(source: &Path, target: &Path, replace: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = if replace {
        MOVEFILE_WRITE_THROUGH | MOVEFILE_REPLACE_EXISTING
    } else {
        MOVEFILE_WRITE_THROUGH
    };
    unsafe {
        MoveFileExW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr()), flags)?;
    }
    Ok(())
}
#[cfg(not(windows))]
pub(crate) fn publish_file(source: &Path, target: &Path, replace: bool) -> Result<()> {
    if replace {
        std::fs::rename(source, target)?;
    } else {
        std::fs::hard_link(source, target)?;
        std::fs::remove_file(source)?;
    }
    Ok(())
}

pub(crate) async fn download(url: String, token: &str) -> Result<Vec<u8>> {
    let c = client()?;
    let mut stream = c
        .get(url)
        .bearer_auth(token)
        .send_retrying()
        .await?
        .error_for_status()?
        .bytes_stream();
    let mut bytes = vec![];
    while let Some(chunk) = stream.next().await {
        let b = chunk?;
        ensure!(
            (bytes.len() + b.len()) as u64 <= miliastra_core::constants::MAX_ARTIFACT_BYTES,
            "Artifact too large"
        );
        bytes.extend(b);
    }
    Ok(bytes)
}
impl Storage {
    pub(crate) async fn token(&self) -> Result<String> {
        oauth::access(&self.config.kind, &self.oauth)
            .await
            .map_err(|error| {
                if matches!(
                    oauth::connection_status(&self.config.kind),
                    oauth::ConnectionStatus::NotConnected | oauth::ConnectionStatus::Reconnect
                ) {
                    error.context(crate::recovery::ReconnectRequired)
                } else {
                    error
                }
            })
    }
    pub(crate) async fn parent(&self, k: &str, create: bool) -> Result<Option<(String, String)>> {
        key(k)?;
        let provider = providers::cloud(&self.config.kind)?;
        let mut parent = self.config.location.clone();
        ensure!(
            !parent.is_empty() && parent != "root",
            "A project folder is required"
        );
        let parts: Vec<_> = k.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            let Some(next) = provider.child_folder(self, &parent, part, create).await? else {
                return Ok(None);
            };
            parent = next;
        }
        Ok(Some((parent, parts[parts.len() - 1].into())))
    }
    pub async fn get(&self, k: &str) -> Result<Vec<u8>> {
        key(k)?;
        let bytes = providers::get(&self.config.kind)?
            .backend
            .get(self, k)
            .await?;
        #[cfg(debug_assertions)]
        {
            let mut bytes = bytes;
            if crate::reliability::transfer_checkpoint("storage-read", k).await?
                && !bytes.is_empty()
            {
                bytes[0] ^= 1;
            }
            return Ok(bytes);
        }
        #[cfg(not(debug_assertions))]
        Ok(bytes)
    }
    pub async fn put(&self, k: &str, bytes: Vec<u8>) -> Result<()> {
        self.write(k, bytes, false).await
    }
    /// Latest can be overwritten; History can't.
    pub async fn put_latest(&self, k: &str, bytes: Vec<u8>) -> Result<()> {
        key(k)?;
        ensure!(
            k.starts_with("Latest/") && k.split('/').count() == 2,
            "Only Latest files can be replaced"
        );
        self.write(k, bytes, true).await
    }
    /// The starting stage can be replaced at any time.
    pub async fn put_base(&self, k: &str, bytes: Vec<u8>) -> Result<()> {
        key(k)?;
        ensure!(
            k.starts_with("Base/") && k.split('/').count() == 2,
            "Only starting-stage files can be replaced"
        );
        self.write(k, bytes, true).await
    }
    pub async fn verify(&self) -> Result<()> {
        let id = uuid::Uuid::new_v4();
        let k = format!("Checks/access-{id}.txt");
        let bytes = format!("miliastra/collab storage check {id}").into_bytes();
        self.put(&k, bytes.clone()).await?;
        ensure!(
            self.get(&k).await? == bytes,
            "Storage read/write check failed"
        );
        let _ = self.remove_check(&k).await;
        Ok(())
    }
    /// Copies a History file to Latest, server-side where the provider supports it.
    pub async fn latest_from_history(
        &self,
        source: &str,
        target: &str,
        bytes: Vec<u8>,
        journal: &crate::db::Db,
    ) -> Result<()> {
        key(source)?;
        key(target)?;
        ensure!(
            source.starts_with("History/") && source.split('/').count() == 3,
            "Latest must come from a History artifact"
        );
        ensure!(
            target.starts_with("Latest/") && target.split('/').count() == 2,
            "Only Latest files can be replaced"
        );
        ensure!(
            source.rsplit('/').next() == target.rsplit('/').next(),
            "Slice filenames differ"
        );
        ensure!(
            bytes.len() as u64 <= miliastra_core::constants::MAX_ARTIFACT_BYTES,
            "Artifact too large"
        );

        let provider = providers::get(&self.config.kind)?;
        let _guard = if provider.cloud.is_some() {
            Some(CLOUD_WRITES.lock().await)
        } else {
            None
        };
        provider
            .backend
            .copy_latest(self, source, target, bytes, journal)
            .await
    }
    async fn write(&self, k: &str, bytes: Vec<u8>, replace: bool) -> Result<()> {
        #[cfg(debug_assertions)]
        crate::reliability::transfer_checkpoint("storage-write", k).await?;
        key(k)?;
        ensure!(
            bytes.len() as u64 <= miliastra_core::constants::MAX_ARTIFACT_BYTES,
            "Artifact too large"
        );
        let provider = providers::get(&self.config.kind)?;
        let _guard = if provider.cloud.is_some() {
            Some(CLOUD_WRITES.lock().await)
        } else {
            None
        };
        provider.backend.write(self, k, bytes, replace).await
    }
    pub async fn ensure_directory(&self, directory: &str) -> Result<()> {
        key(directory)?;
        let provider = providers::get(&self.config.kind)?;
        let _guard = if provider.cloud.is_some() {
            Some(CLOUD_WRITES.lock().await)
        } else {
            None
        };
        provider.backend.ensure_directory(self, directory).await
    }
    async fn remove_check(&self, k: &str) -> Result<()> {
        key(k)?;
        ensure!(
            k.starts_with("Checks/") && k.split('/').count() == 2,
            "Only access checks can be removed"
        );
        providers::get(&self.config.kind)?
            .backend
            .remove_check(self, k)
            .await
    }
    pub async fn folder_url(&self) -> Result<Option<String>> {
        let Some(provider) = providers::get(&self.config.kind)?.cloud else {
            return Ok(None);
        };
        let url = provider.folder_url(&self.config, &self.oauth).await?;
        ensure!(
            url::Url::parse(&url)?.scheme() == "https",
            "Invalid folder URL"
        );
        Ok(Some(url))
    }
}
pub async fn create_cloud(kind: &str, name: &str, oauth: &OAuthConfig) -> Result<StorageConfig> {
    let provider = providers::get(kind)?;
    provider.ensure_available()?;
    let cloud = provider
        .cloud
        .context("This project has no cloud team folder")?;
    let _guard = CLOUD_WRITES.lock().await;
    cloud.create_project(name, oauth).await
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Files under `prefix` in a shared-folder team folder, as storage keys.
    fn files(root: &std::path::Path, prefix: &str) -> Vec<String> {
        let mut out = vec![];
        let mut dirs = vec![root.join(prefix)];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    let key = path.strip_prefix(root).unwrap().to_string_lossy();
                    out.push(key.replace('\\', "/"));
                }
            }
        }
        out.sort();
        out
    }
    #[test]
    fn paths_preserve_real_folders_and_reject_ambiguous_paths() {
        assert_eq!(key("History/update/1.gis").unwrap(), "History/update/1.gis");
        for k in [
            "../secret",
            "/absolute",
            "x/../../y",
            "x\\y",
            "C:/x",
            "",
            "x//y",
            "x/./y",
            "x/",
        ] {
            assert!(key(k).is_err(), "{k}");
        }
    }
    #[tokio::test]
    async fn immutable_history_and_replaceable_latest_are_separate() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let s = Storage {
            config: StorageConfig {
                kind: "folder".into(),
                location: String::new(),
                drive_id: None,
            },
            folder: Some(root.clone()),
            oauth: OAuthConfig::default(),
        };
        let (a, b) = tokio::join!(
            s.put("History/test/1.gis", b"one".to_vec()),
            s.put("History/test/1.gis", b"two".to_vec())
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let history = s.get("History/test/1.gis").await.unwrap();
        s.put_latest("Latest/1.gis", b"one".to_vec()).await.unwrap();
        s.put_latest("Latest/1.gis", b"two".to_vec()).await.unwrap();
        assert_eq!(s.get("Latest/1.gis").await.unwrap(), b"two");
        assert_eq!(s.get("History/test/1.gis").await.unwrap(), history);
        assert!(s
            .put_latest("History/test/1.gis", b"overwrite".to_vec())
            .await
            .is_err());
        let journal = crate::db::Db::open(Path::new(":memory:")).unwrap();
        s.latest_from_history(
            "History/test/1.gis",
            "Latest/1.gis",
            history.clone(),
            &journal,
        )
        .await
        .unwrap();
        assert_eq!(s.get("Latest/1.gis").await.unwrap(), history);
        assert!(s
            .latest_from_history("Latest/1.gis", "Latest/1.gis", vec![], &journal)
            .await
            .is_err());
        assert!(s
            .latest_from_history("History/test/1.gis", "History/test/1.gis", vec![], &journal)
            .await
            .is_err());
        assert!(s
            .latest_from_history("History/test/1.gis", "Latest/2.gis", vec![], &journal)
            .await
            .is_err());
        assert_eq!(files(&root, "History"), vec!["History/test/1.gis"]);
        s.ensure_directory("Latest").await.unwrap();
        s.ensure_directory("History/test/Metadata").await.unwrap();
        assert!(root.join("History/test/Metadata").is_dir());
        s.verify().await.unwrap();
        s.verify().await.unwrap();
        assert!(files(&root, "Checks").is_empty());
        // An old check file must not make a later check skip the write test.
        std::fs::create_dir_all(root.join("Checks")).unwrap();
        std::fs::write(root.join("Checks/access.txt"), b"old").unwrap();
        s.verify().await.unwrap();
        assert_eq!(files(&root, "Checks"), vec!["Checks/access.txt"]);
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(uuid::Uuid::parse_str(&resolved.file_name().unwrap().to_string_lossy()).is_ok());
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
