//! Storage sign-in tokens, encrypted with Windows DPAPI for the current user.
use anyhow::{ensure, Context, Result};
use std::{io::Write, path::PathBuf, sync::OnceLock};

static ROOT: OnceLock<PathBuf> = OnceLock::new();
const HEADER: &[u8] = b"MCCRED01";
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub fn initialize(root: PathBuf) -> Result<()> {
    if let Some(existing) = ROOT.get() {
        ensure!(
            existing == &root,
            "Credential storage is already initialized for another profile"
        );
        return Ok(());
    }
    ROOT.set(root)
        .map_err(|_| anyhow::anyhow!("Credential storage initialization failed"))
}
fn vault() -> Result<Vault> {
    Ok(Vault {
        root: ROOT
            .get()
            .context("Credential storage is not initialized")?
            .clone(),
    })
}
pub fn get(key: &str) -> Result<String> {
    vault()?.read(key)
}
pub fn set(key: &str, value: &str) -> Result<()> {
    vault()?.write(key, value)
}
pub fn delete(key: &str) -> Result<()> {
    vault()?.delete(key)
}

struct Vault {
    root: PathBuf,
}
impl Vault {
    fn path(&self, key: &str) -> Result<PathBuf> {
        ensure!(
            crate::providers::is_provider_credential(key)
                || key
                    .strip_prefix("privacy-v2:")
                    .is_some_and(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())),
            "Unknown provider credential"
        );
        Ok(self
            .root
            .join(format!("{}.dpapi", miliastra_core::hash(key.as_bytes()))))
    }
    fn read(&self, key: &str) -> Result<String> {
        let path = self.path(key)?;
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(error.into()),
            Err(error) => return Err(error.into()),
            Ok(meta) => ensure!(
                meta.is_file() && meta.len() <= MAX_FILE_BYTES,
                "Saved sign-in file is invalid. Reconnect your storage account."
            ),
        }
        let bytes = std::fs::read(path)?;
        ensure!(
            bytes.starts_with(HEADER),
            "Saved sign-in format is invalid. Reconnect your storage account."
        );
        let clear = transform(&bytes[HEADER.len()..], key, false)
            .context("Saved sign-in could not be decrypted. Reconnect your storage account.")?;
        String::from_utf8(clear)
            .context("Saved sign-in is invalid. Reconnect your storage account.")
    }
    fn write(&self, key: &str, value: &str) -> Result<()> {
        let path = self.path(key)?;
        ensure!(
            value.len() <= 1024 * 1024,
            "Sign-in response is too large to store"
        );
        let protected = transform(value.as_bytes(), key, true)
            .context("Unable to encrypt sign-in credentials")?;
        std::fs::create_dir_all(&self.root)?;
        let temp = self.root.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(HEADER)?;
            file.write_all(&protected)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temp, &path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result.context("Unable to save encrypted sign-in credentials")
    }
    fn delete(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(windows)]
