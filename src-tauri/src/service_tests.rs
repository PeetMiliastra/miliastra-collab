//! The app against the real coordinator. These run only when `MCOLLAB_TEST_URL` points at a
//! test coordinator, for example one started by tests/coordinator.cjs.
use miliastra_core::{
    CreateProject, HistoryPage, Joined, Lease, ProjectView, PublishRequest, Revision,
    StorageConfig, Update,
};
use std::sync::Mutex;

struct Service {
    endpoint: String,
}
fn service() -> Option<Service> {
    match std::env::var("MCOLLAB_TEST_URL") {
        Ok(endpoint) => Some(Service {
            endpoint: endpoint.trim_end_matches('/').into(),
        }),
        Err(_) => {
            eprintln!("Skipped: set MCOLLAB_TEST_URL to a test coordinator to run this test.");
            None
        }
    }
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// Encrypts and decrypts the test requests, using made-up keys only.
fn key_store() -> &'static Mutex<std::collections::BTreeMap<String, crate::private_metadata::Keys>>
{
    static KEYS: std::sync::OnceLock<
        Mutex<std::collections::BTreeMap<String, crate::private_metadata::Keys>>,
    > = std::sync::OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(std::collections::BTreeMap::new()))
}
fn fixture_states() -> &'static Mutex<std::collections::BTreeMap<String, serde_json::Value>> {
    static VALUES: std::sync::OnceLock<
        Mutex<std::collections::BTreeMap<String, serde_json::Value>>,
    > = std::sync::OnceLock::new();
    VALUES.get_or_init(|| Mutex::new(Default::default()))
}
fn fixture_codec(
    id: &str,
    ep: &str,
    operation: &str,
    route: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    use std::io::Write;
    let bundle = key_store().lock().unwrap()[id].export().unwrap();
    let mut states = fixture_states().lock().unwrap();
    let input = serde_json::json!({"bundle":bundle,"project":id,"endpoint":ep,"operation":operation,"route":route,"value":value,"state":states.get(id)});
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/private-fixture.cjs");
    let mut command = std::process::Command::new("node");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&input).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Fixture codec: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    states.insert(id.into(), result["state"].take());
    result["value"].take()
}
fn encrypted_fixture(raw: CreateProject, ep: &str) -> miliastra_core::control::CreateProject {
    let id = raw.id.clone();
    key_store()
        .lock()
        .unwrap()
        .insert(id.clone(), crate::private_metadata::Keys::new());
    serde_json::from_value(fixture_codec(
        &id,
        ep,
        "encode",
        "/projects",
        serde_json::to_value(raw).unwrap(),
    ))
    .unwrap()
}
struct FixtureClient(reqwest::Client);
struct FixtureRequest {
    client: reqwest::Client,
    method: reqwest::Method,
    url: String,
    token: Option<String>,
    body: Option<serde_json::Value>,
}
struct FixtureResponse {
    response: reqwest::Response,
    url: url::Url,
    project: Option<String>,
    filters: std::collections::BTreeMap<String, String>,
}
impl FixtureClient {
    fn new() -> Self {
        Self(reqwest::Client::new())
    }
    fn get(&self, url: impl AsRef<str>) -> FixtureRequest {
        self.request(reqwest::Method::GET, url.as_ref())
    }
    fn post(&self, url: impl AsRef<str>) -> FixtureRequest {
        self.request(reqwest::Method::POST, url.as_ref())
    }
    fn request(&self, method: reqwest::Method, url: &str) -> FixtureRequest {
        FixtureRequest {
            client: self.0.clone(),
            method,
            url: url.into(),
            token: None,
            body: None,
        }
    }
    fn project_key(&self, id: &str) -> String {
        key_store().lock().unwrap()[id]
            .current_key()
            .unwrap()
            .into()
    }
}
impl FixtureRequest {
    fn bearer_auth(mut self, value: impl AsRef<str>) -> Self {
        self.token = Some(value.as_ref().into());
        self
    }
    fn json<T: serde::Serialize>(mut self, value: &T) -> Self {
        self.body = Some(serde_json::to_value(value).unwrap());
        self
    }
    async fn send(self) -> reqwest::Result<FixtureResponse> {
        let mut url = url::Url::parse(&self.url).unwrap();
        let ep = url.origin().ascii_serialization();
        let parts: Vec<_> = url
            .path()
            .trim_matches('/')
            .split('/')
            .map(str::to_owned)
            .collect();
        let mut project = parts
            .get(1)
            .filter(|_| parts.first().is_some_and(|s| s == "projects"))
            .cloned();
        let mut body = self.body;
        if parts == ["projects"] && self.method == reqwest::Method::POST {
            let raw: CreateProject = serde_json::from_value(body.take().unwrap()).unwrap();
            project = Some(raw.id.clone());
            body = Some(serde_json::to_value(encrypted_fixture(raw, &ep)).unwrap());
        } else if parts.first().is_some_and(|s| s == "invitations") {
            project = body
                .as_ref()
                .and_then(|v| v["project"].as_str())
                .map(str::to_owned);
        } else if let (Some(id), Some(value)) = (&project, &mut body) {
            let route = parts
                .iter()
                .skip(2)
                .fold(String::new(), |s, p| format!("{s}/{p}"));
            *value = fixture_codec(id, &ep, "encode", &route, value.clone());
        }
        let filters = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if parts.get(2).is_some_and(|p| p == "history") {
            let cursor = url
                .query_pairs()
                .find(|(k, _)| k == "before")
                .map(|(_, v)| v.into_owned());
            url.set_query(None);
            if let Some(cursor) = cursor {
                url.query_pairs_mut().append_pair("before", &cursor);
            }
        }
        let mut request = self.client.request(self.method, url.clone());
        if let Some(token) = self.token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        Ok(FixtureResponse {
            response: request.send().await?,
            url,
            project,
            filters,
        })
    }
}
impl FixtureResponse {
    fn status(&self) -> reqwest::StatusCode {
        self.response.status()
    }
    fn error_for_status(self) -> reqwest::Result<Self> {
        self.response.error_for_status_ref()?;
        Ok(self)
    }
    async fn json<T: serde::de::DeserializeOwned>(self) -> reqwest::Result<T> {
        let mut value: serde_json::Value = self.response.json().await?;
        if let Some(id) = self.project {
            if key_store().lock().unwrap().contains_key(&id) {
                let ep = self.url.origin().ascii_serialization();
                value = fixture_codec(&id, &ep, "decode", "", value);
                if let Some(entries) = value
                    .get_mut("entries")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    entries.retain(|u| {
                        self.filters.get("section").is_none_or(|section| {
                            ["revisions", "nameChanges"].iter().any(|field| {
                                u[*field].as_array().is_some_and(|items| {
                                    items.iter().any(|r| r["section"].as_str() == Some(section))
                                })
                            })
                        })
                    });
                }
            }
        }
        Ok(serde_json::from_value(value).unwrap())
    }
}

