use super::*;
use crate::retry::SendRetrying;
use crate::storage::{client, enc, key};
use anyhow::ensure;
use miliastra_core::hash;
use serde_json::{json, Value};
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
pub struct OneDrive;
async fn ms_folder(
    c: &reqwest::Client,
    token: &str,
    drive: &str,
    parent: &str,
    name: &str,
    create: bool,
) -> Result<Option<String>> {
    let base = if parent == "root" {
        format!("{GRAPH}/drives/{}/root", enc(drive))
    } else {
        format!("{GRAPH}/drives/{}/items/{}", enc(drive), enc(parent))
    };
    let url = format!("{base}:/{}", enc(name));
    let r = c.get(&url).bearer_auth(token).send_retrying().await?;
    if r.status().is_success() {
        let v: Value = r.json().await?;
        ensure!(
            v["folder"].is_object(),
            "A file is blocking a project folder"
        );
        return Ok(Some(v["id"].as_str().context("No folder ID")?.into()));
    }
    ensure!(
        r.status() == 404,
        "Cannot access the team folder. Check your OneDrive connection and folder permission."
    );
    if !create {
        return Ok(None);
    }
    let r = c
        .post(format!("{base}/children"))
        .bearer_auth(token)
        .json(&json!({"name":name,"folder":{},"@microsoft.graph.conflictBehavior":"fail"}))
        .send_retrying()
        .await?;
    let v: Value = if r.status() == 409 {
        c.get(url)
            .bearer_auth(token)
            .send_retrying()
            .await?
            .error_for_status()?
            .json()
            .await?
    } else {
        r.error_for_status()?.json().await?
    };
    ensure!(v["folder"].is_object(), "Destination is not a folder");
    Ok(Some(v["id"].as_str().context("No folder ID")?.into()))
}
fn drive(s: &Storage) -> Result<&str> {
    s.config
        .drive_id
        .as_deref()
        .context("Missing OneDrive drive ID")
}
fn content_url(s: &Storage, k: &str) -> Result<String> {
    key(k)?;
    let path = k.split('/').map(enc).collect::<Vec<_>>().join("/");
    Ok(format!(
        "{GRAPH}/drives/{}/items/{}:/{path}:/content",
        enc(drive(s)?),
        enc(&s.config.location)
    ))
}
impl StorageBackend for OneDrive {
    fn get<'a>(&'a self, s: &'a Storage, k: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(
            async move { crate::storage::download(content_url(s, k)?, &s.token().await?).await },
        )
    }
    fn write<'a>(
        &'a self,
        s: &'a Storage,
        k: &'a str,
        bytes: Vec<u8>,
        replace: bool,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let c = client()?;
            let token = s.token().await?;
            s.parent(k, true)
                .await?
                .context("Project folder unavailable")?;
            let url = content_url(s, k)?;
            if !replace {
                let r = c.get(&url).bearer_auth(&token).send_retrying().await?;
                if r.status().is_success() {
                    ensure!(
                        hash(&r.bytes().await?) == hash(&bytes),
                        "Immutable storage object changed"
                    );
                    return Ok(());
                }
                ensure!(r.status() == 404, "Cannot verify OneDrive storage");
            }
            c.put(url)
                .bearer_auth(token)
                .header("Content-Type", "application/octet-stream")
                .body(bytes)
                .send_retrying()
                .await?
                .error_for_status()?;

            Ok(())
        })
    }
    fn copy_latest<'a>(
        &'a self,
        s: &'a Storage,
        source: &'a str,
        target: &'a str,
        bytes: Vec<u8>,
        journal: &'a crate::db::Db,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let (parent, name) = s
                .parent(target, true)
                .await?
                .context("Latest folder unavailable")?;
            return super::onedrive_copy::latest(
                &s.token().await?,
                drive(s)?,
                &s.config.location,
                source,
                &parent,
                &name,
                &bytes,
                journal,
            )
            .await;
        })
    }
    fn ensure_directory<'a>(
        &'a self,
        s: &'a Storage,
        directory: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            s.parent(&format!("{directory}/placeholder"), true)
                .await?
                .context("Project folder unavailable")?;
            Ok(())
        })
    }
    fn remove_check<'a>(&'a self, s: &'a Storage, k: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let url = content_url(s, k)?.trim_end_matches(":/content").to_owned() + ":";
            client()?
                .delete(url)
                .bearer_auth(s.token().await?)
                .send_retrying()
                .await?
                .error_for_status()?;
            Ok(())
        })
    }
}

