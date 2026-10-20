use crate::retry::SendRetrying;
use anyhow::{ensure, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConfig {
    pub microsoft_client_id: String,
    pub google_client_id: String,
    #[serde(default, skip_serializing)]
    pub google_client_secret: String,
}
// A public client ID, not a secret.
pub const MICROSOFT_CLIENT_ID: &str = "a64478d4-2a7a-414b-ac00-a467e00dcbed";
pub fn registered_microsoft_client_id() -> &'static str {
    option_env!("MCOLLAB_MICROSOFT_CLIENT_ID")
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(MICROSOFT_CLIENT_ID)
}
pub const GOOGLE_CLIENT_ID: &str =
    "417518576886-28frelv6e9c4tqiam38ioul14srsvgra.apps.googleusercontent.com";
pub fn registered_google_client_id() -> &'static str {
    option_env!("MCOLLAB_GOOGLE_CLIENT_ID")
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(GOOGLE_CLIENT_ID)
}
pub fn registered_google_client_secret() -> &'static str {
    option_env!("MCOLLAB_GOOGLE_CLIENT_SECRET").unwrap_or("")
}
impl OAuthConfig {
    pub fn apply_registration_defaults(&mut self) {
        if self.microsoft_client_id.trim().is_empty() {
            self.microsoft_client_id = registered_microsoft_client_id().into();
        }
        if self.google_client_id.trim().is_empty() {
            self.google_client_id = registered_google_client_id().into();
        }
        if self.google_client_id == registered_google_client_id()
            && self.google_client_secret.is_empty()
        {
            self.google_client_secret = registered_google_client_secret().into();
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
    /// Scopes the provider granted, space-separated. Missing for older sign-ins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}
fn has_required_scope(provider: &str, scope: Option<&str>) -> bool {
    let Ok(provider) = crate::providers::cloud(provider) else {
        return false;
    };
    provider
        .oauth(&OAuthConfig::default())
        .required_scope
        .is_none_or(|required| {
            scope.is_some_and(|scopes| scopes.split_whitespace().any(|scope| scope == required))
        })
}
fn registration<'a>(
    provider: &str,
    config: &'a OAuthConfig,
) -> Result<crate::providers::OAuthRegistration<'a>> {
    Ok(crate::providers::cloud(provider)?.oauth(config))
}
pub fn secret_get(key: &str) -> Result<String> {
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("MCOLLAB_TEST_DATA_DIR") {
        return Ok(std::fs::read_to_string(
            std::path::PathBuf::from(root)
                .join("test-secrets")
                .join(miliastra_core::hash(key.as_bytes())),
        )?);
    }
    if crate::providers::is_provider_credential(key) {
        return crate::credentials::get(key);
    }
    Ok(keyring::Entry::new(&crate::profile::credential_service(), key)?.get_password()?)
}
pub fn secret_set(key: &str, v: &str) -> Result<()> {
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("MCOLLAB_TEST_DATA_DIR") {
        let path = std::path::PathBuf::from(root).join("test-secrets");
        std::fs::create_dir_all(&path)?;
        std::fs::write(path.join(miliastra_core::hash(key.as_bytes())), v)?;
        return Ok(());
    }
    if crate::providers::is_provider_credential(key) {
        crate::credentials::set(key, v)?;
        return Ok(());
    }
    keyring::Entry::new(&crate::profile::credential_service(), key)?.set_password(v)?;
    Ok(())
}
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionStatus {
    Connected,
    NotConnected,
    Reconnect,
    Unavailable,
}

fn stored_connection_status(provider: &str, saved: Result<String>, now: i64) -> ConnectionStatus {
    match saved {
        Ok(value) => match serde_json::from_str::<Tokens>(&value) {
            Ok(tokens) if !has_required_scope(provider, tokens.scope.as_deref()) => {
                ConnectionStatus::Reconnect
            }
            Ok(tokens)
                if (!tokens.access_token.trim().is_empty() && tokens.expires_at > now + 60)
                    || tokens
                        .refresh_token
                        .as_ref()
                        .is_some_and(|token| !token.trim().is_empty()) =>
            {
                ConnectionStatus::Connected
            }
            _ => ConnectionStatus::Reconnect,
        },
        Err(error)
            if error
                .downcast_ref::<keyring::Error>()
                .is_some_and(|e| matches!(e, keyring::Error::NoEntry))
                || error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            ConnectionStatus::NotConnected
        }
        Err(_) => ConnectionStatus::Unavailable,
    }
}

