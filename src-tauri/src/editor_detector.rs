//! What the game is doing with the stage, and change detection. No network access.
use anyhow::{ensure, Result};
use miliastra_core::{
    binary::{self, Stage},
    Revision,
};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Mutex,
};
const SAVE_SETTLE_MILLIS: u64 = 750;
const MAX_COMPARISON_CACHE_ENTRIES: usize = 4096;

type MatchKey = (PathBuf, String, String);
type MatchStamp = (u64, Option<std::time::SystemTime>, bool);
/// Missing reference bytes are an unknown comparison, not evidence of an edit.
pub(crate) struct SectionStatus {
    pub changed: bool,
    pub remote_changed: bool,
    pub comparison_pending: bool,
}
#[derive(Default)]
pub struct EditorDetector {
    cache: Mutex<HashMap<MatchKey, (MatchStamp, bool)>>,
}
/// True when Miliastra can't be using this stage right now.
pub fn stage_released(stage: &std::path::Path) -> bool {
    let game = crate::native::game_process_ids();
    if game.is_empty() {
        return true;
    }
    player_file_shows_unloaded(stage, crate::native::game_started_at(&game))
}
/// The player file is rewritten when a stage is loaded or saved (not when it's unloaded)
/// and records the last loaded stage. So while the game runs, this stage is definitely not
/// loaded if the file hasn't changed since the game started, or another stage was loaded
/// last. Otherwise it might be loaded.
fn player_file_shows_unloaded(
    stage: &std::path::Path,
    game_started: Option<std::time::SystemTime>,
) -> bool {
    let Some(player) = stage
        .parent()
        .and_then(std::path::Path::parent)
        .map(|account| account.join("Beyond_Local_Save_Player.gip"))
    else {
        return false;
    };
    let Ok(modified) = std::fs::metadata(&player).and_then(|m| m.modified()) else {
        return false;
    };
    // Nothing loaded since launch; allow a second for timestamp rounding.
    if game_started.is_some_and(|start| modified + std::time::Duration::from_secs(1) < start) {
        return true;
    }
    let Some(level) = stage.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    std::fs::read(&player)
        .ok()
        .and_then(|b| miliastra_core::binary::last_loaded_level(&b).ok().flatten())
        .is_some_and(|last| last != level)
}
impl EditorDetector {
    pub(crate) fn section_status(
        &self,
        stage: &Stage,
        baseline: Option<&Revision>,
        head: Option<&Revision>,
        cache: &std::path::Path,
        installation_pending: bool,
    ) -> SectionStatus {
        let remote_changed = head.is_some_and(|h| baseline.is_none_or(|b| b.hash != h.hash));
        if installation_pending {
            return SectionStatus {
                changed: false,
                remote_changed,
                comparison_pending: false,
            };
        }
        // A verified incoming version may already be present locally, before the
        // downloader has persisted its installed-revision reconciliation.
        if head.is_some_and(|r| self.matches(stage, r, cache)) {
            return SectionStatus {
                changed: false,
                remote_changed: false,
                comparison_pending: false,
            };
        }
        let baseline_matches = baseline.is_some_and(|r| self.matches(stage, r, cache));
        let comparison_pending = !baseline_matches
            && baseline
                .or(head)
                .is_some_and(|r| !cache.join(&r.hash).is_file());
        SectionStatus {
            changed: !comparison_pending && !baseline_matches,
            remote_changed,
            comparison_pending,
        }
    }
    pub fn editing_active(&self) -> bool {
        #[cfg(debug_assertions)]
        if crate::reliability::editor_running() {
            return true;
        }
        crate::native::game_state().running
    }

