use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
/// Where tests/coordinator.cjs runs the test coordinator by default.
#[cfg(any(test, test_build))]
pub const TEST_SERVICE_URL: &str = "http://127.0.0.1:8787";
/// The hosted coordinator. `MCOLLAB_SERVICE_URL` at build time overrides it.
pub const HOSTED_SERVICE_URL: &str = "https://collabs.miliastra.dev";
pub const FILE_NAME: &str = "coordination.json";
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    service_url: String,
}
pub fn load(path: &Path) -> Result<String> {
    ensure!(
        fs::metadata(path)?.len() <= 16 * 1024,
        "Coordination settings file is too large"
    );
    let config: Config = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("Invalid coordination settings in {}", path.display()))?;
    crate::runtime::endpoint(config.service_url.trim())
}
pub fn save(path: &Path, service_url: &str) -> Result<()> {
    let service_url = crate::runtime::endpoint(service_url.trim())?;
    let mut bytes = serde_json::to_vec_pretty(&Config { service_url })?;
    bytes.push(b'\n');
    let temp = path.with_file_name(format!(".coordination-{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path).with_context(|| format!("Could not save {}", path.display()))?;
    Ok(())
}
/// True for addresses on this computer.
pub fn is_loopback(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| match u.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        None => false,
    })
}
pub fn load_or_create(path: &Path, default_url: &str) -> Result<String> {
    if !path.exists() {
        save(path, default_url)?;
    }
    load(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_install_uses_hosted_service_and_preserves_an_explicit_override() {
        let path = path();
        let defaults = crate::runtime::Settings::default();
        assert_eq!(defaults.endpoint, HOSTED_SERVICE_URL);
        assert_eq!(
            load_or_create(&path, &defaults.endpoint).unwrap(),
            HOSTED_SERVICE_URL
        );
        save(&path, "https://self-hosted.example").unwrap();
        assert_eq!(
            load_or_create(&path, &defaults.endpoint).unwrap(),
            "https://self-hosted.example"
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn loopback_addresses_are_recognized() {
        assert!(is_loopback(TEST_SERVICE_URL));
        assert!(is_loopback("http://localhost:9000"));
        assert!(is_loopback("http://[::1]:8787"));
        assert!(!is_loopback(HOSTED_SERVICE_URL));
        assert!(!is_loopback("not a url"));
    }
    fn path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("coordination-{}.json", uuid::Uuid::new_v4()))
    }
    #[test]
    fn creates_readable_local_file_and_preserves_an_edit() {
        let path = path();
        assert_eq!(
            load_or_create(&path, TEST_SERVICE_URL).unwrap(),
            TEST_SERVICE_URL
        );
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert_eq!(value["serviceUrl"], TEST_SERVICE_URL);
        save(&path, "https://collabs.miliastra.dev/").unwrap();
        assert_eq!(
            load_or_create(&path, TEST_SERVICE_URL).unwrap(),
            "https://collabs.miliastra.dev"
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn keeps_a_saved_loopback_address() {
        let path = path();
        assert_eq!(
            load_or_create(&path, "http://127.0.0.1:63846").unwrap(),
            "http://127.0.0.1:63846"
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_bad_json_unknown_fields_and_unsafe_urls_without_overwriting() {
        let path = path();
        for bytes in [
            br#"broken"#.as_slice(),
            br#"{"serviceURL":"http://127.0.0.1:8787"}"#,
            br#"{"serviceUrl":"http://external.example"}"#,
            br#"{"serviceUrl":"https://user:password@example.com"}"#,
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(load_or_create(&path, TEST_SERVICE_URL).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_file(path).unwrap();
    }
}