pub fn connection_status(provider: &str) -> ConnectionStatus {
    if crate::providers::cloud(provider).is_err() {
        return ConnectionStatus::Unavailable;
    }
    stored_connection_status(
        provider,
        secret_get(&format!("oauth:{provider}")),
        chrono::Utc::now().timestamp(),
    )
}

pub fn disconnect(provider: &str) -> Result<()> {
    crate::providers::cloud(provider)?;
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os("MCOLLAB_TEST_DATA_DIR") {
        let path = std::path::PathBuf::from(root)
            .join("test-secrets")
            .join(miliastra_core::hash(format!("oauth:{provider}").as_bytes()));
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    }
    crate::credentials::delete(&format!("oauth:{provider}"))
}
fn client<'a>(provider: &str, config: &'a OAuthConfig) -> Result<&'a str> {
    let id = registration(provider, config)?.client_id;
    ensure!(
        !id.is_empty(),
        "This build needs its {provider} application registration configured before sign-in"
    );
    Ok(id)
}
fn endpoints(provider: &str, config: &OAuthConfig) -> Result<(&'static str, &'static str)> {
    let registration = registration(provider, config)?;
    Ok((registration.authorize, registration.token))
}
async fn exchange(
    p: &str,
    config: &OAuthConfig,
    mut form: Vec<(&str, String)>,
    previous: Option<String>,
    previous_scope: Option<String>,
) -> Result<String> {
    form.push(("client_id", client(p, config)?.into()));
    if let Some(secret) = registration(p, config)?.client_secret {
        form.push(("client_secret", secret.into()));
    }
    let response = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(endpoints(p, config)?.1)
        .form(&form)
        .send_retrying()
        .await?;
    if !response.status().is_success() {
        // 400/401 from the token endpoint means the user has to sign in again. Other
        // statuses are kept for the storage recovery screen.
        let reconnect = matches!(response.status().as_u16(), 400 | 401);
        let error = response.error_for_status().unwrap_err();
        return Err(if reconnect {
            anyhow::Error::from(error).context(crate::recovery::ReconnectRequired)
        } else {
            error.into()
        });
    }
    let v: serde_json::Value = response.json().await?;
    let access = v["access_token"]
        .as_str()
        .context("Missing access token")?
        .to_string();
    let t = Tokens {
        access_token: access.clone(),
        refresh_token: v["refresh_token"].as_str().map(str::to_owned).or(previous),
        expires_at: chrono::Utc::now().timestamp() + v["expires_in"].as_i64().unwrap_or(3600),
        scope: v["scope"].as_str().map(str::to_owned).or(previous_scope),
    };
    ensure!(
        has_required_scope(p, t.scope.as_deref()),
        "{}",
        registration(p, config)?.permission_message
    );
    secret_set(&format!("oauth:{p}"), &serde_json::to_string(&t)?)?;
    Ok(access)
}
pub async fn access(p: &str, config: &OAuthConfig) -> Result<String> {
    client(p, config)?;
    let t: Tokens = serde_json::from_str(
        &secret_get(&format!("oauth:{p}")).context("Connect your storage account in Settings")?,
    )?;
    ensure!(
        has_required_scope(p, t.scope.as_deref()),
        "{}",
        registration(p, config)?.reconnect_message
    );
    if t.expires_at > chrono::Utc::now().timestamp() + 60 {
        return Ok(t.access_token);
    }
    let refresh = t.refresh_token.context("Reconnect your storage account")?;
    exchange(
        p,
        config,
        vec![
            ("grant_type", "refresh_token".into()),
            ("refresh_token", refresh.clone()),
        ],
        Some(refresh),
        t.scope,
    )
    .await
}
fn redirect_uri(provider: &str, port: u16) -> Result<String> {
    let host = registration(provider, &OAuthConfig::default())?.callback_host;
    Ok(format!("http://{host}:{port}/callback"))
}

