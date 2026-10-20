//! Automatic folder access: the joiner's app asks, and the owner's app shares the folder.
use crate::retry::SendRetrying;
use crate::{oauth::OAuthConfig, runtime::Core};
use anyhow::{ensure, Context, Result};
use miliastra_core::{coordination::AccessRequest, Invitation, StorageConfig};
use serde_json::{json, Value};

/// A plausible email address to share the team folder with. Emails never reach the
/// coordinator; this only guards what we send to the storage provider.
pub fn valid_email(email: &str) -> bool {
    let email = email.trim();
    email.len() <= 254
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
        && email.split_once('@').is_some_and(|(user, domain)| {
            !user.is_empty() && domain.contains('.') && !domain.contains('@')
        })
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessStatus {
    pub status: String,
    pub message: String,
    pub owner_online: Option<bool>,
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

/// Email address of the signed-in storage account.
pub async fn account_email(provider: &str, config: &OAuthConfig) -> Result<String> {
    let email = crate::providers::cloud(provider)?
        .account_email(config)
        .await?;
    ensure!(
        valid_email(&email),
        "Your storage account has no usable email address"
    );
    Ok(email)
}
/// Gives an email address edit access to the team folder.
pub async fn grant(storage: &StorageConfig, config: &OAuthConfig, email: &str) -> Result<()> {
    ensure!(valid_email(email), "Invalid email address");
    crate::providers::cloud(&storage.kind)?
        .grant_access(storage, config, email)
        .await
}
impl Core {
    /// Joiner: asks the owner's app to share the team folder with this account.
    pub async fn request_folder_access(
        &self,
        invitation: Invitation,
        display_name: String,
    ) -> Result<String> {
        let info = self.invite_details(invitation.clone()).await?;
        ensure!(
            info.storage.kind != "folder",
            "Shared-folder projects don't need folder access requests"
        );
        let oauth = self.settings.lock().unwrap().oauth.clone();
        let key = zeroize::Zeroizing::new(
            self.read_privacy_key(&invitation.endpoint, &invitation.project)?,
        );
        let email = zeroize::Zeroizing::new(account_email(&info.storage.kind, &oauth).await?);
        let id = uuid::Uuid::new_v4().to_string();
        let sealed = crate::privacy::seal(
            &key,
            &invitation,
            &id,
            &email,
            &display_name,
            chrono::Utc::now().timestamp(),
        )?;
        let ep = crate::runtime::endpoint(&invitation.endpoint)?;
        let r = client()?
            .post(format!(
                "{ep}/projects/{}/access-requests",
                invitation.project
            ))
            .json(&json!({"invite":invitation.invite,"id":id,"sealed":sealed}))
            .send_retrying()
            .await?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or_default();
        ensure!(
            status.is_success(),
            "{}",
            v["error"]
                .as_str()
                .unwrap_or("Could not ask for folder access")
        );
        v["id"]
            .as_str()
            .map(str::to_owned)
            .context("The service did not return a request")
    }
    /// Joiner: status of the request ("pending", "granted" or "failed") and its message.
    pub async fn folder_access_status(&self, invitation: Invitation, id: String) -> Result<Value> {
        let ep = crate::runtime::endpoint(&invitation.endpoint)?;
        let r = client()?
            .post(format!(
                "{ep}/projects/{}/access-requests/status",
                invitation.project
            ))
            .json(&json!({"invite":invitation.invite,"id":id}))
            .send_retrying()
            .await?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or_default();
        ensure!(
            status.is_success(),
            "{}",
            v["error"]
                .as_str()
                .unwrap_or("Could not check folder access")
        );
        if v["status"] == "settled" {
            let keys = self.privacy_keys(&invitation.endpoint, &invitation.project)?;
            let mut result = keys.open::<Value>(&invitation.endpoint,&invitation.project,&format!("access-result:{id}"),
                v["message"].as_str().context("Missing encrypted access result")?)
                .unwrap_or_else(|_|json!({"status":"failed","message":"The Encryption Key did not match. Ask the owner for the current key, then retry."}));
            result["ownerOnline"] = v["ownerOnline"].clone();
            return Ok(result);
        }
        Ok(v)
    }
    /// Owner: shares the team folder with everyone waiting and reports each result.
    pub async fn grant_pending_access(&self) -> Result<()> {
        let p = self.active()?;
        if p.project.storage.kind == "folder"
            || !p
                .project
                .members
                .iter()
                .any(|m| m.id == p.member && m.owner)
        {
            return Ok(());
        }
        let pending: Vec<AccessRequest> = self
            .api(&p, reqwest::Method::GET, "/access-requests", None)
            .await?;
        if pending.is_empty() {
            return Ok(());
        }
        let key = zeroize::Zeroizing::new(self.read_privacy_key(&p.endpoint, &p.project.id)?);
        let oauth = self.settings.lock().unwrap().oauth.clone();
        for r in pending {
            let contact = crate::privacy::open(
                &key,
                &p.endpoint,
                &p.project.id,
                &r,
                chrono::Utc::now().timestamp(),
            );
            let (granted, message) = match contact {
                Ok(contact) => {
                    if grant(&p.project.storage, &oauth, &contact.email)
                        .await
                        .is_ok()
                    {
                        (true, "")
                    } else {
                        (false, "storage")
                    }
                }
                Err(_) => (false, "privacy_key"),
            };
            // Only send fixed messages back, never provider errors or email addresses.
            let keys = self.privacy_keys(&p.endpoint, &p.project.id)?;
            let explanation = if granted {
                "Folder shared"
            } else if message == "privacy_key" {
                "The Encryption Key did not match. Ask the owner for the current key, then retry."
            } else {
                "The owner could not share the team folder. Ask them to check their storage connection, then retry."
            };
            let result = keys.seal(
                &p.endpoint,
                &p.project.id,
                &format!("access-result:{}", r.id),
                &json!({"status":if granted {"granted"}else{"failed"},"message":explanation}),
            )?;
            self.api::<Value>(
                &p,
                reqwest::Method::POST,
                "/access-requests/result",
                Some(json!({"id":r.id,"result":result})),
            )
            .await?;
            self.changed();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emails_have_a_bounded_non_control_shape() {
        assert!(valid_email("creator@example.com"));
        for value in [
            "",
            "creator",
            "creator@localhost",
            "a b@example.com",
            "a@example.com@evil",
        ] {
            assert!(!valid_email(value));
        }
    }
}
