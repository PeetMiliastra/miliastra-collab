//! Storage providers. A new provider needs an adapter here and an entry in catalog.json.
mod folder;
pub(crate) mod google;
mod onedrive;
mod onedrive_copy;
use crate::{oauth::OAuthConfig, storage::Storage};
use anyhow::{Context, Result};
use futures_util::future::BoxFuture;
use miliastra_core::StorageConfig;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    pub cloud: bool,
    pub enabled: bool,
    pub setup_message: String,
}
pub fn catalog() -> &'static [ProviderInfo] {
    static CATALOG: std::sync::LazyLock<Vec<ProviderInfo>> = std::sync::LazyLock::new(|| {
        serde_json::from_str(include_str!("catalog.json")).expect("Valid provider catalog")
    });
    &CATALOG
}
pub struct Provider {
    pub id: &'static str,
    pub backend: &'static dyn StorageBackend,
    pub cloud: Option<&'static dyn CloudProvider>,
}
impl Provider {
    pub fn info(&self) -> &'static ProviderInfo {
        catalog()
            .iter()
            .find(|info| info.id == self.id)
            .expect("Registered provider metadata")
    }
    pub fn ensure_available(&self) -> Result<()> {
        anyhow::ensure!(self.info().enabled,
            "{} isn't available in this version of miliastra/collab. Use OneDrive or a shared folder.", self.info().name);
        Ok(())
    }
}
static PROVIDERS: &[Provider] = &[
    Provider {
        id: "folder",
        backend: &folder::SharedFolder,
        cloud: None,
    },
    Provider {
        id: "onedrive",
        backend: &onedrive::OneDrive,
        cloud: Some(&onedrive::OneDrive),
    },
    Provider {
        id: "google",
        backend: &google::GoogleDrive,
        cloud: Some(&google::GoogleDrive),
    },
];
pub fn all() -> &'static [Provider] {
    PROVIDERS
}
pub fn get(id: &str) -> Result<&'static Provider> {
    PROVIDERS
        .iter()
        .find(|p| p.id == id)
        .context("Unknown storage provider")
}
pub fn cloud(id: &str) -> Result<&'static dyn CloudProvider> {
    get(id)?
        .cloud
        .context("This project has no cloud team folder")
}
pub fn is_provider_credential(key: &str) -> bool {
    key.strip_prefix("oauth:")
        .is_some_and(|id| cloud(id).is_ok())
}
/// Shorter timeout for account lookups and sharing than for file transfers.
fn account_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}
pub struct OAuthRegistration<'a> {
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub authorize: &'static str,
    pub token: &'static str,
    pub scope: &'static str,
    pub required_scope: Option<&'static str>,
    pub callback_host: &'static str,
    pub extra_authorization: &'static [(&'static str, &'static str)],
    pub permission_message: &'static str,
    pub reconnect_message: &'static str,
}
pub trait StorageBackend: Send + Sync {
    fn get<'a>(&'a self, s: &'a Storage, key: &'a str) -> BoxFuture<'a, Result<Vec<u8>>>;
    fn write<'a>(
        &'a self,
        s: &'a Storage,
        key: &'a str,
        bytes: Vec<u8>,
        replace: bool,
    ) -> BoxFuture<'a, Result<()>>;
    fn copy_latest<'a>(
        &'a self,
        s: &'a Storage,
        source: &'a str,
        target: &'a str,
        bytes: Vec<u8>,
        journal: &'a crate::db::Db,
    ) -> BoxFuture<'a, Result<()>>;
    fn ensure_directory<'a>(
        &'a self,
        s: &'a Storage,
        directory: &'a str,
    ) -> BoxFuture<'a, Result<()>>;
    fn remove_check<'a>(&'a self, s: &'a Storage, key: &'a str) -> BoxFuture<'a, Result<()>>;
}
pub trait CloudProvider: Send + Sync {
    fn oauth<'a>(&self, config: &'a OAuthConfig) -> OAuthRegistration<'a>;
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        config: &'a OAuthConfig,
    ) -> BoxFuture<'a, Result<StorageConfig>>;
    fn child_folder<'a>(
        &'a self,
        s: &'a Storage,
        parent: &'a str,
        name: &'a str,
        create: bool,
    ) -> BoxFuture<'a, Result<Option<String>>>;
    fn account_email<'a>(&'a self, config: &'a OAuthConfig) -> BoxFuture<'a, Result<String>>;
    fn grant_access<'a>(
        &'a self,
        storage: &'a StorageConfig,
        config: &'a OAuthConfig,
        email: &'a str,
    ) -> BoxFuture<'a, Result<()>>;
    fn folder_url<'a>(
        &'a self,
        storage: &'a StorageConfig,
        config: &'a OAuthConfig,
    ) -> BoxFuture<'a, Result<String>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn catalog_and_adapters_have_identical_unique_ids_and_capabilities() {
        let ids: BTreeSet<_> = all().iter().map(|provider| provider.id).collect();
        assert_eq!(ids.len(), all().len());
        assert_eq!(catalog().len(), all().len());
        assert_eq!(ids, catalog().iter().map(|info| info.id.as_str()).collect());
        for provider in all() {
            let info = provider.info();
            assert!(!info.name.trim().is_empty());
            assert!(!info.setup_message.trim().is_empty());
            assert_eq!(info.cloud, provider.cloud.is_some(), "{}", provider.id);
            assert_eq!(provider.ensure_available().is_ok(), info.enabled);
            assert_eq!(
                is_provider_credential(&format!("oauth:{}", provider.id)),
                info.cloud
            );
        }
        assert!(get("onedrive").unwrap().info().enabled);
        // Google Drive stays off until after the first release.
        assert!(!get("google").unwrap().info().enabled);
        assert_eq!(crate::storage::cloud_providers(), vec!["onedrive"]);
        let config = OAuthConfig::default();
        let google = cloud("google").unwrap().oauth(&config);
        assert_eq!(google.scope, "https://www.googleapis.com/auth/drive");
        assert_eq!(google.required_scope, Some(google.scope));
    }
    #[test]
    fn unknown_providers_never_fall_back_to_another_adapter_or_credential() {
        for id in ["", "unregistered", "../onedrive", "Google", "sealed"] {
            assert!(get(id).is_err());
            assert!(cloud(id).is_err());
            assert!(!is_provider_credential(&format!("oauth:{id}")));
        }
        assert!(cloud("folder").is_err());
        assert!(!is_provider_credential("oauth:onedrive/../google"));
        assert!(!is_provider_credential("privacy-v2:anything"));
    }
    #[tokio::test]
    async fn invalid_provider_setup_fails_before_files_or_sign_in() {
        let storage = Storage {
            config: StorageConfig {
                kind: "unregistered".into(),
                location: String::new(),
                drive_id: None,
            },
            folder: None,
            oauth: OAuthConfig::default(),
        };
        assert!(storage
            .get("History/test/1.gis")
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown storage"));
        assert!(storage.put("History/test/1.gis", vec![1]).await.is_err());
        assert!(storage.ensure_directory("History").await.is_err());
        assert!(storage.folder_url().await.is_err());
        let error = crate::storage::create_cloud("unregistered", "test", &storage.oauth)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("Unknown storage provider"));
        assert!(
            crate::storage::create_cloud("folder", "test", &storage.oauth)
                .await
                .is_err()
        );
        assert!(crate::oauth::access("unregistered", &storage.oauth)
            .await
            .is_err());
    }
}