#[derive(Debug, PartialEq)]
enum Callback {
    Invalid,
    Failed(String),
    Code(String),
}

fn parse_callback(target: &str, expected_state: &str) -> Callback {
    if !target.starts_with('/') || target.starts_with("//") {
        return Callback::Invalid;
    }
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Invalid;
    };
    if url.path() != "/callback" || url.fragment().is_some() {
        return Callback::Invalid;
    }
    let mut args = std::collections::BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if args.insert(key.into_owned(), value.into_owned()).is_some() {
            return Callback::Invalid;
        }
    }
    if args.get("state").map(String::as_str) != Some(expected_state) {
        return Callback::Invalid;
    }
    if let Some(error) = args.get("error") {
        let clean = |value: &str| {
            value
                .chars()
                .filter(|ch| !ch.is_control())
                .take(512)
                .collect::<String>()
        };
        let description = args
            .get("error_description")
            .map(|value| clean(value))
            .unwrap_or_else(|| "The provider rejected the sign-in request.".into());
        return Callback::Failed(format!(
            "Sign-in failed ({}): {}",
            clean(error),
            description
        ));
    }
    match args.remove("code").filter(|code| !code.trim().is_empty()) {
        Some(code) => Callback::Code(code),
        None => Callback::Failed(
            "Sign-in failed: the provider did not return an authorization code.".into(),
        ),
    }
}

async fn callback_reply(
    stream: &mut tokio::net::TcpStream,
    success: bool,
    body: &str,
) -> Result<()> {
    let status = if success { "200 OK" } else { "400 Bad Request" };
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn receive_callback<F, Fut>(
    listener: tokio::net::TcpListener,
    state: &str,
    complete: F,
) -> Result<()>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    loop {
        let (mut stream, _) = listener.accept().await?;
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut data = Vec::new();
            loop {
                let mut chunk = [0u8; 1024];
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    anyhow::bail!("Incomplete callback request");
                }
                data.extend_from_slice(&chunk[..n]);
                ensure!(data.len() <= 8192, "Callback request too large");
                if data.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(String::from_utf8_lossy(&data).into_owned())
        })
        .await;
        let Ok(Ok(request)) = request else {
            continue;
        };
        let mut line = request.lines().next().unwrap_or("").split_whitespace();
        let callback = if line.next() == Some("GET") {
            parse_callback(line.next().unwrap_or(""), state)
        } else {
            Callback::Invalid
        };
        let result = match callback {
            Callback::Invalid => {
                let _ = callback_reply(
                    &mut stream,
                    false,
                    "Invalid sign-in callback. Return to miliastra/collab to try again.",
                )
                .await;
                continue;
            }
            Callback::Failed(message) => Err(anyhow::anyhow!(message)),
            Callback::Code(code) => complete(code).await,
        };
        let body = match &result {
            Ok(()) => {
                "Connected to miliastra/collab. You can return to the app and close this window."
                    .into()
            }
            Err(error) => format!("{error}\n\nReturn to miliastra/collab to try again."),
        };
        // Sign-in succeeded once the credentials are saved, even if the browser tab closes early.
        let _ = callback_reply(&mut stream, result.is_ok(), &body).await;
        return result;
    }
}

