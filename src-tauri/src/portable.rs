//! App self-update: download the signed new exe and swap it in. No installer.
use crate::runtime::Core;
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use miliastra_core::hash;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_updater::UpdaterExt;
static UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MAX_BYTES: usize = 256 * 1024 * 1024;
const HELPER_ARG: &str = "--apply-portable-update";
const STARTED_ARG: &str = "--portable-update-complete";
const RESULT_ARG: &str = "--portable-update-result";
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    version: String,
    notes: String,
    can_replace: bool,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    message: String,
    percent: Option<u64>,
}
fn progress(core: &Core, message: &str, percent: Option<u64>) {
    let _ = core.app.emit(
        "application-update-progress",
        Progress {
            message: message.into(),
            percent,
        },
    );
}
fn key() -> Result<&'static str> {
    option_env!("MCOLLAB_UPDATE_PUBKEY")
        .filter(|s| !s.trim().is_empty())
        .context("App updates are not configured for this build.")
}
async fn available(core: &Core) -> Result<Option<tauri_plugin_updater::Update>> {
    let endpoint = option_env!("MCOLLAB_UPDATE_URL")
        .filter(|s| !s.trim().is_empty())
        .context("App updates are not configured for this build.")?;
    let url = url::Url::parse(endpoint)?;
    ensure!(url.scheme() == "https", "Application updates require HTTPS");
    let update = core
        .app
        .updater_builder()
        .pubkey(key()?)
        .endpoints(vec![url])?
        .timeout(Duration::from_secs(20))
        .build()?
        .check()
        .await?;
    if let Some(u) = &update {
        ensure!(
            u.raw_json.get("format").and_then(|v| v.as_str()) == Some("portable-exe"),
            "This release is not a portable app update"
        );
        ensure!(
            u.download_url.scheme() == "https",
            "Application downloads require HTTPS"
        );
        newer_version(&u.version, &u.current_version)?;
    }
    Ok(update)
}
fn newer_version(new: &str, current: &str) -> Result<()> {
    ensure!(
        semver::Version::parse(new.trim_start_matches('v'))?
            > semver::Version::parse(current.trim_start_matches('v'))?,
        "Only a newer app version can be applied"
    );
    Ok(())
}
pub async fn check(core: &Core) -> Result<Option<AppUpdate>> {
    Ok(available(core).await?.map(|u| AppUpdate {
        version: u.version,
        notes: u.body.unwrap_or_default(),
        can_replace: std::env::current_exe().is_ok_and(|p| writable(&p)),
    }))
}
fn writable(exe: &Path) -> bool {
    let Some(parent) = exe.parent() else {
        return false;
    };
    if fs::metadata(exe).map_or(true, |m| m.permissions().readonly()) {
        return false;
    }
    let probe = parent.join(format!(".miliastra-write-{}.tmp", uuid::Uuid::new_v4()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            fs::remove_file(probe).is_ok()
        }
        Err(_) => false,
    }
}
fn durable(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(temp);
        return Err(error.into());
    }
    Ok(())
}
fn pe_x64(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() > 64 && &bytes[..2] == b"MZ",
        "The update is not a portable Windows application"
    );
    let offset = u32::from_le_bytes(bytes[60..64].try_into()?) as usize;
    let header = bytes
        .get(offset..offset.saturating_add(26))
        .context("Invalid application header")?;
    ensure!(
        &header[..4] == b"PE\0\0"
            && u16::from_le_bytes(header[4..6].try_into()?) == 0x8664
            && u16::from_le_bytes(header[24..26].try_into()?) == 0x20b,
        "The update must be a Windows x64 application"
    );
    Ok(())
}
fn signed(bytes: &[u8], signature: &str, public_key: &str, version: &str) -> Result<()> {
    let pk = String::from_utf8(STANDARD.decode(public_key.trim())?)?;
    let sig = String::from_utf8(STANDARD.decode(signature.trim())?)?;
    let pk = minisign_verify::PublicKey::decode(&pk)?;
    let sig = minisign_verify::Signature::decode(&sig)?;
    pk.verify(bytes, &sig, true)
        .context("The update signature is invalid")?;
    let signed_version = sig
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .context("The signature has no app version")?;
    ensure!(
        semver::Version::parse(signed_version.trim_start_matches('v'))?
            == semver::Version::parse(version.trim_start_matches('v'))?,
        "The signed version does not match the release"
    );
    pe_x64(bytes)
}
async fn download(core: &Core, update: &tauri_plugin_updater::Update) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(180))
        .build()?;
    progress(core, "Downloading the new app…", Some(0));
    let response = client
        .get(update.download_url.clone())
        .send()
        .await?
        .error_for_status()?;
    let total = response.content_length();
    ensure!(
        total.is_none_or(|n| n <= MAX_BYTES as u64),
        "The app download is too large"
    );
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_BYTES,
            "The app download is too large"
        );
        bytes.extend_from_slice(&chunk);
        progress(
            core,
            "Downloading the new app…",
            total
                .filter(|n| *n > 0)
                .map(|n| ((bytes.len() as u64 * 100) / n).min(100)),
        );
    }
    progress(core, "Verifying the download…", None);
    update.verify(&bytes)?;
    signed(&bytes, &update.signature, key()?, &update.version)?;
    Ok(bytes)
}
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    target: PathBuf,
    original_hash: String,
    new_hash: String,
    version: String,
    signature: String,
    parent: u32,
    args: Vec<String>,
    phase: String,
    error: Option<String>,
}
fn save(root: &Path, plan: &Plan) -> Result<()> {
    durable(&root.join("plan.json"), &serde_json::to_vec_pretty(plan)?)
}
fn read_plan(path: &Path) -> Result<(PathBuf, Plan)> {
    ensure!(
        path.file_name().is_some_and(|n| n == "plan.json"),
        "Invalid update journal"
    );
    let root = path
        .parent()
        .context("Missing update folder")?
        .canonicalize()?;
    uuid::Uuid::parse_str(
        root.file_name()
            .and_then(|s| s.to_str())
            .context("Invalid update folder")?,
    )?;
    let bytes = fs::read(root.join("plan.json"))?;
    ensure!(bytes.len() < 65536, "Invalid update journal");
    let plan: Plan = serde_json::from_slice(&bytes)?;
    ensure!(
        plan.target.is_absolute()
            && plan
                .target
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe")),
        "Invalid application path"
    );
    ensure!(
        plan.args.is_empty()
            || plan.args == ["--headless-smoke"]
            || plan.args.len() == 2
                && plan.args[0] == "--profile"
                && crate::profile::ALLOWED
                && plan.args[1].len() <= 32
                && !plan.args[1].is_empty()
                && plan.args[1]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Invalid relaunch arguments"
    );
    ensure!(
        plan.target
            .parent()
            .context("Invalid application folder")?
            .canonicalize()?
            != root,
        "The application and helper must be separate"
    );
    Ok((root, plan))
}
fn sibling(root: &Path, plan: &Plan, suffix: &str) -> Result<PathBuf> {
    Ok(plan
        .target
        .parent()
        .context("Missing app folder")?
        .join(format!(
            ".miliastra-{}.{suffix}",
            root.file_name().unwrap().to_string_lossy()
        )))
}
fn prepare(
    data: &Path,
    target: &Path,
    bytes: &[u8],
    signature: &str,
    version: &str,
    args: Vec<String>,
) -> Result<PathBuf> {
    signed(bytes, signature, key()?, version)?;
    newer_version(version, env!("CARGO_PKG_VERSION"))?;
    ensure!(
        writable(target),
        "This app folder is read-only. Use Download updated app instead."
    );
    let target = target.canonicalize()?;
    let root = data
        .join("app-updates")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&root)?;
    durable(&root.join("update.exe"), bytes)?;
    // The helper is a copy of this exe that exits before Tauri starts.
    fs::copy(&target, root.join("helper.exe"))?;
    save(
        &root,
        &Plan {
            original_hash: hash(&fs::read(&target)?),
            target,
            new_hash: hash(bytes),
            version: version.into(),
            signature: signature.into(),
            parent: std::process::id(),
            args,
            phase: "prepared".into(),
            error: None,
        },
    )?;
    Ok(root)
}
fn hidden(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}
fn launch_helper(root: &Path) -> Result<()> {
    let mut child = hidden(
        Command::new(root.join("helper.exe"))
            .arg(HELPER_ARG)
            .arg(root.join("plan.json")),
    )
    .spawn()?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if root.join("ready").exists() {
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            let error = read_plan(&root.join("plan.json"))?
                .1
                .error
                .unwrap_or_else(|| "The update helper could not start".into());
            anyhow::bail!("{error}. Use Download updated app instead.");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!("The update helper did not become ready. Your app is unchanged.")
}
pub async fn update_and_restart(core: &Core) -> Result<()> {
    let _lock = UPDATE_LOCK
        .try_lock()
        .context("An app update is already in progress")?;
    let update = available(core)
        .await?
        .context("No app update is available")?;
    let target = std::env::current_exe()?;
    ensure!(
        writable(&target),
        "This app folder is read-only. Use Download updated app instead."
    );
    let bytes = download(core, &update).await?;
    let args = match crate::profile::current() {
        Some(name) => vec!["--profile".into(), name.into()],
        None => {
            if cfg!(debug_assertions) && std::env::args().any(|a| a == "--headless-smoke") {
                vec!["--headless-smoke".into()]
            } else {
                vec![]
            }
        }
    };
    let data = core.data.clone();
    let signature = update.signature;
    let version = update.version;
    let root = tauri::async_runtime::spawn_blocking(move || {
        prepare(&data, &target, &bytes, &signature, &version, args)
    })
    .await??;
    if core.active.lock().unwrap().is_some() {
        // Drafts are saved locally, so an offline coordinator only delays the release.
        let _ = tokio::time::timeout(Duration::from_secs(4), core.finish()).await;
    }
    // Wait for any share or stage-file change to finish.
    let _operation = core.operation.lock().await;
    progress(core, "Restarting with the new version…", None);
    tauri::async_runtime::spawn_blocking(move || launch_helper(&root)).await??;
    core.app.exit(0);
    Ok(())
}
pub async fn download_copy(core: &Core) -> Result<Option<String>> {
    let _lock = UPDATE_LOCK
        .try_lock()
        .context("An app update is already in progress")?;
    let update = available(core)
        .await?
        .context("No app update is available")?;
    let app = core.app.clone();
    let destination = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Save updated app")
            .set_file_name("miliastra-collab.exe")
            .add_filter("Windows application", &["exe"])
            .blocking_save_file()
    })
    .await?;
    let Some(destination) = destination else {
        return Ok(None);
    };
    let destination = destination.into_path()?;
    let current = std::env::current_exe()?.canonicalize()?;
    ensure!(
        destination.canonicalize().ok().as_ref() != Some(&current),
        "Save the new copy in another folder, then close this app and open it."
    );
    let bytes = download(core, &update).await?;
    let displayed = destination.to_string_lossy().to_string();
    tauri::async_runtime::spawn_blocking(move || durable(&destination, &bytes)).await??;
    progress(
        core,
        "Downloaded. Close this app and open the new copy.",
        None,
    );
    Ok(Some(displayed))
}
#[cfg(windows)]
struct Parent(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Parent {
    fn open(id: u32, target: &Path) -> Result<Self> {
        use windows::{core::PWSTR, Win32::System::Threading::*};
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                id,
            )
        }?;
        let parent = Self(handle);
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }?;
        let image = PathBuf::from(String::from_utf16(&buffer[..length as usize])?);
        ensure!(
            image.canonicalize()? == target.canonicalize()?,
            "The original app process does not match"
        );
        Ok(parent)
    }
    fn wait(&self) -> Result<()> {
        ensure!(
            unsafe { windows::Win32::System::Threading::WaitForSingleObject(self.0, 45000) }
                == windows::Win32::Foundation::WAIT_OBJECT_0,
            "The app did not close; the update was cancelled"
        );
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for Parent {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}
#[cfg(windows)]
fn replace(target: &Path, candidate: &Path, backup: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS},
    };
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let t = wide(target);
    let c = wide(candidate);
    let b = wide(backup);
    unsafe {
        ReplaceFileW(
            PCWSTR(t.as_ptr()),
            PCWSTR(c.as_ptr()),
            PCWSTR(b.as_ptr()),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    }?;
    Ok(())
}
#[cfg(not(windows))]
fn replace(_: &Path, _: &Path, _: &Path) -> Result<()> {
    anyhow::bail!("Portable updates require Windows")
}
fn relaunch(plan: &Plan, flag: &str, path: &Path) -> Result<Child> {
    hidden(
        Command::new(&plan.target)
            .args(&plan.args)
            .arg(flag)
            .arg(path),
    )
    .spawn()
    .map_err(Into::into)
}
#[cfg(windows)]
fn apply(root: &Path, plan: &mut Plan) -> Result<()> {
    let current = std::env::current_exe()?.canonicalize()?;
    ensure!(
        current == root.join("helper.exe").canonicalize()?,
        "Invalid update helper location"
    );
    ensure!(
        plan.phase == "prepared",
        "This update has already been processed"
    );
    newer_version(&plan.version, env!("CARGO_PKG_VERSION"))?;
    let bytes = fs::read(root.join("update.exe"))?;
    ensure!(
        bytes.len() <= MAX_BYTES && hash(&bytes) == plan.new_hash,
        "The staged app changed"
    );
    signed(&bytes, &plan.signature, key()?, &plan.version)?;
    ensure!(
        hash(&fs::read(&plan.target)?) == plan.original_hash,
        "The original app changed"
    );
    let parent = Parent::open(plan.parent, &plan.target)?;
    let candidate = sibling(root, plan, "new.exe")?;
    ensure!(!candidate.exists(), "An update replacement already exists");
    durable(&candidate, &bytes)?;
    plan.phase = "ready".into();
    save(root, plan)?;
    durable(&root.join("ready"), b"ready")?;
    parent.wait()?;
    ensure!(
        hash(&fs::read(&plan.target)?) == plan.original_hash,
        "The original app changed while closing"
    );
    let candidate_bytes = fs::read(&candidate)?;
    ensure!(
        hash(&candidate_bytes) == plan.new_hash,
        "The replacement changed"
    );
    signed(&candidate_bytes, &plan.signature, key()?, &plan.version)?;
    plan.phase = "replacing".into();
    save(root, plan)?;
    let backup = sibling(root, plan, "previous.exe")?;
    ensure!(!backup.exists(), "An update backup already exists");
    let mut last_error = None;
    for _ in 0..50 {
        match replace(&plan.target, &candidate, &backup) {
            Ok(()) => {
                last_error = None;
                break;
            }
            Err(e) => {
                last_error = Some(e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    if let Some(e) = last_error {
        return Err(e.context("Windows could not replace the app"));
    }
    plan.phase = "restarting".into();
    save(root, plan)?;
    let mut child = relaunch(plan, STARTED_ARG, &root.join("plan.json"))?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if root.join("started").exists() {
            plan.phase = "complete".into();
            save(root, plan)?;
            return Ok(());
        }
        if child.try_wait()?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!("The new app did not start")
}
fn recover(root: &Path, plan: &mut Plan, error: &anyhow::Error) -> Result<()> {
    let backup = sibling(root, plan, "previous.exe")?;
    let current = fs::read(&plan.target).ok().map(|v| hash(&v));
    let after_close = matches!(plan.phase.as_str(), "replacing" | "restarting");
    if after_close
        && backup.exists()
        && (current.is_none() || current.as_deref() == Some(plan.new_hash.as_str()))
    {
        ensure!(
            hash(&fs::read(&backup)?) == plan.original_hash,
            "The recovery copy changed"
        );
        let failed = sibling(root, plan, "failed.exe")?;
        if plan.target.exists() {
            replace(&plan.target, &backup, &failed)?
        } else {
            fs::rename(&backup, &plan.target)?
        }
    }
    ensure!(
        hash(&fs::read(&plan.target)?) == plan.original_hash,
        "Automatic recovery is unavailable; open the previous app copy"
    );
    plan.phase = "failed".into();
    plan.error = Some(format!("{error:#}"));
    save(root, plan)?;
    if after_close {
        let _ = relaunch(plan, RESULT_ARG, &root.join("plan.json"));
    }
    Ok(())
}
pub fn helper_entry() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_none_or(|a| a != HELPER_ARG) {
        return None;
    }
    let result = (|| -> Result<()> {
        ensure!(args.len() == 3, "Invalid helper arguments");
        let (root, mut plan) = read_plan(Path::new(&args[2]))?;
        ensure!(
            plan.phase == "prepared"
                && !root.join("ready").exists()
                && !root.join("started").exists(),
            "This update has already been processed"
        );
        #[cfg(windows)]
        if let Err(error) = apply(&root, &mut plan) {
            if recover(&root, &mut plan, &error).is_err() {
                plan.phase = "failed".into();
                plan.error = Some(format!("{error:#}. Open the retained previous app copy."));
                let _ = save(&root, &plan);
            }
            return Err(error);
        }
        #[cfg(not(windows))]
        anyhow::bail!("Portable updates require Windows");
        Ok(())
    })();
    Some(if let Err(error) = result {
        eprintln!("{error:#}");
        2
    } else {
        0
    })
}
pub fn startup(app: &tauri::AppHandle, data: &Path) -> Option<String> {
    let args: Vec<_> = std::env::args_os().collect();
    let index = args
        .iter()
        .position(|a| a == STARTED_ARG || a == RESULT_ARG)?;
    let path = PathBuf::from(args.get(index + 1)?);
    let (root, plan) = read_plan(&path).ok()?;
    if !root.starts_with(data.join("app-updates").canonicalize().ok()?) {
        return None;
    }
    if plan.target.canonicalize().ok()? != std::env::current_exe().ok()?.canonicalize().ok()? {
        return None;
    }
    if args[index] == STARTED_ARG {
        if plan.phase != "restarting"
            || plan.version.trim_start_matches('v') != app.package_info().version.to_string()
            || hash(&fs::read(&plan.target).ok()?) != plan.new_hash
        {
            return None;
        }
        durable(&root.join("started"), b"started").ok()?;
        Some(format!("Updated to version {}.", plan.version))
    } else {
        Some(format!(
            "The app update could not finish. {} Your projects and saved work are unchanged.",
            plan.error
                .as_deref()
                .unwrap_or("The previous version is still available.")
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_newer_semver_and_portable_x64_images_are_accepted() {
        assert!(newer_version("0.2.0", "0.1.33").is_ok());
        for v in ["0.1.33", "0.1.32", "invalid"] {
            assert!(newer_version(v, "0.1.33").is_err());
        }
        for b in [vec![], b"installer zip".to_vec(), vec![0; 120]] {
            assert!(pe_x64(&b).is_err());
        }
        let mut b = vec![0; 128];
        b[..2].copy_from_slice(b"MZ");
        b[60] = 64;
        b[64..68].copy_from_slice(b"PE\0\0");
        b[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        b[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        assert!(pe_x64(&b).is_ok());
        b[68..70].copy_from_slice(&0x14cu16.to_le_bytes());
        assert!(pe_x64(&b).is_err());
    }
    #[test]
    fn writable_probe_preserves_the_app_and_durable_writes_replace_only_the_selected_file() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&root).unwrap();
        let target = root.join("app.exe");
        fs::write(&target, b"old app").unwrap();
        assert!(writable(&target));
        assert_eq!(fs::read(&target).unwrap(), b"old app");
        let data = root.join("plan.json");
        durable(&data, b"one").unwrap();
        durable(&data, b"two").unwrap();
        assert_eq!(fs::read(&data).unwrap(), b"two");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_file(data).unwrap();
        fs::remove_file(target).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn recovery_restores_a_missing_target_but_preserves_an_unexpected_replacement() {
        let parent = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = parent.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let target = parent.join("app.exe");
        let mut plan = Plan {
            target: target.clone(),
            original_hash: hash(b"original"),
            new_hash: hash(b"new"),
            version: "0.2.0".into(),
            signature: String::new(),
            parent: 0,
            args: vec![],
            phase: "replacing".into(),
            error: None,
        };
        let backup = sibling(&root, &plan, "previous.exe").unwrap();
        fs::write(&backup, b"original").unwrap();
        fs::write(&target, b"another app version").unwrap();
        assert!(recover(&root, &mut plan, &anyhow::anyhow!("interrupted")).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"another app version");
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        fs::remove_file(&target).unwrap();
        recover(&root, &mut plan, &anyhow::anyhow!("interrupted")).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(plan.phase, "failed");
        fs::remove_file(target).unwrap();
        fs::remove_file(root.join("plan.json")).unwrap();
        fs::remove_dir(root).unwrap();
        fs::remove_dir(parent).unwrap();
    }
    #[test]
    fn invalid_signatures_never_accept_an_executable() {
        assert!(signed(b"MZ", "bad", "bad", "0.2.0").is_err());
    }
}
