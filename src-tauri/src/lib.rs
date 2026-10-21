mod access;
mod assets;
mod conflict_runtime;
mod conflicts;
mod coordination;
mod coordinator_client;
mod coordinator_sync;
mod credentials;
mod db;
mod editor_detector;
mod installer;
mod invitation;
mod lease_manager;
mod names;
mod native;
mod notifications;
mod oauth;
mod portable;
#[cfg(test_build)]
mod practice;
mod privacy;
mod private_metadata;
mod profile;
mod providers;
mod recovery;
#[cfg(debug_assertions)]
mod reliability;
mod restore;
mod retry;
mod runtime;
#[cfg(test)]
mod service_tests;
mod slice_tools;
mod storage;
mod tasks;
mod team_metadata;
mod transaction;
mod transfer_queue;
mod tray_status;
use runtime::{Core, Draft, Settings};
use std::{path::PathBuf, sync::atomic::Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_deep_link::DeepLinkExt;
type Cmd<T> = Result<T, String>;
fn err(e: anyhow::Error) -> String {
    e.to_string()
}
#[tauri::command]
async fn snapshot(core: tauri::State<'_, Core>) -> Cmd<runtime::Snapshot> {
    let app = core.app.clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<Core>().snapshot())
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}
#[tauri::command]
async fn inspect_conflict(
    core: tauri::State<'_, Core>,
    preview_id: String,
) -> Cmd<conflicts::Comparison> {
    core.inspect_conflict(&preview_id).await.map_err(err)
}
#[tauri::command]
async fn inspect_slice(core: tauri::State<'_, Core>, section: String) -> Cmd<assets::Inspection> {
    core.inspect_slice(&section).await.map_err(err)
}
#[tauri::command]
async fn preview_revert(
    core: tauri::State<'_, Core>,
    section: String,
) -> Cmd<slice_tools::RevertPreview> {
    core.preview_revert(&section).await.map_err(err)
}
#[tauri::command]
async fn revert_slice(
    core: tauri::State<'_, Core>,
    preview_id: String,
    editor_ready: bool,
) -> Cmd<()> {
    core.revert_slice(&preview_id, editor_ready)
        .await
        .map_err(err)
}
#[tauri::command]
async fn generate_move_id(
    core: tauri::State<'_, Core>,
    source: String,
    asset_key: String,
    target: String,
) -> Cmd<slice_tools::MoveId> {
    core.generate_move_id(&source, &asset_key, &target)
        .await
        .map_err(err)
}
#[tauri::command]
async fn validate_move_id(core: tauri::State<'_, Core>, token: String) -> Cmd<String> {
    core.validate_move_id(&token).await.map_err(err)
}
#[tauri::command]
async fn discover(core: tauri::State<'_, Core>) -> Cmd<Vec<miliastra_core::binary::Stage>> {
    let root = core.settings.lock().unwrap().save_root.clone();
    tauri::async_runtime::spawn_blocking(move || miliastra_core::binary::discover(&root))
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn choose_folder(app: tauri::AppHandle) -> Cmd<Option<PathBuf>> {
    #[cfg(debug_assertions)]
    if let (Some(root), Some(folder)) = (
        std::env::var_os("MCOLLAB_TEST_DATA_DIR"),
        std::env::var_os("MCOLLAB_TEST_FOLDER"),
    ) {
        let root = PathBuf::from(root)
            .canonicalize()
            .map_err(|_| "Test profile is unavailable")?;
        let folder = PathBuf::from(folder)
            .canonicalize()
            .map_err(|_| "Test folder is unavailable")?;
        if !folder.starts_with(&root) || !folder.is_dir() {
            return Err("Test folder must stay inside the isolated profile".into());
        }
        return Ok(Some(folder));
    }
    use tauri_plugin_dialog::DialogExt;
    let value = app.dialog().file().blocking_pick_folder();
    value
        .map(|v| v.into_path().map_err(|e| e.to_string()))
        .transpose()
}
#[tauri::command]
async fn create_project(
    core: tauri::State<'_, Core>,
    name: String,
    display_name: String,
    stage_path: PathBuf,
    kind: String,
    folder: Option<PathBuf>,
) -> Cmd<()> {
    core.create(name, display_name, stage_path, kind, folder)
        .await
        .map_err(err)
}
#[tauri::command]
async fn join_project(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
    display_name: String,
    stage_path: PathBuf,
    folder: Option<PathBuf>,
) -> Cmd<()> {
    core.join(invitation, display_name, stage_path, folder)
        .await
        .map_err(err)
}
#[tauri::command]
async fn start_work(core: tauri::State<'_, Core>, section: String, takeover: bool) -> Cmd<()> {
    core.start(&section, takeover).await.map_err(err)
}
#[tauri::command]
async fn request_folder_access(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
    display_name: String,
) -> Cmd<String> {
    core.request_folder_access(invitation, display_name)
        .await
        .map_err(err)
}
/// The joiner's storage account email, so they can tell the owner who to share with.
#[tauri::command]
async fn storage_account_email(core: tauri::State<'_, Core>, provider: String) -> Cmd<String> {
    providers::cloud(&provider).map_err(err)?;
    let config = core.settings.lock().unwrap().oauth.clone();
    access::account_email(&provider, &config).await.map_err(err)
}
#[tauri::command]
fn invitation_privacy_key_ready(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
) -> bool {
    core.read_privacy_key(&invitation.endpoint, &invitation.project)
        .is_ok_and(|key| privacy::key_id(&zeroize::Zeroizing::new(key)).is_ok())
}
#[tauri::command]
fn folder_privacy_key(core: tauri::State<'_, Core>) -> Cmd<String> {
    core.owner_privacy_key().map_err(err)
}
#[tauri::command]
fn save_folder_privacy_key(
    core: tauri::State<'_, Core>,
    invitation: Option<miliastra_core::Invitation>,
    key: String,
) -> Cmd<()> {
    let key = zeroize::Zeroizing::new(key);
    if let Some(i) = invitation {
        core.save_privacy_key(&i.endpoint, &i.project, &key)
            .map_err(err)
    } else {
        let p = core.active().map_err(err)?;
        core.save_privacy_key(&p.endpoint, &p.project.id, &key)
            .map_err(err)
    }
}
#[tauri::command]
async fn folder_access_status(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
    id: String,
) -> Cmd<access::AccessStatus> {
    let status = core
        .folder_access_status(invitation, id)
        .await
        .map_err(err)?;
    serde_json::from_value(status).map_err(|e| e.to_string())
}
/// Test profiles can't receive invitation links from the browser; paste them instead.
#[tauri::command]
async fn open_invitation_link(core: tauri::State<'_, Core>, link: String) -> Cmd<()> {
    // Test profiles and isolated test runs can't receive links from the browser.
    if profile::current().is_none()
        && !(cfg!(debug_assertions) && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some())
    {
        return Err("Open invitations from the invitation website".into());
    }
    let parsed = invitation::resolve(&link).await.map_err(err)?;
    core.db.set("incoming-invitation", &parsed).map_err(err)?;
    let _ = core.app.emit("invitation-opened", ());
    Ok(())
}
#[tauri::command]
async fn restore_version(
    core: tauri::State<'_, Core>,
    update_id: String,
    section: String,
    reason: String,
) -> Cmd<miliastra_core::Update> {
    core.restore_version(&update_id, &section, &reason)
        .await
        .map_err(err)
}
#[tauri::command]
async fn remove_member(core: tauri::State<'_, Core>, member: String) -> Cmd<()> {
    core.manage_team("/members/remove", serde_json::json!({ "member": member }))
        .await
        .map_err(err)
}
#[tauri::command]
async fn transfer_ownership(core: tauri::State<'_, Core>, member: String) -> Cmd<()> {
    core.manage_team("/members/owner", serde_json::json!({ "member": member }))
        .await
        .map_err(err)
}
#[tauri::command]
async fn request_finish(core: tauri::State<'_, Core>, section: String) -> Cmd<()> {
    core.request_finish(&section).await.map_err(err)
}
#[tauri::command]
async fn finish_work(core: tauri::State<'_, Core>, section: Option<String>) -> Cmd<()> {
    core.finish_slices(section.as_deref()).await.map_err(err)
}
#[tauri::command]
fn save_draft(core: tauri::State<Core>, project_id: String, draft: Draft) -> Cmd<()> {
    let p = core.active().map_err(err)?;
    if p.project.id != project_id {
        return Err(
            "The active project changed; reopen your draft in its original project.".into(),
        );
    }
    core.db
        .set(&format!("draft:{}", p.project.id), &draft)
        .map_err(err)
}
#[tauri::command]
async fn share_update(
    core: tauri::State<'_, Core>,
    sections: Vec<String>,
    draft: Draft,
    keep_reserved: Option<bool>,
) -> Cmd<miliastra_core::Update> {
    core.share(sections, draft, keep_reserved.unwrap_or(false))
        .await
        .map_err(err)
}
#[tauri::command]
fn save_names_draft(core: tauri::State<Core>, project_id: String, draft: names::Draft) -> Cmd<()> {
    let p = core.active().map_err(err)?;
    if p.project.id != project_id {
        return Err("The active project changed".into());
    }
    if draft
        .edits
        .keys()
        .any(|id| !p.stage.sections.iter().any(|s| &s.id == id))
    {
        return Err("Unknown slice name".into());
    }
    names::save_draft(&core.db, &p.project.id, draft).map_err(err)
}
#[tauri::command]
async fn share_section_names(
    core: tauri::State<'_, Core>,
    names: std::collections::BTreeMap<String, String>,
    parent_version: u64,
) -> Cmd<miliastra_core::Update> {
    core.share_names(names, parent_version).await.map_err(err)
}
#[tauri::command]
async fn refresh_saved_changes(core: tauri::State<'_, Core>) -> Cmd<()> {
    core.saved_detection().await.map_err(err)
}
#[tauri::command]
async fn refresh(core: tauri::State<'_, Core>) -> Cmd<()> {
    let _guard = core.operation.lock().await;
    core.request_latest_rebuild().map_err(err)?;
    core.sync(true).await.map_err(err)
}
#[tauri::command]
async fn invite(core: tauri::State<'_, Core>) -> Cmd<String> {
    let p = core.active().map_err(err)?;
    let i: miliastra_core::Invitation = core
        .api(
            &p,
            reqwest::Method::POST,
            "/invite",
            Some(serde_json::json!({})),
        )
        .await
        .map_err(err)?;
    Ok(invitation::share_link(&i))
}
/// The test coordinator's address, after checking it's running on this PC.
#[cfg(test_build)]
#[tauri::command]
async fn test_service(core: tauri::State<'_, Core>) -> Cmd<String> {
    core.practice_endpoint().await.map_err(err)
}
#[cfg(test_build)]
#[tauri::command]
async fn begin_practice(core: tauri::State<'_, Core>) -> Cmd<()> {
    core.begin_practice().await.map_err(err)
}
#[cfg(test_build)]
#[tauri::command]
async fn leave_practice(core: tauri::State<'_, Core>) -> Cmd<()> {
    let p = core.active().map_err(err)?;
    if !core.is_practice(&p) {
        return Err("This is not the practice project".into());
    }
    let previous = core.db.get::<String>("practice-return").map_err(err)?;
    leave_project(core.clone()).await?;
    core.db.remove("practice-return").map_err(err)?;
    if let Some(id) = previous {
        core.reopen(&id).await.map_err(err)?;
    }
    Ok(())
}
#[cfg(test_build)]
#[tauri::command]
async fn practice_conflict(core: tauri::State<'_, Core>) -> Cmd<()> {
    core.practice_conflict().await.map_err(err)
}
#[tauri::command]
async fn preview_conflict(
    core: tauri::State<'_, Core>,
    section: String,
) -> Cmd<conflicts::Preview> {
    core.preview_conflict(&section).await.map_err(err)
}
#[tauri::command]
async fn resolve_conflict(
    core: tauri::State<'_, Core>,
    preview_id: String,
    choice: String,
    draft: Draft,
    editor_ready: bool,
) -> Cmd<()> {
    core.resolve_conflict(&preview_id, &choice, draft, editor_ready)
        .await
        .map_err(err)
}
#[tauri::command]
fn save_conflict_draft(core: tauri::State<Core>, preview_id: String, draft: Draft) -> Cmd<()> {
    uuid::Uuid::parse_str(&preview_id).map_err(|e| e.to_string())?;
    let p = core.active().map_err(err)?;
    let preview = core
        .db
        .get::<conflicts::Preview>(&format!("conflict-preview:{preview_id}"))
        .map_err(err)?
        .ok_or("Review the conflict first")?;
    if preview.project != p.project.id {
        return Err("The selected project changed".into());
    }
    core.db
        .set(
            &format!("conflict-draft:{}:{}", p.project.id, preview.section),
            &draft,
        )
        .map_err(err)
}
#[tauri::command]
fn open_preserved_conflict(core: tauri::State<Core>, preview_id: String) -> Cmd<()> {
    uuid::Uuid::parse_str(&preview_id).map_err(|e| e.to_string())?;
    let preview = core
        .db
        .get::<conflicts::Preview>(&format!("conflict-preview:{preview_id}"))
        .map_err(err)?
        .ok_or("Preserved versions unavailable")?;
    let folder = preview.folder.canonicalize().map_err(|e| e.to_string())?;
    if !folder.starts_with(
        core.data
            .join("conflicts")
            .canonicalize()
            .map_err(|e| e.to_string())?,
    ) {
        return Err("Invalid preserved-copy location".into());
    }
    reveal(&folder)
}
#[tauri::command]
async fn invitation_details(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
) -> Cmd<invitation::Details> {
    let mut info = match core.invite_details(invitation.clone()).await {
        Ok(info) => info,
        Err(_) => {
            invitation::resolve(&invitation::share_link(&invitation))
                .await
                .map_err(err)?;
            return Ok(invitation::Details {
                project_name: "Encrypted project".into(),
                owner_name: None,
                privacy_key_required: true,
                storage_kind: "locked".into(),
                mapped_folder: None,
                storage: miliastra_core::StorageConfig {
                    kind: "locked".into(),
                    location: String::new(),
                    drive_id: None,
                },
            });
        }
    };
    info.mapped_folder = core
        .db
        .get::<Vec<runtime::LocalProject>>("connections")
        .map_err(err)?
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.project.id == invitation.project && p.endpoint == invitation.endpoint)
        .and_then(|p| p.folder)
        .filter(|p| p.is_dir());
    Ok(info)
}
#[tauri::command]
async fn check_invitation_storage(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
    folder: Option<PathBuf>,
    stage_path: Option<PathBuf>,
) -> Cmd<tasks::StorageCheck> {
    core.check_invitation_storage(
        invitation,
        folder,
        stage_path.filter(|p| !p.as_os_str().is_empty()),
    )
    .await
    .map_err(err)
}
#[tauri::command]
async fn download_base_stage(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
    folder: Option<PathBuf>,
) -> Cmd<PathBuf> {
    let path = core
        .download_base_stage(invitation, folder)
        .await
        .map_err(err)?;
    reveal(&path)?;
    Ok(path)
}
#[tauri::command]
async fn find_project_stage(
    core: tauri::State<'_, Core>,
    invitation: miliastra_core::Invitation,
    folder: Option<PathBuf>,
) -> Cmd<Option<PathBuf>> {
    core.find_project_stage(invitation, folder)
        .await
        .map_err(err)
}
#[tauri::command]
async fn publish_base_stage(core: tauri::State<'_, Core>) -> Cmd<()> {
    let _guard = core.operation.lock().await;
    core.publish_base_stage().await.map_err(err)
}
#[tauri::command]
fn save_task(core: tauri::State<Core>, project_id: String, task: Option<runtime::Task>) -> Cmd<()> {
    let p = core.active().map_err(err)?;
    if p.project.id != project_id {
        return Err("The active project changed".into());
    }
    let key = format!("task:{project_id}");
    if let Some(task) = task {
        if !["share", "apply"].contains(&task.kind.as_str())
            || task.selected.len() > 65
            || task
                .selected
                .iter()
                .any(|id| !miliastra_core::valid_section(id))
        {
            return Err("Invalid task".into());
        }
        if task
            .installation_id
            .as_ref()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
        {
            return Err("Invalid installation task".into());
        }
        core.db.set(&key, &task).map_err(err)?;
    } else {
        core.db.remove(&key).map_err(err)?;
    }
    Ok(())
}
#[tauri::command]
async fn forget_project(core: tauri::State<'_, Core>) -> Cmd<()> {
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), core.finish()).await;
    let _guard = core.operation.lock().await;
    let p = core.active().map_err(err)?;
    core.db
        .forget_connection(&p.project.id, &p.endpoint)
        .map_err(err)?;
    *core.active.lock().unwrap() = None;
    *core.watcher.lock().unwrap() = None;
    core.leases.lock().unwrap().clear();
    core.lease_activity.lock().unwrap().clear();
    core.online.store(false, Ordering::Relaxed);
    core.changed();
    Ok(())
}