pub async fn login(p: &str, config: &OAuthConfig) -> Result<()> {
    ensure!(
        !(cfg!(debug_assertions) && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()),
        "Live provider sign-in is disabled in the isolated smoke-test profile"
    );
    let id = client(p, config)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect = redirect_uri(p, listener.local_addr()?.port())?;
    let state = uuid::Uuid::new_v4().to_string();
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let mut auth = url::Url::parse(endpoints(p, config)?.0)?;
    auth.query_pairs_mut().extend_pairs([
        ("client_id", id),
        ("redirect_uri", &redirect),
        ("response_type", "code"),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("scope", registration(p, config)?.scope),
    ]);
    auth.query_pairs_mut()
        .extend_pairs(registration(p, config)?.extra_authorization.iter().copied());
    webbrowser::open(auth.as_str())?;
    tokio::time::timeout(
        std::time::Duration::from_secs(300),
        receive_callback(listener, &state, |code| async move {
            exchange(
                p,
                config,
                vec![
                    ("grant_type", "authorization_code".into()),
                    ("code", code),
                    ("redirect_uri", redirect),
                    ("code_verifier", verifier),
                ],
                None,
                None,
            )
            .await
            .map(|_| ())
        }),
    )
    .await
    .context("Sign-in timed out. Try again.")??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_registration_has_no_callback_endpoint_or_permission_fallback() {
        let config = OAuthConfig::default();
        assert!(client("unknown", &config).is_err());
        assert!(endpoints("unknown", &config).is_err());
        assert!(redirect_uri("unknown", 12345).is_err());
        assert!(!has_required_scope("unknown", None));
        assert_eq!(connection_status("unknown"), ConnectionStatus::Unavailable);
    }
    #[test]
    fn connection_status_distinguishes_saved_renewable_missing_and_unreadable_sign_ins() {
        let saved = |access: &str, refresh: Option<&str>, expires| {
            Ok(serde_json::json!({"access_token":access,"refresh_token":refresh,"expires_at":expires}).to_string())
        };
        assert_eq!(
            stored_connection_status("onedrive", saved("access", None, 2000), 1000),
            ConnectionStatus::Connected
        );
        assert_eq!(
            stored_connection_status("onedrive", saved("expired", Some("refresh"), 500), 1000),
            ConnectionStatus::Connected
        );
        assert_eq!(
            stored_connection_status("onedrive", saved("expired", None, 500), 1000),
            ConnectionStatus::Reconnect
        );
        assert_eq!(
            stored_connection_status("onedrive", saved("", Some("  "), 2000), 1000),
            ConnectionStatus::Reconnect
        );
        assert_eq!(
            stored_connection_status("onedrive", Ok("invalid".into()), 1000),
            ConnectionStatus::Reconnect
        );
        assert_eq!(
            stored_connection_status("onedrive", Err(keyring::Error::NoEntry.into()), 1000),
            ConnectionStatus::NotConnected
        );
        assert_eq!(
            stored_connection_status(
                "onedrive",
                Err(std::io::Error::from(std::io::ErrorKind::NotFound).into()),
                1000
            ),
            ConnectionStatus::NotConnected
        );
        assert_eq!(
            stored_connection_status(
                "onedrive",
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
                1000
            ),
            ConnectionStatus::Unavailable
        );
    }
    #[test]
    fn google_sign_ins_without_full_drive_access_must_reconnect() {
        let saved = |scope: Option<&str>| {
            Ok(serde_json::json!({"access_token":"a","refresh_token":"r","expires_at":2000,"scope":scope}).to_string())
        };
        assert_eq!(
            stored_connection_status("google", saved(None), 1000),
            ConnectionStatus::Reconnect,
            "sign-ins from before the scope was recorded"
        );
        assert_eq!(
            stored_connection_status(
                "google",
                saved(Some("https://www.googleapis.com/auth/drive.file")),
                1000
            ),
            ConnectionStatus::Reconnect
        );
        assert_eq!(
            stored_connection_status(
                "google",
                saved(Some("openid https://www.googleapis.com/auth/drive")),
                1000
            ),
            ConnectionStatus::Connected
        );
        assert!(has_required_scope("onedrive", None));
    }
    #[test]
    fn connection_status_serializes_only_the_display_state() {
        let value = serde_json::to_string(&stored_connection_status("onedrive", Ok(serde_json::json!({"access_token":"private-access","refresh_token":"private-refresh","expires_at":0}).to_string()), 1000)).unwrap();
        assert_eq!(value, "\"connected\"");
    }
    #[test]
    fn provider_callback_hosts_keep_dynamic_ports_and_matching_paths() {
        for port in [49152, 61234] {
            assert_eq!(
                redirect_uri("onedrive", port).unwrap(),
                format!("http://localhost:{port}/callback")
            );
            assert_eq!(
                redirect_uri("google", port).unwrap(),
                format!("http://127.0.0.1:{port}/callback")
            );
        }
    }
    #[tokio::test]
    async fn azure_localhost_callback_reaches_the_ipv4_loopback_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect = redirect_uri("onedrive", listener.local_addr().unwrap().port()).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            assert!(peer.ip().is_loopback());
            let mut data = [0u8; 4096];
            let n = stream.read(&mut data).await.unwrap();
            assert!(String::from_utf8_lossy(&data[..n])
                .starts_with("GET /callback?code=test&state=local "));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let reply = client
            .get(format!("{redirect}?code=test&state=local"))
            .send()
            .await
            .unwrap();
        assert_eq!(reply.text().await.unwrap(), "ok");
        server.await.unwrap();
    }
    #[test]
    fn fresh_settings_include_microsoft_registration() {
        let settings = crate::runtime::Settings::default();
        assert_eq!(
            settings.oauth.microsoft_client_id,
            registered_microsoft_client_id()
        );
        uuid::Uuid::parse_str(&settings.oauth.microsoft_client_id).unwrap();
    }
    #[test]
    fn blank_saved_registration_uses_default_without_changing_other_settings() {
        for value in ["", "   "] {
            let mut settings = crate::runtime::Settings::default();
            settings.oauth.microsoft_client_id = value.into();
            settings.oauth.google_client_id = "existing-google-app".into();
            settings.endpoint = "http://127.0.0.1:8787".into();
            let mut restored: crate::runtime::Settings =
                serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
            restored.oauth.apply_registration_defaults();
            assert_eq!(
                restored.oauth.microsoft_client_id,
                registered_microsoft_client_id()
            );
            assert_eq!(restored.oauth.google_client_id, "existing-google-app");
            assert_eq!(restored.endpoint, settings.endpoint);
        }
    }
    #[test]
    fn explicit_microsoft_registration_is_preserved() {
        let id = "11111111-1111-4111-8111-111111111111";
        let mut config = OAuthConfig {
            microsoft_client_id: id.into(),
            ..OAuthConfig::default()
        };
        config.apply_registration_defaults();
        assert_eq!(client("onedrive", &config).unwrap(), id);
    }
    #[test]
    fn fresh_settings_include_google_registration_without_serializing_secret() {
        let settings = crate::runtime::Settings::default();
        assert_eq!(
            settings.oauth.google_client_id,
            registered_google_client_id()
        );
        assert!(settings
            .oauth
            .google_client_id
            .ends_with(".apps.googleusercontent.com"));
        assert!(serde_json::to_value(&settings.oauth)
            .unwrap()
            .get("googleClientSecret")
            .is_none());
    }
    #[test]
    fn empty_registration_uses_build_defaults() {
        let mut config = OAuthConfig::default();
        config.apply_registration_defaults();
        assert_eq!(config.google_client_id, registered_google_client_id());
        assert!(
            config.google_client_secret.is_empty() == registered_google_client_secret().is_empty()
        );
        config.google_client_secret = "device-stored-secret".into();
        config.apply_registration_defaults();
        assert!(config.google_client_secret == "device-stored-secret");
        assert!(serde_json::to_value(&config)
            .unwrap()
            .get("googleClientSecret")
            .is_none());
    }
    #[test]
    fn custom_google_registration_never_gets_another_apps_build_secret() {
        let mut config = OAuthConfig {
            google_client_id: "custom-google-app".into(),
            ..OAuthConfig::default()
        };
        config.apply_registration_defaults();
        assert_eq!(config.google_client_id, "custom-google-app");
        assert!(config.google_client_secret.is_empty());
    }
}

