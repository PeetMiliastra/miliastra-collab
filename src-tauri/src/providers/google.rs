use super::*;
use crate::retry::SendRetrying;
use crate::storage::{client, enc};
use anyhow::ensure;
use miliastra_core::hash;
use serde_json::{json, Value};
const GOOGLE: &str = "https://www.googleapis.com/drive/v3";
const FOLDER: &str = "application/vnd.google-apps.folder";
pub struct GoogleDrive;
async fn google_child(
    c: &reqwest::Client,
    token: &str,
    api: &str,
    parent: &str,
    name: &str,
) -> Result<Option<Value>> {
    ensure!(
        parent
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Invalid Drive folder"
    );
    let name = name.replace('\\', "\\\\").replace('\'', "\\'");
    let q = format!("'{parent}' in parents and name = '{name}' and trashed = false");
    let v: Value = c
        .get(format!("{api}/files"))
        .bearer_auth(token)
        .query(&[
            ("q", q.as_str()),
            (
                "fields",
                "files(id,name,mimeType,md5Checksum,size,createdTime),nextPageToken",
            ),
            ("supportsAllDrives", "true"),
            ("includeItemsFromAllDrives", "true"),
        ])
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let files = v["files"].as_array().context("Drive listing unavailable")?;
    ensure!(
        v["nextPageToken"].is_null() && (files.len() <= 1 || identical_files(files)),
        "Duplicate storage names found. Ask the owner to remove the duplicate folder or file."
    );
    // Drive allows duplicate names, so two clients archiving the same file can create
    // identical copies. Read the oldest one.
    Ok(files
        .iter()
        .min_by_key(|f| {
            (
                f["createdTime"].as_str().unwrap_or_default().to_owned(),
                f["id"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .cloned())
}
fn identical_files(files: &[Value]) -> bool {
    let first = &files[0];
    files.iter().all(|f| {
        f["mimeType"].as_str().is_some_and(|m| m != FOLDER)
            && f["md5Checksum"].as_str().is_some()
            && f["size"].as_str().is_some()
            && f["createdTime"].as_str().is_some()
            && f["md5Checksum"] == first["md5Checksum"]
            && f["size"] == first["size"]
    })
}
// Copy History -> Latest on Drive's side: copy to a temporary name, check the
// checksums, then rename. History is never touched.
const COPY_FIELDS: &str = "id,name,mimeType,parents,size,sha256Checksum,createdTime,trashed";
fn drive_matches(v: &Value, expected_hash: &str, size: usize) -> bool {
    v["sha256Checksum"].as_str() == Some(expected_hash)
        && v["size"].as_str().and_then(|s| s.parse::<usize>().ok()) == Some(size)
        && v["mimeType"]
            .as_str()
            .is_some_and(|s| !s.starts_with("application/vnd.google-apps."))
        && v["trashed"] != true
}
async fn google_latest_files(
    c: &reqwest::Client,
    token: &str,
    api: &str,
    parent: &str,
    name: &str,
    stage: &str,
) -> Result<Vec<Value>> {
    // IDs and names come from the storage layer, never from user input.
    let quote = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");
    let q = format!(
        "'{}' in parents and (name = '{}' or name = '{}') and trashed = false",
        quote(parent),
        quote(name),
        quote(stage)
    );
    let v: Value = c
        .get(format!("{api}/files"))
        .bearer_auth(token)
        .query(&[
            ("q", q.as_str()),
            ("fields", &format!("files({COPY_FIELDS}),nextPageToken")),
            ("pageSize", "1000"),
            ("supportsAllDrives", "true"),
            ("includeItemsFromAllDrives", "true"),
        ])
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        v["nextPageToken"].is_null(),
        "Too many Latest copies; refresh stopped safely"
    );
    let files = v["files"]
        .as_array()
        .context("Drive listing unavailable")?
        .clone();
    for f in &files {
        ensure!(
            f["id"].as_str().is_some()
                && (f["name"] == name || f["name"] == stage)
                && f["parents"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|v| v == parent))
                && f["mimeType"]
                    .as_str()
                    .is_some_and(|m| !m.starts_with("application/vnd.google-apps.")),
            "Unexpected item in Latest; refresh stopped safely"
        );
    }
    Ok(files)
}
async fn google_copy_latest(
    c: &reqwest::Client,
    token: &str,
    api: &str,
    source: &str,
    parent: &str,
    name: &str,
    expected_hash: &str,
    size: usize,
) -> Result<()> {
    let source_file: Value = c
        .get(format!("{api}/files/{}", enc(source)))
        .bearer_auth(token)
        .query(&[("fields", COPY_FIELDS), ("supportsAllDrives", "true")])
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(source_file["id"] == source && drive_matches(&source_file, expected_hash, size),
        "Drive has not confirmed the History checksum yet. Latest will retry without uploading again.");
    let stage = format!(".{name}.{expected_hash}.pending");
    let before = google_latest_files(c, token, api, parent, name, &stage).await?;
    ensure!(
        before.iter().all(|f| f["id"] != source),
        "History cannot be replaced by Latest"
    );
    let mut candidates: Vec<Value> = before
        .iter()
        .filter(|f| drive_matches(f, expected_hash, size))
        .cloned()
        .collect();
    if candidates.is_empty() {
        let copied: Value = c
            .post(format!("{api}/files/{}/copy", enc(source)))
            .bearer_auth(token)
            .query(&[("fields", COPY_FIELDS), ("supportsAllDrives", "true")])
            .json(&json!({"name":stage,"parents":[parent]}))
            .send_retrying()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(
            copied["id"].as_str().is_some_and(|id| id != source)
                && copied["name"] == stage
                && copied["parents"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|v| v == parent))
                && drive_matches(&copied, expected_hash, size),
            "Drive copy could not be verified. The previous Latest file was kept."
        );
        candidates.push(copied);
    }
    // Clean up copies left by interrupted or simultaneous refreshes, keeping the oldest
    // verified one so every client agrees.
    let after = google_latest_files(c, token, api, parent, name, &stage).await?;
    candidates.extend(
        after
            .iter()
            .filter(|f| drive_matches(f, expected_hash, size))
            .cloned(),
    );
    for f in &candidates {
        ensure!(f["id"] != source, "History cannot be replaced by Latest");
        chrono::DateTime::parse_from_rfc3339(
            f["createdTime"]
                .as_str()
                .context("Drive copy time missing")?,
        )?;
    }
    candidates.sort_by_key(|f| {
        (
            chrono::DateTime::parse_from_rfc3339(f["createdTime"].as_str().unwrap()).unwrap(),
            f["id"].as_str().unwrap().to_owned(),
        )
    });
    let chosen = &candidates[0];
    let id = chosen["id"].as_str().context("Drive copy ID missing")?;
    if chosen["name"] != name {
        let promoted: Value = c
            .patch(format!("{api}/files/{}", enc(id)))
            .bearer_auth(token)
            .query(&[("fields", COPY_FIELDS), ("supportsAllDrives", "true")])
            .json(&json!({"name":name}))
            .send_retrying()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(
            promoted["id"] == id
                && promoted["name"] == name
                && promoted["parents"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|v| v == parent))
                && drive_matches(&promoted, expected_hash, size),
            "Drive did not confirm Latest promotion"
        );
    }
    // Only remove old files we saw before making our copy, plus verified duplicates of
    // it. A different revision that arrived meanwhile is left alone.
    let mut retire: Vec<String> = before
        .iter()
        .chain(candidates.iter())
        .filter_map(|f| f["id"].as_str())
        .filter(|other| *other != id && *other != source)
        .map(str::to_owned)
        .collect();
    retire.sort();
    retire.dedup();
    for other in retire {
        c.patch(format!("{api}/files/{}", enc(&other)))
            .bearer_auth(token)
            .query(&[("supportsAllDrives", "true")])
            .json(&json!({"trashed":true}))
            .send_retrying()
            .await?
            .error_for_status()?;
    }
    Ok(())
}
async fn google_folder(
    c: &reqwest::Client,
    token: &str,
    api: &str,
    parent: &str,
    name: &str,
    create: bool,
) -> Result<Option<String>> {
    if let Some(v) = google_child(c, token, api, parent, name).await? {
        ensure!(
            v["mimeType"] == FOLDER,
            "A file is blocking a project folder"
        );
        return Ok(Some(v["id"].as_str().context("No folder ID")?.into()));
    }
    if !create {
        return Ok(None);
    }
    let v: Value = c
        .post(format!("{api}/files"))
        .bearer_auth(token)
        .query(&[("fields", "id,parents"), ("supportsAllDrives", "true")])
        .json(&json!({"name":name,"mimeType":FOLDER,"parents":[parent]}))
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        v["parents"]
            .as_array()
            .is_some_and(|p| p.iter().any(|v| v == parent)),
        "Drive did not confirm the destination folder"
    );
    Ok(Some(v["id"].as_str().context("No folder ID")?.into()))
}
async fn google_project_folder(
    c: &reqwest::Client,
    token: &str,
    api: &str,
    name: &str,
) -> Result<String> {
    // Drive accepts "root" in requests but returns its canonical ID in parents.
    // Resolve it before validating creation responses against the destination.
    let drive_root: Value = c
        .get(format!("{api}/files/root"))
        .bearer_auth(token)
        .query(&[("fields", "id,mimeType")])
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        drive_root["mimeType"] == FOLDER,
        "Drive root is not a folder"
    );
    let drive_root = drive_root["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .context("Drive root folder ID is missing")?;
    let root = google_folder(c, token, api, drive_root, "miliastra-collab", true)
        .await?
        .context("No app folder")?;
    // Each new project gets its own folder.
    let v: Value = c
        .post(format!("{api}/files"))
        .bearer_auth(token)
        .query(&[("fields", "id,parents")])
        .json(&json!({"name":name.trim(),"mimeType":FOLDER,"parents":[root]}))
        .send_retrying()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        v["parents"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p == &root)),
        "Drive did not confirm the project folder"
    );
    Ok(v["id"].as_str().context("No project folder ID")?.into())
}

