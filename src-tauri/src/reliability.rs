//! Fault injection for the reliability test runs. Debug builds only, and only inside a
//! marked test folder.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn confined(run: &Path, profile: &Path) -> Result<PathBuf> {
    let run = run.canonicalize()?;
    let profile = profile.canonicalize()?;
    ensure!(
        run.join("reliability-run.json").is_file(),
        "Reliability run marker missing"
    );
    ensure!(
        profile != run && profile.starts_with(&run),
        "Reliability profile escapes run"
    );
    Ok(profile)
}
fn profile() -> Result<Option<PathBuf>> {
    let Some(run) = std::env::var_os("MCOLLAB_RELIABILITY_ROOT") else {
        return Ok(None);
    };
    let profile = std::env::var_os("MCOLLAB_TEST_DATA_DIR")
        .context("Reliability needs an isolated test profile")?;
    Ok(Some(confined(Path::new(&run), Path::new(&profile))?))
}
/// Pretend the editor is running (reliability runs only).
pub fn editor_running() -> bool {
    profile()
        .ok()
        .flatten()
        .is_some_and(|root| root.join("editor-running.json").is_file())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fault {
    id: String,
    point: String,
    reference: String,
    action: String,
}
fn matches_reference(pattern: &str, reference: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return !suffix.contains('*')
            && reference.starts_with(prefix)
            && reference.ends_with(suffix);
    }
    reference.ends_with(pattern)
}
/// Async version of `checkpoint`, so a paused transfer doesn't block the thread.
pub async fn transfer_checkpoint(point: &str, reference: &str) -> Result<bool> {
    let point = point.to_owned();
    let reference = reference.to_owned();
    tokio::task::spawn_blocking(move || checkpoint(&point, &reference)).await?
}
/// Returns true when the test asked for a corrupt read. Pauses time out, so a crashed
/// test runner can't leave the app stuck.
pub fn checkpoint(point: &str, reference: &str) -> Result<bool> {
    let Some(profile) = profile()? else {
        return Ok(false);
    };
    let path = profile.join("fault.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let fault: Fault = serde_json::from_slice(&bytes)?;
    if fault.point != point || !matches_reference(&fault.reference, reference) {
        return Ok(false);
    }
    ensure!(
        uuid::Uuid::parse_str(&fault.id).is_ok(),
        "Invalid test checkpoint"
    );
    let marker = profile.join(format!("hit-{}.json", fault.id));
    if marker.exists() {
        return Ok(false);
    }
    std::fs::write(
        &marker,
        serde_json::to_vec(&serde_json::json!({"point":point,"action":fault.action}))?,
    )?;
    match fault.action.as_str() {
        "fail" => anyhow::bail!(
            "Simulated storage access failure; reconnect and retry. Saved work is preserved."
        ),
        "corrupt" => Ok(true),
        "pause" => {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            anyhow::bail!("Synthetic checkpoint timed out")
        }
        _ => anyhow::bail!("Unknown synthetic fault action"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_faults_do_not_hit_latest_maintenance() {
        assert!(matches_reference(
            "History/*/43.gis",
            "History/publication/43.gis"
        ));
        assert!(!matches_reference("History/*/43.gis", "Latest/43.gis"));
        assert!(!matches_reference(
            "History/*/43.gis",
            "History/publication/42.gis"
        ));
    }
    #[test]
    fn checkpoints_require_a_marked_run_and_a_strict_child_profile() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let child = root.join("client");
        std::fs::create_dir_all(&child).unwrap();
        assert!(confined(&root, &child).is_err());
        std::fs::write(root.join("reliability-run.json"), b"{}").unwrap();
        assert!(confined(&root, &child).is_ok());
        assert!(confined(&root, &root).is_err());
        assert!(confined(&child, &root).is_err());
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(resolved).unwrap();
    }
}

pub fn install_panic_hook() {
    if let Ok(Some(root)) = profile() {
        std::panic::set_hook(Box::new(move |info| {
            let _ = std::fs::write(root.join("panic.txt"), info.to_string());
        }));
    }
}