#[cfg(test)]
mod sign_in_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    }
    async fn receiver<F, Fut>(complete: F) -> (String, tokio::task::JoinHandle<Result<()>>)
    where
        F: FnOnce(String) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                receive_callback(listener, "expected-state", complete),
            )
            .await
            .unwrap()
        });
        (url, server)
    }
    #[test]
    fn personal_registration_uses_consumers_for_authorization_and_tokens() {
        let mut config = OAuthConfig {
            microsoft_client_id: MICROSOFT_CLIENT_ID.into(),
            ..OAuthConfig::default()
        };
        let (authorize, token) = endpoints("onedrive", &config).unwrap();
        assert_eq!(
            authorize,
            "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize"
        );
        assert_eq!(
            token,
            "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
        );
        config.microsoft_client_id = "custom-registration".into();
        let (authorize, token) = endpoints("onedrive", &config).unwrap();
        assert!(authorize.contains("/common/"));
        assert!(token.contains("/common/"));
        assert_eq!(
            endpoints("google", &config).unwrap(),
            (
                "https://accounts.google.com/o/oauth2/v2/auth",
                "https://oauth2.googleapis.com/token"
            )
        );
    }
    #[test]
    fn azure_audience_error_is_a_failure_even_when_a_code_is_present() {
        let target = "/callback?state=expected-state&error=invalid_request&error_description=The%20request%20is%20not%20valid%20for%20Consumer%20userAudience.&code=must-not-exchange";
        assert_eq!(parse_callback(target, "expected-state"), Callback::Failed("Sign-in failed (invalid_request): The request is not valid for Consumer userAudience.".into()));
        assert!(matches!(
            parse_callback("/callback?state=expected-state", "expected-state"),
            Callback::Failed(_)
        ));
        assert!(matches!(
            parse_callback("/callback?state=expected-state&code=", "expected-state"),
            Callback::Failed(_)
        ));
    }
    #[test]
    fn untrusted_callbacks_do_not_complete_sign_in() {
        for target in [
            "/callback?state=wrong&error=invalid_request",
            "/callback?code=secret-code",
            "/favicon.ico?state=expected-state&code=secret-code",
            "/callback?state=expected-state&state=wrong&code=secret-code",
            "/callback?state=expected-state&code=one&code=two",
            "http://other/callback?state=expected-state&code=secret-code",
            "//other/callback?state=expected-state&code=secret-code",
            "/callback?state=expected-state&code=secret-code#fragment",
        ] {
            assert_eq!(parse_callback(target, "expected-state"), Callback::Invalid);
        }
    }
    #[tokio::test]
    async fn provider_error_replies_with_failure_and_never_exchanges_tokens() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let (url, server) = receiver(move |_| async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
        let response = http_client().get(format!("{url}/callback?state=expected-state&error=invalid_request&error_description=Consumer%20userAudience%20cannot%20use%20common" )).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body = response.text().await.unwrap();
        assert!(body.contains("Consumer userAudience cannot use common"));
        assert!(!body.contains("Connected to"));
        assert!(!body.contains("This window can be closed"));
        let error = server.await.unwrap().unwrap_err().to_string();
        assert!(error.contains("invalid_request"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn token_or_credential_failure_is_not_reported_as_success() {
        let (url, server) = receiver(|code| async move {
            assert_eq!(code, "test-code");
            anyhow::bail!("Unable to save sign-in credentials");
        })
        .await;
        let response = http_client()
            .get(format!(
                "{url}/callback?state=expected-state&code=test-code"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body = response.text().await.unwrap();
        assert!(body.contains("Unable to save sign-in credentials"));
        assert!(!body.contains("Connected to"));
        assert!(!body.contains("test-code"));
        assert!(server.await.unwrap().is_err());
    }
    #[tokio::test]
    async fn success_reply_waits_until_credentials_have_been_saved() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (saved_tx, saved_rx) = tokio::sync::oneshot::channel();
        let (url, server) = receiver(move |code| async move {
            assert_eq!(code, "private-test-code");
            started_tx.send(()).unwrap();
            saved_rx.await.unwrap();
            Ok(())
        })
        .await;
        let request = tokio::spawn(async move {
            http_client()
                .get(format!(
                    "{url}/callback?state=expected-state&code=private-test-code"
                ))
                .send()
                .await
                .unwrap()
        });
        started_rx.await.unwrap();
        assert!(
            !request.is_finished(),
            "Browser must not claim success before completion"
        );
        saved_tx.send(()).unwrap();
        let response = request.await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = response.text().await.unwrap();
        assert!(body.contains("Connected to miliastra/collab"));
        assert!(!body.contains("private-test-code"));
        server.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn unrelated_request_does_not_consume_the_pending_sign_in() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let (url, server) = receiver(move |_| async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
        let response = http_client()
            .get(format!("{url}/callback?state=wrong&error=invalid_request"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let response = http_client()
            .get(format!(
                "{url}/callback?state=expected-state&code=test-code"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        server.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