#[tauri::command]
async fn install_updates(
    core: tauri::State<'_, Core>,
    plan_id: String,
    editor_ready: bool,
) -> Cmd<()> {
    core.install_slices(&plan_id, editor_ready)
        .await
        .map_err(err)
}
#[tauri::command]
async fn preview_install(core: tauri::State<'_, Core>) -> Cmd<installer::Plan> {
    core.preview_install().await.map_err(err)
}
#[tauri::command]
async fn verify_install(core: tauri::State<'_, Core>) -> Cmd<()> {
    core.verify_install().await.map_err(err)
}
#[tauri::command]
async fn undo_install(core: tauri::State<'_, Core>, editor_ready: bool) -> Cmd<()> {
    core.undo_install(editor_ready).await.map_err(err)
}
#[tauri::command]
async fn sign_in(core: tauri::State<'_, Core>, provider: String) -> Cmd<()> {
    storage::ensure_provider_available(&provider).map_err(err)?;
    let config = core.settings.lock().unwrap().oauth.clone();
    oauth::login(&provider, &config).await.map_err(err)?;
    core.changed();
    Ok(())
}
#[tauri::command]
fn disconnect(core: tauri::State<'_, Core>, provider: String) -> Cmd<()> {
    oauth::disconnect(&provider).map_err(err)?;
    core.changed();
    Ok(())
}
#[tauri::command]
fn save_settings(core: tauri::State<Core>, mut settings: Settings) -> Cmd<()> {
    settings.oauth = core.settings.lock().unwrap().oauth.clone();
    runtime::endpoint(&settings.endpoint).map_err(err)?;
    if settings.screen_recognition {
        return Err(
            "Screen recognition is not verified yet. No screen capture has been enabled.".into(),
        );
    }
    use tauri_plugin_autostart::ManagerExt;
    if settings.autostart && profile::current().is_some() {
        return Err("Start with Windows is unavailable in a test profile".into());
    }
    if settings.autostart != core.settings.lock().unwrap().autostart {
        if settings.autostart {
            core.app.autolaunch().enable().map_err(|e| e.to_string())?;
        } else {
            core.app.autolaunch().disable().map_err(|e| e.to_string())?;
        }
    }
    coordination::save(&core.data.join(coordination::FILE_NAME), &settings.endpoint)
        .map_err(err)?;
    core.db.set("settings", &settings).map_err(err)?;
    *core.settings.lock().unwrap() = settings;
    core.changed();
    Ok(())
}
#[tauri::command]
async fn leave_project(core: tauri::State<'_, Core>) -> Cmd<()> {
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), core.finish()).await;
    let _guard = core.operation.lock().await;
    // leave_practice reads its return target before calling this, so clearing it here is safe.
    core.db.remove("practice-return").map_err(err)?;
    core.db.remove("active").map_err(err)?;
    *core.active.lock().unwrap() = None;
    *core.watcher.lock().unwrap() = None;
    core.leases.lock().unwrap().clear();
    core.lease_activity.lock().unwrap().clear();
    core.online.store(false, Ordering::Relaxed);
    core.changed();
    Ok(())
}
#[tauri::command]
fn dismiss_invitation(core: tauri::State<Core>) -> Cmd<()> {
    core.db.remove("incoming-invitation").map_err(err)
}
#[tauri::command]
async fn history_page(
    core: tauri::State<'_, Core>,
    filter: db::HistoryFilter,
) -> Cmd<miliastra_core::HistoryPage> {
    let p = core.active().map_err(err)?;
    // Background sync keeps the cache fresh; browsing History only reads it, so it works offline.
    core.db.history_page(&p.project.id, &filter).map_err(err)
}
#[tauri::command]
fn pending_invitation(core: tauri::State<Core>) -> Cmd<Option<miliastra_core::Invitation>> {
    core.db.get("incoming-invitation").map_err(err)
}
#[tauri::command]
fn inspect_stage(path: PathBuf) -> Cmd<miliastra_core::binary::Stage> {
    miliastra_core::binary::inspect(&path).map_err(err)
}
#[tauri::command]
async fn open_storage(core: tauri::State<'_, Core>) -> Cmd<()> {
    let p = core.active().map_err(err)?;
    if let Some(url) = core.storage(&p).folder_url().await.map_err(err)? {
        webbrowser::open(&url).map_err(|e| e.to_string())?;
        Ok(())
    } else {
        reveal(p.folder.as_deref().ok_or("Choose a shared folder")?)
    }
}
#[tauri::command]
async fn check_application_update(
    core: tauri::State<'_, Core>,
) -> Cmd<Option<portable::AppUpdate>> {
    portable::check(&core).await.map_err(err)
}
#[tauri::command]
async fn install_application_update(core: tauri::State<'_, Core>) -> Cmd<()> {
    portable::update_and_restart(&core).await.map_err(err)
}
#[tauri::command]
async fn download_application_update(core: tauri::State<'_, Core>) -> Cmd<Option<String>> {
    portable::download_copy(&core).await.map_err(err)
}

