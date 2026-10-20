//! The app's own data types and limits: names, changelogs, storage locations and file
//! sizes. The coordinator never handles these, so its copy of the core leaves this file out.
use crate::{Invitation, Presence};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// App-only limits, reachable as `miliastra_core::constants::*` (see lib.rs). Text limits
/// count UTF-16 code units, like HTML maxLength.
pub mod limits {
    pub const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
    pub const MAX_STAGE_BYTES: u64 = 256 * 1024 * 1024;
    pub const LEASE_RENEW_INTERVAL_SECS: u64 = 20;
    pub const LEASE_RENEW_TIMEOUT_SECS: u64 = 8;
    pub const LEASE_RENEW_CONCURRENCY: usize = 16;
    pub const PREVIEW_TTL_SECS: i64 = 600;
    pub const MAX_SUMMARY_UNITS: usize = 500;
    pub const MAX_DETAILS_UNITS: usize = 10_000;
    pub const MAX_NAME_UNITS: usize = 100;
    pub const MAX_SLICE_NAME_UNITS: usize = 256;
    pub fn text_fits(value: &str, limit: usize) -> bool {
        value.encode_utf16().count() <= limit
    }
    pub fn validate_changelog(summary: &str, details: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            !summary.trim().is_empty() && text_fits(summary, MAX_SUMMARY_UNITS),
            "Write what changed (at most {MAX_SUMMARY_UNITS} characters)"
        );
        anyhow::ensure!(
            text_fits(details, MAX_DETAILS_UNITS),
            "Details must be at most {MAX_DETAILS_UNITS} characters"
        );
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::constants::{LEASE_TTL_SECS, MAX_PROJECT_RESOURCES};
        #[test]
        fn unicode_limits_match_html() {
            assert!(validate_changelog(&"島".repeat(500), &"島".repeat(10_000)).is_ok());
            assert!(validate_changelog(&"🚤".repeat(250), "").is_ok());
            assert!(validate_changelog(&"島".repeat(501), "").is_err());
            assert!(validate_changelog(&"🚤".repeat(251), "").is_err());
            assert!(validate_changelog(" ", "").is_err());
            assert!(validate_changelog("Changed", &"🚤".repeat(5001)).is_err());
        }
        #[test]
        fn renewals_have_time_to_finish_before_expiry() {
            let batches = MAX_PROJECT_RESOURCES.div_ceil(LEASE_RENEW_CONCURRENCY) as u64;
            assert!(
                batches * LEASE_RENEW_TIMEOUT_SECS + LEASE_RENEW_INTERVAL_SECS
                    < LEASE_TTL_SECS as u64
            );
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageConfig {
    pub kind: String,
    pub location: String,
    #[serde(default)]
    pub drive_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Revision {
    pub section: String,
    pub name: String,
    pub hash: String,
    pub content_hash: String,
    pub parent: Option<String>,
    pub artifact: String,
    pub size: u64,
    #[serde(default)]
    pub shared_dependency: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishRequest {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub details: String,
    pub revisions: Vec<Revision>,
    pub fences: BTreeMap<String, u64>,
    #[serde(default)]
    pub restores: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NameChange {
    pub section: String,
    pub before: String,
    pub after: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishNames {
    pub id: String,
    pub parent_version: u64,
    pub summary: String,
    #[serde(default)]
    pub details: String,
    pub changes: Vec<NameChange>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub id: String,
    pub author: String,
    pub author_id: String,
    pub summary: String,
    pub details: String,
    pub at: i64,
    pub sequence: u64,
    pub revisions: Vec<Revision>,
    pub restores: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_changes: Vec<NameChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names_parent: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub section: String,
    pub member: String,
    pub name: String,
    pub fence: u64,
    pub expires: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub name: String,
    pub owner: bool,
    /// Unix time the member joined; 0 for members from before this was recorded.
    #[serde(default)]
    pub joined: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub storage: StorageConfig,
    pub members: Vec<Member>,
    pub locks: Vec<Lease>,
    pub heads: BTreeMap<String, Revision>,
    pub sequence: u64,
    #[serde(default)]
    pub section_names: BTreeMap<String, String>,
    #[serde(default)]
    pub names_version: u64,
    /// Members connected right now. Added by the coordinator, never stored.
    #[serde(default)]
    pub presence: Vec<Presence>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProject {
    pub id: String,
    pub member_id: String,
    pub summary: String,
    pub details: String,
    pub name: String,
    pub display_name: String,
    pub storage: StorageConfig,
    pub baseline: Vec<Revision>,
    pub baseline_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Joined {
    pub project: ProjectView,
    pub member: String,
    pub token: String,
    #[serde(default)]
    pub invitation: Option<Invitation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub entries: Vec<Update>,
    pub next: Option<u64>,
}
pub fn valid_section(s: &str) -> bool {
    s == "shared"
        || s.parse::<u8>()
            .is_ok_and(|n| n < crate::constants::SLICE_COUNT && n.to_string() == s)
}
pub fn validate_revision(r: &Revision) -> anyhow::Result<()> {
    anyhow::ensure!(valid_section(&r.section), "Unknown section");
    anyhow::ensure!(
        !r.name.trim().is_empty()
            && crate::constants::text_fits(&r.name, crate::constants::MAX_SLICE_NAME_UNITS),
        "Invalid section name"
    );
    for h in [&r.hash, &r.content_hash] {
        anyhow::ensure!(
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid checksum"
        );
    }
    anyhow::ensure!(
        r.size > 0 && r.size <= crate::constants::MAX_ARTIFACT_BYTES,
        "Invalid artifact size"
    );
    anyhow::ensure!(
        r.artifact.rsplit_once('/').is_some_and(|(folder, file)| {
            folder
                .strip_prefix("History/")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id))
                && slice_filename(&r.section).is_ok_and(|expected| file == expected)
        }),
        "Invalid artifact reference"
    );
    Ok(())
}

pub fn validate_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.trim().is_empty()
            && crate::constants::text_fits(name, crate::constants::MAX_SLICE_NAME_UNITS)
            && !name.chars().any(char::is_control),
        "Enter a section name of at most 256 characters without control characters"
    );
    Ok(())
}

/// File name in team storage, numbered from 1 (section IDs start at 0).
pub fn slice_filename(section: &str) -> anyhow::Result<String> {
    anyhow::ensure!(valid_section(section), "Unknown slice");
    Ok(if section == "shared" {
        "shared.gilh".into()
    } else {
        format!("{}.gis", section.parse::<u8>()? + 1)
    })
}
pub fn history_artifact(publication: &str, section: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        uuid::Uuid::parse_str(publication)?.to_string() == publication,
        "Invalid publication ID"
    );
    Ok(format!(
        "History/{publication}/{}",
        slice_filename(section)?
    ))
}

#[cfg(test)]
mod artifact_tests {
    use super::*;
    #[test]
    fn history_references_use_stable_one_based_numbers_only() {
        let id = uuid::Uuid::new_v4().to_string();
        let mut r = Revision {
            section: "5".into(),
            name: "Boats".into(),
            hash: "a".repeat(64),
            content_hash: "b".repeat(64),
            parent: None,
            artifact: history_artifact(&id, "5").unwrap(),
            size: 1,
            shared_dependency: None,
        };
        assert_eq!(r.artifact, format!("History/{id}/6.gis"));
        validate_revision(&r).unwrap();
        assert_eq!(slice_filename("0").unwrap(), "1.gis");
        assert_eq!(slice_filename("63").unwrap(), "64.gis");
        assert_eq!(slice_filename("shared").unwrap(), "shared.gilh");
        for reference in [
            format!("objects/{}", r.hash),
            "Latest/6.gis".into(),
            format!("History/{id}/5.gis"),
            "History/../6.gis".into(),
            format!("History/{id}/nested/6.gis"),
        ] {
            r.artifact = reference;
            assert!(validate_revision(&r).is_err());
        }
    }
}
