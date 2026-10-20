use crate::{
    db::Db,
    notifications::Notice,
    oauth::{self, OAuthConfig},
    storage::Storage,
};
use anyhow::{ensure, Context, Result};
use miliastra_core::{
    binary::{self, Stage},
    *,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalProject {
    pub endpoint: String,
    pub project: ProjectView,
    pub member: String,
    pub stage: Stage,
    pub folder: Option<PathBuf>,
    pub installed: BTreeMap<String, Revision>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub endpoint: String,
    pub save_root: PathBuf,
    pub oauth: OAuthConfig,
    pub autostart: bool,
    pub screen_recognition: bool,
    /// Apply teammates' updates automatically when nothing conflicts and the stage isn't loaded.
    #[serde(default = "enabled")]
    pub auto_apply: bool,
}
fn enabled() -> bool {
    true
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: option_env!("MCOLLAB_SERVICE_URL")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(crate::coordination::HOSTED_SERVICE_URL)
                .into(),
            save_root: PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
                .join("AppData/LocalLow/miHoYo/Genshin Impact/BeyondLocal"),
            oauth: OAuthConfig {
                microsoft_client_id: oauth::registered_microsoft_client_id().into(),
                google_client_id: oauth::registered_google_client_id().into(),
                google_client_secret: oauth::registered_google_client_secret().into(),
            },
            autostart: false,
            screen_recognition: false,
            auto_apply: true,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub summary: String,
    pub details: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionView {
    pub id: String,
    pub name: String,
    pub changed: bool,
    pub conflict: bool,
    pub comparison_pending: bool,
    pub lock: Option<Lease>,
    pub downloaded: bool,
    pub download_error: Option<String>,
    pub installed: bool,
    pub installation_pending: bool,
    /// Who published the incoming revision and their changelog line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming: Option<Incoming>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub author: String,
    pub summary: String,
    pub at: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub test_data_directory: Option<PathBuf>,
    pub metadata_pending: bool,
    pub storage_issue: Option<crate::recovery::StorageIssue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_profile: Option<String>,
    pub cloud_providers: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_overlay_hwnd: Option<isize>,
    pub storage_accounts: BTreeMap<String, oauth::ConnectionStatus>,
    pub settings: Settings,
    pub stage: Option<Stage>,
    pub project: Option<ProjectView>,
    pub member: Option<String>,
    pub sections: Vec<SectionView>,
    pub draft: Draft,
    pub online: bool,
    pub protocol_mismatch: bool,
    pub message: String,
    pub compatibility: crate::installer::Compatibility,
    pub game: crate::native::GameState,
    pub installation: Option<crate::installer::Installation>,
    pub name_sync: Option<crate::names::Status>,
    pub practice: bool,
    /// Set when the owner has removed this member from the active project.
    pub removed: bool,
    pub task: Option<Task>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    pub kind: String,
    pub selected: Vec<String>,
    pub manual_selection: bool,
    pub installation_id: Option<String>,
}
/// Fetches History pages in the background until the local cache is complete.
/// The History screen itself always reads from SQLite.
fn next_history_cursor(
    known: u64,
    full: bool,
    before: Option<u64>,
    page: &HistoryPage,
) -> Result<Option<u64>> {
    if page.next.is_none() {
        return Ok(None);
    }
    let next = page.next.unwrap();
    ensure!(
        !page.entries.is_empty() && before.is_none_or(|old| next < old),
        "The service returned an invalid history cursor"
    );
    let reached_cached = known > 0 && page.entries.last().is_some_and(|u| u.sequence <= known);
    Ok(if !full && reached_cached {
        None
    } else {
        Some(next)
    })
}
pub struct Core {
    pub coordinator: crate::coordinator_client::CoordinatorClient,
    pub app: AppHandle,
    pub data: PathBuf,
    pub db: Db,
    pub settings: Mutex<Settings>,
    pub active: Mutex<Option<LocalProject>>,
    pub leases: Mutex<BTreeMap<String, Lease>>,
    pub online: AtomicBool,
    pub dirty: AtomicBool,
    pub operation: tokio::sync::Mutex<()>,
    pub transfers: crate::transfer_queue::TransferQueue,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub message: Mutex<String>,
    pub notice_sequence: AtomicU64,
    pub last_notice: Mutex<Option<(String, i64)>>,
    pub observed: Mutex<Option<String>>,
    pub editor: crate::editor_detector::EditorDetector,
    pub auto_apply_attempt: Mutex<Option<(String, i64)>>,
    pub fullscreen_warned: AtomicBool,
    pub lease_activity: Mutex<BTreeMap<String, i64>>,
    pub idle_reminded: Mutex<std::collections::BTreeSet<String>>,
    pub conflict_warnings: Mutex<std::collections::BTreeSet<(String, u64)>>,
    pub held_notices: Mutex<Vec<String>>,
}
fn removed_key(project: &str) -> String {
    format!("removed:{project}")
}
/// Reservations without saves for this long are released automatically.
pub const IDLE_RELEASE_SECS: i64 = 30 * 60;
pub use crate::editor_detector::stage_released;
pub fn endpoint(s: &str) -> Result<String> {
    let u = url::Url::parse(s)?;
    ensure!(
        u.scheme() == "https"
            || (u.scheme() == "http"
                && ["127.0.0.1", "localhost", "[::1]"].contains(&u.host_str().unwrap_or(""))),
        "The coordination service needs HTTPS (except localhost development)"
    );
    ensure!(
        u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none(),
        "Invalid service URL"
    );
    Ok(s.trim_end_matches('/').into())
}
pub(crate) async fn check_creation_service(ep: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client.get(format!("{ep}/health")).send().await.context("The collaboration service is unavailable. Check its address before creating a project.")?.error_for_status().context("The collaboration service is unavailable. Check its address before creating a project.")?;
    ensure!(response.text().await?.trim() == "ok", "The service address did not respond as a collaboration service. Check its address before creating a project.");
    Ok(())
}
impl Core {
    pub fn new(app: AppHandle) -> Result<Self> {
        let synthetic = if cfg!(debug_assertions) {
            std::env::var_os("MCOLLAB_TEST_DATA_DIR").map(PathBuf::from)
        } else {
            None
        };
        let data = if let Some(root) = synthetic {
            root
        } else if let Some(name) = crate::profile::current() {
            app.path().app_local_data_dir()?.join("profiles").join(name)
        } else {
            app.path().app_local_data_dir()?
        };
        std::fs::create_dir_all(&data)?;
        crate::credentials::initialize(data.join("credentials"))?;
        std::fs::create_dir_all(data.join("objects"))?;
        std::fs::create_dir_all(data.join("exports"))?;
        let db = Db::open(&data.join("collab.db"))?;
        let mut settings: Settings = db.get("settings")?.unwrap_or_default();
        let config = data.join(crate::coordination::FILE_NAME);
        settings.endpoint =
            crate::coordination::load_or_create(&config, &Settings::default().endpoint)?;
        // A loopback address saved by a test build points at a test coordinator, which
        // normal builds never use.
        if !cfg!(test_build) && crate::coordination::is_loopback(&settings.endpoint) {
            settings.endpoint = Settings::default().endpoint;
            crate::coordination::save(&config, &settings.endpoint)?;
        }

        settings.oauth.apply_registration_defaults();
        let active: Option<LocalProject> = db.get("active")?;
        if let Some(p) = &active {
            for id in p.project.heads.keys() {
                db.remove(&format!("latest:{}:{}", p.project.id, id))?;
            }
        }
        Ok(Self {
            coordinator: crate::coordinator_client::CoordinatorClient::new()?,
            app,
            data,
            db,
            settings: Mutex::new(settings),
            active: Mutex::new(active),
            leases: Mutex::new(BTreeMap::new()),
            online: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            operation: tokio::sync::Mutex::new(()),
            transfers: crate::transfer_queue::TransferQueue::default(),
            watcher: Mutex::new(None),
            message: Mutex::new(String::new()),
            notice_sequence: AtomicU64::new(0),
            last_notice: Mutex::new(None),
            observed: Mutex::new(None),
            editor: Default::default(),
            auto_apply_attempt: Mutex::new(None),
            fullscreen_warned: AtomicBool::new(false),
            lease_activity: Mutex::new(BTreeMap::new()),
            idle_reminded: Mutex::new(Default::default()),
            conflict_warnings: Mutex::new(Default::default()),
            held_notices: Mutex::new(vec![]),
        })
    }
    pub fn active(&self) -> Result<LocalProject> {
        self.active
            .lock()
            .unwrap()
            .clone()
            .context("Join or create a project first")
    }
    pub fn storage(&self, p: &LocalProject) -> Storage {
        Storage {
            config: p.project.storage.clone(),
            folder: p.folder.clone(),
            oauth: self.settings.lock().unwrap().oauth.clone(),
        }
    }
    pub fn persist(&self, p: &LocalProject) -> Result<()> {
        let mut connections = self
            .db
            .get::<Vec<LocalProject>>("connections")?
            .unwrap_or_default();
        connections.retain(|old| old.project.id != p.project.id || old.endpoint != p.endpoint);
        connections.push(p.clone());
        self.db.set("connections", &connections)?;
        self.db.set("active", p)?;
        *self.active.lock().unwrap() = Some(p.clone());
        self.complete_metadata_view(p)?;
        Ok(())
    }
    pub(crate) fn credentials(&self, p: &LocalProject) -> Result<String> {
        oauth::secret_get(&format!("membership:{}:{}", p.endpoint, p.project.id))
    }
    /// Normal builds have no practice project.
    #[cfg(not(test_build))]
    pub fn is_practice(&self, _: &LocalProject) -> bool {
        false
    }
    pub async fn reopen(&self, project_id: &str) -> Result<()> {
        if self.active.lock().unwrap().is_some() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), self.finish()).await;
        }
        let _guard = self.operation.lock().await;
        let mut p = self
            .db
            .get::<Vec<LocalProject>>("connections")?
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.project.id == project_id)
            .context("Saved workspace not found")?;
        p.stage = binary::inspect(&p.stage.path)?;
        self.leases.lock().unwrap().clear();
        self.lease_activity.lock().unwrap().clear();
        *self.observed.lock().unwrap() = Some(p.stage.settings_version.clone());
        self.persist(&p)?;
        self.watch()?;
        if self.sync(false).await.is_err() {
            self.online
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        self.changed();
        Ok(())
    }
    pub async fn api<T: serde::de::DeserializeOwned>(
        &self,
        p: &LocalProject,
        method: reqwest::Method,
        suffix: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        crate::private_metadata::validate_history_route(suffix)?;
        let body = match body {
            Some(value) => Some(self.control_command(p, suffix, value).await?),
            None => None,
        };
        let value = self.control_http(p, method, suffix, body).await?;
        self.control_response(p, suffix, value)
            .await
            .and_then(|value| Ok(serde_json::from_value(value)?))
    }
    pub(crate) async fn control_http(
        &self,
        p: &LocalProject,
        method: reqwest::Method,
        suffix: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        crate::private_metadata::validate_history_route(suffix)?;
        #[cfg(debug_assertions)]
        if suffix == "/publish" || suffix == "/names" {
            crate::reliability::checkpoint("control-finalize", suffix)?;
        }
        let result = self
            .coordinator
            .request(
                &endpoint(&p.endpoint)?,
                &p.project.id,
                self.credentials(p)?,
                method,
                suffix,
                body,
            )
            .await;
        if result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<miliastra_core::errors::ServiceError>())
            .is_some_and(|e| e.code == miliastra_core::errors::ErrorCode::Removed)
        {
            self.mark_removed(p);
        }
        result
    }
    pub fn cache_path(&self, hash: &str) -> Result<PathBuf> {
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid object checksum"
        );
        Ok(self.data.join("objects").join(hash))
    }
    pub fn cache(&self, bytes: &[u8]) -> Result<String> {
        let h = hash(bytes);
        let p = self.cache_path(&h)?;
        if !p.exists() {
            let temp = p.with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
            std::fs::write(&temp, bytes)?;
            std::fs::rename(temp, p)?;
        }
        Ok(h)
    }
    pub(crate) async fn stable(&self, stage: &Stage) -> Result<BTreeMap<String, Vec<u8>>> {
        self.editor.stable(stage).await
    }
    pub fn installation(&self, p: &LocalProject) -> Result<Option<crate::installer::Installation>> {
        self.db.get(&format!("installation:{}", p.project.id))
    }
    /// Comparing hashes the whole file and every UI refresh compares every slice, so the
    /// result is cached until the file or the baseline changes.
    pub fn matches(&self, stage: &Stage, revision: &Revision) -> bool {
        self.editor
            .matches(stage, revision, &self.data.join("objects"))
    }
    fn reconcile_installed(&self, p: &mut LocalProject) -> Result<()> {
        let pending = self.installation(p)?;
        for (id, head) in &p.project.heads {
            if !pending.as_ref().is_some_and(|r| r.affects(id)) && self.matches(&p.stage, head) {
                p.installed.insert(id.clone(), head.clone());
            }
        }
        Ok(())
    }
    pub async fn preview_install(&self) -> Result<crate::installer::Plan> {
        let _guard = self.operation.lock().await;
        let mut p = self.active()?;
        ensure!(
            !self.installation(&p)?.is_some_and(|r| r.pending()),
            "Finish applying the pending update, or recover it from History"
        );
        self.reconcile_installed(&mut p)?;
        self.persist(&p)?;
        let stage = binary::inspect(&p.stage.path)?;
        self.details_ready(&p)?;
        let plan = crate::installer::preview(
            &stage,
            &p.project.id,
            &p.project.heads,
            &p.installed,
            &self.data.join("objects"),
        )?;
        self.db
            .set(&format!("install-preview:{}", plan.id), &plan)?;
        Ok(plan)
    }
    pub(crate) fn save_installation(&self, record: &crate::installer::Installation) -> Result<()> {
        let key = format!("task:{}", record.project);
        if let Some(mut task) = self.db.get::<Task>(&key)? {
            task.installation_id = Some(record.id.clone());
            self.db.set(&key, &task)?;
        }
        self.db
            .set(&format!("installation:{}", record.project), record)?;
        self.db
            .set(&format!("install-record:{}", record.id), record)
    }
    pub async fn install_slices(&self, plan_id: &str, editor_ready: bool) -> Result<()> {
        ensure!(
            editor_ready,
            "Save and unload the stage in Miliastra before applying updates"
        );
        uuid::Uuid::parse_str(plan_id)?;
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        ensure!(
            !self.installation(&p)?.is_some_and(|r| r.pending()),
            "Finish applying the pending update, or recover it from History"
        );
        let plan: crate::installer::Plan = self
            .db
            .get(&format!("install-preview:{plan_id}"))?
            .context("The update preview expired. Open it again.")?;
        ensure!(
            plan.project == p.project.id && chrono::Utc::now().timestamp() - plan.at <= 600,
            "The project or update preview changed. Open it again."
        );
        self.stable(&p.stage).await?;
        let stage = binary::inspect(&p.stage.path)?;
        let fresh = crate::installer::preview(
            &stage,
            &p.project.id,
            &p.project.heads,
            &p.installed,
            &self.data.join("objects"),
        )?;
        ensure!(
            plan.originals == fresh.originals
                && serde_json::to_value(&plan.revisions)?
                    == serde_json::to_value(&fresh.revisions)?,
            "Local files or incoming revisions changed since the preview. Open it again."
        );
        let mut record = crate::installer::prepare(
            &plan,
            &stage,
            &self.data.join("objects"),
            &self.data.join("backups"),
        )?;
        self.save_installation(&record)?;
        match crate::transaction::apply(&record.journal, &self.data.join("objects")) {
            Ok(()) => {
                record.phase = crate::installer::InstallationPhase::AwaitingReload;
                record.error = None;
            }
            Err(e) => {
                record.phase = crate::installer::InstallationPhase::Interrupted;
                record.error = Some(e.to_string());
                self.save_installation(&record)?;
                self.changed();
                anyhow::bail!(
                    "Installation did not finish: {e}. Open History to recover this installation."
                );
            }
        }
        self.save_installation(&record)?;
        self.db.remove(&format!("install-preview:{plan_id}"))?;
        self.finish_installation(&mut record).await
    }
    /// Best effort; errors are ignored.
    fn prune_backups(&self) {
        let referenced = ["installation:"]
            .iter()
            .map(|prefix| self.db.raw_values_with_prefix(prefix))
            .collect::<Result<Vec<_>>>()
            .map(|v| v.concat());
        if let Ok(referenced) = referenced {
            let _ = crate::transaction::prune_backups(&self.data.join("backups"), &referenced, 3);
        }
    }
    /// Retry a file check after interruption; normal installs verify automatically.
    pub async fn verify_install(&self) -> Result<()> {
        let _guard = self.operation.lock().await;
        let mut record = self
            .installation(&self.active()?)?
            .context("There is no pending update to finish")?;
        ensure!(record.pending(), "There is no pending update to finish");
        self.finish_installation(&mut record).await
    }
    /// Resume a completed copy after a process interruption. Failed checks wait for
    /// an explicit retry or Undo instead of repeatedly notifying the creator.
    pub async fn resume_install_verification(&self) -> Result<()> {
        let Ok(_guard) = self.operation.try_lock() else {
            return Ok(());
        };
        let p = self.active()?;
        let Some(mut record) = self.installation(&p)? else {
            return Ok(());
        };
        if record.error.is_none()
            && matches!(
                record.phase,
                crate::installer::InstallationPhase::AwaitingReload
                    | crate::installer::InstallationPhase::UndoReload
            )
        {
            self.finish_installation(&mut record).await?;
        }
        Ok(())
    }
    /// The caller holds `operation`, including while verification is running.
    pub(crate) async fn finish_installation(
        &self,
        record: &mut crate::installer::Installation,
    ) -> Result<()> {
        if let Err(error) = self.verify_installation_files(record).await {
            record.error = Some(error.to_string());
            self.save_installation(record)?;
            self.changed();
            anyhow::bail!("The copied files could not be verified: {error}. Retry the file check or open History to recover this installation.");
        }
        self.prune_backups();
        self.changed();
        Ok(())
    }
    async fn verify_installation_files(
        &self,
        record: &mut crate::installer::Installation,
    ) -> Result<()> {
        let mut p = self.active()?;
        ensure!(
            record.project == p.project.id,
            "The selected project changed"
        );
        ensure!(
            matches!(
                record.phase,
                crate::installer::InstallationPhase::AwaitingReload
                    | crate::installer::InstallationPhase::UndoReload
            ),
            "This update needs recovery from History before its files can be checked"
        );
        self.stable(&p.stage).await?;
        let stage = binary::inspect(&p.stage.path)?;
        let undo = record.phase == crate::installer::InstallationPhase::UndoReload;
        let hashes = crate::installer::verify(&record, &stage, &self.data.join("objects"), undo)?;
        if !undo {
            crate::transaction::authorize_saved_files(&record.journal, &hashes)?;
        }
        for r in &record.revisions {
            let expected = if undo {
                record
                    .previous
                    .get(&r.section)
                    .context("Previous revision unavailable")?
            } else {
                r
            };
            if undo {
                if let Some(original) = &record.undo_baselines {
                    if let Some(base) = original.get(&r.section) {
                        p.installed.insert(r.section.clone(), base.clone());
                    } else {
                        p.installed.remove(&r.section);
                    }
                } else {
                    p.installed.insert(r.section.clone(), expected.clone());
                }
            } else {
                p.installed.insert(r.section.clone(), expected.clone());
            }
        }
        p.stage = stage;
        self.persist(&p)?;
        record.phase = if undo {
            crate::installer::InstallationPhase::Undone
        } else {
            crate::installer::InstallationPhase::Verified
        };
        record.error = None;
        self.save_installation(record)?;
        Ok(())
    }
    pub async fn undo_install(&self, editor_ready: bool) -> Result<()> {
        ensure!(
            editor_ready,
            "Save and unload the stage before restoring its slices"
        );
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        let mut record = self
            .installation(&p)?
            .context("No installation backup is available")?;
        ensure!(
            !matches!(
                record.phase,
                crate::installer::InstallationPhase::Undone
                    | crate::installer::InstallationPhase::UndoReload
            ),
            "This update has already been undone"
        );
        ensure!(
            record.project == p.project.id
                && record.stage.canonicalize()? == p.stage.path.canonicalize()?,
            "The selected stage changed"
        );
        ensure!(
            record
                .journal
                .canonicalize()?
                .starts_with(self.data.join("backups").canonicalize()?),
            "The recovery journal is outside this app's backups"
        );
        self.stable(&p.stage).await?;
        binary::inspect(&p.stage.path)?;
        let journal: crate::transaction::Journal =
            serde_json::from_slice(&std::fs::read(&record.journal)?)?;
        ensure!(
            journal.root.canonicalize()? == p.stage.path.canonicalize()?
                && journal
                    .backup
                    .canonicalize()?
                    .starts_with(self.data.join("backups").canonicalize()?),
            "The recovery journal does not match this stage"
        );
        if journal.phase != crate::transaction::JournalPhase::Restored {
            let mut hashes = BTreeMap::new();
            for r in &record.revisions {
                let path = binary::resource_path(&p.stage, &r.section)?;
                let bytes = std::fs::read(&path)?;
                let previous = record
                    .previous
                    .get(&r.section)
                    .context("Previous revision unavailable")?;
                ensure!(
                    crate::installer::matches_bytes(&bytes, r, &self.data.join("objects"))?
                        || crate::installer::matches_bytes(
                            &bytes,
                            previous,
                            &self.data.join("objects")
                        )?,
                    "{} has new local edits. Preserve them before undoing; the backup is intact.",
                    r.name
                );
                hashes.insert(path.strip_prefix(&p.stage.path)?.to_owned(), hash(&bytes));
            }
            record.phase = crate::installer::InstallationPhase::Restoring;
            record.error = None;
            self.save_installation(&record)?;
            let result = crate::transaction::authorize_saved_files(&record.journal, &hashes)
                .and_then(|_| crate::transaction::restore(&record.journal));
            if let Err(e) = result {
                record.phase = crate::installer::InstallationPhase::Interrupted;
                record.error = Some(e.to_string());
                self.save_installation(&record)?;
                self.changed();
                anyhow::bail!(
                    "Restoration did not finish: {e}. The backup and journal are preserved."
                );
            }
        }
        record.phase = crate::installer::InstallationPhase::UndoReload;
        record.error = None;
        self.save_installation(&record)?;
        self.finish_installation(&mut record).await
    }
    /// Installs teammates' updates without asking when the stage isn't loaded, everything is
    /// downloaded and nothing conflicts with local work. Same checks, backup and Undo as a
    /// manual install.
    pub async fn auto_apply(&self) -> Result<()> {
        if !self.settings.lock().unwrap().auto_apply {
            return Ok(());
        }
        let p = self.active()?;
        if self.is_practice(&p) {
            return Ok(());
        }
        if self.installation(&p)?.is_some_and(|r| r.pending()) {
            return Ok(());
        }
        let incoming: Vec<&Revision> = p
            .project
            .heads
            .values()
            .filter(|h| {
                h.section != "shared"
                    && p.installed.get(&h.section).is_none_or(|r| r.hash != h.hash)
            })
            .collect();
        if incoming.is_empty() || !stage_released(&p.stage.path) {
            return Ok(());
        }
        // Don't retry the same set of updates more than once every 5 minutes.
        let fingerprint = hash(
            serde_json::to_string(&(&p.project.heads, &p.installed, &p.stage.settings_version))?
                .as_bytes(),
        );
        let now = chrono::Utc::now().timestamp();
        {
            let mut last = self.auto_apply_attempt.lock().unwrap();
            if last
                .as_ref()
                .is_some_and(|(f, at)| f == &fingerprint && now - at < 300)
            {
                return Ok(());
            }
            *last = Some((fingerprint, now));
        }
        // The preview fails on local changes, conflicts or missing downloads.
        let Ok(plan) = self.preview_install().await else {
            return Ok(());
        };
        if !stage_released(&p.stage.path) {
            return Ok(());
        }
        self.install_slices(&plan.id, true).await?;
        let names: Vec<String> = plan.revisions.iter().map(|r| r.name.clone()).collect();
        let authors: Vec<String> = self
            .db
            .history(&p.project.id)?
            .into_iter()
            .filter(|u| {
                u.revisions
                    .iter()
                    .any(|r| plan.revisions.iter().any(|x| x.hash == r.hash))
            })
            .map(|u| u.author)
            .collect();
        self.alert(Notice::Applied {
            creator: authors
                .first()
                .filter(|a| authors.iter().all(|x| x == *a))
                .map(String::as_str),
            slices: &names.join(", "),
        });
        Ok(())
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let active = self.active.lock().unwrap().clone();
        let mut sections = vec![];
        let draft = if let Some(p) = &active {
            let pending = self.installation(p)?;
            let incoming_history = if p
                .project
                .heads
                .iter()
                .any(|(id, h)| p.installed.get(id).is_none_or(|r| r.hash != h.hash))
            {
                self.db.history(&p.project.id)?
            } else {
                vec![]
            };
            for (id, name) in p
                .stage
                .sections
                .iter()
                .map(|s| (s.id.clone(), s.name.clone()))
                .chain(std::iter::once((
                    "shared".into(),
                    "Shared level settings".into(),
                )))
            {
                let base = p.installed.get(&id);
                let head = p.project.heads.get(&id);
                let installation_pending = pending.as_ref().is_some_and(|r| r.affects(&id));
                let status = self.editor.section_status(
                    &p.stage,
                    base,
                    head,
                    &self.data.join("objects"),
                    installation_pending,
                );
                let changed = status.changed;
                let remote_changed = status.remote_changed;
                sections.push(SectionView {
                    download_error: head
                        .filter(|r| self.cache_path(&r.hash).is_ok_and(|path| !path.exists()))
                        .and_then(|r| {
                            self.db
                                .get::<String>(&format!("download-error:{}", r.hash))
                                .ok()
                                .flatten()
                        }),
                    id: id.clone(),
                    name: crate::names::effective_names(&p.project)
                        .get(&id)
                        .cloned()
                        .unwrap_or(name),
                    changed,
                    conflict: changed && remote_changed,
                    comparison_pending: status.comparison_pending,
                    lock: p
                        .project
                        .locks
                        .iter()
                        .find(|l| l.section == id && l.expires > chrono::Utc::now().timestamp())
                        .cloned(),
                    downloaded: head
                        .is_some_and(|h| self.cache_path(&h.hash).is_ok_and(|p| p.exists())),
                    installed: !remote_changed,
                    installation_pending,
                    incoming: head.filter(|_| remote_changed).and_then(|h| {
                        incoming_history
                            .iter()
                            .find(|u| {
                                u.revisions
                                    .iter()
                                    .any(|r| r.section == id && r.hash == h.hash)
                            })
                            .map(|u| Incoming {
                                author: u.author.clone(),
                                summary: u.summary.clone(),
                                at: u.at,
                            })
                    }),
                });
            }
            self.db
                .get(&format!("draft:{}", p.project.id))?
                .unwrap_or_default()
        } else {
            Draft::default()
        };
        let storage_issue = if let Some(p) = &active {
            let metadata = self.metadata_storage_issue(&p.project.id)?;
            let mut issue = metadata;
            if issue.is_none() {
                for head in p.project.heads.values() {
                    if p.installed
                        .get(&head.section)
                        .is_none_or(|r| r.hash != head.hash)
                        && self.cache_path(&head.hash).is_ok_and(|path| !path.exists())
                    {
                        issue = self.db.get(&format!("download-issue:{}", head.hash))?;
                        if issue.is_some() {
                            break;
                        }
                    }
                }
            }
            issue
        } else {
            None
        };
        Ok(Snapshot {
            storage_issue,
            metadata_pending: active
                .as_ref()
                .is_some_and(|p| self.metadata_view_pending(p).unwrap_or(true)),
            test_profile: crate::profile::current().map(str::to_owned),
            task: active
                .as_ref()
                .map(|p| self.db.get(&format!("task:{}", p.project.id)))
                .transpose()?
                .flatten(),
            cloud_providers: crate::storage::cloud_providers(),
            storage_accounts: crate::providers::all()
                .iter()
                .filter(|provider| provider.cloud.is_some())
                .map(|provider| (provider.id.into(), oauth::connection_status(provider.id)))
                .collect(),
            name_sync: active
                .as_ref()
                .map(|p| -> Result<_> {
                    Ok(crate::names::Status {
                        pending: crate::names::differences(&p.stage, &p.project),
                        draft: self
                            .db
                            .get(&format!("names-draft:{}", p.project.id))?
                            .unwrap_or_default(),
                        error: self.db.get(&format!("name-sync-error:{}", p.project.id))?,
                    })
                })
                .transpose()?,
            test_overlay_hwnd: {
                #[cfg(windows)]
                let value = if cfg!(debug_assertions)
                    && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()
                {
                    self.app
                        .get_webview_window("overlay")
                        .and_then(|w| w.hwnd().ok())
                        .map(|h| h.0 as isize)
                } else {
                    None
                };
                #[cfg(not(windows))]
                let value = None;
                value
            },
            test_data_directory: if cfg!(debug_assertions)
                && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()
            {
                Some(self.data.clone())
            } else {
                None
            },
            settings: {
                let mut settings = self.settings.lock().unwrap().clone();
                settings.oauth.google_client_secret.clear();
                settings
            },
            stage: active.as_ref().map(|p| p.stage.clone()),
            project: active.as_ref().map(|p| p.project.clone()),
            member: active.as_ref().map(|p| p.member.clone()),
            sections,
            draft,
            online: self.online.load(Ordering::Relaxed),
            protocol_mismatch: active.as_ref().is_some_and(|p| {
                self.coordinator
                    .protocol_mismatch(&p.endpoint, &p.project.id)
            }),
            message: self.message.lock().unwrap().clone(),
            practice: active.as_ref().is_some_and(|p| self.is_practice(p)),
            removed: active.as_ref().is_some_and(|p| {
                self.db
                    .get::<bool>(&removed_key(&p.project.id))
                    .ok()
                    .flatten()
                    .unwrap_or(false)
            }),
            compatibility: {
                let mut c = crate::installer::compatibility(active.as_ref().map(|p| &p.stage));
                if active.as_ref().is_some_and(|p| self.is_practice(p)) {
                    c.direct_install = true;
                    c.reason = "Local practice files use the normal update workflow.".into();
                }
                c
            },
            game: {
                // Fullscreen can only be detected while the game is in front, so keep showing the
                // hint once a notice has been held back.
                let mut game = crate::native::game_state();
                game.exclusive_fullscreen |= self.fullscreen_warned.load(Ordering::Relaxed);
                game
            },
            installation: active
                .as_ref()
                .map(|p| self.installation(p))
                .transpose()?
                .flatten(),
        })
    }
    pub fn changed(&self) {
        let _ = self.app.emit("state-changed", ());
    }
    pub fn alert(&self, notice: Notice<'_>) {
        let text = notice.message();
        let now = chrono::Utc::now().timestamp();
        {
            let mut previous = self.last_notice.lock().unwrap();
            if previous
                .as_ref()
                .is_some_and(|(message, at)| message == &text && now - *at < 8)
            {
                return;
            }
            *previous = Some((text.clone(), now));
        }
        *self.message.lock().unwrap() = text.clone();
        let _ = self.app.emit("notice", &text);
        crate::show_notice(&self.app, &text);
        self.changed();
    }
    pub fn watch(&self) -> Result<()> {
        use notify::Watcher;
        let p = self.active()?;
        let app = self.app.clone();
        let mut w = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                app.state::<Core>().dirty.store(true, Ordering::Relaxed);
            }
        })?;
        w.watch(&p.stage.path, notify::RecursiveMode::Recursive)?;
        *self.watcher.lock().unwrap() = Some(w);
        Ok(())
    }
    pub async fn attach(
        &self,
        joined: Joined,
        endpoint_url: String,
        stage: Stage,
        folder: Option<PathBuf>,
    ) -> Result<()> {
        if joined.project.storage.kind == "folder" {
            validate_exchange(
                folder
                    .as_ref()
                    .context("Choose the shared exchange folder")?,
                &stage.path,
                &self.settings.lock().unwrap().save_root,
            )?;
        }
        let storage = Storage {
            config: joined.project.storage.clone(),
            folder: folder.clone(),
            oauth: self.settings.lock().unwrap().oauth.clone(),
        };
        storage.verify().await.context("Could not read and write project storage. Check your connection, sign-in, and folder edit permission.")?;
        let marker: ProjectView = serde_json::from_slice(&storage.get("project.json").await?)?;
        ensure!(
            marker.id == joined.project.id,
            "This storage folder belongs to another project"
        );
        let installed = joined
            .project
            .heads
            .iter()
            .filter(|(_, r)| self.matches(&stage, r))
            .map(|(id, r)| (id.clone(), r.clone()))
            .collect();
        oauth::secret_set(
            &format!("membership:{}:{}", endpoint_url, joined.project.id),
            &joined.token,
        )?;
        let p = LocalProject {
            endpoint: endpoint_url,
            project: joined.project,
            member: joined.member,
            stage,
            folder,
            installed,
        };
        if let Some(i) = joined.invitation {
            self.db.set("invitation", &i)?;
        }
        self.persist(&p)?;
        self.db.remove("incoming-invitation")?;
        *self.observed.lock().unwrap() = Some(p.stage.settings_version.clone());
        self.watch()?;
        self.sync(false).await?;
        Ok(())
    }
    pub async fn create(
        &self,
        name: String,
        display: String,
        stage_path: PathBuf,
        kind: String,
        folder: Option<PathBuf>,
    ) -> Result<()> {
        self.create_at(name, display, stage_path, kind, folder, None)
            .await
    }
    pub(crate) async fn create_at(
        &self,
        name: String,
        display: String,
        stage_path: PathBuf,
        kind: String,
        folder: Option<PathBuf>,
        endpoint_override: Option<String>,
    ) -> Result<()> {
        let _guard = self.operation.lock().await;
        ensure!(
            self.active.lock().unwrap().is_none(),
            "Finish or leave the current project first"
        );
        let ep = endpoint(
            endpoint_override
                .as_deref()
                .unwrap_or(&self.settings.lock().unwrap().endpoint),
        )?;
        let progress = |message: String| {
            let _ = self.app.emit("project-setup-progress", message);
        };
        progress("Checking collaboration service…".into());
        check_creation_service(&ep).await?;
        progress("Reading saved slices…".into());
        let stage = binary::inspect(&stage_path)?;
        crate::storage::ensure_provider_available(&kind)?;
        let settings = self.settings.lock().unwrap().clone();
        let files = self.stable(&stage).await?;
        progress(crate::providers::get(&kind)?.info().setup_message.clone());
        let pending_storage_key = format!(
            "create-storage:{}",
            hash(&serde_json::to_vec(
                &serde_json::json!({"name":name,"display":display,"stage":stage.path,"kind":kind,"endpoint":ep,"folder":folder})
            )?)
        );
        let config = if let Some(config) = self.db.get::<StorageConfig>(&pending_storage_key)? {
            config
        } else {
            let config = if kind == "folder" {
                validate_exchange(
                    folder
                        .as_ref()
                        .context("Choose the shared exchange folder")?,
                    &stage.path,
                    &settings.save_root,
                )?;
                let root = folder
                    .as_ref()
                    .context("Choose the shared exchange folder")?
                    .canonicalize()?;
                ensure!(
                    !root.starts_with(&stage.path) && !stage.path.starts_with(&root),
                    "Choose an exchange folder separate from game saves"
                );
                StorageConfig {
                    kind,
                    location: String::new(),
                    drive_id: None,
                }
            } else {
                crate::storage::create_cloud(&kind, &name, &settings.oauth).await?
            };
            self.db.set(&pending_storage_key, &config)?;
            config
        };
        let storage = Storage {
            config: config.clone(),
            folder: folder.clone(),
            oauth: settings.oauth,
        };
        let resuming = self
            .db
            .get::<Joined>(&format!("{pending_storage_key}:joined"))?
            .is_some();
        if let Some(root) = &folder {
            ensure!(
                resuming || !root.join("project.json").exists(),
                "This folder already contains a project; join it instead"
            );
        }
        storage.verify().await.context("Could not read and write project storage. Check your connection, sign-in, and folder edit permission.")?;
        storage.ensure_directory("Latest").await?;
        let baseline_key = format!("{pending_storage_key}:baseline");
        let baseline_id = self
            .db
            .get::<String>(&baseline_key)?
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.db.set(&baseline_key, &baseline_id)?;
        let shared = hash(&files["shared"]);
        let mut baseline = vec![];
        for (index, (id, b)) in files.iter().enumerate() {
            progress(format!(
                "Uploading saved slices ({} of {})…",
                index + 1,
                files.len()
            ));
            let h = self.cache(b)?;
            storage
                .put(&history_artifact(&baseline_id, id)?, b.clone())
                .await?;
            baseline.push(Revision {
                section: id.clone(),
                name: stage
                    .sections
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "Shared level settings".into()),
                hash: h.clone(),
                content_hash: binary::content_hash(b, id, &binary::resource_level(b, id)?)?,
                parent: None,
                artifact: history_artifact(&baseline_id, id)?,
                size: b.len() as u64,
                shared_dependency: if id == "shared" {
                    None
                } else {
                    Some(shared.clone())
                },
            });
        }
        let private_id_key = format!("{pending_storage_key}:project");
        let private_id = self
            .db
            .get::<String>(&private_id_key)?
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.db.set(&private_id_key, &private_id)?;
        let private_member_key = format!("{pending_storage_key}:member");
        let private_member = self
            .db
            .get::<String>(&private_member_key)?
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.db.set(&private_member_key, &private_member)?;
        self.initialize_created_privacy_key(&ep, &private_id)?;
        let req = CreateProject {
            id: private_id.clone(),
            member_id: private_member.clone(),
            summary: "Initialized project".into(),
            details: "Initial saved project version.".into(),
            name,
            display_name: display,
            storage: config,
            baseline,
            baseline_id,
        };
        storage
            .put(
                &format!("History/{}/pending.json", req.baseline_id),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"summary":"Initialized project","revisions":req.baseline}),
                )?,
            )
            .await?;
        progress("Sharing the starting stage…".into());
        let base_heads: BTreeMap<String, Revision> = req
            .baseline
            .iter()
            .map(|r| (r.section.clone(), r.clone()))
            .collect();
        // Joiners import this stage first. If the upload fails, creating the project still succeeds.
        let _ = crate::tasks::publish_base(&storage, &stage, &base_heads).await;
        progress("Creating project…".into());
        // If a later setup step fails, a retry reuses this coordinator project instead of
        // creating a second one for the same folder.
        let joined_key = format!("{pending_storage_key}:joined");
        let created = self.db.get::<Joined>(&joined_key)?.and_then(|mut joined| {
            joined.token =
                oauth::secret_get(&format!("membership:{}:{}", ep, joined.project.id)).ok()?;
            Some(joined)
        });
        let wire_create = self
            .prepare_control_create(&ep, &req, &storage, stage.clone(), folder.clone())
            .await?;
        let joined = if let Some(joined) = created {
            joined
        } else {
            let wire = if let Some((pending, _)) = self.pending_membership(&ep, &private_id)? {
                pending
            } else {
                let response = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()?
                    .post(format!("{ep}/projects"))
                    .json(&wire_create)
                    .send()
                    .await?;
                if !response.status().is_success() {
                    let v: serde_json::Value = response.json().await.unwrap_or_default();
                    anyhow::bail!(
                        "{}",
                        v["error"]
                            .as_str()
                            .unwrap_or("The project could not be created")
                    );
                }
                response.json::<miliastra_core::control::Joined>().await?
            };
            ensure!(
                wire.project.id == private_id && wire.member == private_member,
                "The service changed the project identity"
            );
            oauth::secret_set(&format!("membership:{}:{}", ep, private_id), &wire.token)?;
            let joined = self
                .hydrate_joined(
                    &ep,
                    wire,
                    req.storage.clone(),
                    stage.clone(),
                    folder.clone(),
                )
                .await?;
            let mut saved = joined.clone();
            saved.token.clear();
            self.db.set(&joined_key, &saved)?;
            joined
        };

        storage
            .put("project.json", serde_json::to_vec_pretty(&joined.project)?)
            .await?;
        progress("Finishing setup…".into());
        self.attach(joined, ep.clone(), stage, folder).await?;
        self.finish_membership(&ep, &private_id)?;
        self.db.remove(&pending_storage_key)?;
        self.db.remove(&joined_key)?;
        self.db.remove(&baseline_key)?;
        Ok(())
    }
    pub async fn join(
        &self,
        invite: Invitation,
        display: String,
        stage_path: PathBuf,
        folder: Option<PathBuf>,
    ) -> Result<()> {
        let _guard = self.operation.lock().await;
        ensure!(
            self.active.lock().unwrap().is_none(),
            "Finish or leave the current project first"
        );
        ensure!(
            invite.version == 1,
            "This invitation needs a newer app version"
        );
        uuid::Uuid::parse_str(&invite.project)?;
        let ep = endpoint(&invite.endpoint)?;
        let stage = binary::inspect(&stage_path)?;
        if let Some(root) = &folder {
            validate_exchange(root, &stage.path, &self.settings.lock().unwrap().save_root)?;
            let root = root.canonicalize()?;
            ensure!(
                !root.starts_with(&stage.path) && !stage.path.starts_with(&root),
                "Choose an exchange folder separate from game saves"
            );
        }
        let folder = folder.or_else(|| {
            self.db
                .get::<Vec<LocalProject>>("connections")
                .ok()
                .flatten()
                .unwrap_or_default()
                .into_iter()
                .find(|p| p.project.id == invite.project && p.endpoint == ep)
                .and_then(|p| p.folder)
        });
        if let Some(saved) = self
            .db
            .get::<Vec<LocalProject>>("connections")?
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.project.id == invite.project && p.endpoint == ep)
        {
            if let Ok(token) = self.credentials(&saved) {
                if let Ok(view) = self
                    .api::<ProjectView>(&saved, reqwest::Method::GET, "", None)
                    .await
                {
                    return self
                        .attach(
                            Joined {
                                project: view,
                                member: saved.member,
                                token,
                                invitation: None,
                            },
                            ep,
                            stage,
                            folder,
                        )
                        .await;
                }
            }
        }
        if let Some((pending, config)) = self.pending_membership(&ep, &invite.project)? {
            let joined = self
                .hydrate_joined(&ep, pending, config, stage.clone(), folder.clone())
                .await?;
            self.attach(joined, ep.clone(), stage, folder).await?;
            self.finish_membership(&ep, &invite.project)?;
            return Ok(());
        }
        self.check_invitation_storage(invite.clone(), folder.clone(), Some(stage.path.clone()))
            .await?;
        ensure!(
            !display.trim().is_empty()
                && miliastra_core::constants::text_fits(
                    &display,
                    miliastra_core::constants::MAX_NAME_UNITS
                ),
            "Enter your display name"
        );
        let member = uuid::Uuid::new_v4().to_string();
        let info = self.invite_details(invite.clone()).await?;
        let storage = Storage {
            config: info.storage.clone(),
            folder: folder.clone(),
            oauth: self.settings.lock().unwrap().oauth.clone(),
        };
        let profile = self
            .store_profile(&storage, &invite.project, &member, &display)
            .await?;
        let response = reqwest::Client::new()
            .post(format!("{ep}/projects/{}/join", invite.project))
            .json(&serde_json::json!({"invite":invite.invite,"profileHash":profile,"memberId":member}))
            .send()
            .await?;
        if !response.status().is_success() {
            let v: serde_json::Value = response.json().await?;
            anyhow::bail!("{}", v["error"].as_str().unwrap_or("Could not join"));
        }
        let wire: miliastra_core::control::Joined = response.json().await?;
        ensure!(
            wire.member == member && wire.project.id == invite.project,
            "The service changed the project identity"
        );
        oauth::secret_set(
            &format!("membership:{}:{}", ep, invite.project),
            &wire.token,
        )?;
        let joined = self
            .hydrate_joined(&ep, wire, info.storage, stage.clone(), folder.clone())
            .await?;
        self.attach(joined, ep.clone(), stage, folder).await?;
        self.finish_membership(&ep, &invite.project)?;
        Ok(())
    }
    pub async fn share_names(
        &self,
        names: BTreeMap<String, String>,
        parent_version: u64,
    ) -> Result<Update> {
        let draft = Draft {
            summary: "Updated slice names".into(),
            details: String::new(),
        };
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        let current_names = crate::names::effective_names(&p.project);
        crate::names::save_draft(
            &self.db,
            &p.project.id,
            crate::names::Draft {
                edits: names
                    .iter()
                    .map(|(id, name)| {
                        Ok((
                            id.clone(),
                            crate::names::Edit {
                                name: name.clone(),
                                base_name: current_names
                                    .get(id)
                                    .context("Unknown section")?
                                    .clone(),
                                parent_version,
                            },
                        ))
                    })
                    .collect::<Result<_>>()?,
            },
        )?;
        ensure!(
            parent_version == p.project.names_version,
            "Project names changed. Use current project names, review your edits, and share again."
        );
        let current = crate::names::effective_names(&p.project);
        let mut changes = vec![];
        for (id, name) in &names {
            validate_name(name)?;
            let before = current.get(id).context("Unknown section")?;
            if before != name {
                changes.push(NameChange {
                    section: id.clone(),
                    before: before.clone(),
                    after: name.clone(),
                });
            }
        }
        let key = format!("pending-names:{}", p.project.id);
        let old = self.db.get::<PublishNames>(&key)?;
        let req = if let Some(old) = old.filter(|r| {
            r.summary == draft.summary
                && r.details == draft.details
                && r.changes
                    .iter()
                    .all(|c| names.get(&c.section) == Some(&c.after))
                && r.changes.len() == changes.len()
                && r.parent_version == parent_version
        }) {
            old
        } else {
            PublishNames {
                id: uuid::Uuid::new_v4().to_string(),
                parent_version,
                summary: draft.summary,
                details: draft.details,
                changes,
            }
        };
        ensure!(
            !req.summary.trim().is_empty(),
            "Write what changed before sharing slice names"
        );
        ensure!(!req.changes.is_empty(), "No slice names changed");
        self.db.set(&key, &req)?;
        self.storage(&p)
            .put(
                &format!("History/{}/pending.json", req.id),
                serde_json::to_vec_pretty(&req)?,
            )
            .await?;
        let u: Update = self
            .api(
                &p,
                reqwest::Method::POST,
                "/names",
                Some(serde_json::to_value(&req)?),
            )
            .await?;
        self.db.cache(&p.project.id, &u)?;
        // Sync archives the update, and retries if that fails.
        self.sync(false).await?;
        self.db.remove(&key)?;
        crate::names::clear_published(&self.db, &p.project.id, &u.name_changes)?;
        self.changed();
        Ok(u)
    }
    fn synchronize_names(&self, p: &mut LocalProject) -> Result<()> {
        let before = binary::inspect(&p.stage.path)?;
        let external_save = before.settings_version != p.stage.settings_version;
        let backups = self
            .data
            .join("name-backups")
            .join(hash(p.project.id.as_bytes()));
        match crate::names::synchronize(&before, &p.project, &backups) {
            Ok(written) => {
                self.db
                    .remove(&format!("name-sync-error:{}", p.project.id))?;
                p.stage = if written {
                    binary::inspect(&before.path)?
                } else {
                    before
                };
                if written && !external_save {
                    *self.observed.lock().unwrap() = Some(p.stage.settings_version.clone());
                } else if external_save {
                    self.dirty.store(true, Ordering::Relaxed);
                }
            }
            Err(e) => {
                self.db
                    .set(&format!("name-sync-error:{}", p.project.id), &e.to_string())?;
                self.dirty.store(true, Ordering::Relaxed);
            }
        }
        Ok(())
    }
    pub async fn acquire(&self, id: &str, takeover: bool) -> Result<Lease> {
        let p = self.active()?;
        let lease: Lease = self
            .api(
                &p,
                reqwest::Method::POST,
                "/locks/acquire",
                Some(serde_json::json!({"section":id,"takeover":takeover})),
            )
            .await?;
        self.leases.lock().unwrap().insert(id.into(), lease.clone());
        self.touch_lease(id);
        Ok(lease)
    }
    /// Warns when a save changes a slice someone else has reserved, once per reservation,
    /// instead of only at Share time.
    fn warn_reserved_changes(
        &self,
        p: &LocalProject,
        previous: &BTreeMap<String, String>,
        drafts: &BTreeMap<String, String>,
    ) -> std::collections::BTreeSet<String> {
        let now = chrono::Utc::now().timestamp();
        let names = crate::names::effective_names(&p.project);
        let mut warned = std::collections::BTreeSet::new();
        for (id, h) in drafts {
            if previous.get(id) == Some(h) {
                continue;
            }
            let Some(l) = p
                .project
                .locks
                .iter()
                .find(|l| &l.section == id && l.member != p.member && l.expires > now)
            else {
                continue;
            };
            warned.insert(id.clone());
            if self
                .conflict_warnings
                .lock()
                .unwrap()
                .insert((id.clone(), l.fence))
            {
                self.alert(Notice::ReservedBy {
                    creator: &l.name,
                    slice: names
                        .get(id)
                        .map(String::as_str)
                        .unwrap_or("Shared level settings"),
                });
            }
        }
        warned
    }
    /// Handles an event from the live connection. Notices for one member are shown
    /// directly; everything else triggers a sync.
    pub async fn handle_event(&self, p: &LocalProject, event: serde_json::Value) {
        let event = match self
            .privacy_keys(&p.endpoint, &p.project.id)
            .and_then(|keys| keys.open_events(&p.endpoint, &p.project.id, event))
        {
            Ok(value) => value,
            Err(_) => {
                self.changed();
                return;
            }
        };

        let mut event = event;
        for field in ["from", "holder"] {
            if let Some(id) = event[field].as_str() {
                event[field] = p
                    .project
                    .members
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "A teammate".into())
                    .into();
            }
        }
        if event["kind"] == "finish-request" {
            if event["to"].as_str() == Some(p.member.as_str()) {
                let section = event["section"].as_str().unwrap_or_default();
                let name = crate::names::effective_names(&p.project)
                    .get(section)
                    .cloned()
                    .unwrap_or_else(|| "Shared level settings".into());
                self.alert(Notice::FinishRequested {
                    creator: event["from"].as_str().unwrap_or("A teammate"),
                    slice: &name,
                });
            }
            return;
        }
        if event["kind"] == "takeover" {
            if event["to"].as_str() == Some(p.member.as_str()) {
                let section = event["section"].as_str().unwrap_or_default();
                self.leases.lock().unwrap().remove(section);
                let name = crate::names::effective_names(&p.project)
                    .get(section)
                    .cloned()
                    .unwrap_or_else(|| "Shared level settings".into());
                self.alert(Notice::TakenOver {
                    creator: event["from"].as_str().unwrap_or("A teammate"),
                    slice: &name,
                });
            }
            let _guard = self.operation.lock().await;
            let _ = self.sync(false).await;
            return;
        }
        if event["kind"] == "removed" {
            if event["member"].as_str() == Some(p.member.as_str()) {
                self.mark_removed(p);
            } else {
                let _guard = self.operation.lock().await;
                let _ = self.sync(false).await;
            }
            return;
        }
        if event["kind"] == "access-request" {
            let _ = self.grant_pending_access().await;
            return;
        }
        let _guard = self.operation.lock().await;
        let full = event["kind"] == "history" || event["kind"] == "reconcile";
        if self.sync(full).await.is_err() {
            self.online.store(false, Ordering::Relaxed);
            self.changed();
        }
    }
    /// Team management: remove a member or hand over ownership.
    pub async fn manage_team(&self, route: &str, body: serde_json::Value) -> Result<()> {
        ensure!(
            ["/members/remove", "/members/owner"].contains(&route),
            "Unknown team action"
        );
        let _guard = self.operation.lock().await;
        let p = self.active()?;
        let _: serde_json::Value = self
            .api(&p, reqwest::Method::POST, route, Some(body))
            .await?;
        self.sync(false).await?;
        Ok(())
    }
    /// Asks whoever has a slice reserved to share it.
    pub async fn request_finish(&self, section: &str) -> Result<()> {
        ensure!(valid_section(section), "Unknown slice");
        let p = self.active()?;
        let _: serde_json::Value = self
            .api(
                &p,
                reqwest::Method::POST,
                "/locks/request",
                Some(serde_json::json!({ "section": section })),
            )
            .await?;
        self.changed();
        Ok(())
    }
    /// Records reserving or saving a slice, for the idle release.
    fn touch_lease(&self, id: &str) {
        self.lease_activity
            .lock()
            .unwrap()
            .insert(id.into(), chrono::Utc::now().timestamp());
        self.idle_reminded.lock().unwrap().remove(id);
    }
    pub async fn start(&self, id: &str, takeover: bool) -> Result<()> {
        let _guard = self.operation.lock().await;
        let previous = self.held_leases()?.get(id).cloned();
        let lease = self.acquire(id, takeover).await?;
        let mut p = self.active()?;
        p.project.locks.retain(|old| old.section != lease.section);
        p.project.locks.push(lease.clone());
        self.online.store(true, Ordering::Relaxed);
        self.persist(&p)?;
        // Every save re-checks the reservation. Only announce it when it's new; a changed
        // fence means it was taken again.
        if previous
            .as_ref()
            .is_none_or(|held| held.member != lease.member || held.fence != lease.fence)
        {
            self.alert(Notice::Reserved(
                crate::names::effective_names(&p.project)
                    .get(&lease.section)
                    .map(String::as_str)
                    .unwrap_or("shared settings"),
            ));
        }
        Ok(())
    }
    pub async fn finish(&self) -> Result<()> {
        self.finish_slices(None).await
    }
    pub async fn finish_slices(&self, section: Option<&str>) -> Result<()> {
        let _guard = self.operation.lock().await;
        if let Some(id) = section {
            ensure!(valid_section(id), "Unknown slice");
        }
        let held = self.held_leases()?;
        if let Some(id) = section {
            ensure!(held.contains_key(id), "This slice is not reserved by you");
        }
        let p = self.active()?;
        if let Some(id) = section {
            ensure!(
                !self.has_local_changes(&p, id),
                "Share your saved changes before finishing this slice. It stays reserved."
            );
        }
        let ids: Vec<String> = held
            .into_keys()
            .filter(|id| section.is_none_or(|s| id == s) && !self.has_local_changes(&p, id))
            .collect();
        // Changed slices stay reserved until they expire: sharing is how work is handed over.
        self.release_held(&ids).await?;
        Ok(())
    }
    fn held_leases(&self) -> Result<BTreeMap<String, Lease>> {
        let p = self.active()?;
        let mut leases: BTreeMap<String, Lease> = p
            .project
            .locks
            .iter()
            .filter(|l| l.member == p.member)
            .map(|l| (l.section.clone(), l.clone()))
            .collect();
        // Our own lease records win, even if the cached project view is out of date.
        leases.extend(self.leases.lock().unwrap().clone());
        Ok(leases)
    }
    /// Callers hold the operation lock, except best-effort releases.
    pub(crate) async fn release_held(&self, ids: &[String]) -> Result<()> {
        let p = self.active()?;
        let held = self.held_leases()?;
        for id in ids {
            let Some(l) = held.get(id) else { continue };
            self.api::<serde_json::Value>(
                &p,
                reqwest::Method::POST,
                "/locks/release",
                Some(serde_json::json!({"section":id,"fence":l.fence})),
            )
            .await?;
            self.leases.lock().unwrap().remove(id);
            self.lease_activity.lock().unwrap().remove(id);
            let mut current = self.active()?;
            current.project.locks.retain(|old| &old.section != id);
            self.persist(&current)?;
            self.changed();
        }
        Ok(())
    }
    /// A slice still differs from what this member last installed or shared.
    pub(crate) fn has_local_changes(&self, p: &LocalProject, section: &str) -> bool {
        p.installed
            .get(section)
            .is_none_or(|r| !self.matches(&p.stage, r))
    }
    pub async fn sync(&self, full_history: bool) -> Result<()> {
        let mut p = self.active()?;
        let view: ProjectView = self.api(&p, reqwest::Method::GET, "", None).await?;
        ensure!(view.id==p.project.id&&view.storage==p.project.storage,"The project storage binding changed. Stop and verify with your team before reconnecting.");
        p.project = view;
        self.synchronize_names(&mut p)?;
        self.adopt_leases(&p);
        self.reconcile_installed(&mut p)?;
        self.online.store(true, Ordering::Relaxed);
        let _ = self.db.remove(&removed_key(&p.project.id));
        self.persist(&p)?;
        for r in p.project.heads.values() {
            if !self.cache_path(&r.hash)?.exists() {
                self.db
                    .set(&format!("download-state:{}", r.hash), &"queued")?;
            }
        }
        let marker = format!("history-cache-complete:{}", p.project.id);
        let known = self.db.get::<u64>(&marker)?.unwrap_or(0);
        let mut latest = known;
        let mut before: Option<u64> = None;
        loop {
            let suffix = format!(
                "/history{}",
                before
                    .map(|n: u64| format!("?before={n}"))
                    .unwrap_or_default()
            );
            let page: HistoryPage = self.api(&p, reqwest::Method::GET, &suffix, None).await?;
            for u in &page.entries {
                latest = latest.max(u.sequence);
                self.db.cache(&p.project.id, u)?;
                let key = format!("pending:{}", p.project.id);
                if self
                    .db
                    .get::<PublishRequest>(&key)?
                    .is_some_and(|r| r.id == u.id)
                {
                    for r in &u.revisions {
                        if self.matches(&p.stage, r) {
                            p.installed.insert(r.section.clone(), r.clone());
                        }
                    }
                    self.persist(&p)?;
                    self.db.remove(&key)?;
                    self.db
                        .set(&format!("draft:{}", p.project.id), &Draft::default())?;
                    let _ = self.app.emit("draft-shared", ());
                }
                let names_key = format!("pending-names:{}", p.project.id);
                if self
                    .db
                    .get::<PublishNames>(&names_key)?
                    .is_some_and(|r| r.id == u.id)
                {
                    self.db.remove(&names_key)?;
                    crate::names::clear_published(&self.db, &p.project.id, &u.name_changes)?;
                }
            }
            match next_history_cursor(known, full_history, before, &page)? {
                Some(next) => before = Some(next),
                None => break,
            }
        }
        self.db.set(&marker, &latest)?;
        let _ = self.app.emit("history-changed", ());
        self.changed();
        Ok(())
    }
    pub async fn share(
        &self,
        selected: Vec<String>,
        draft: Draft,
        keep_reserved: bool,
    ) -> Result<Update> {
        let _publication = self.transfers.publication.lock().await;
        let guard = self.operation.lock().await;
        miliastra_core::constants::validate_changelog(&draft.summary, &draft.details)?;
        ensure!(
            !draft.summary.trim().is_empty(),
            "Tell your teammates what changed before sharing"
        );
        let mut p = self.active()?;
        let progress_project = p.project.id.clone();
        let progress = |message: String| {
            let _ = self.app.emit(
                "publication-progress",
                serde_json::json!({"project":progress_project,"message":message}),
            );
        };
        progress("Checking saved changes…".into());
        self.db.set(&format!("draft:{}", p.project.id), &draft)?;
        ensure!(!self.installation(&p)?.is_some_and(|r|r.pending()),"Finish applying the pending update, or recover it from History, before sharing. Your changelog is preserved.");
        ensure!(
            self.online.load(Ordering::Relaxed),
            "Reconnect before sharing; your draft is preserved"
        );
        let current: ProjectView = self.api(&p, reqwest::Method::GET, "", None).await?;
        ensure!(
            current.storage == p.project.storage,
            "Project storage changed"
        );
        p.project = current;
        self.persist(&p)?;
        self.details_ready(&p)?;
        ensure!(
            selected
                .iter()
                .map(String::as_str)
                .chain(std::iter::once("shared"))
                .all(|id| {
                    !self
                        .editor
                        .section_status(
                            &p.stage,
                            p.installed.get(id),
                            p.project.heads.get(id),
                            &self.data.join("objects"),
                            false,
                        )
                        .comparison_pending
                }),
            "Wait for saved slices to finish checking before sharing. Your changelog is preserved."
        );
        let files = self.stable(&p.stage).await?;
        let storage = self.storage(&p);
        let mut revisions = vec![];
        let mut fences = BTreeMap::new();
        if !selected.iter().any(|s| s == "shared") {
            ensure!(p.installed.get("shared").is_some_and(|r|self.matches(&p.stage,r)),"Shared level settings also changed. Include them in this update before sharing dependent slices.");
        }
        let shared = if selected.iter().any(|s| s == "shared") {
            Some(hash(&files["shared"]))
        } else {
            p.installed.get("shared").map(|r| r.hash.clone())
        };
        progress("Reserving the changed slices…".into());
        for id in selected {
            let b = files.get(&id).context("Unknown section")?;
            let h = self.cache(b)?;
            let content = binary::content_hash(b, &id, &binary::resource_level(b, &id)?)?;
            ensure!(
                p.installed
                    .get(&id)
                    .is_none_or(|r| !crate::installer::matches_bytes(
                        b,
                        r,
                        &self.data.join("objects")
                    )
                    .unwrap_or(false)),
                "This slice has no saved changes"
            );
            let lease = self.acquire(&id, false).await?;
            fences.insert(id.clone(), lease.fence);
            revisions.push(Revision {
                section: id.clone(),
                name: crate::names::effective_names(&p.project)
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "Shared level settings".into()),
                hash: h.clone(),
                content_hash: content,
                parent: p.installed.get(&id).map(|r| r.hash.clone()),
                artifact: String::new(),
                size: b.len() as u64,
                shared_dependency: if id == "shared" { None } else { shared.clone() },
            });
        }
        ensure!(!revisions.is_empty(), "Choose saved changes to share");
        for r in &revisions {
            ensure!(
                p.project.heads.get(&r.section).map(|h| h.hash.clone()) == r.parent,
                "{} has shared changes. Review the conflict; your changelog is preserved.",
                r.name
            );
        }
        let key = format!("pending:{}", p.project.id);
        let previous = self.db.get::<PublishRequest>(&key)?;
        let id = previous
            .filter(|r| {
                r.summary == draft.summary
                    && r.details == draft.details
                    && r.revisions.len() == revisions.len()
                    && r.revisions.iter().zip(&revisions).all(|(a, b)| {
                        a.section == b.section
                            && a.hash == b.hash
                            && a.parent == b.parent
                            && a.shared_dependency == b.shared_dependency
                    })
            })
            .map(|r| r.id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        for r in &mut revisions {
            r.artifact = history_artifact(&id, &r.section)?;
        }
        let req = PublishRequest {
            id,
            summary: draft.summary,
            details: draft.details,
            revisions,
            fences,
            restores: None,
        };
        self.db.set(&key, &req)?;
        drop(guard);
        for (index, r) in req.revisions.iter().enumerate() {
            progress(format!(
                "Uploading {} ({} of {})…",
                r.name,
                index + 1,
                req.revisions.len()
            ));
            storage.put(&r.artifact, files[&r.section].clone()).await?;
        }
        storage.put(&format!("History/{}/pending.json",req.id),serde_json::to_vec_pretty(&serde_json::json!({"id":req.id,"authorId":p.member,"summary":req.summary,"details":req.details,"revisions":req.revisions,"restores":req.restores}))?).await?;
        progress("Publishing your changelog…".into());
        let body = self
            .control_command(&p, "/publish", serde_json::to_value(&req)?)
            .await?;
        let _guard = self.operation.lock().await;
        let current = self.active()?;
        ensure!(current.project.id==p.project.id&&current.endpoint==p.endpoint&&current.member==p.member&&current.stage.path==p.stage.path,"The active project changed during upload. Reopen the original project to retry your saved draft.");
        ensure!(!self.installation(&current)?.is_some_and(|r| r.pending()), "Finish applying the update started during upload, or recover it from History, before sharing. Your draft is preserved.");
        p = current;
        let raw = self
            .control_http(&p, reqwest::Method::POST, "/publish", Some(body))
            .await?;
        let u: Update = serde_json::from_value(self.control_response(&p, "/publish", raw).await?)?;
        progress("Finishing your update…".into());
        for r in &u.revisions {
            p.installed.insert(r.section.clone(), r.clone());
        }
        self.persist(&p)?;
        self.db.cache(&p.project.id, &u)?;
        self.db.remove(&key)?;
        self.db
            .set(&format!("draft:{}", p.project.id), &Draft::default())?;
        // Sharing frees the shared slices unless "Keep reserved" is ticked. If that fails the
        // reservation just expires. (archive_history retries the changelog archive later.)
        if !keep_reserved {
            // A save made during the upload is still unshared work.
            let ids: Vec<String> = u
                .revisions
                .iter()
                .filter(|r| !self.has_local_changes(&p, &r.section))
                .map(|r| r.section.clone())
                .collect();
            if !ids.is_empty() {
                let _ = self.release_held(&ids).await;
            }
        }
        if self.sync(false).await.is_err() {
            self.online.store(false, Ordering::Relaxed);
        }
        self.changed();
        Ok(u)
    }
    /// The owner removed this creator: stop reserving and show the in-app removal state.
    pub fn mark_removed(&self, p: &LocalProject) {
        if !self.active.lock().unwrap().as_ref().is_some_and(|current| {
            current.endpoint == p.endpoint
                && current.project.id == p.project.id
                && current.member == p.member
        }) {
            return;
        }
        let key = removed_key(&p.project.id);
        if self.db.get::<bool>(&key).ok().flatten() == Some(true) {
            return;
        }
        let _ = self.db.set(&key, &true);
        self.leases.lock().unwrap().clear();
        self.online.store(false, Ordering::Relaxed);
        self.changed();
    }
    /// Our presence status (in Miliastra or not), encrypted so the coordinator can't see it.
    pub fn presence_message(&self, p: &LocalProject, game: bool) -> Result<String> {
        let sealed = self.privacy_keys(&p.endpoint, &p.project.id)?.seal(
            &p.endpoint,
            &p.project.id,
            &miliastra_core::presence::status_purpose(&p.member),
            &serde_json::json!({ "game": game }),
        )?;
        Ok(serde_json::json!({ "kind": "status", "status": sealed }).to_string())
    }
    /// Keeps renewing reservations we still hold after a restart or reopen.
    fn adopt_leases(&self, p: &LocalProject) {
        let now = chrono::Utc::now().timestamp();
        // Ignore this if the user switched projects while the sync ran.
        if !self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.project.id == p.project.id && a.endpoint == p.endpoint)
        {
            return;
        }
        let mut leases = self.leases.lock().unwrap();
        for l in &p.project.locks {
            if l.member == p.member && l.expires > now {
                self.lease_activity
                    .lock()
                    .unwrap()
                    .entry(l.section.clone())
                    .or_insert(now);
                leases
                    .entry(l.section.clone())
                    .and_modify(|held| {
                        if held.fence < l.fence {
                            *held = l.clone();
                        }
                    })
                    .or_insert_with(|| l.clone());
            }
        }
    }
    /// Releases reservations with no saves for a while. Slices with unshared changes stay
    /// reserved and get one reminder.
    async fn release_idle(&self, p: &LocalProject) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let idle: Vec<String> = self
            .leases
            .lock()
            .unwrap()
            .keys()
            .filter(|id| {
                self.lease_activity
                    .lock()
                    .unwrap()
                    .get(*id)
                    .is_some_and(|at| now - at >= IDLE_RELEASE_SECS)
            })
            .cloned()
            .collect();
        if idle.is_empty() {
            return Ok(());
        }
        // Skip this round if a share or reservation change is running.
        let Ok(_guard) = self.operation.try_lock() else {
            return Ok(());
        };
        let names = crate::names::effective_names(&p.project);
        let label = |id: &str| {
            names
                .get(id)
                .cloned()
                .unwrap_or_else(|| "Shared level settings".into())
        };
        let (unshared, unused): (Vec<String>, Vec<String>) = idle
            .into_iter()
            .partition(|id| self.has_local_changes(p, id));
        for id in unshared {
            if self.idle_reminded.lock().unwrap().insert(id.clone()) {
                self.alert(Notice::UnsharedChanges(&label(&id)));
            }
        }
        if !unused.is_empty() {
            self.release_held(&unused).await?;
            self.alert(Notice::AutoReleased(
                &unused
                    .iter()
                    .map(|id| label(id))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        Ok(())
    }
    pub async fn heartbeat(&self) -> Result<()> {
        use miliastra_core::errors::{ErrorCode, ServiceError};
        let p = self.active()?;
        self.release_idle(&p).await?;
        let leases = self.leases.lock().unwrap().clone();
        let results = crate::lease_manager::renew(leases, |id, l| {
            let p = &p;
            async move {
                self.api::<Lease>(
                    p,
                    reqwest::Method::POST,
                    "/locks/renew",
                    Some(serde_json::json!({"section":id,"fence":l.fence})),
                )
                .await
            }
        })
        .await;
        // Ignore responses that belong to the previous project.
        let active = self.active.lock().unwrap();
        if !active.as_ref().is_some_and(|a| {
            a.project.id == p.project.id && a.endpoint == p.endpoint && a.member == p.member
        }) {
            return Ok(());
        }
        let mut failure = None;
        let mut lost = false;
        for (id, previous, result) in results {
            let mut current = self.leases.lock().unwrap();
            if current
                .get(&id)
                .is_none_or(|l| l.fence != previous.fence || l.member != previous.member)
            {
                continue;
            }
            match result {
                Ok(renewed) => {
                    current.insert(id, renewed);
                }
                Err(e)
                    if e.downcast_ref::<ServiceError>()
                        .is_some_and(|e| e.code == ErrorCode::ReservationLost) =>
                {
                    current.remove(&id);
                    lost = true;
                }
                Err(e) => {
                    failure = Some(e);
                }
            }
        }
        drop(active);
        if lost {
            self.changed();
        }
        failure.map_or(Ok(()), Err)
    }
    pub async fn saved_detection(&self) -> Result<()> {
        let guard = self.operation.lock().await;
        let p = self.active()?;
        let stage = binary::inspect(&p.stage.path)?;
        let previous = self.observed.lock().unwrap().clone();
        let observation = miliastra_core::detector::saved(
            stage.last_saved_section.clone(),
            &stage.settings_version,
            previous.as_deref(),
            chrono::Utc::now().timestamp(),
        );
        let changed = !observation.historical;
        let mut warned = std::collections::BTreeSet::new();
        self.db.set("observation", &observation)?;
        *self.observed.lock().unwrap() = Some(stage.settings_version.clone());
        let id = stage.last_saved_section.clone();
        let mut local = p;
        let pending = self.installation(&local)?;
        for (id, head) in &local.project.heads {
            if !pending.as_ref().is_some_and(|r| r.affects(id)) && self.matches(&stage, head) {
                local.installed.insert(id.clone(), head.clone());
            }
        }
        local.stage = stage;
        self.persist(&local)?;
        if changed {
            let files = self.stable(&local.stage).await?;
            let previous_drafts: BTreeMap<String, String> = self
                .db
                .get(&format!("saved-drafts:{}", local.project.id))?
                .unwrap_or_default();
            let mut drafts = BTreeMap::new();
            for (id, bytes) in files {
                if local.installed.get(&id).is_none_or(|r| {
                    !crate::installer::matches_bytes(&bytes, r, &self.data.join("objects"))
                        .unwrap_or(false)
                }) {
                    if !pending.as_ref().is_some_and(|r| r.affects(&id)) {
                        let h = self.cache(&bytes)?;
                        // Only a new save of this slice counts as activity on its reservation.
                        if previous_drafts.get(&id) != Some(&h)
                            && self.leases.lock().unwrap().contains_key(&id)
                        {
                            self.touch_lease(&id);
                        }
                        drafts.insert(id, h);
                    }
                }
            }
            self.db
                .set(&format!("saved-drafts:{}", local.project.id), &drafts)?;
            warned = self.warn_reserved_changes(&local, &previous_drafts, &drafts);
        }
        self.synchronize_names(&mut local)?;
        self.persist(&local)?;
        drop(guard);
        if changed && self.editor.editing_active() && !self.is_practice(&local) {
            // The reservation warning already explains this.
            if let Some(id) = id.filter(|id| !warned.contains(id)) {
                let _ = self.start(&id, false).await;
            }
        }
        self.changed();
        Ok(())
    }
    pub async fn refresh_latest(&self) -> Result<()> {
        let p = self.active()?;
        let storage = self.storage(&p);
        // Latest is rebuilt from the project heads, which sync and events keep current, so
        // there's no need to ask the coordinator here.
        for r in p.project.heads.values() {
            if !self
                .active
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|a| a.project.id == p.project.id && a.endpoint == p.endpoint)
            {
                break;
            }
            validate_revision(r)?;
            let path = self.cache_path(&r.hash)?;
            if !path.exists() {
                continue;
            }
            let marker = format!("latest:{}:{}", p.project.id, r.section);
            if self.db.get::<String>(&marker)?.as_deref() == Some(&r.hash) {
                continue;
            }
            let bytes = std::fs::read(path)?;
            ensure!(hash(&bytes) == r.hash, "Cached slice checksum failed");
            storage
                .latest_from_history(
                    &r.artifact,
                    &format!("Latest/{}", slice_filename(&r.section)?),
                    bytes,
                    &self.db,
                )
                .await?;
            self.db.set(&marker, &r.hash)?;
        }
        Ok(())
    }
    pub fn request_latest_rebuild(&self) -> Result<()> {
        let p = self.active()?;
        self.retry_team_metadata(&p.project.id)?;
        for (id, head) in &p.project.heads {
            self.db.remove(&format!("download-retry:{}", head.hash))?;
            self.db.remove(&format!("latest:{}:{}", p.project.id, id))?;
        }
        Ok(())
    }
    async fn archive_history(&self, p: &LocalProject) -> Result<()> {
        let storage = self.storage(p);
        // Keep going after a failure so newer entries still get archived.
        let mut failure = None;
        for u in self.db.history(&p.project.id)? {
            if !self
                .active
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|a| a.project.id == p.project.id && a.endpoint == p.endpoint)
            {
                break;
            }
            let marker = format!("archived:{}", u.id);
            if self.db.get::<bool>(&marker)?.unwrap_or(false) {
                continue;
            }
            let receipt = self
                .db
                .get::<miliastra_core::control::Update>(&format!(
                    "receipt:{}:{}",
                    p.project.id, u.sequence
                ))?
                .context("Publication receipt missing")?;
            if let Err(error) = storage
                .put(
                    &format!("History/{}/receipt.json", u.id),
                    serde_json::to_vec(&receipt)?,
                )
                .await
            {
                failure = Some(error);
                continue;
            }
            let archive = format!("History/{}/changelog.json", u.id);
            match storage.put(&archive, serde_json::to_vec_pretty(&u)?).await {
                Ok(()) => {
                    self.db.set(&marker, &true)?;
                    self.db.remove(&format!("archive-error:{}", u.id))?;
                }
                Err(e) => {
                    self.db
                        .set(&format!("archive-error:{}", u.id), &e.to_string())?;
                    failure = Some(e);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub async fn download_pending(&self) -> Result<()> {
        let Ok(_storage_guard) = self.transfers.downloads.try_lock() else {
            return Ok(());
        };
        let initial = self.active()?;
        let metadata_key = format!("metadata-reconcile:{}", initial.project.id);
        if self.download_team_metadata(&initial).await? {
            self.db.set(&metadata_key, &true)?;
        }
        if self.db.get::<bool>(&metadata_key)?.unwrap_or(false) {
            if let Ok(_guard) = self.operation.try_lock() {
                if self.sync(false).await.is_ok() {
                    self.db.remove(&metadata_key)?;
                }
            }
        }
        let p = self.active()?;
        let storage = self.storage(&p);
        let mut count = 0;
        let mut activity = false;
        for r in p.project.heads.values() {
            let path = self.cache_path(&r.hash)?;
            if path.exists() {
                continue;
            }
            if !self
                .active
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|a| a.project.id == p.project.id)
            {
                break;
            }
            if !self
                .transfers
                .ready(&self.db, &r.hash, chrono::Utc::now().timestamp())?
            {
                continue;
            }
            activity = true;
            let result = async {
                let b = storage.get(&r.artifact).await?;
                ensure!(
                    b.len() as u64 == r.size && hash(&b) == r.hash,
                    "Downloaded file failed checksum validation"
                );
                let level = binary::resource_level(&b, &r.section)?;
                ensure!(
                    binary::content_hash(&b, &r.section, &level)? == r.content_hash,
                    "Downloaded slice failed content validation"
                );
                self.cache(&b)?;
                Ok::<_, anyhow::Error>(())
            }
            .await;
            match result {
                Ok(()) => {
                    self.transfers.completed(&self.db, &r.hash)?;
                    if p.installed
                        .get(&r.section)
                        .is_none_or(|base| base.hash != r.hash)
                        && !self.matches(&p.stage, r)
                    {
                        count += 1;
                    }
                }
                Err(e) => {
                    self.transfers
                        .failed(&self.db, &r.hash, &e, chrono::Utc::now().timestamp())?;
                }
            }
        }
        if count > 0 {
            let latest = self.db.history(&p.project.id)?.into_iter().find(|u| {
                u.author_id != p.member
                    && u.revisions.iter().any(|r| {
                        p.project
                            .heads
                            .get(&r.section)
                            .is_some_and(|h| h.hash == r.hash)
                            && p.installed
                                .get(&r.section)
                                .is_none_or(|base| base.hash != r.hash)
                    })
            });
            self.alert(Notice::Downloaded {
                count,
                creator: latest
                    .as_ref()
                    .map(|u| u.author.as_str())
                    .unwrap_or("Teammates"),
            });
        }
        if activity {
            let _guard = self.operation.lock().await;
            if let Ok(mut current) = self.active() {
                if current.project.id == p.project.id {
                    self.reconcile_installed(&mut current)?;
                    self.persist(&current)?;
                }
            }
            self.changed();
        }
        if self
            .active
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|a| a.project.id == p.project.id && a.endpoint == p.endpoint)
        {
            let _ = self.archive_history(&p).await;
            if let Err(e) = self.refresh_latest().await {
                self.db
                    .set(&format!("latest-error:{}", p.project.id), &e.to_string())?;
            } else {
                self.db.remove(&format!("latest-error:{}", p.project.id))?;
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_exchange(
    root: &std::path::Path,
    stage: &std::path::Path,
    save_root: &std::path::Path,
) -> Result<()> {
    let root = root.canonicalize()?;
    let stage = stage.canonicalize()?;
    let mut protected = vec![stage.clone()];
    if let Ok(path) = save_root.canonicalize() {
        protected.push(path);
    }
    if let Some(levels) = stage.parent() {
        if levels
            .file_name()
            .is_some_and(|n| n == "Beyond_Local_Save_Level")
        {
            if let Some(beyond) = levels.parent().and_then(std::path::Path::parent) {
                protected.push(beyond.to_path_buf());
            }
        }
    }
    ensure!(
        protected
            .iter()
            .all(|path| !root.starts_with(path) && !path.starts_with(&root)),
        "Choose an exchange folder outside all Miliastra save folders"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_hydration_fills_cold_cache_and_checks_incremental_pagination() {
        let update = |sequence| {
            serde_json::from_value::<Update>(serde_json::json!({
            "id": "test", "authorId": "member", "author": "Author", "at": 1,
            "sequence": sequence, "summary": "Saved update", "details": "", "revisions": [], "restores": null
        })).unwrap()
        };
        let page = HistoryPage {
            entries: vec![update(100), update(51)],
            next: Some(51),
        };
        assert_eq!(
            next_history_cursor(0, false, None, &page).unwrap(),
            Some(51)
        );
        assert_eq!(next_history_cursor(80, false, None, &page).unwrap(), None);
        assert_eq!(
            next_history_cursor(80, true, None, &page).unwrap(),
            Some(51)
        );
        assert!(next_history_cursor(0, false, Some(51), &page).is_err());
        assert!(next_history_cursor(
            0,
            false,
            None,
            &HistoryPage {
                entries: vec![],
                next: Some(3)
            }
        )
        .is_err());
        assert_eq!(
            next_history_cursor(
                0,
                false,
                Some(51),
                &HistoryPage {
                    entries: vec![update(1)],
                    next: None
                }
            )
            .unwrap(),
            None
        );
    }
    #[test]
    fn exchange_cannot_target_another_game_stage() {
        let base = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let saves = base.join("BeyondLocal");
        let stage = saves.join("account/Beyond_Local_Save_Level/1");
        let other = saves.join("account/Beyond_Local_Save_Level/2");
        let exchange = base.join("exchange");
        for p in [&stage, &other, &exchange] {
            std::fs::create_dir_all(p).unwrap();
        }
        assert!(validate_exchange(&other, &stage, &saves).is_err());
        assert!(validate_exchange(&base, &stage, &saves).is_err());
        assert!(validate_exchange(&exchange, &stage, &saves).is_ok());
        let resolved = base.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(uuid::Uuid::parse_str(&resolved.file_name().unwrap().to_string_lossy()).is_ok());
        std::fs::remove_dir_all(resolved).unwrap();
    }
}

#[cfg(test)]
mod creation_service_tests {
    use super::*;
    #[tokio::test]
    async fn creation_preflight_accepts_only_a_healthy_coordinator() {
        use axum::{routing::get, Router};
        for (body, healthy) in [("ok", true), ("Unrelated website", false)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let ep = format!("http://{}", listener.local_addr().unwrap());
            let app = Router::new().route("/health", get(move || async move { body }));
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            assert_eq!(check_creation_service(&ep).await.is_ok(), healthy);
            server.abort();
        }
    }
    #[tokio::test]
    async fn unreachable_coordinator_fails_before_creation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ep = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let error = check_creation_service(&ep).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("collaboration service is unavailable"));
    }
}