    pub(crate) async fn stable(&self, stage: &Stage) -> Result<BTreeMap<String, Vec<u8>>> {
        let read = || -> Result<BTreeMap<String, Vec<u8>>> {
            let mut files = BTreeMap::new();
            let mut total = 0u64;
            for id in stage
                .sections
                .iter()
                .map(|s| s.id.clone())
                .chain(std::iter::once("shared".into()))
            {
                let p = binary::resource_path(stage, &id)?;
                let size = std::fs::metadata(&p)?.len();
                total += size;
                ensure!(
                    size <= miliastra_core::constants::MAX_ARTIFACT_BYTES
                        && total <= miliastra_core::constants::MAX_STAGE_BYTES,
                    "Stage snapshot exceeds the supported size"
                );
                let b = std::fs::read(p)?;
                // Imported files keep the sender's bytes, including their level ID.
                let level = binary::resource_level(&b, &id)?;
                binary::content_hash(&b, &id, &level)?;
                files.insert(id, b);
            }
            Ok(files)
        };
        let a = read()?;
        tokio::time::sleep(std::time::Duration::from_millis(SAVE_SETTLE_MILLIS)).await;
        let b = read()?;
        ensure!(
            a == b,
            "Miliastra is still saving. Wait a moment, then try again."
        );
        Ok(b)
    }
    pub fn matches(&self, stage: &Stage, revision: &Revision, cache: &std::path::Path) -> bool {
        let objects = cache.to_path_buf();
        let Ok(path) = binary::resource_path(stage, &revision.section) else {
            return false;
        };
        let stamp = std::fs::metadata(&path).ok().map(|m| {
            (
                m.len(),
                m.modified().ok(),
                objects.join(&revision.hash).exists(),
            )
        });
        let key = (path, revision.hash.clone(), revision.content_hash.clone());
        if let Some(stamp) = &stamp {
            if let Some((cached, result)) = self.cache.lock().unwrap().get(&key) {
                if cached == stamp {
                    return *result;
                }
            }
        }
        let result = crate::installer::matches_file(stage, revision, &objects).unwrap_or(false);
        if let Some(stamp) = stamp {
            let mut cache = self.cache.lock().unwrap();
            if cache.len() > MAX_COMPARISON_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(key, (stamp, result));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vi(mut value: u64) -> Vec<u8> {
        let mut out = vec![];
        while value >= 128 {
            out.push(value as u8 | 128);
            value >>= 7;
        }
        out.push(value as u8);
        out
    }
    fn number(field: u64, value: u64) -> Vec<u8> {
        [vi(field << 3), vi(value)].concat()
    }
    fn nested(field: u64, bytes: &[u8]) -> Vec<u8> {
        [vi(field << 3 | 2), vi(bytes.len() as u64), bytes.to_vec()].concat()
    }
    fn framed(kind: u32, bytes: &[u8]) -> Vec<u8> {
        let mut out = vec![];
        for n in [bytes.len() as u32 + 20, 1, 806, kind, bytes.len() as u32] {
            out.extend(n.to_be_bytes());
        }
        out.extend(bytes);
        out.extend(1657u32.to_be_bytes());
        out
    }
    struct Fixture {
        stage: Stage,
        slice: Vec<u8>,
        shared: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("collab-imported-snapshot-{}", uuid::Uuid::new_v4()))
                .join("501");
            std::fs::create_dir_all(path.join("beyond_level_slice")).unwrap();
            let stage = Stage {
                path,
                level: "501".into(),
                account: "test".into(),
                name: "Test".into(),
                sections: vec![binary::Section {
                    id: "5".into(),
                    name: "Boats".into(),
                }],
                last_saved_section: Some("5".into()),
                settings_hash: String::new(),
                settings_version: String::new(),
                editor_version: "7.1.0".into(),
            };
            let slice = framed(6, &[number(1, 5), nested(2, &number(1, 601))].concat());
            let shared = framed(
                7,
                &[number(1, 701), nested(3, b"imported shared settings")].concat(),
            );
            std::fs::write(binary::resource_path(&stage, "5").unwrap(), &slice).unwrap();
            std::fs::write(binary::resource_path(&stage, "shared").unwrap(), &shared).unwrap();
            Self {
                stage,
                slice,
                shared,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(self.stage.path.parent().unwrap()).unwrap();
        }
    }
    #[tokio::test]
    async fn imported_shared_settings_can_be_snapshotted_without_rewriting_their_identity() {
        let fixture = Fixture::new();
        let files = EditorDetector::default()
            .stable(&fixture.stage)
            .await
            .unwrap();
        assert_eq!(files["5"], fixture.slice);
        assert_eq!(files["shared"], fixture.shared);
        assert_eq!(
            binary::resource_level(&files["shared"], "shared").unwrap(),
            "701"
        );
        let fingerprints = binary::fingerprints(&fixture.stage).unwrap();
        assert_eq!(fingerprints.len(), 2);
        assert_eq!(
            fingerprints["shared"],
            binary::content_hash(&fixture.shared, "shared", "701").unwrap()
        );
        assert_eq!(
            std::fs::read(binary::resource_path(&fixture.stage, "shared").unwrap()).unwrap(),
            fixture.shared
        );
    }
    #[tokio::test]
    async fn accepting_donor_levels_keeps_section_and_framing_validation() {
        let fixture = Fixture::new();
        let path = binary::resource_path(&fixture.stage, "5").unwrap();
        std::fs::write(
            &path,
            framed(6, &[number(1, 6), nested(2, &number(1, 601))].concat()),
        )
        .unwrap();
        assert!(EditorDetector::default()
            .stable(&fixture.stage)
            .await
            .unwrap_err()
            .to_string()
            .contains("Slice section mismatch"));
        std::fs::write(&path, b"invalid file").unwrap();
        assert!(EditorDetector::default()
            .stable(&fixture.stage)
            .await
            .is_err());
    }
    fn reference(bytes: &[u8], section: &str) -> Revision {
        let level = binary::resource_level(bytes, section).unwrap();
        Revision {
            section: section.into(),
            name: "Fixture".into(),
            hash: miliastra_core::hash(bytes),
            content_hash: binary::content_hash(bytes, section, &level).unwrap(),
            parent: None,
            artifact: "History/fixture.gis".into(),
            size: bytes.len() as u64,
            shared_dependency: None,
        }
    }
    #[test]
    fn cold_join_waits_for_reference_then_recognizes_imported_content() {
        let fixture = Fixture::new();
        let cache = fixture.stage.path.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let source = framed(6, &[number(1, 5), nested(2, &number(1, 900))].concat());
        let revision = reference(&source, "5");
        let detector = EditorDetector::default();
        let before = detector.section_status(&fixture.stage, None, Some(&revision), &cache, false);
        assert!(before.comparison_pending);
        assert!(!before.changed);
        std::fs::write(cache.join(&revision.hash), &source).unwrap();
        let after = detector.section_status(&fixture.stage, None, Some(&revision), &cache, false);
        assert!(!after.comparison_pending);
        assert!(!after.changed);
        assert!(!after.remote_changed);
    }
    #[test]
    fn edits_made_while_reference_downloads_become_confirmed_conflicts() {
        let fixture = Fixture::new();
        let cache = fixture.stage.path.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let source = framed(6, &[number(1, 5), nested(2, &number(1, 900))].concat());
        let revision = reference(&source, "5");
        let local = framed(
            6,
            &[
                number(1, 5),
                nested(2, &[number(1, 601), nested(3, b"local edit")].concat()),
            ]
            .concat(),
        );
        std::fs::write(binary::resource_path(&fixture.stage, "5").unwrap(), local).unwrap();
        let detector = EditorDetector::default();
        assert!(
            detector
                .section_status(&fixture.stage, None, Some(&revision), &cache, false)
                .comparison_pending
        );
        std::fs::write(cache.join(&revision.hash), source).unwrap();
        let after = detector.section_status(&fixture.stage, None, Some(&revision), &cache, false);
        assert!(!after.comparison_pending);
        assert!(after.changed && after.remote_changed);
    }
    #[test]
    fn existing_local_baseline_keeps_real_changes_visible_while_updates_download() {
        let fixture = Fixture::new();
        let cache = fixture.stage.path.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let baseline = reference(&fixture.slice, "5");
        std::fs::write(cache.join(&baseline.hash), &fixture.slice).unwrap();
        let incoming = framed(
            6,
            &[
                number(1, 5),
                nested(2, &[number(1, 900), nested(3, b"remote edit")].concat()),
            ]
            .concat(),
        );
        let head = reference(&incoming, "5");
        let detector = EditorDetector::default();
        let clean =
            detector.section_status(&fixture.stage, Some(&baseline), Some(&head), &cache, false);
        assert!(!clean.comparison_pending && !clean.changed && clean.remote_changed);
        let local = framed(
            6,
            &[
                number(1, 5),
                nested(2, &[number(1, 601), nested(3, b"local edit")].concat()),
            ]
            .concat(),
        );
        std::fs::write(binary::resource_path(&fixture.stage, "5").unwrap(), local).unwrap();
        let dirty =
            detector.section_status(&fixture.stage, Some(&baseline), Some(&head), &cache, false);
        assert!(!dirty.comparison_pending && dirty.changed && dirty.remote_changed);
    }
    #[test]
    fn shared_settings_and_reload_verification_use_the_same_pending_rules() {
        let fixture = Fixture::new();
        let cache = fixture.stage.path.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let source = framed(
            7,
            &[number(1, 900), nested(3, b"imported shared settings")].concat(),
        );
        let revision = reference(&source, "shared");
        let detector = EditorDetector::default();
        assert!(
            detector
                .section_status(&fixture.stage, None, Some(&revision), &cache, false)
                .comparison_pending
        );
        std::fs::write(cache.join(&revision.hash), source).unwrap();
        let ready = detector.section_status(&fixture.stage, None, Some(&revision), &cache, false);
        assert!(!ready.comparison_pending && !ready.changed && !ready.remote_changed);
        let reload = detector.section_status(&fixture.stage, None, Some(&revision), &cache, true);
        assert!(!reload.comparison_pending && !reload.changed && reload.remote_changed);
    }
    #[test]
    fn exact_content_hash_can_establish_a_match_without_cached_bytes() {
        let fixture = Fixture::new();
        let revision = reference(&fixture.slice, "5");
        let status = EditorDetector::default().section_status(
            &fixture.stage,
            None,
            Some(&revision),
            &fixture.stage.path.join("cache"),
            false,
        );
        assert!(!status.comparison_pending && !status.changed && !status.remote_changed);
    }
}