#[derive(serde::Serialize)]
struct SavedConnection {
    id: String,
    name: String,
}
#[tauri::command]
fn connections(core: tauri::State<Core>) -> Cmd<Vec<SavedConnection>> {
    Ok(core
        .db
        .get::<Vec<runtime::LocalProject>>("connections")
        .map_err(err)?
        .unwrap_or_default()
        .into_iter()
        .map(|p| SavedConnection {
            id: p.project.id,
            name: p.project.name,
        })
        .collect())
}
#[tauri::command]
async fn reopen_project(core: tauri::State<'_, Core>, project_id: String) -> Cmd<()> {
    core.reopen(&project_id).await.map_err(err)
}

fn reveal(path: &std::path::Path) -> Cmd<()> {
    #[cfg(debug_assertions)]
    if std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new("explorer.exe");
        if path.is_dir() {
            c.arg(path);
        } else {
            c.arg(format!("/select,{}", path.display()));
        }
        c.spawn().map_err(|e| e.to_string())?;
    }
    Ok(())
}
/// Shows notices that were held back during exclusive fullscreen once Miliastra loses
/// focus, with a one-time note explaining why.
fn deliver_held_notices(app: &tauri::AppHandle) {
    let c = app.state::<Core>();
    if c.held_notices.lock().unwrap().is_empty() || native::game_state().exclusive_fullscreen {
        return;
    }
    let held = std::mem::take(&mut *c.held_notices.lock().unwrap());
    let latest = held.last().cloned().unwrap_or_default();
    let mut text = if held.len() == 1 {
        latest
    } else {
        format!(
            "{} notifications while Miliastra was fullscreen. Latest: {latest}",
            held.len()
        )
    };
    if !c.fullscreen_warned.swap(true, Ordering::Relaxed) {
        text += " Notifications can’t appear over Miliastra in exclusive fullscreen; use borderless or windowed mode to see them as they happen.";
    }
    show_notice(app, &text);
}
pub fn show_notice(app: &tauri::AppHandle, text: &str) {
    #[cfg(debug_assertions)]
    if std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()
        && std::env::var_os("MCOLLAB_TEST_OVERLAY").is_none()
    {
        return;
    }
    // Nothing can draw over exclusive fullscreen, so hold the notice (see deliver_held_notices).
    if native::game_state().exclusive_fullscreen {
        app.state::<Core>()
            .held_notices
            .lock()
            .unwrap()
            .push(text.into());
        return;
    }
    let shown = (|| -> Option<()> {
        let w = app.get_webview_window("overlay")?;
        let point = native::foreground_center();
        let monitor = w
            .available_monitors()
            .ok()
            .and_then(|monitors| {
                monitors.into_iter().find(|m| {
                    point.is_some_and(|(x, y)| {
                        let p = m.position();
                        let s = m.size();
                        x >= p.x
                            && y >= p.y
                            && x < p.x + s.width as i32
                            && y < p.y + s.height as i32
                    })
                })
            })
            .or_else(|| w.primary_monitor().ok().flatten())?;
        let area = monitor.work_area();
        let size = tauri::PhysicalSize::new(
            (420.0 * monitor.scale_factor()).round() as u32,
            (170.0 * monitor.scale_factor()).round() as u32,
        );
        w.set_size(size).ok()?;
        let (x, y) = native::notification_position(
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
            size.width,
            size.height,
            monitor.scale_factor(),
        );
        w.set_position(tauri::PhysicalPosition::new(x, y)).ok()?;
        w.emit("overlay-message", text).ok()?;
        w.show().ok()?;
        let sequence = app
            .state::<Core>()
            .notice_sequence
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(7)).await;
            if app.state::<Core>().notice_sequence.load(Ordering::Relaxed) == sequence {
                if let Some(w) = app.get_webview_window("overlay") {
                    let _ = w.hide();
                }
            }
        });
        Some(())
    })()
    .is_some();
    if shown {
        return;
    }
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("miliastra/collab")
        .body(text)
        .show();
}
fn start_background(app: tauri::AppHandle) {
    use coordinator_sync::{Refresh, KEEPALIVE, RECONNECT_DELAY, SOCKET_CHECK};
    use tokio::time::MissedTickBehavior;
    let sync_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let c = sync_app.state::<Core>();
        loop {
            let work = c.coordinator_sync.next().await;
            {
                let _guard = c.operation.lock().await;
                if !c
                    .active
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|p| p.sync_session() == work.session)
                {
                    continue;
                }
                if work.refresh != Refresh::None
                    && c.sync(work.refresh == Refresh::Full).await.is_err()
                {
                    c.online.store(false, Ordering::Relaxed);
                    c.changed();
                }
            }
            if work.grant_access
                && c.active
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|p| p.sync_session() == work.session)
            {
                let _ = c.grant_pending_access().await;
            }
        }
    });
    let heartbeat_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            miliastra_core::constants::LEASE_RENEW_INTERVAL_SECS,
        ));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let c = heartbeat_app.state::<Core>();
            if c.active.lock().unwrap().is_some() {
                if c.heartbeat().await.is_err() {
                    c.online.store(false, Ordering::Relaxed);
                    c.changed();
                }
            }
        }
    });
    let download_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let c = download_app.state::<Core>();
            if c.active.lock().unwrap().is_some() {
                if c.resume_install_verification().await.is_err() {
                    c.changed();
                }
                let _ = c.download_pending().await;
                if c.auto_apply().await.is_err() {
                    c.changed();
                }
            }
        }
    });
    let polling_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut counter = 0u64;
        loop {
            tick.tick().await;
            counter += 1;
            let c = polling_app.state::<Core>();
            deliver_held_notices(&polling_app);
            if counter % 3 == 0 {
                tray_status::refresh(&polling_app);
            }
            if c.active.lock().unwrap().is_none() {
                continue;
            }
            if c.dirty.swap(false, Ordering::Relaxed) {
                if let Err(e) = c.saved_detection().await {
                    *c.message.lock().unwrap() = format!("Waiting for a complete save: {e}");
                    c.dirty.store(true, Ordering::Relaxed);
                }
            }

            let observed = c.active.lock().unwrap().as_ref().map(|p| {
                // Renewals are not broadcast by the service. Recheck teammates' locks
                // before their cached expiry rather than showing an occupied slice as free.
                let now = chrono::Utc::now().timestamp();
                let reservation_due = p.project.locks.iter().any(|l| {
                    l.member != p.member && coordinator_sync::reservation_due(l.expires, now)
                });
                (p.sync_session(), reservation_due)
            });
            if let Some((session, reservation_due)) = observed {
                if c.coordinator_sync.due(
                    &session,
                    c.online.load(Ordering::Relaxed),
                    reservation_due,
                    std::time::Instant::now(),
                ) {
                    c.coordinator_sync
                        .request(session, Refresh::Incremental, true);
                }
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        loop {
            let c = app.state::<Core>();
            let active = c.active.lock().unwrap().clone();
            if let Some(p) = active {
                if let Ok(secret) =
                    oauth::secret_get(&format!("membership:{}:{}", p.endpoint, p.project.id))
                {
                    let url = format!(
                        "{}/projects/{}/events",
                        p.endpoint
                            .replace("https://", "wss://")
                            .replace("http://", "ws://"),
                        p.project.id
                    );
                    if let Ok(mut req) = url.into_client_request() {
                        if let Ok(value) = format!("Bearer {secret}").parse() {
                            req.headers_mut().insert("Authorization", value);
                            if let Ok((mut socket, _)) = tokio_tungstenite::connect_async(req).await
                            {
                                let session = p.sync_session();
                                c.coordinator_sync
                                    .received(session.clone(), std::time::Instant::now());
                                // Keep reading and pinging the socket while reconciliation or
                                // storage access runs in the worker. Reconnect always catches up.
                                c.coordinator_sync.request(
                                    session.clone(),
                                    Refresh::Incremental,
                                    true,
                                );
                                // Presence: send our encrypted "in Miliastra" status whenever it changes.
                                let mut reported_game = None;
                                // Cloudflare drops connections that are quiet for about 100 seconds.
                                let mut last_ping = std::time::Instant::now();
                                let mut socket_tick = tokio::time::interval(SOCKET_CHECK);
                                socket_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
                                let game = native::game_state().running;
                                if let Ok(text) = c.presence_message(&p, game) {
                                    if socket
                                        .send(tokio_tungstenite::tungstenite::Message::Text(
                                            text.into(),
                                        ))
                                        .await
                                        .is_ok()
                                    {
                                        reported_game = Some(game);
                                    }
                                }
                                loop {
                                    tokio::select! {
                                        message = socket.next() => match message {
                                            Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None | Some(Err(_)) => break,
                                            Some(Ok(msg)) => {
                                                c.coordinator_sync.received(session.clone(), std::time::Instant::now());
                                                if let Ok(text) = msg.to_text() {
                                                    if let Ok(event) = serde_json::from_str(text) {
                                                        c.handle_event(&p, event);
                                                    }
                                                }
                                            }
                                        },
                                        _ = socket_tick.tick() => {
                                            if !c.coordinator_sync.socket_alive(&session, std::time::Instant::now()) {
                                                break;
                                            }
                                            if last_ping.elapsed() >= KEEPALIVE {
                                                if socket
                                                    .send(tokio_tungstenite::tungstenite::Message::Ping(Vec::new().into()))
                                                    .await
                                                    .is_err()
                                                {
                                                    break;
                                                }
                                                last_ping = std::time::Instant::now();
                                            }
                                            let game = native::game_state().running;
                                            if reported_game != Some(game) {
                                                if let Ok(text) = c.presence_message(&p, game) {
                                                    if socket
                                                        .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                                                        .await
                                                        .is_ok()
                                                    {
                                                        reported_game = Some(game);
                                                    }
                                                }
                                            }
                                            if !c
                                                .active
                                                .lock()
                                                .unwrap()
                                                .as_ref()
                                                .is_some_and(|a| a.sync_session() == session)
                                            {
                                                break;
                                            }
                                        }
                                    }
                                }
                                c.coordinator_sync.disconnected(&session);
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
}
pub fn run() {
    #[cfg(debug_assertions)]
    reliability::install_panic_hook();
    if let Some(code) = portable::helper_entry() {
        std::process::exit(code);
    }
    if let Err(e) = profile::init() {
        eprintln!("{e}");
        std::process::exit(2);
    }
    let mut builder = tauri::Builder::default();
    // Test profiles run beside the normal instance, so they skip the single-instance lock.
    if profile::current().is_none()
        && !(cfg!(debug_assertions) && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some())
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if !(cfg!(debug_assertions) && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        }));
    }
    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(include!(concat!(env!("OUT_DIR"), "/command-handler.rs")))
        .setup(|app| {
            // Test-only commands (see build.rs) need their own permission entry.
            #[cfg(test_build)]
            {
                const TEST_COMMANDS: &[&str] = include!("../commands.test.generated.rs");
                let mut capability = tauri::ipc::CapabilityBuilder::new("test-build").window("main");
                for name in TEST_COMMANDS {
                    capability = capability.permission(format!("allow-{}", name.replace('_', "-")));
                }
                app.add_capability(capability)?;
            }
            if cfg!(debug_assertions) && (std::env::args().any(|a| a == "--headless-smoke") || std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()) {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            let core = Core::new(app.handle().clone())?;
            app.manage(core);
            if let (Some(name), Some(w)) = (profile::current(), app.get_webview_window("main")) {
                let _ = w.set_title(&format!("miliastra/collab — test profile {name}"));
            }
            // Invitation links belong to the normal instance; test profiles paste them.
            #[cfg(windows)]
            if profile::current().is_none() && !(cfg!(debug_assertions) && std::env::var_os("MCOLLAB_TEST_DATA_DIR").is_some()) {
                if let Err(error) = app.deep_link().register_all() {
                    let _ = app.emit("notice", format!("Invitation links could not be registered. Restart the app, then open the invitation link again: {error}"));
                }
            }
            let incoming = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() { invitation::open_protocol(&incoming, url.as_str()); }
            });
            let overlay_builder = tauri::WebviewWindowBuilder::new(
                app,
                "overlay",
                tauri::WebviewUrl::App("index.html?overlay=1".into()),
            )
            .title("Collaboration notification")
            .inner_size(420.0, 170.0)
            .transparent(true)
            .background_color(tauri::window::Color(0, 0, 0, 0))
            .shadow(false)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .focusable(false)
            .visible(false);
            let overlay = overlay_builder.build()?;
            overlay.set_ignore_cursor_events(true)?;
            let open =
                MenuItem::with_id(app, "open", "Open miliastra/collab", true, None::<&str>)?;
            let quit = MenuItem::with_id(
                app,
                "quit",
                "Quit",
                true,
                None::<&str>,
            )?;
            let apply = MenuItem::with_id(app,"apply","Review and apply updates",true,None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &apply, &quit])?;
            let mut tray =
                TrayIconBuilder::with_id(tray_status::TRAY_ID)
                    .menu(&menu)
                    .on_menu_event(|app, event| match event.id().as_ref() {
                        "open" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "apply" => {
                            if let Some(w)=app.get_webview_window("main") { let _=w.show();let _=w.set_focus(); }
                            let _=app.emit("review-and-apply",());
                        }
                        "quit" => {
                            let a = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let c = a.state::<Core>();
                                if c.active.lock().unwrap().is_some() {
                                    let _ = tokio::time::timeout(
                                        std::time::Duration::from_secs(3),
                                        c.finish(),
                                    )
                                    .await;
                                }
                                a.exit(0);
                            });
                        }
                        _ => {}
                    });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            let c = app.state::<Core>();
            if let Ok(p) = c.active() {
                *c.observed.lock().unwrap() = Some(
                    miliastra_core::binary::inspect(&p.stage.path)
                        .map(|stage| stage.settings_version)
                        .unwrap_or(p.stage.settings_version.clone()),
                );
                let _ = c.watch();
            }
            if let Some(message) = portable::startup(app.handle(), &c.data) {
                *c.message.lock().unwrap() = message;
                c.changed();
            }
            start_background(app.handle().clone());
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls { invitation::open_protocol(app.handle(), url.as_str()); }
            }
            Ok(())
        })
        .on_window_event(|w, event| {
            if w.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Unable to start miliastra/collab");
}