fn transform(bytes: &[u8], key: &str, encrypt: bool) -> Result<Vec<u8>> {
    use windows::{
        core::w,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::Cryptography::{
                CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        },
    };
    struct Buffer(CRYPT_INTEGER_BLOB);
    impl Drop for Buffer {
        fn drop(&mut self) {
            unsafe {
                if !self.0.pbData.is_null() {
                    let _ = LocalFree(Some(HLOCAL(self.0.pbData.cast())));
                }
            }
        }
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into()?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let binding = format!("dev.miliastra.collab|{key}|v1");
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: binding.len().try_into()?,
        pbData: binding.as_ptr() as *mut u8,
    };
    let mut output = Buffer(CRYPT_INTEGER_BLOB::default());
    unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                w!("miliastra/collab sign-in"),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )?;
        } else {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.0,
            )?;
        }
        ensure!(
            !output.0.pbData.is_null() && output.0.cbData > 0,
            "Windows credential encryption returned no data"
        );
        Ok(std::slice::from_raw_parts(output.0.pbData, output.0.cbData as usize).to_vec())
    }
}
#[cfg(not(windows))]
fn transform(_: &[u8], _: &str, _: bool) -> Result<Vec<u8>> {
    anyhow::bail!("Provider credential storage requires Windows")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    struct Temp(Vault);
    impl Temp {
        fn new() -> Self {
            Self(Vault {
                root: std::env::temp_dir()
                    .join(format!("miliastra-credentials-{}", uuid::Uuid::new_v4())),
            })
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            if let Ok(path) = self.0.root.canonicalize() {
                assert!(path.starts_with(std::env::temp_dir().canonicalize().unwrap()));
                assert!(path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("miliastra-credentials-"));
                std::fs::remove_dir_all(path).unwrap();
            }
        }
    }
    #[test]
    fn project_privacy_keys_are_dpapi_protected_and_bound_to_the_project() {
        let temp = Temp::new();
        let a = format!("privacy-v2:{}", "a".repeat(64));
        let b = format!("privacy-v2:{}", "b".repeat(64));
        let secret = "test-privacy-key";
        temp.0.write(&a, secret).unwrap();
        assert_eq!(temp.0.read(&a).unwrap(), secret);
        let bytes = std::fs::read(temp.0.path(&a).unwrap()).unwrap();
        assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
        std::fs::copy(temp.0.path(&a).unwrap(), temp.0.path(&b).unwrap()).unwrap();
        assert!(temp.0.read(&b).is_err());
        assert!(temp.0.write("privacy-v2:../../outside", secret).is_err());
    }
    #[test]
    fn large_unicode_token_bundle_roundtrips_as_ciphertext_and_replaces_atomically() {
        let temp = Temp::new();
        let vault = &temp.0;
        let value = serde_json::json!({"access_token": "fake-access-token".repeat(600), "refresh_token": "fake-refresh-東京-🛶".repeat(600), "expires_at": 9999999999i64}).to_string();
        assert!(value.encode_utf16().count() * 2 > 2560);
        vault.write("oauth:onedrive", &value).unwrap();
        let path = vault.path("oauth:onedrive").unwrap();
        let encrypted = std::fs::read(&path).unwrap();
        assert!(encrypted.starts_with(HEADER));
        assert!(!encrypted
            .windows(b"fake-access-token".len())
            .any(|w| w == b"fake-access-token"));
        assert!(!encrypted
            .windows(b"fake-refresh".len())
            .any(|w| w == b"fake-refresh"));
        let reopened = Vault {
            root: vault.root.clone(),
        };
        assert_eq!(reopened.read("oauth:onedrive").unwrap(), value);
        reopened
            .write("oauth:onedrive", "replacement-token-bundle")
            .unwrap();
        assert_eq!(
            reopened.read("oauth:onedrive").unwrap(),
            "replacement-token-bundle"
        );
        assert_eq!(
            std::fs::read_dir(&vault.root).unwrap().count(),
            1,
            "No temporary token files left"
        );
        reopened.delete("oauth:onedrive").unwrap();
        reopened.delete("oauth:onedrive").unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn missing_sign_in_requires_connection() {
        let temp = Temp::new();
        let error = temp.0.read("oauth:google").unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(!temp.0.root.exists());
    }
    #[test]
    fn corrupt_and_wrong_provider_files_fail_without_falling_back_to_stale_credentials() {
        let temp = Temp::new();
        let vault = &temp.0;
        vault.write("oauth:google", "private-test-token").unwrap();
        let google = vault.path("oauth:google").unwrap();
        let onedrive = vault.path("oauth:onedrive").unwrap();
        std::fs::copy(&google, &onedrive).unwrap();
        assert!(vault.read("oauth:onedrive").is_err());
        let mut bytes = std::fs::read(&google).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&google, bytes).unwrap();
        assert!(vault.read("oauth:google").is_err());
        vault.write("oauth:google", "reconnected").unwrap();
        assert_eq!(vault.read("oauth:google").unwrap(), "reconnected");
        std::fs::write(&google, b"unknown-format").unwrap();
        assert!(vault.read("oauth:google").is_err());
    }
    #[test]
    fn failed_replacement_preserves_previous_sign_in_and_rejects_unknown_keys() {
        let temp = Temp::new();
        let vault = &temp.0;
        vault.write("oauth:onedrive", "previous-sign-in").unwrap();
        assert!(vault
            .write("oauth:onedrive", &"x".repeat(1024 * 1024 + 1))
            .is_err());
        assert_eq!(vault.read("oauth:onedrive").unwrap(), "previous-sign-in");
        // Without FILE_SHARE_DELETE, Windows blocks replacing the file while it's open.
        use std::os::windows::fs::OpenOptionsExt;
        let guard = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(windows::Win32::Storage::FileSystem::FILE_SHARE_READ.0)
            .open(vault.path("oauth:onedrive").unwrap())
            .unwrap();
        assert!(vault.write("oauth:onedrive", "must-not-replace").is_err());
        drop(guard);
        assert_eq!(vault.read("oauth:onedrive").unwrap(), "previous-sign-in");
        assert!(vault.write("../outside", "secret").is_err());
        assert_eq!(std::fs::read_dir(&vault.root).unwrap().count(), 1);
    }
}