impl GoogleDrive {
    async fn file_id(&self, s: &Storage, k: &str) -> Result<Option<String>> {
        let Some((parent, name)) = s.parent(k, false).await? else {
            return Ok(None);
        };
        let v = google_child(&client()?, &s.token().await?, GOOGLE, &parent, &name).await?;
        if let Some(v) = &v {
            ensure!(v["mimeType"] != FOLDER, "Expected a file");
        }
        Ok(v.and_then(|v| v["id"].as_str().map(str::to_owned)))
    }
}
impl StorageBackend for GoogleDrive {
    fn get<'a>(&'a self, s: &'a Storage, k: &'a str) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let url = format!(
                "{GOOGLE}/files/{}?alt=media&supportsAllDrives=true",
                enc(&self
                    .file_id(s, k)
                    .await?
                    .context("Update is still syncing. Will retry.")?)
            );
            crate::storage::download(url, &s.token().await?).await
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
            let c = client()?;
            let token = s.token().await?;
            let (parent, name) = s
                .parent(k, true)
                .await?
                .context("Project folder unavailable")?;
            if let Some(file) = google_child(&c, &token, GOOGLE, &parent, &name).await? {
                ensure!(file["mimeType"] != FOLDER, "Expected a file");
                if !replace {
                    ensure!(
                        hash(&s.get(k).await?) == hash(&bytes),
                        "Immutable storage object changed"
                    );
                    return Ok(());
                }
                let id = file["id"].as_str().context("No file ID")?;
                c.patch(format!("https://www.googleapis.com/upload/drive/v3/files/{}?uploadType=media&supportsAllDrives=true",enc(id)))
                    .bearer_auth(token).header("Content-Type","application/octet-stream").body(bytes).send_retrying().await?.error_for_status()?;
            } else {
                let boundary = uuid::Uuid::new_v4().simple().to_string();
                let meta = json!({"name":name,"parents":[parent]}).to_string();
                let mut body=format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
                body.extend(bytes);
                body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
                c.post("https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&supportsAllDrives=true").bearer_auth(token)
                    .header("Content-Type",format!("multipart/related; boundary={boundary}")).body(body).send_retrying().await?.error_for_status()?;
            }

            Ok(())
        })
    }
    fn copy_latest<'a>(
        &'a self,
        s: &'a Storage,
        source: &'a str,
        target: &'a str,
        bytes: Vec<u8>,
        _journal: &'a crate::db::Db,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let source_id = self
                .file_id(s, source)
                .await?
                .context("History file is still syncing. Will retry.")?;
            let (parent, name) = s
                .parent(target, true)
                .await?
                .context("Latest folder unavailable")?;
            google_copy_latest(
                &client()?,
                &s.token().await?,
                GOOGLE,
                &source_id,
                &parent,
                &name,
                &hash(&bytes),
                bytes.len(),
            )
            .await
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
            let url = format!(
                "{GOOGLE}/files/{}?supportsAllDrives=true",
                enc(&self.file_id(s, k).await?.context("Check file missing")?)
            );
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

