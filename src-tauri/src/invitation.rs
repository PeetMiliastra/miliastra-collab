use anyhow::{ensure, Context, Result};
use miliastra_core::Invitation;
pub fn parse_link(link: &str) -> Result<Invitation> {
    let mut url = url::Url::parse(link.trim())?;
    ensure!(
        url.query().is_none(),
        "Use the complete invitation link, including the part after #"
    );
    let invite = url
        .fragment()
        .ok_or_else(|| anyhow::anyhow!("This invitation link is incomplete"))?
        .to_owned();
    uuid::Uuid::parse_str(&invite)?;
    let (base, project) = url
        .path()
        .rsplit_once("/invite/")
        .ok_or_else(|| anyhow::anyhow!("This is not a project invitation link"))?;
    let base = base.to_owned();
    let project = project.to_owned();
    uuid::Uuid::parse_str(&project)?;
    url.set_fragment(None);
    url.set_path(&base);
    let endpoint = crate::runtime::endpoint(url.as_str())?;
    Ok(Invitation {
        version: 1,
        endpoint,
        project,
        invite,
    })
}
pub const SCHEME: &str = "miliastra-collab";
pub fn parse_protocol(link: &str) -> Result<Invitation> {
    ensure!(link.len() <= 16384, "Invitation link is too long");
    let url = url::Url::parse(link.trim())?;
    ensure!(
        url.scheme() == SCHEME
            && url.host_str() == Some("invite")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none(),
        "This is not a miliastra/collab invitation"
    );
    let project = url
        .path()
        .strip_prefix('/')
        .ok_or_else(|| anyhow::anyhow!("This invitation link is incomplete"))?;
    uuid::Uuid::parse_str(project)?;
    let invite = url
        .fragment()
        .ok_or_else(|| anyhow::anyhow!("This invitation link is incomplete"))?;
    uuid::Uuid::parse_str(invite)?;
    let args: Vec<_> = url.query_pairs().collect();
    ensure!(
        args.len() == 1 && args[0].0 == "service",
        "This invitation link is incomplete or ambiguous"
    );
    Ok(Invitation {
        version: 1,
        endpoint: crate::runtime::endpoint(&args[0].1)?,
        project: project.into(),
        invite: invite.into(),
    })
}
impl crate::runtime::Core {
    pub async fn invite_details(&self, invitation: Invitation) -> Result<Details> {
        let keys = self.privacy_keys(&invitation.endpoint, &invitation.project)?;
        details(invitation, &keys).await
    }
}
pub fn open_protocol(app: &tauri::AppHandle, link: &str) {
    use tauri::{Emitter, Manager};
    let parsed = match parse_protocol(link) {
        Ok(value) => value,
        Err(error) => {
            let _ = app.emit("invitation-error", error.to_string());
            return;
        }
    };
    if !(cfg!(debug_assertions) && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()) {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let core = app.state::<crate::runtime::Core>();
            let invitation = resolve(&share_link(&parsed)).await?;
            core.db.set("incoming-invitation", &invitation)?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => {
                let _ = app.emit("invitation-opened", ());
            }
            Err(error) => {
                let _ = app.emit("invitation-error", error.to_string());
            }
        }
    });
}