impl CloudProvider for OneDrive {
    fn oauth<'a>(&self, config: &'a OAuthConfig) -> OAuthRegistration<'a> {
        let (authorize, token) = if config
            .microsoft_client_id
            .eq_ignore_ascii_case(crate::oauth::MICROSOFT_CLIENT_ID)
        {
            (
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
            )
        } else {
            (
                "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
                "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            )
        };
        OAuthRegistration {
            client_id: &config.microsoft_client_id,
            client_secret: None,
            authorize,
            token,
            scope: "offline_access Files.ReadWrite.All User.Read",
            required_scope: None,
            callback_host: "localhost",
            extra_authorization: &[],
            permission_message:
                "Reconnect your storage account and allow the required permissions.",
            reconnect_message: "Reconnect your storage account in Settings",
        }
    }
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        config: &'a OAuthConfig,
    ) -> BoxFuture<'a, Result<StorageConfig>> {
        Box::pin(async move {
            let token = crate::oauth::access("onedrive", config).await?;
            let c = client()?;
            let drive: Value = c
        .get(format!("{GRAPH}/me/drive"))
        .bearer_auth(&token)
        .send_retrying()
        .await
        .and_then(|r| r.error_for_status())
        .context("OneDrive didn't respond. Wait a minute, then click Create project again; your entries are kept.")?
        .json()
        .await?;
            let drive = drive["id"].as_str().context("No drive ID")?;
            let root = ms_folder(&c, &token, drive, "root", "miliastra-collab", true)
                .await?
                .context("No app folder")?;
            let folder: Value = c
        .post(format!(
            "{GRAPH}/drives/{}/items/{}/children",
            enc(drive),
            enc(&root)
        ))
        .bearer_auth(token)
        .json(&json!({"name":name.trim(),"folder":{},"@microsoft.graph.conflictBehavior":"rename"}))
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
            Ok(StorageConfig {
                kind: "onedrive".into(),
                location: folder["id"].as_str().context("No folder ID")?.into(),
                drive_id: Some(drive.into()),
            })
        })
    }
    fn child_folder<'a>(
        &'a self,
        s: &'a Storage,
        parent: &'a str,
        name: &'a str,
        create: bool,
    ) -> BoxFuture<'a, Result<Option<String>>> {
        Box::pin(async move {
            ms_folder(
                &client()?,
                &s.token().await?,
                drive(s)?,
                parent,
                name,
                create,
            )
            .await
        })
    }
    fn account_email<'a>(&'a self, config: &'a OAuthConfig) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let token = crate::oauth::access("onedrive", config).await?;
            let c = account_client()?;
            let email = {
                let v: Value = c
                    .get(format!("{GRAPH}/me"))
                    .query(&[("$select", "mail,userPrincipalName")])
                    .bearer_auth(token)
                    .send_retrying()
                    .await?
                    .error_for_status()
                    .context("Could not read your Microsoft account email. Reconnect OneDrive and retry.")?
                    .json()
                    .await?;
                v["mail"]
                    .as_str()
                    .or(v["userPrincipalName"].as_str())
                    .map(str::to_owned)
            };
            email.context("Your storage account has no email address")
        })
    }
    fn grant_access<'a>(
        &'a self,
        storage: &'a StorageConfig,
        config: &'a OAuthConfig,
        email: &'a str,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let token = crate::oauth::access("onedrive", config).await?;
            let c = account_client()?;
            let folder = urlencoding::encode(&storage.location).into_owned();
            let drive = urlencoding::encode(
                storage
                    .drive_id
                    .as_deref()
                    .context("Missing OneDrive drive ID")?,
            )
            .into_owned();
            let url = format!("{GRAPH}/drives/{drive}/items/{folder}/invite");
            let body = |send: bool| json!({"recipients":[{"email":email}],"roles":["write"],"requireSignIn":true,"sendInvitation":send});
            let quiet = c
                .post(&url)
                .bearer_auth(&token)
                .json(&body(false))
                .send_retrying()
                .await?;
            if quiet.status().is_success() {
                return Ok(());
            }
            // Some account types only allow sharing with an invitation email.
            c.post(&url)
                .bearer_auth(&token)
                .json(&body(true))
                .send_retrying()
                .await?
                .error_for_status()
                .context("OneDrive refused to share the team folder")?;
            Ok(())
        })
    }
    fn folder_url<'a>(
        &'a self,
        storage: &'a StorageConfig,
        config: &'a OAuthConfig,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let token = crate::oauth::access("onedrive", config).await?;
            let c = client()?;

            let drive = storage.drive_id.as_deref().context("Missing drive ID")?;
            let value: Value = c
                .get(format!(
                    "{GRAPH}/drives/{}/items/{}",
                    enc(drive),
                    enc(&storage.location)
                ))
                .bearer_auth(token)
                .send_retrying()
                .await?
                .error_for_status()?
                .json()
                .await?;
            Ok(value["webUrl"]
                .as_str()
                .context("Folder link unavailable")?
                .into())
        })
    }
}