#[cfg(test)]
mod duplicate_tests {
    use super::*;
    #[test]
    fn only_byte_identical_drive_duplicates_are_tolerated() {
        let f = |id: &str, md5: &str| json!({"id":id,"mimeType":"application/json","md5Checksum":md5,"size":"3","createdTime":"2026-10-16T10:00:00.000Z"});
        assert!(identical_files(&[f("a", "x"), f("b", "x")]));
        assert!(!identical_files(&[f("a", "x"), f("b", "y")]));
        assert!(!identical_files(&[json!({"id":"a"}), json!({"id":"b"})]));
        let mut folder = f("b", "x");
        folder["mimeType"] = json!(FOLDER);
        assert!(!identical_files(&[f("a", "x"), folder]));
    }
}
#[cfg(test)]
mod cloud_folder_tests {
    use super::*;
    use axum::{
        extract::{Query, State},
        http::StatusCode,
        routing::get,
        Json, Router,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    #[derive(Clone)]
    struct Mock {
        mode: &'static str,
        posts: Arc<Mutex<Vec<Value>>>,
    }
    async fn drive_root(State(m): State<Mock>) -> (StatusCode, Json<Value>) {
        if m.mode == "root-denied" {
            return (StatusCode::FORBIDDEN, Json(json!({"error":"denied"})));
        }
        let value = match m.mode {
            "root-missing-id" => json!({"mimeType":FOLDER}),
            "root-empty-id" => json!({"id":"","mimeType":FOLDER}),
            "root-not-folder" => json!({"id":"actual-drive-root","mimeType":"text/plain"}),
            _ => json!({"id":"actual-drive-root","mimeType":FOLDER}),
        };
        (StatusCode::OK, Json(value))
    }
    async fn listing(
        State(m): State<Mock>,
        Query(q): Query<HashMap<String, String>>,
    ) -> Json<Value> {
        assert!(
            q["q"].contains("'actual-drive-root' in parents")
                || q["q"].contains("'root' in parents")
        );
        assert!(q["q"].contains("name = 'miliastra-collab'"));
        assert!(!q["q"].contains("Together"));
        Json(match m.mode {
            "existing" => {
                json!({"files":[{"id":"app-root","name":"miliastra-collab","mimeType":FOLDER}]})
            }
            "duplicate" => json!({"files":[{"id":"one"},{"id":"two"}]}),
            "file" => json!({"files":[{"id":"blocking-file","mimeType":"text/plain"}]}),
            _ => json!({"files":[]}),
        })
    }
    async fn create(State(m): State<Mock>, Json(v): Json<Value>) -> (StatusCode, Json<Value>) {
        m.posts.lock().unwrap().push(v.clone());
        if m.mode == "denied" {
            return (StatusCode::FORBIDDEN, Json(json!({"error":"denied"})));
        }
        assert_eq!(v["mimeType"], FOLDER);
        let root = v["name"] == "miliastra-collab";
        (
            StatusCode::OK,
            Json(
                json!({"id":if root{"app-root"}else{"project-folder"},"parents":if m.mode=="wrong-parent"{json!(["elsewhere"])}else if root{json!(["actual-drive-root"])}else{v["parents"].clone()}}),
            ),
        )
    }
    async fn run(mode: &'static str) -> (Result<String>, Vec<Value>) {
        let m = Mock {
            mode,
            posts: Arc::new(Mutex::new(vec![])),
        };
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap());
        let router = Router::new()
            .route("/files", get(listing).post(create))
            .route("/files/root", get(drive_root))
            .with_state(m.clone());
        let server = tokio::spawn(async move {
            axum::serve(socket, router).await.unwrap();
        });
        let result =
            google_project_folder(&reqwest::Client::new(), "fake-token", &url, "Fisch").await;
        server.abort();
        let posts = m.posts.lock().unwrap().clone();
        (result, posts)
    }
    #[tokio::test]
    async fn new_google_projects_have_an_explicit_app_parent_and_project_parent() {
        let (result, posts) = run("new").await;
        assert_eq!(result.unwrap(), "project-folder");
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0]["name"], "miliastra-collab");
        assert_eq!(posts[0]["parents"], json!(["actual-drive-root"]));
        assert_eq!(posts[1]["name"], "Fisch");
        assert_eq!(posts[1]["parents"], json!(["app-root"]));
        let (result, posts) = run("existing").await;
        assert_eq!(result.unwrap(), "project-folder");
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0]["parents"], json!(["app-root"]));
    }
    #[tokio::test]
    async fn invalid_or_inaccessible_drive_root_never_creates_folders() {
        for mode in [
            "root-denied",
            "root-missing-id",
            "root-empty-id",
            "root-not-folder",
        ] {
            let (result, posts) = run(mode).await;
            assert!(result.is_err(), "{mode}");
            assert!(posts.is_empty(), "{mode}");
        }
    }
    #[tokio::test]
    async fn folder_permission_failures_duplicates_files_and_wrong_parents_stop_provisioning() {
        for mode in ["denied", "duplicate", "file", "wrong-parent"] {
            let (result, posts) = run(mode).await;
            assert!(result.is_err(), "{mode}");
            assert!(posts.len() <= 1, "{mode}");
        }
    }
}

