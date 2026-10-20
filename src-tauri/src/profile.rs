//! Test profiles: extra app instances acting as different people on one Windows account.
//!
//! ```text
//! miliastra-collab.exe --profile B
//! ```
//!
//! Each profile has its own data folder, sign-ins and memberships, and skips the
//! single-instance lock. Only in test builds (debug, or `MCOLLAB_TEST_PROFILES=1`).
use anyhow::{ensure, Result};
use std::sync::OnceLock;

static PROFILE: OnceLock<Option<String>> = OnceLock::new();

pub const ALLOWED: bool = cfg!(test_build);

fn parse(args: impl Iterator<Item = String>) -> Result<Option<String>> {
    let args: Vec<String> = args.collect();
    let Some(i) = args.iter().position(|a| a == "--profile") else {
        return Ok(None);
    };
    ensure!(ALLOWED, "Test profiles are not available in this build");
    let name = args
        .get(i + 1)
        .ok_or_else(|| anyhow::anyhow!("--profile needs a name, for example --profile B"))?;
    ensure!(
        !name.is_empty()
            && name.len() <= 32
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Profile names use letters, numbers, - and _ (at most 32)"
    );
    Ok(Some(name.clone()))
}

/// Reads `--profile` from the command line at startup.
pub fn init() -> Result<()> {
    let profile = parse(std::env::args())?;
    let _ = PROFILE.set(profile);
    Ok(())
}

/// The active test profile, if this instance was started with `--profile`.
pub fn current() -> Option<&'static str> {
    PROFILE.get().and_then(|p| p.as_deref())
}

/// Credential Manager name, separate per profile.
pub fn credential_service() -> String {
    match current() {
        Some(name) => format!("miliastra/collab#profile-{name}"),
        None => "miliastra/collab".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }
    #[test]
    fn profiles_are_opt_in_and_names_are_safe_folder_names() {
        assert_eq!(parse(args(&["app"])).unwrap(), None);
        assert_eq!(
            parse(args(&["app", "--profile", "B"])).unwrap().as_deref(),
            Some("B")
        );
        for bad in ["", "../x", "a b", "C:", &"x".repeat(33)] {
            assert!(parse(args(&["app", "--profile", bad])).is_err(), "{bad}");
        }
        assert!(parse(args(&["app", "--profile"])).is_err());
    }
}