pub fn share_link(invitation: &Invitation) -> String {
    format!(
        "{}/invite/{}#{}",
        invitation.endpoint.trim_end_matches('/'),
        invitation.project,
        invitation.invite
    )
}
async fn resolve_response(link: &str) -> Result<serde_json::Value> {
    let parsed = if url::Url::parse(link.trim())?.scheme() == SCHEME {
        parse_protocol(link)?
    } else {
        parse_link(link)?
    };
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .post(format!("{}/invitations/resolve", parsed.endpoint))
        .json(&serde_json::json!({"project":parsed.project,"invite":parsed.invite}))
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "This invitation is invalid or expired. Ask the project owner for a new link."
    );
    let value: serde_json::Value = response.json().await?;
    let result: Invitation = serde_json::from_value(value.clone())?;
    ensure!(
        result.version == 1
            && result.project == parsed.project
            && result.invite == parsed.invite
            && crate::runtime::endpoint(&result.endpoint)? == parsed.endpoint,
        "The invitation service returned a different project or address"
    );
    Ok(value)
}
pub async fn resolve(link: &str) -> Result<Invitation> {
    Ok(serde_json::from_value(resolve_response(link).await?)?)
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    pub project_name: String,
    /// The owner's name, so the join screen can say who it's waiting for. Missing if it
    /// can't be decrypted.
    pub owner_name: Option<String>,
    pub privacy_key_required: bool,
    pub storage_kind: String,
    pub mapped_folder: Option<std::path::PathBuf>,
    #[serde(skip_serializing)]
    pub storage: miliastra_core::StorageConfig,
}
pub async fn details(
    invitation: Invitation,
    keys: &crate::private_metadata::Keys,
) -> Result<Details> {
    let value = resolve_response(&share_link(&invitation)).await?;
    let storage: miliastra_core::StorageConfig = keys.open(
        &invitation.endpoint,
        &invitation.project,
        "storage-bootstrap",
        value["bootstrap"]
            .as_str()
            .context("Invitation storage locator unavailable")?,
    )?;
    crate::providers::get(&storage.kind)?;
    let name = "Shared project";
    let owner_name = None;
    Ok(Details {
        project_name: name.into(),
        owner_name,
        privacy_key_required: false,
        storage_kind: storage.kind.clone(),
        mapped_folder: None,
        storage,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn protocol_link(invitation: &Invitation) -> String {
        let mut url =
            url::Url::parse(&format!("{SCHEME}://invite/{}", invitation.project)).unwrap();
        url.query_pairs_mut()
            .append_pair("service", &invitation.endpoint);
        url.set_fragment(Some(&invitation.invite));
        url.into()
    }
    #[test]
    fn round_trip_keeps_secret_in_fragment() {
        let i = Invitation {
            version: 1,
            endpoint: "https://collabs.miliastra.dev".into(),
            project: uuid::Uuid::new_v4().to_string(),
            invite: uuid::Uuid::new_v4().to_string(),
        };
        let link = share_link(&i);
        let parsed = parse_link(&link).unwrap();
        assert_eq!(parsed.project, i.project);
        assert_eq!(parsed.invite, i.invite);
        assert_eq!(parsed.endpoint, i.endpoint);
        assert!(!url::Url::parse(&link).unwrap().path().contains(&i.invite));
    }
    #[test]
    fn rejects_unsafe_or_incomplete_links() {
        for url in [
            "file:///invite/a#b",
            "http://example.com/invite/a#b",
            "https://example.com/invite/a#b",
            "https://example.com/invite/a?invite=b",
            "https://example.com/invite/a",
        ] {
            assert!(parse_link(url).is_err());
        }
        let p = uuid::Uuid::new_v4();
        let i = uuid::Uuid::new_v4();
        assert!(parse_link(&format!("http://example.com/invite/{p}#{i}")).is_err());
        assert!(parse_link(&format!("https://user:password@example.com/invite/{p}#{i}")).is_err());
    }
    #[test]
    fn protocol_roundtrip_keeps_invitation_secret_out_of_query_and_service() {
        for endpoint in [
            "https://collabs.miliastra.dev",
            "https://example.com/collab",
            "http://127.0.0.1:63846",
        ] {
            let invitation = Invitation {
                version: 1,
                endpoint: endpoint.into(),
                project: uuid::Uuid::new_v4().to_string(),
                invite: uuid::Uuid::new_v4().to_string(),
            };
            let link = protocol_link(&invitation);
            let parsed = parse_protocol(&link).unwrap();
            assert_eq!(parsed.project, invitation.project);
            assert_eq!(parsed.invite, invitation.invite);
            assert_eq!(parsed.endpoint, invitation.endpoint);
            let url = url::Url::parse(&link).unwrap();
            assert!(!url.query().unwrap().contains(&invitation.invite));
        }
    }
    #[test]
    fn protocol_rejects_wrong_scheme_action_unsafe_service_and_ambiguous_input() {
        let p = uuid::Uuid::new_v4();
        let secret = uuid::Uuid::new_v4();
        for link in [
            format!("other://invite/{p}?service=https%3A%2F%2Fexample.com#{secret}"),
            format!("miliastra-collab://delete/{p}?service=https%3A%2F%2Fexample.com#{secret}"),
            format!("miliastra-collab://user:pass@invite/{p}?service=https%3A%2F%2Fexample.com#{secret}"),
            format!("miliastra-collab://invite/{p}?service=http%3A%2F%2Fexample.com#{secret}"),
            format!("miliastra-collab://invite/{p}?service=file%3A%2F%2F%2Ftmp#{secret}"),
            format!("miliastra-collab://invite/{p}?service=https%3A%2F%2Fexample.com&service=https%3A%2F%2Fother.example#{secret}"),
            format!("miliastra-collab://invite/{p}/extra?service=https%3A%2F%2Fexample.com#{secret}"),
            format!("miliastra-collab://invite/{p}?service=https%3A%2F%2Fexample.com"),
        ] { assert!(parse_protocol(&link).is_err()); }
        assert!(parse_protocol(&"x".repeat(16385)).is_err());
    }
}