#[cfg(test)]
mod google_copy_tests {
    use super::*;
    use axum::{
        extract::{Path as ApiPath, Query, State},
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    #[derive(Default)]
    struct Data {
        files: Vec<Value>,
        copies: usize,
        patches: Vec<(String, Value)>,
        mode: &'static str,
    }
    type Mock = Arc<Mutex<Data>>;
    fn file(id: &str, name: &str, parent: &str, checksum: &str, second: u8) -> Value {
        json!({"id":id,"name":name,"parents":[parent],"size":"3",
            "sha256Checksum":checksum,"mimeType":"application/octet-stream",
            "createdTime":format!("2026-10-16T10:00:{second:02}.000Z"),"trashed":false})
    }
    async fn list(State(m): State<Mock>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
        assert!(q["q"].contains("'latest' in parents"));
        assert_eq!(q["supportsAllDrives"], "true");
        assert!(q["fields"].contains("sha256Checksum"));
        let d = m.lock().unwrap();
        Json(
            json!({"files":d.files.iter().filter(|f| f["parents"]==json!(["latest"]) && f["trashed"]!=true).collect::<Vec<_>>()}),
        )
    }
    async fn metadata(
        State(m): State<Mock>,
        ApiPath(id): ApiPath<String>,
        Query(q): Query<HashMap<String, String>>,
    ) -> Json<Value> {
        assert_eq!(id, "history");
        assert!(!q.contains_key("alt"), "Copy must never download media");
        Json(
            m.lock()
                .unwrap()
                .files
                .iter()
                .find(|f| f["id"] == id)
                .unwrap()
                .clone(),
        )
    }
    async fn copy(
        State(m): State<Mock>,
        ApiPath(id): ApiPath<String>,
        Query(q): Query<HashMap<String, String>>,
        Json(v): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        assert_eq!(id, "history");
        assert_eq!(q["supportsAllDrives"], "true");
        assert_eq!(v["parents"], json!(["latest"]));
        assert!(v["name"].as_str().unwrap().ends_with(".pending"));
        assert_eq!(
            v.as_object().unwrap().len(),
            2,
            "Only copy metadata is sent"
        );
        let mut d = m.lock().unwrap();
        d.copies += 1;
        if d.mode == "denied" {
            return (StatusCode::FORBIDDEN, Json(json!({})));
        }
        let mut f = file(
            &format!("copy-{}", d.copies),
            v["name"].as_str().unwrap(),
            "latest",
            &hash(b"new"),
            d.copies as u8,
        );
        if d.mode == "bad-copy" {
            f["sha256Checksum"] = json!(hash(b"bad"));
        }
        if d.mode == "wrong-parent" {
            f["parents"] = json!(["elsewhere"]);
        }
        d.files.push(f.clone());
        if d.mode == "copy-response-lost" {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
        }
        (StatusCode::OK, Json(f))
    }
    async fn patch(
        State(m): State<Mock>,
        ApiPath(id): ApiPath<String>,
        Json(v): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        assert_ne!(id, "history", "History must never be modified");
        let mut d = m.lock().unwrap();
        d.patches.push((id.clone(), v.clone()));
        if d.mode == "promotion-fails" && v.get("name").is_some() {
            return (StatusCode::FORBIDDEN, Json(json!({})));
        }
        let f = d.files.iter_mut().find(|f| f["id"] == id).unwrap();
        for (k, v) in v.as_object().unwrap() {
            f[k] = v.clone();
        }
        (StatusCode::OK, Json(f.clone()))
    }
    async fn fixture(mode: &'static str) -> (Mock, String, tokio::task::JoinHandle<()>) {
        let m = Arc::new(Mutex::new(Data {
            mode,
            files: vec![
                file("history", "1.gis", "history-folder", &hash(b"new"), 0),
                file("old", "1.gis", "latest", &hash(b"old"), 0),
            ],
            ..Default::default()
        }));
        if mode == "bad-source" {
            m.lock().unwrap().files[0]["sha256Checksum"] = Value::Null;
        }
        let sock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", sock.local_addr().unwrap());
        let app = Router::new()
            .route("/files", get(list))
            .route("/files/{id}", get(metadata).patch(patch))
            .route("/files/{id}/copy", post(copy))
            .with_state(m.clone());
        let server = tokio::spawn(async move {
            axum::serve(sock, app).await.unwrap();
        });
        (m, url, server)
    }
    async fn refresh(url: &str) -> Result<()> {
        google_copy_latest(
            &reqwest::Client::new(),
            "fake-token",
            url,
            "history",
            "latest",
            "1.gis",
            &hash(b"new"),
            3,
        )
        .await
    }
    fn assert_ready(d: &Data) {
        let latest: Vec<_> = d
            .files
            .iter()
            .filter(|f| {
                f["parents"] == json!(["latest"]) && f["name"] == "1.gis" && f["trashed"] != true
            })
            .collect();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0]["sha256Checksum"], hash(b"new"));
        assert_eq!(
            d.files[0],
            file("history", "1.gis", "history-folder", &hash(b"new"), 0)
        );
    }
    #[tokio::test]
    async fn server_copy_promotes_verified_bytes_and_retry_does_not_copy_again() {
        let (m, url, server) = fixture("").await;
        refresh(&url).await.unwrap();
        refresh(&url).await.unwrap();
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 1);
        assert!(d
            .patches
            .iter()
            .any(|(id, v)| id == "old" && v["trashed"] == true));
        server.abort();
    }
    #[tokio::test]
    async fn interrupted_promotion_reuses_the_staged_copy() {
        let (m, url, server) = fixture("promotion-fails").await;
        assert!(refresh(&url).await.is_err());
        {
            let mut d = m.lock().unwrap();
            assert_eq!(d.files[1]["trashed"], false);
            d.mode = "";
        }
        refresh(&url).await.unwrap();
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 1);
        server.abort();
    }
    #[tokio::test]
    async fn lost_copy_response_reuses_provider_copy_on_retry() {
        let (m, url, server) = fixture("copy-response-lost").await;
        assert!(refresh(&url).await.is_err());
        {
            let mut d = m.lock().unwrap();
            assert_eq!(d.files[1]["trashed"], false);
            d.mode = "";
        }
        refresh(&url).await.unwrap();
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 1);
        server.abort();
    }
    #[tokio::test]
    async fn duplicate_staged_copies_converge_on_oldest_verified_copy() {
        let (m, url, server) = fixture("").await;
        let stage = format!(".1.gis.{}.pending", hash(b"new"));
        {
            let mut d = m.lock().unwrap();
            d.files
                .push(file("second", &stage, "latest", &hash(b"new"), 2));
            d.files
                .push(file("first", &stage, "latest", &hash(b"new"), 1));
        }
        refresh(&url).await.unwrap();
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 0);
        assert!(d
            .files
            .iter()
            .any(|f| f["id"] == "first" && f["name"] == "1.gis" && f["trashed"] == false));
        assert!(d
            .files
            .iter()
            .any(|f| f["id"] == "second" && f["trashed"] == true));
        server.abort();
    }
    #[tokio::test]
    async fn unverified_or_denied_copies_preserve_previous_latest() {
        for mode in ["bad-source", "bad-copy", "wrong-parent", "denied"] {
            let (m, url, server) = fixture(mode).await;
            assert!(refresh(&url).await.is_err(), "{mode}");
            let d = m.lock().unwrap();
            assert_eq!(d.files[1]["trashed"], false);
            assert!(d.patches.is_empty());
            if mode == "bad-source" {
                assert_eq!(d.copies, 0);
            }
            server.abort();
        }
    }
    #[tokio::test]
    async fn concurrent_refreshes_reconcile_without_modifying_history() {
        let (m, url, server) = fixture("").await;
        let (a, b) = tokio::join!(refresh(&url), refresh(&url));
        assert!(a.is_ok() || b.is_ok());
        refresh(&url).await.unwrap();
        assert_ready(&m.lock().unwrap());
        server.abort();
    }
}