#[tokio::test]
async fn encrypted_access_requests_roundtrip_without_contacts_or_keys_on_the_wire() {
    let Some(server) = service() else { return };
    let client = FixtureClient::new();
    let joined: Joined = client
        .post(format!("{}/projects", server.endpoint))
        .json(&CreateProject {
            id: uuid::Uuid::new_v4().to_string(),
            member_id: uuid::Uuid::new_v4().to_string(),
            summary: "Initialized project".into(),
            details: "Initial saved project version.".into(),
            name: "Test".into(),
            display_name: "Owner".into(),
            storage: StorageConfig {
                kind: "onedrive".into(),
                location: "folder".into(),
                drive_id: Some("drive".into()),
            },
            baseline: vec![],
            baseline_id: uuid::Uuid::new_v4().to_string(),
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let invitation = joined.invitation.unwrap();
    let url = format!(
        "{}/projects/{}/access-requests",
        server.endpoint, joined.project.id
    );
    let id = uuid::Uuid::new_v4().to_string();
    let key = client.project_key(&joined.project.id);
    let sealed = crate::privacy::seal(
        &key,
        &invitation,
        &id,
        "private@example.com",
        "Private joiner",
        now(),
    )
    .unwrap();
    let body = serde_json::json!({"invite":invitation.invite,"id":id,"sealed":sealed});
    let wire = serde_json::to_string(&body).unwrap();
    for private in [&key, "private@example.com", "Private joiner"] {
        assert!(!wire.contains(private));
    }
    for _ in 0..2 {
        let result: serde_json::Value = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(result["id"], id);
    }
    let old = client
        .post(&url)
        .json(&serde_json::json!({"invite":invitation.invite,
        "email":"private@example.com","displayName":"Private joiner"}))
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    let pending: Vec<miliastra_core::coordination::AccessRequest> = client
        .get(&url)
        .bearer_auth(&joined.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    let contact = crate::privacy::open(
        &key,
        &server.endpoint,
        &joined.project.id,
        &pending[0],
        now(),
    )
    .unwrap();
    assert_eq!(contact.email, "private@example.com");
    client.post(format!("{url}/result")).bearer_auth(&joined.token)
        .json(&serde_json::json!({"id":id,"granted":false,"message":"provider leaked private@example.com"}))
        .send().await.unwrap().error_for_status().unwrap();
}
#[tokio::test]
async fn app_and_coordinator_agree_on_auth_presence_publication_history_and_removal() {
    let Some(server) = service() else { return };
    let client = FixtureClient::new();
    let baseline_id = uuid::Uuid::new_v4().to_string();
    let publication_id = uuid::Uuid::new_v4().to_string();
    let base = Revision {
        section: "5".into(),
        name: "Boats".into(),
        hash: "a".repeat(64),
        content_hash: "b".repeat(64),
        artifact: miliastra_core::history_artifact(&baseline_id, "5").unwrap(),
        size: 10,
        parent: None,
        shared_dependency: None,
    };
    let joined: Joined = client
        .post(format!("{}/projects", server.endpoint))
        .json(&CreateProject {
            id: uuid::Uuid::new_v4().to_string(),
            member_id: uuid::Uuid::new_v4().to_string(),
            summary: "Initialized project".into(),
            details: "Initial saved project version.".into(),
            name: "Practice".into(),
            display_name: "Owner".into(),
            storage: StorageConfig {
                kind: "folder".into(),
                location: "".into(),
                drive_id: None,
            },
            baseline: vec![base.clone()],
            baseline_id,
        })
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let invitation = joined.invitation.as_ref().unwrap().clone();
    let preview_keys = crate::private_metadata::Keys::import(
        &key_store().lock().unwrap()[&joined.project.id]
            .export()
            .unwrap(),
    )
    .unwrap();
    let preview = crate::invitation::details(invitation.clone(), &preview_keys)
        .await
        .unwrap();
    assert_eq!(preview.project_name, "Shared project");
    assert_eq!(preview.storage_kind, "folder");
    let metadata: serde_json::Value = client
        .post(format!("{}/invitations/resolve", server.endpoint))
        .json(&serde_json::json!({"project":invitation.project,"invite":invitation.invite}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let keys: std::collections::BTreeSet<_> = metadata
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "version",
            "endpoint",
            "project",
            "invite",
            "projectName",
            "storageKind",
            "storage",
            "owner"
        ])
    );
    assert!(metadata["owner"].get("name").is_none());
    assert_eq!(preview.owner_name, None);
    let mut invalid = invitation.clone();
    invalid.invite = uuid::Uuid::new_v4().to_string();
    assert!(crate::invitation::details(invalid, &preview_keys)
        .await
        .is_err());
    let url = format!("{}/projects/{}", server.endpoint, joined.project.id);
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let guest: Joined = client
        .post(format!("{url}/join"))
        .json(&serde_json::json!({"invite":joined.invitation.unwrap().invite,"displayName":"Alex"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    // Presence: an open connection means online; the in-game status stays encrypted.
    {
        use futures_util::SinkExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("{}/events", url.replace("http://", "ws://"))
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {}", guest.token).parse().unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let status = preview_keys
            .seal(
                &server.endpoint,
                &joined.project.id,
                &miliastra_core::presence::status_purpose(&guest.member),
                &serde_json::json!({ "game": true }),
            )
            .unwrap();
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({ "kind": "status", "status": status })
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let raw = reqwest::Client::new();
        let mut presence = serde_json::Value::Null;
        for _ in 0..50 {
            let view: serde_json::Value = raw
                .get(&url)
                .bearer_auth(&joined.token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            presence = view["presence"].clone();
            if presence[0]["status"].is_string() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(presence[0]["member"], guest.member.as_str());
        let sealed = presence[0]["status"].as_str().unwrap();
        assert!(!sealed.contains("game"), "the service only sees ciphertext");
        let opened = preview_keys
            .open_events(&server.endpoint, &joined.project.id, presence.clone())
            .unwrap();
        assert_eq!(opened[0]["status"]["game"], true);
        // Parse the response the same way Core::sync does.
        let project: ProjectView = client
            .get(&url)
            .bearer_auth(&joined.token)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&project).unwrap()["presence"][0]["status"]["game"],
            true
        );
        let cached = serde_json::to_string(&project).unwrap();
        let restored: ProjectView = serde_json::from_str(&cached).unwrap();
        assert_eq!(restored.presence, project.presence);
        drop(socket);
        for _ in 0..50 {
            let view: serde_json::Value = raw
                .get(&url)
                .bearer_auth(&joined.token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if view["presence"].as_array().is_some_and(|p| p.is_empty()) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }
    let lease: Lease = client
        .post(format!("{url}/locks/acquire"))
        .bearer_auth(&guest.token)
        .json(&serde_json::json!({"section":"5"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        client
            .post(format!("{url}/locks/acquire"))
            .bearer_auth(&joined.token)
            .json(&serde_json::json!({"section":"5"}))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let revision = Revision {
        hash: "c".repeat(64),
        artifact: miliastra_core::history_artifact(&publication_id, "5").unwrap(),
        parent: Some(base.hash),
        ..base
    };
    let req = PublishRequest {
        id: publication_id,
        summary: "Fixed the deck".into(),
        details: "Local files, real coordinator".into(),
        revisions: vec![revision],
        fences: std::collections::BTreeMap::from([("5".into(), lease.fence)]),
        restores: None,
    };
    for _ in 0..2 {
        let update: Update = client
            .post(format!("{url}/publish"))
            .bearer_auth(&guest.token)
            .json(&req)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(update.sequence, 2);
    }
    let history: HistoryPage = client
        .get(format!("{url}/history?section=5"))
        .bearer_auth(&joined.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[0].id, req.id);
    assert_eq!(history.entries[0].summary, "Fixed the deck");
    assert_eq!(history.entries[1].sequence, 1);
    assert_eq!(
        client
            .post(format!("{url}/publish"))
            .bearer_auth(&guest.token)
            .json(&serde_json::json!({"bytes":"forbidden"}))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(serde_json::to_value(&history.entries[0])
        .unwrap()
        .get("corrections")
        .is_none());
    assert_eq!(
        client
            .post(format!("{url}/history/{}/corrections", req.id))
            .bearer_auth(&joined.token)
            .json(&serde_json::json!({"text":"Unused"}))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let view: ProjectView = client
        .get(&url)
        .bearer_auth(&joined.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(view.sequence, 2);
    assert_eq!(view.heads["5"].hash, "c".repeat(64));
    // Team management: only the owner removes; a removed member is told why.
    let raw = reqwest::Client::new();
    let remove = |token: &str, member: &str| {
        raw.post(format!("{url}/members/remove"))
            .bearer_auth(token.to_owned())
            .json(&serde_json::json!({ "member": member }))
            .send()
    };
    assert!(!remove(&guest.token, &joined.member)
        .await
        .unwrap()
        .status()
        .is_success());
    remove(&joined.token, &guest.member)
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let denied = raw
        .get(&url)
        .bearer_auth(&guest.token)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = denied.json().await.unwrap();
    assert_eq!(body["error"], miliastra_core::coordination::REMOVED_MESSAGE);
}
