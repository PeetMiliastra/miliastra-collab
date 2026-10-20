//! Copies History -> Latest on OneDrive's side, without downloading the files.
use crate::db::Db;
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use miliastra_core::hash;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::time::Duration;
const FIELDS: &str = "id,name,size,file,folder,parentReference,eTag,deleted";

// Microsoft's QuickXorHash: a 160-bit circular XOR with the length in the last 8 bytes.
// Only used to check OneDrive's copy; installs still check SHA-256.
fn quick_xor(bytes: &[u8]) -> String {
    let mut out = [0u8; 20];
    for (i, b) in bytes.iter().enumerate() {
        let bit = (i % 160) * 11 % 160;
        out[bit / 8] ^= b << (bit % 8);
        if bit % 8 != 0 {
            out[(bit / 8 + 1) % 20] ^= b >> (8 - bit % 8);
        }
    }
    for (i, b) in (bytes.len() as u64).to_le_bytes().iter().enumerate() {
        out[12 + i] ^= b;
    }
    STANDARD.encode(out)
}
struct Integrity {
    size: u64,
    sha1: String,
    xor: String,
}
impl Integrity {
    fn new(bytes: &[u8]) -> Self {
        Self {
            size: bytes.len() as u64,
            sha1: format!("{:x}", Sha1::digest(bytes)),
            xor: quick_xor(bytes),
        }
    }
    fn matches(&self, v: &Value) -> bool {
        if v["size"].as_u64() != Some(self.size)
            || !v["file"].is_object()
            || v["folder"].is_object()
            || !v["deleted"].is_null()
        {
            return false;
        }
        let h = &v["file"]["hashes"];
        // Check every hash OneDrive gives us (at least one). Graph doesn't support
        // sha256Hash; personal and business accounts both have quickXorHash.
        let sha1 = h["sha1Hash"].as_str().filter(|s| !s.is_empty());
        let xor = h["quickXorHash"].as_str().filter(|s| !s.is_empty());
        (sha1.is_some() || xor.is_some())
            && sha1.is_none_or(|s| s.eq_ignore_ascii_case(&self.sha1))
            && xor.is_none_or(|s| s == self.xor)
    }
}
fn monitor_url(raw: &str, api: &str) -> Result<url::Url> {
    let u = url::Url::parse(raw).context("OneDrive copy returned an invalid monitor address")?;
    ensure!(
        u.username().is_empty() && u.password().is_none() && u.fragment().is_none(),
        "Invalid OneDrive monitor address"
    );
    let host = u.host_str().unwrap_or("");
    let allowed = host == "graph.microsoft.com"
        || host == "api.onedrive.com"
        || host.ends_with(".sharepoint.com");
    let production = u.scheme() == "https" && u.port_or_known_default() == Some(443) && allowed;
    #[cfg(test)]
    let fixture = {
        let a = url::Url::parse(api)?;
        u.scheme() == "http" && host == "127.0.0.1" && u.origin() == a.origin()
    };
    #[cfg(not(test))]
    let fixture = {
        let _ = api;
        false
    };
    ensure!(production || fixture, "Untrusted OneDrive monitor address");
    Ok(u)
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Job {
    monitor: String,
    retry_at: i64,
}
struct Copy<'a> {
    c: reqwest::Client,
    api: &'a str,
    token: &'a str,
    drive: &'a str,
    db: &'a Db,
    interval: Duration,
}
impl Copy<'_> {
    fn base(&self) -> String {
        format!("{}/drives/{}", self.api, urlencoding::encode(self.drive))
    }
    async fn get(&self, address: &str) -> Result<Option<Value>> {
        let r = self
            .c
            .get(format!("{}{address}", self.base()))
            .bearer_auth(self.token)
            .query(&[("$select", FIELDS)])
            .send()
            .await?
            .error_for_status();
        match r {
            Err(e) if e.status() == Some(reqwest::StatusCode::NOT_FOUND) => Ok(None),
            Err(e) => Err(e.into()),
            Ok(r) => Ok(Some(r.json().await?)),
        }
    }
    async fn child(&self, parent: &str, name: &str) -> Result<Option<Value>> {
        let v = self
            .get(&format!(
                "/items/{}:/{}",
                urlencoding::encode(parent),
                urlencoding::encode(name)
            ))
            .await?;
        if let Some(v) = &v {
            ensure!(
                v["parentReference"]["id"] == parent
                    && v["parentReference"]["driveId"] == self.drive
                    && v["name"] == name
                    && v["id"].as_str().is_some()
                    && v["file"].is_object()
                    && !v["folder"].is_object(),
                "Unexpected item in OneDrive Latest"
            );
        }
        Ok(v)
    }
    async fn rename(&self, item: &Value, name: &str) -> Result<Value> {
        let id = item["id"].as_str().context("Missing OneDrive item ID")?;
        Ok(self
            .c
            .patch(format!("{}/items/{}", self.base(), urlencoding::encode(id)))
            .bearer_auth(self.token)
            .header(
                "If-Match",
                item["eTag"]
                    .as_str()
                    .context("Missing OneDrive version tag")?,
            )
            .json(&json!({"name":name}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    async fn recycle(&self, item: &Value) -> Result<()> {
        let id = item["id"].as_str().context("Missing OneDrive item ID")?;
        let r = self
            .c
            .delete(format!("{}/items/{}", self.base(), urlencoding::encode(id)))
            .bearer_auth(self.token)
            .header(
                "If-Match",
                item["eTag"]
                    .as_str()
                    .context("Missing OneDrive version tag")?,
            )
            .send()
            .await?;
        if r.status() != reqwest::StatusCode::NOT_FOUND {
            r.error_for_status()?;
        }
        Ok(())
    }
    async fn wait(&self, raw: &str, job: &str) -> Result<()> {
        let u = monitor_url(raw, self.api)?;
        if self
            .db
            .get::<Job>(job)?
            .is_some_and(|j| j.retry_at > chrono::Utc::now().timestamp())
        {
            anyhow::bail!(
                "OneDrive requested a pause. Copy progress is saved and will resume automatically."
            );
        }
        for _ in 0..8 {
            // The copy-status URL works on its own: don't send our token to it, don't follow
            // redirects, and keep it out of error messages.
            let r = self.c.get(u.clone()).send().await.map_err(|_| {
                anyhow::anyhow!("Could not check OneDrive copy progress. Will retry.")
            })?;
            let status = r.status();
            let delay = r
                .headers()
                .get("Retry-After")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| {
                    s.parse::<u64>().ok().or_else(|| {
                        chrono::DateTime::parse_from_rfc2822(s).ok().map(|at| {
                            (at.timestamp() - chrono::Utc::now().timestamp()).max(1) as u64
                        })
                    })
                })
                .unwrap_or(1)
                .max(1);
            if status.is_redirection() {
                return Ok(());
            }
            if status == 429 || status == 503 {
                self.pause(raw, job, delay)?;
                tokio::time::sleep(self.interval * delay as u32).await;
                continue;
            }
            if status == 404 || status == 410 {
                self.db.remove(job)?;
                anyhow::bail!("OneDrive copy status expired. Will reconcile and retry.");
            }
            ensure!(
                status.is_success(),
                "OneDrive copy progress is unavailable. Will retry."
            );
            let v: Value = r
                .json()
                .await
                .map_err(|_| anyhow::anyhow!("OneDrive copy progress was unreadable"))?;
            match v["status"].as_str() {
                Some("completed") => return Ok(()),
                Some("failed" | "deleteFailed" | "cancelled") => {
                    self.db.remove(job)?;
                    anyhow::bail!(
                        "OneDrive could not complete the copy. The previous Latest file was kept."
                    );
                }
                Some("inProgress" | "notStarted" | "waiting" | "updating" | "running") => (),
                _ => anyhow::bail!("OneDrive returned an unknown copy status. Will retry."),
            }
            self.pause(raw, job, delay)?;
            tokio::time::sleep(self.interval * delay as u32).await;
        }
        anyhow::bail!("OneDrive is still copying this slice. Progress is saved and will resume automatically.")
    }
    fn pause(&self, raw: &str, key: &str, seconds: u64) -> Result<()> {
        if seconds > 5 {
            self.db.set(
                key,
                &Job {
                    monitor: raw.into(),
                    retry_at: chrono::Utc::now()
                        .timestamp()
                        .saturating_add(seconds.min(i64::MAX as u64) as i64),
                },
            )?;
            anyhow::bail!(
                "OneDrive requested a pause. Copy progress is saved and will resume automatically."
            );
        }
        Ok(())
    }
    async fn run(
        &self,
        root: &str,
        source: &str,
        parent: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let encoded = source
            .split('/')
            .map(|s| urlencoding::encode(s).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let original = self
            .get(&format!("/items/{}:/{encoded}", urlencoding::encode(root)))
            .await?
            .context("History file is still syncing. Will retry.")?;
        let integrity = Integrity::new(bytes);
        ensure!(
            integrity.matches(&original),
            "OneDrive has not confirmed the History checksum. Will retry without uploading again."
        );
        let original_id = original["id"].as_str().context("History item ID missing")?;
        let stage = format!(".{name}.{}.pending", hash(bytes));
        let backup = format!(".{name}.previous");
        let job = format!(
            "onedrive-copy:{}",
            hash(&serde_json::to_vec(&(
                self.drive,
                parent,
                name,
                hash(bytes)
            ))?)
        );
        let mut current = self.child(parent, name).await?;
        if !current.as_ref().is_some_and(|v| integrity.matches(v)) {
            let mut staged = self.child(parent, &stage).await?;
            if staged.is_none() {
                let monitor = if let Some(saved) = self.db.get::<Job>(&job)? {
                    saved.monitor
                } else {
                    let r = self.c.post(format!("{}/items/{}/copy", self.base(), urlencoding::encode(original_id)))
                        .bearer_auth(self.token).json(&json!({"parentReference":{"driveId":self.drive,"id":parent},"name":stage}))
                        .send().await?.error_for_status()?;
                    ensure!(
                        r.status() == reqwest::StatusCode::ACCEPTED,
                        "OneDrive did not accept the copy job"
                    );
                    let raw = r
                        .headers()
                        .get("Location")
                        .context("OneDrive copy monitor missing")?
                        .to_str()?
                        .to_owned();
                    monitor_url(&raw, self.api)?;
                    self.db.set(
                        &job,
                        &Job {
                            monitor: raw.clone(),
                            retry_at: 0,
                        },
                    )?;
                    raw
                };
                self.wait(&monitor, &job).await?;
                staged = self.child(parent, &stage).await?;
            }
            let staged = staged.context("OneDrive copy is still syncing. Will retry.")?;
            ensure!(
                staged["id"] != original_id && integrity.matches(&staged),
                "OneDrive staged copy checksum failed. The previous Latest file was kept."
            );
            self.db.remove(&job)?;
            current = self.child(parent, name).await?;
            if !current.as_ref().is_some_and(|v| integrity.matches(v)) {
                if let Some(old) = &current {
                    ensure!(
                        old["id"] != original_id,
                        "History cannot be replaced by Latest"
                    );
                    if let Some(previous) = self.child(parent, &backup).await? {
                        ensure!(
                            previous["id"] != original_id,
                            "History cannot be replaced by Latest"
                        );
                        self.recycle(&previous).await?;
                    }
                    self.rename(old, &backup).await?;
                }
                // Unique names and eTag checks make simultaneous swaps fail safely. After a crash,
                // the next run finds the verified copy and the previous file.
                if let Err(e) = self.rename(&staged, name).await {
                    let now = self.child(parent, name).await?;
                    if !now.as_ref().is_some_and(|v| integrity.matches(v)) {
                        if now.is_none() {
                            if let Some(previous) = self.child(parent, &backup).await? {
                                ensure!(
                                    previous["id"] != original_id,
                                    "History cannot be replaced by Latest"
                                );
                                let _ = self.rename(&previous, name).await;
                            }
                        }
                        return Err(e);
                    }
                }
            }
        }
        let installed = self
            .child(parent, name)
            .await?
            .context("Latest promotion is still syncing")?;
        ensure!(
            installed["id"] != original_id && integrity.matches(&installed),
            "OneDrive Latest checksum could not be confirmed"
        );
        self.db.remove(&job)?;
        if let Some(staged) = self.child(parent, &stage).await? {
            ensure!(
                staged["id"] != original_id,
                "History cannot be replaced by Latest"
            );
            self.recycle(&staged).await?;
        }
        if let Some(previous) = self.child(parent, &backup).await? {
            ensure!(
                previous["id"] != original_id,
                "History cannot be replaced by Latest"
            );
            self.recycle(&previous).await?;
        }
        Ok(())
    }
}
pub async fn latest(
    token: &str,
    drive: &str,
    root: &str,
    source: &str,
    parent: &str,
    name: &str,
    bytes: &[u8],
    db: &Db,
) -> Result<()> {
    let c = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()?;
    Copy {
        c,
        api: "https://graph.microsoft.com/v1.0",
        token,
        drive,
        db,
        interval: Duration::from_secs(1),
    }
    .run(root, source, parent, name, bytes)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::State,
        http::{HeaderMap, Method, StatusCode, Uri},
        response::{IntoResponse, Response},
        Router,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    struct Data {
        files: HashMap<String, Value>,
        mode: &'static str,
        copies: usize,
        polls: usize,
        staged_name: String,
        url: String,
    }
    type Mock = Arc<Mutex<Data>>;
    fn item(id: &str, name: &str, parent: &str, bytes: &[u8]) -> Value {
        json!({"id":id,"name":name,"size":bytes.len(),"file":{"hashes":{"quickXorHash":quick_xor(bytes)}},
            "parentReference":{"id":parent,"driveId":"drive"},"eTag":format!("tag-{id}-0")})
    }
    fn response(code: StatusCode, v: Value) -> Response {
        (code, axum::Json(v)).into_response()
    }
    async fn handle(
        State(m): State<Mock>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let path = urlencoding::decode(uri.path()).unwrap().into_owned();
        let mut d = m.lock().unwrap();
        if path == "/monitor/job" {
            assert!(
                !headers.contains_key("authorization"),
                "Monitor must never receive OAuth credentials"
            );
            d.polls += 1;
            if d.mode == "failed" {
                return response(StatusCode::OK, json!({"status":"failed"}));
            }
            if d.mode == "throttle" && d.polls == 1 {
                return (StatusCode::TOO_MANY_REQUESTS, [("Retry-After", "1")]).into_response();
            }
            if d.mode == "backoff" {
                return (StatusCode::TOO_MANY_REQUESTS, [("Retry-After", "3600")]).into_response();
            }
            if d.mode == "timeout" || d.polls == 1 {
                return response(StatusCode::ACCEPTED, json!({"status":"inProgress"}));
            }
            let name = d.staged_name.clone();
            if !d.files.contains_key("copy") {
                let mut f = item("copy", &name, "latest", b"new");
                if d.mode == "bad-copy" {
                    f["file"]["hashes"]["quickXorHash"] = json!(quick_xor(b"bad"));
                }
                if d.mode == "wrong-parent" {
                    f["parentReference"]["id"] = json!("elsewhere");
                }
                d.files.insert("copy".into(), f);
            }
            if d.mode == "redirect" {
                return (
                    StatusCode::SEE_OTHER,
                    [("Location", "https://untrusted.example/content")],
                )
                    .into_response();
            }
            return response(
                StatusCode::OK,
                json!({"status":"completed","resourceId":"copy"}),
            );
        }
        assert_eq!(headers.get("authorization").unwrap(), "Bearer fake-token");
        assert!(path.starts_with("/drives/drive/items/"));
        assert!(
            !path.ends_with("/content"),
            "No media transfers are allowed"
        );
        let address = path.strip_prefix("/drives/drive/items/").unwrap();
        if method == Method::GET {
            if address == "root:/History/update/1.gis" {
                return response(StatusCode::OK, d.files["history"].clone());
            }
            if let Some(name) = address.strip_prefix("latest:/") {
                let f = d
                    .files
                    .values()
                    .find(|f| f["parentReference"]["id"] == "latest" && f["name"] == name);
                return match f {
                    Some(f) => response(StatusCode::OK, f.clone()),
                    None => response(StatusCode::NOT_FOUND, json!({})),
                };
            }
            panic!("Unexpected metadata path: {address}");
        }
        if method == Method::POST {
            assert_eq!(address, "history/copy");
            assert!(
                uri.query().is_none(),
                "Consumer copy cannot use conflictBehavior"
            );
            let v: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                v.as_object().unwrap().len(),
                2,
                "Copy request must contain metadata only"
            );
            assert_eq!(
                v["parentReference"],
                json!({"id":"latest","driveId":"drive"})
            );
            d.copies += 1;
            d.staged_name = v["name"].as_str().unwrap().into();
            if d.mode == "denied" {
                return response(StatusCode::FORBIDDEN, json!({}));
            }
            if d.mode == "lost-response" {
                let f = item("copy", &d.staged_name, "latest", b"new");
                d.files.insert("copy".into(), f);
                return response(StatusCode::INTERNAL_SERVER_ERROR, json!({}));
            }
            let monitor = if d.mode == "unsafe-monitor" {
                "https://untrusted.example/monitor".into()
            } else {
                format!("{}/monitor/job", d.url)
            };
            return (StatusCode::ACCEPTED, [("Location", monitor)]).into_response();
        }
        assert_ne!(address, "history", "History must never be changed");
        let Some(f) = d.files.get(address) else {
            return response(StatusCode::NOT_FOUND, json!({}));
        };
        assert!(
            headers.contains_key("if-match"),
            "Latest mutations require conditional version tags"
        );
        if headers["if-match"] != f["eTag"].as_str().unwrap() {
            return response(StatusCode::PRECONDITION_FAILED, json!({}));
        }
        if method == Method::DELETE {
            d.files.remove(address);
            return StatusCode::NO_CONTENT.into_response();
        }
        assert_eq!(method, Method::PATCH);
        let v: Value = serde_json::from_slice(&body).unwrap();
        let name = v["name"].as_str().unwrap();
        if d.mode == "promotion-fails" && address == "copy" && name == "1.gis" {
            return response(StatusCode::FORBIDDEN, json!({}));
        }
        if d.mode == "etag-race" && address == "old" {
            return response(StatusCode::PRECONDITION_FAILED, json!({}));
        }
        if d.files.iter().any(|(id, f)| {
            id != address && f["parentReference"]["id"] == "latest" && f["name"] == name
        }) {
            return response(StatusCode::CONFLICT, json!({}));
        }
        let f = d.files.get_mut(address).unwrap();
        f["name"] = json!(name);
        f["eTag"] = json!(format!("{}-next", f["eTag"].as_str().unwrap()));
        response(StatusCode::OK, f.clone())
    }
    async fn fixture(mode: &'static str) -> (Mock, String, tokio::task::JoinHandle<()>) {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", socket.local_addr().unwrap());
        let mut files = HashMap::new();
        files.insert(
            "history".into(),
            item("history", "1.gis", "history-folder", b"new"),
        );
        files.insert("old".into(), item("old", "1.gis", "latest", b"old"));
        if mode == "bad-source" {
            files.get_mut("history").unwrap()["file"]["hashes"] = json!({});
        }
        let m = Arc::new(Mutex::new(Data {
            files,
            mode,
            copies: 0,
            polls: 0,
            staged_name: String::new(),
            url: url.clone(),
        }));
        let app = Router::new().fallback(handle).with_state(m.clone());
        let server = tokio::spawn(async move {
            axum::serve(socket, app).await.unwrap();
        });
        (m, url, server)
    }
    async fn refresh(url: &str, db: &Db) -> Result<()> {
        let c = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Copy {
            c,
            api: url,
            token: "fake-token",
            drive: "drive",
            db,
            interval: Duration::ZERO,
        }
        .run("root", "History/update/1.gis", "latest", "1.gis", b"new")
        .await
    }
    fn assert_ready(d: &Data) {
        let current: Vec<_> = d
            .files
            .values()
            .filter(|f| f["parentReference"]["id"] == "latest")
            .collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0]["name"], "1.gis");
        assert!(Integrity::new(b"new").matches(current[0]));
        assert_eq!(
            d.files["history"],
            item("history", "1.gis", "history-folder", b"new")
        );
    }
    #[test]
    fn provider_integrity_handles_personal_and_business_hashes() {
        assert_eq!(quick_xor(b""), "AAAAAAAAAAAAAAAAAAAAAAAAAAA=");
        assert_eq!(quick_xor(b"a"), "YQAAAAAAAAAAAAAAAQAAAAAAAAA=");
        // A slow bit-by-bit version, to check wraparound and the length encoding.
        for len in [1, 19, 20, 159, 160, 161, 1024, 10001] {
            let bytes: Vec<_> = (0..len).map(|i| (i * 37) as u8).collect();
            let mut reference = [0u8; 20];
            for (i, b) in bytes.iter().enumerate() {
                for bit in 0..8 {
                    if b & (1 << bit) != 0 {
                        let at = (i * 11 + bit) % 160;
                        reference[at / 8] ^= 1 << (at % 8);
                    }
                }
            }
            for (i, b) in (len as u64).to_le_bytes().iter().enumerate() {
                reference[12 + i] ^= b;
            }
            assert_eq!(quick_xor(&bytes), STANDARD.encode(reference));
        }
        let mut v = item("copy", "1.gis", "latest", b"new");
        assert!(Integrity::new(b"new").matches(&v));
        v["file"]["hashes"] = json!({"sha1Hash":"C2A6B03F190DFB2B4AA91F8AF8D477A9BC3401DC"});
        assert!(Integrity::new(b"new").matches(&v));
        v["file"]["hashes"]["quickXorHash"] = json!(quick_xor(b"bad"));
        assert!(!Integrity::new(b"new").matches(&v));
        v["file"]["hashes"] = json!({"sha256Hash":hash(b"new")});
        assert!(!Integrity::new(b"new").matches(&v));
    }
    #[test]
    fn monitor_addresses_reject_credentials_and_untrusted_hosts() {
        for raw in [
            "https://evil.example/monitor",
            "http://api.onedrive.com/monitor",
            "https://api.onedrive.com.evil.example/monitor",
            "https://user@api.onedrive.com/monitor",
            "https://api.onedrive.com:444/monitor",
        ] {
            assert!(monitor_url(raw, "https://graph.microsoft.com/v1.0").is_err());
        }
        for raw in [
            "https://api.onedrive.com/monitor/job",
            "https://tenant.sharepoint.com/_api/v2.0/monitor/job",
            "https://graph.microsoft.com/v1.0/monitor/job",
        ] {
            assert!(monitor_url(raw, "https://graph.microsoft.com/v1.0").is_ok());
        }
    }
    #[tokio::test]
    async fn asynchronous_copy_uses_metadata_only_and_reuses_latest() {
        for mode in ["", "throttle", "redirect"] {
            let (m, url, server) = fixture(mode).await;
            let db = Db::open(std::path::Path::new(":memory:")).unwrap();
            refresh(&url, &db).await.unwrap();
            refresh(&url, &db).await.unwrap();
            let d = m.lock().unwrap();
            assert_ready(&d);
            assert_eq!(d.copies, 1);
            assert!(d.polls >= 2);
            server.abort();
        }
    }
    #[tokio::test]
    async fn failures_keep_old_latest_and_never_change_history() {
        for mode in [
            "bad-source",
            "bad-copy",
            "wrong-parent",
            "denied",
            "failed",
            "unsafe-monitor",
            "etag-race",
        ] {
            let (m, url, server) = fixture(mode).await;
            let db = Db::open(std::path::Path::new(":memory:")).unwrap();
            assert!(refresh(&url, &db).await.is_err(), "{mode}");
            let d = m.lock().unwrap();
            assert_eq!(d.files["old"]["name"], "1.gis", "{mode}");
            assert_eq!(d.files["history"]["name"], "1.gis");
            if mode == "bad-source" {
                assert_eq!(d.copies, 0);
            }
            if mode == "unsafe-monitor" {
                assert_eq!(d.polls, 0);
            }
            server.abort();
        }
    }
    #[tokio::test]
    async fn timeout_and_restart_resume_the_saved_job_without_another_copy() {
        let (m, url, server) = fixture("timeout").await;
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&root).unwrap();
        let path = root.join("collab.db");
        {
            let db = Db::open(&path).unwrap();
            assert!(refresh(&url, &db).await.is_err());
            assert_eq!(m.lock().unwrap().copies, 1);
        }
        m.lock().unwrap().mode = "";
        {
            let db = Db::open(&path).unwrap();
            refresh(&url, &db).await.unwrap();
        }
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 1);
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn long_retry_after_is_saved_and_blocks_early_monitor_requests() {
        let (m, url, server) = fixture("backoff").await;
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        assert!(refresh(&url, &db).await.is_err());
        assert!(refresh(&url, &db).await.is_err());
        {
            let d = m.lock().unwrap();
            assert_eq!(d.copies, 1);
            assert_eq!(d.polls, 1);
        }
        let key = format!(
            "onedrive-copy:{}",
            hash(&serde_json::to_vec(&("drive", "latest", "1.gis", hash(b"new"))).unwrap())
        );
        let mut job: Job = db.get(&key).unwrap().unwrap();
        job.retry_at = 0;
        db.set(&key, &job).unwrap();
        m.lock().unwrap().mode = "";
        refresh(&url, &db).await.unwrap();
        assert_ready(&m.lock().unwrap());
        server.abort();
    }
    #[tokio::test]
    async fn promotion_failure_restores_previous_and_reuses_staged_copy() {
        let (m, url, server) = fixture("promotion-fails").await;
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        assert!(refresh(&url, &db).await.is_err());
        assert_eq!(m.lock().unwrap().files["old"]["name"], "1.gis");
        m.lock().unwrap().mode = "";
        refresh(&url, &db).await.unwrap();
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 1);
        server.abort();
    }
    #[tokio::test]
    async fn lost_copy_response_and_interrupted_swap_reconcile_without_another_copy() {
        let (m, url, server) = fixture("lost-response").await;
        let db = Db::open(std::path::Path::new(":memory:")).unwrap();
        assert!(refresh(&url, &db).await.is_err());
        {
            let mut d = m.lock().unwrap();
            d.mode = "";
            d.files.get_mut("old").unwrap()["name"] = json!(".1.gis.previous");
        }
        refresh(&url, &db).await.unwrap();
        let d = m.lock().unwrap();
        assert_ready(&d);
        assert_eq!(d.copies, 1);
        server.abort();
    }
    #[tokio::test]
    async fn concurrent_swaps_converge_without_deleting_history() {
        let (m, url, server) = fixture("").await;
        let a = Db::open(std::path::Path::new(":memory:")).unwrap();
        let b = Db::open(std::path::Path::new(":memory:")).unwrap();
        let _ = tokio::join!(refresh(&url, &a), refresh(&url, &b));
        refresh(&url, &a).await.unwrap();
        assert_ready(&m.lock().unwrap());
        server.abort();
    }
}