impl CloudProvider for GoogleDrive {
    fn oauth<'a>(&self, config: &'a OAuthConfig) -> OAuthRegistration<'a> {
        OAuthRegistration {
            client_id: &config.google_client_id,
            client_secret: (!config.google_client_secret.is_empty()).then_some(config.google_client_secret.as_str()),
            authorize: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
            scope: DRIVE_SCOPE, required_scope: Some(DRIVE_SCOPE), callback_host: "127.0.0.1",
            extra_authorization: &[("access_type", "offline"), ("prompt", "consent")],
            permission_message: "Allow miliastra/collab to see and edit your Google Drive files when signing in. It needs this to open team folders other people share with you.",
            reconnect_message: "Reconnect Google Drive in Settings so the app can open shared team folders",
        }
    }
    fn create_project<'a>(
        &'a self,
        name: &'a str,
        config: &'a OAuthConfig,
    ) -> BoxFuture<'a, Result<StorageConfig>> {
        Box::pin(async move {
            let token = crate::oauth::access("google", config).await?;
            let c = client()?;
            Ok(StorageConfig {
                kind: "google".into(),
                location: google_project_folder(&c, &token, GOOGLE, name).await?,
                drive_id: None,
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
            google_folder(&client()?, &s.token().await?, GOOGLE, parent, name, create).await
        })
    }
    fn account_email<'a>(&'a self, config: &'a OAuthConfig) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let token = crate::oauth::access("google", config).await?;
            let c = account_client()?;
            let email = {
                let v: Value = c
                    .get(format!("{GOOGLE}/about"))
                    .query(&[("fields", "user(emailAddress)")])
                    .bearer_auth(token)
                    .send_retrying()
                    .await?
                    .error_for_status()
                    .context("Could not read your Google account email. Reconnect Google Drive and retry.")?
                    .json()
                    .await?;
                v["user"]["emailAddress"].as_str().map(str::to_owned)
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
            let token = crate::oauth::access("google", config).await?;
            let c = account_client()?;
            let folder = urlencoding::encode(&storage.location).into_owned();
            let url = format!("{GOOGLE}/files/{folder}/permissions");
            let body = json!({"role":"writer","type":"user","emailAddress":email});
            let quiet = c
                .post(&url)
                .query(&[
                    ("sendNotificationEmail", "false"),
                    ("supportsAllDrives", "true"),
                ])
                .bearer_auth(&token)
                .json(&body)
                .send_retrying()
                .await?;
            if quiet.status().is_success() {
                return Ok(());
            }
            // Google requires a notification for addresses without a Google account.
            c.post(&url)
                .query(&[
                    ("sendNotificationEmail", "true"),
                    ("supportsAllDrives", "true"),
                ])
                .bearer_auth(&token)
                .json(&body)
                .send_retrying()
                .await?
                .error_for_status()
                .context("Google Drive refused to share the team folder")?;
            return Ok(());
        })
    }
    fn folder_url<'a>(
        &'a self,
        storage: &'a StorageConfig,
        _config: &'a OAuthConfig,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            Ok(format!(
                "https://drive.google.com/drive/folders/{}",
                enc(&storage.location)
            ))
        })
    }
}

pub const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
