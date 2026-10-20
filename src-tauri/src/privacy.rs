//! The project key: creating, storing and restoring it, and encrypting folder-access
//! requests with it. The key itself never leaves the device.
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use anyhow::{ensure, Context, Result};
use miliastra_core::{
    coordination::{AccessRequest, SealedAccess, ACCESS_REQUEST_LIFETIME},
    Invitation,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode(s: &str) -> Result<Vec<u8>> {
    ensure!(
        s.len() % 2 == 0 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid Encryption Key or encrypted request"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
        .collect()
}
fn key_bytes(key: &str) -> Result<Zeroizing<Vec<u8>>> {
    let bytes = Zeroizing::new(decode(key.trim())?);
    ensure!(
        bytes.len() == 32,
        "Paste the full 64-character Encryption Key from the owner"
    );
    Ok(bytes)
}
pub fn key_id(key: &str) -> Result<String> {
    Ok(miliastra_core::hash(&key_bytes(key)?))
}
fn binding(endpoint: &str, project: &str) -> Result<String> {
    uuid::Uuid::parse_str(project)?;
    let ep = crate::runtime::endpoint(endpoint)?;
    Ok(format!(
        "privacy-v2:{}",
        miliastra_core::hash(&serde_json::to_vec(&(ep, project))?)
    ))
}
fn aad(
    endpoint: &str,
    project: &str,
    invite_hash: &str,
    id: &str,
    key_id: &str,
) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        "mcollab-folder-access",
        1,
        crate::runtime::endpoint(endpoint)?,
        project,
        invite_hash,
        id,
        key_id,
    ))?)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    pub email: String,
    pub name: String,
    at: i64,
}
impl Drop for Contact {
    fn drop(&mut self) {
        self.email.zeroize();
        self.name.zeroize();
    }
}
pub fn seal(
    key: &str,
    invitation: &Invitation,
    id: &str,
    email: &str,
    name: &str,
    at: i64,
) -> Result<SealedAccess> {
    ensure!(
        crate::access::valid_email(email),
        "Your storage account has no usable email address"
    );
    ensure!(
        !name.trim().is_empty()
            && miliastra_core::constants::text_fits(
                name,
                miliastra_core::constants::MAX_NAME_UNITS
            ),
        "Enter your display name"
    );
    let key = key_bytes(key)?;
    let key_id = miliastra_core::hash(&key);
    let clear = Zeroizing::new(serde_json::to_vec(&Contact {
        email: email.trim().to_lowercase(),
        name: name.trim().into(),
        at,
    })?);
    ensure!(clear.len() <= 1022, "Folder access request is too large");
    let mut padded = Zeroizing::new(vec![0u8; 1024]);
    padded[..2].copy_from_slice(&(clear.len() as u16).to_be_bytes());
    padded[2..2 + clear.len()].copy_from_slice(&clear);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("Invalid Encryption Key"))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let context = aad(
        &invitation.endpoint,
        &invitation.project,
        &miliastra_core::hash(invitation.invite.as_bytes()),
        id,
        &key_id,
    )?;
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &padded,
                aad: &context,
            },
        )
        .map_err(|_| anyhow::anyhow!("Could not encrypt the folder access request"))?;
    Ok(SealedAccess {
        version: 1,
        key_id,
        nonce: encode(&nonce),
        ciphertext: encode(&ciphertext),
    })
}
pub fn open(
    key: &str,
    endpoint: &str,
    project: &str,
    request: &AccessRequest,
    now: i64,
) -> Result<Contact> {
    request.sealed.validate()?;
    let key = key_bytes(key)?;
    ensure!(
        miliastra_core::hash(&key) == request.sealed.key_id,
        "privacy_key"
    );
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("privacy_key"))?;
    let nonce = decode(&request.sealed.nonce)?;
    let ciphertext = decode(&request.sealed.ciphertext)?;
    let context = aad(
        endpoint,
        project,
        &request.invite,
        &request.id,
        &request.sealed.key_id,
    )?;
    let clear = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &context,
                },
            )
            .map_err(|_| anyhow::anyhow!("privacy_key"))?,
    );
    ensure!(clear.len() == 1024, "Invalid encrypted request");
    let len = u16::from_be_bytes([clear[0], clear[1]]) as usize;
    ensure!(
        len <= 1022 && clear[2 + len..].iter().all(|b| *b == 0),
        "Invalid encrypted request"
    );
    let contact: Contact = serde_json::from_slice(&clear[2..2 + len])?;
    ensure!(
        contact.at <= now + 300 && now - contact.at < ACCESS_REQUEST_LIFETIME,
        "Expired encrypted request"
    );
    ensure!(
        crate::access::valid_email(&contact.email)
            && !contact.name.trim().is_empty()
            && miliastra_core::constants::text_fits(
                &contact.name,
                miliastra_core::constants::MAX_NAME_UNITS
            ),
        "Invalid encrypted request"
    );
    Ok(contact)
}
impl crate::runtime::Core {
    pub fn privacy_keys(&self, ep: &str, project: &str) -> Result<crate::private_metadata::Keys> {
        let clear = Zeroizing::new(crate::credentials::get(&binding(ep, project)?)?);
        let keys: crate::private_metadata::Keys =
            serde_json::from_str(&clear).context("Restore the project's Encryption Key")?;
        keys.validate()?;
        Ok(keys)
    }
    pub fn store_privacy_keys(
        &self,
        ep: &str,
        project: &str,
        keys: &crate::private_metadata::Keys,
    ) -> Result<()> {
        keys.validate()?;
        let clear = Zeroizing::new(serde_json::to_string(keys)?);
        crate::credentials::set(&binding(ep, project)?, &clear)
    }
    pub fn save_privacy_key(&self, ep: &str, project: &str, value: &str) -> Result<()> {
        let mut incoming = crate::private_metadata::Keys::import(value)?;
        if let Ok(old) = self.privacy_keys(ep, project) {
            let known = self
                .db
                .get::<Vec<crate::runtime::LocalProject>>("connections")?
                .unwrap_or_default()
                .iter()
                .any(|p| {
                    p.project.id == project
                        && p.endpoint == crate::runtime::endpoint(ep).unwrap_or_default()
                });
            ensure!(
                old.same_project(&incoming) || !known,
                "This key belongs to a different project"
            );
            if old.same_project(&incoming) {
                incoming.merge_history(&old);
            }
        }
        self.store_privacy_keys(ep, project, &incoming)
    }
    pub fn read_privacy_key(&self, ep: &str, project: &str) -> Result<String> {
        Ok(self.privacy_keys(ep, project)?.current_key()?.to_owned())
    }
    pub fn initialize_created_privacy_key(&self, ep: &str, project: &str) -> Result<()> {
        match self.privacy_keys(ep, project) {
            Ok(_) => Ok(()),
            Err(error) => {
                ensure!(
                    error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound),
                    "Restore your team's Encryption Key before resuming project setup"
                );
                self.store_privacy_keys(ep, project, &crate::private_metadata::Keys::new())
            }
        }
    }
    pub fn owner_privacy_key(&self) -> Result<String> {
        let p = self.active()?;
        self.privacy_keys(&p.endpoint, &p.project.id)?.export()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn generate() -> String {
        encode(&Aes256Gcm::generate_key(OsRng))
    }
    fn invitation() -> Invitation {
        Invitation {
            version: 1,
            endpoint: "https://collabs.example.com".into(),
            project: uuid::Uuid::new_v4().to_string(),
            invite: uuid::Uuid::new_v4().to_string(),
        }
    }
    #[test]
    fn access_names_use_the_same_unicode_limit_as_the_ui() {
        let invitation = invitation();
        let key = generate();
        let id = uuid::Uuid::new_v4().to_string();
        for (name, valid) in [
            ("名".repeat(100), true),
            ("😀".repeat(50), true),
            ("名".repeat(101), false),
            ("😀".repeat(51), false),
        ] {
            let result = seal(&key, &invitation, &id, "sam@example.com", &name, 100);
            assert_eq!(result.is_ok(), valid);
            if let Ok(sealed) = result {
                let request = AccessRequest {
                    id: id.clone(),
                    invite: miliastra_core::hash(invitation.invite.as_bytes()),
                    payload_hash: miliastra_core::hash(&serde_json::to_vec(&sealed).unwrap()),
                    sealed,
                    at: 100,
                    status: "pending".into(),
                    message: String::new(),
                };
                assert_eq!(
                    open(
                        &key,
                        &invitation.endpoint,
                        &invitation.project,
                        &request,
                        101
                    )
                    .unwrap()
                    .name,
                    name
                );
            }
        }
    }
    #[test]
    fn encrypted_access_hides_contacts_and_rejects_tampering_wrong_keys_and_replays() {
        let i = invitation();
        let key = generate();
        let id = uuid::Uuid::new_v4().to_string();
        let sealed = seal(&key, &i, &id, "sam@example.com", "Sam", 100).unwrap();
        let mut request = AccessRequest {
            id: id.clone(),
            invite: miliastra_core::hash(i.invite.as_bytes()),
            sealed: sealed.clone(),
            payload_hash: miliastra_core::hash(&serde_json::to_vec(&sealed).unwrap()),
            at: 100,
            status: "pending".into(),
            message: String::new(),
        };
        let wire = serde_json::to_string(&request).unwrap();
        assert!(!wire.contains("sam@example.com") && !wire.contains("Sam") && !wire.contains(&key));
        assert_eq!(
            open(&key, &i.endpoint, &i.project, &request, 101)
                .unwrap()
                .email,
            "sam@example.com"
        );
        assert!(open(&generate(), &i.endpoint, &i.project, &request, 101).is_err());
        assert!(open(&key, "https://other.example.com", &i.project, &request, 101).is_err());
        assert!(open(
            &key,
            &i.endpoint,
            &uuid::Uuid::new_v4().to_string(),
            &request,
            101
        )
        .is_err());
        assert!(open(
            &key,
            &i.endpoint,
            &i.project,
            &request,
            100 + ACCESS_REQUEST_LIFETIME
        )
        .is_err());
        assert!(open(&key, &i.endpoint, &i.project, &request, -201).is_err());
        request.id = uuid::Uuid::new_v4().to_string();
        assert!(open(&key, &i.endpoint, &i.project, &request, 101).is_err());
        request.id = id;
        request.invite = "0".repeat(64);
        assert!(open(&key, &i.endpoint, &i.project, &request, 101).is_err());
        request.invite = miliastra_core::hash(i.invite.as_bytes());
        request.sealed.ciphertext.replace_range(
            0..2,
            if &sealed.ciphertext[..2] == "00" {
                "01"
            } else {
                "00"
            },
        );
        assert!(open(&key, &i.endpoint, &i.project, &request, 101).is_err());
        let another = seal(
            &key,
            &i,
            &request.id,
            "longer@example.com",
            "Another creator",
            100,
        )
        .unwrap();
        assert_eq!(another.ciphertext.len(), sealed.ciphertext.len());
        assert_ne!(another.nonce, sealed.nonce);
        assert!(key_bytes("password").is_err());
    }
}
